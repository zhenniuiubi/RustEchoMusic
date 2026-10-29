//! 每日畅听 VIP 自动领取 + 登录态保活 + 会员到期时间。
//!
//! 酷狗的「每日畅听 VIP」每天可领一次（叠加到现有会员上，实测领到的是 **超级VIP 1 天**）：
//! 当天没领过时调一次领取接口即可，VIP 歌曲随即能正常取播放地址。
//!
//! 接口来源（与参考项目一致）：
//! - `_ref/EchoMusic-main/src/renderer/api/user.ts`：`claimDayVip` / `upgradeDayVip` /
//!   `getVipMonthRecord` / `getUserVipDetail`
//! - `_kgapi/KuGouMusicApi-main/module/youth_day_vip*.js`
//!
//! 实测的错误码语义（2026-10）：
//! - `131001` 今天已经领过；
//! - `304003` 该日期不能领（不是今天）；
//! - `20017/20018` 登录态失效。
//!
//! 四条触发路径：
//! 1. **启动**（`bootstrap`）：刷新 token → 用户详情 + 会员到期时间 → 今天没领过就领。
//! 2. **播放被挡**（`claim_loop`，由 `ui::spawn_resolve` 兜底调用）：取不到地址且提示
//!    需要购买/VIP 时，先确认当前是否真的不是 VIP，再领一次并重试那首歌。
//! 3. **播放中到期**（`start_watchdog`）：每 60s 看一次，`vip_end_time` 过了、或今天还没领
//!    就自动领；跨零点后也能补领。
//! 4. **手动**（`claim_now`）：资料页「领取今日VIP」按钮，忽略「今天领过」的缓存。

use crate::state::AppStore;
use echo_api::{ClaimError, KuGouClient, VipStatus};
use gpui::{App, AsyncApp, Entity};
use std::sync::Arc;

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn now_ts() -> i64 {
    chrono::Local::now().timestamp()
}

/// 待尝试的 `receive_day`（日期串）。
///
/// 服务端按「日期」校验（`304003` 表示不是今天），时区/跨零点可能差一天，
/// 所以今天 → 明天 → 昨天 依次试，第一个成功的即成。
fn claim_dates() -> Vec<String> {
    let now = chrono::Local::now();
    let mut out = Vec::with_capacity(3);
    for offset in [0i64, 1, -1] {
        if let Some(d) = now.checked_add_signed(chrono::Duration::days(offset)) {
            let s = d.format("%Y-%m-%d").to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// 今天是否已经领到（含「今天已经领过」）。
fn claimed_today(s: &AppStore) -> bool {
    let st = s.settings.get();
    st.vip_claim_date == today() && st.vip_claim_ok
}

/// 自动领取是否放行：今天还没领到，且距上次尝试超过 `gap_secs`。
///
/// 失败也要退避（否则看门狗每分钟打一次接口），但不能像「今天已领过」那样
/// 锁死一整天 —— 失败可能只是网络抖了一下。
fn can_auto_claim(s: &AppStore, gap_secs: i64) -> bool {
    if claimed_today(s) {
        return false;
    }
    now_ts() - s.settings.get().vip_claim_try_ts >= gap_secs
}

/// 播放取地址失败时，是否值得先确认 VIP 状态并尝试领取。
///
/// 注意**不能**看 `session.vip_type`：那是上次拉到的快照，
/// 播放过程中会员过期时它仍然是旧值（正是「明明显示 VIP 却播不了」的来源）。
pub fn should_claim_for_playback(s: &AppStore) -> bool {
    let session = s.session.get();
    session.is_valid() && !s.session_expired && can_auto_claim(s, 300)
}

/// 取地址失败的错误是否属于「需要 VIP / 付费」这一类（只有这类才值得去领一天 VIP）。
pub fn is_vip_block(err: &anyhow::Error) -> bool {
    let t = err.to_string();
    t.contains("VIP") || t.contains("购买") || t.contains("付费") || t.contains("Paid")
}

/// 领取结果。
pub struct Outcome {
    /// 领取成功，或「今天已经领过」
    pub ok: bool,
    /// 是「今天已经领过」而不是刚领到
    pub already: bool,
    /// 命中的 `receive_day`（日期串）
    pub date: String,
    /// 失败原因（最后一次错误）
    pub err: String,
    /// 是否为登录态失效
    pub expired: bool,
    /// 领取后重新查到的会员状态
    pub vip: Option<VipStatus>,
}

/// 领一次每日 VIP（异步，需在 tokio 上下文里跑）。
pub async fn claim_loop(client: &KuGouClient) -> Outcome {
    let mut ok = false;
    let mut already = false;
    let mut date = String::new();
    let mut err = String::new();
    let mut expired = false;

    for d in claim_dates() {
        match client.claim_daily_vip(&d).await {
            Ok(v) => {
                log::info!("claim_daily_vip: date={d} OK {}", truncate(&v.to_string()));
                ok = true;
                date = d;
                break;
            }
            Err(e) => match echo_api::classify_claim_error(&e) {
                ClaimError::Already => {
                    log::info!("claim_daily_vip: {d} 今天已经领过");
                    ok = true;
                    already = true;
                    date = d;
                    break;
                }
                ClaimError::WrongDay => {
                    log::info!("claim_daily_vip: {d} 不是可领取日期，换一个");
                    continue;
                }
                ClaimError::AuthExpired => {
                    log::warn!("claim_daily_vip: 登录态失效 {e}");
                    err = e.to_string();
                    expired = true;
                    break;
                }
                ClaimError::Other => {
                    log::info!("claim_daily_vip: date={d} ERR {e}");
                    err = e.to_string();
                    continue;
                }
            },
        }
    }

    let mut vip = None;
    if ok {
        // 概念会员的每日升级奖励（失败不影响主流程）
        match client.claim_upgrade_reward().await {
            Ok(v) => log::info!("upgrade_reward OK {}", truncate(&v.to_string())),
            Err(e) => log::info!("upgrade_reward ERR {e}"),
        }
        let st = fetch_vip(client).await;
        log::info!("after claim: type={} end={}", st.vip_type, st.end_time);
        vip = Some(st);
    }

    Outcome { ok, already, date, err, expired, vip }
}

/// 拉会员状态（等级 + 到期时间 + 各产品线）。
///
/// 会员接口偶发失败时退回 `user_detail` 的等级（没有到期时间）。
pub async fn fetch_vip(client: &KuGouClient) -> VipStatus {
    let mut st = match client.vip_status().await {
        Ok(s) => s,
        Err(e) => {
            log::info!("vip_status ERR {e}");
            VipStatus::default()
        }
    };
    if st.vip_type == 0 {
        if let Ok(u) = client.user_detail().await {
            if u.vip_type > 0 {
                st.vip_type = u.vip_type;
            }
        }
    }
    st
}

/// 会员状态的展示文案（含精确到秒的到期时间）。
pub fn vip_note_text(vip: Option<&VipStatus>) -> String {
    match vip {
        Some(st) if st.vip_type > 0 => st.detail_text(),
        Some(st) if !st.products.is_empty() => st.detail_text(),
        _ => "非VIP（每日可领畅听VIP）".into(),
    }
}

/// 启动引导：刷新 token → 用户详情 + 会员状态 → 今天没领过就领一次。
///
/// 酷狗 token 会过期（过期后所有登录态接口返回 20018，表现是
/// 「界面看着还登录着，但歌单/VIP 歌曲全都拉不到」），所以启动时先换新 token。
pub fn bootstrap(store: &Entity<AppStore>, cx: &mut App) {
    let store = store.clone();
    let (client, valid, want_claim) = {
        let s = store.read(cx);
        (
            s.client.clone(),
            s.session.get().is_valid(),
            can_auto_claim(s, 3600),
        )
    };
    if !valid {
        return;
    }

    let task = cx
        .background_executor()
        .spawn(async move { crate::ui::run_tokio(run_bootstrap(client, want_claim)) });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let out = task.await;
        let date = today();
        let _ = store.update(cx, |s, cx| {
            if let Some(r) = out.refreshed.clone() {
                s.session.update(|x| {
                    x.token = r.token.clone();
                    if !r.t1.is_empty() {
                        x.t1 = r.t1.clone();
                    }
                    if r.expires > 0 {
                        x.expires = r.expires;
                    }
                });
                s.client.set_t1(&r.t1);
                s.token_refresh_at = now_ts();
            }
            if out.expired {
                // token 被服务端判过期：自动退出登录（见 `force_logout`）
                crate::actions::force_logout(s, "登录已过期，请重新登录");
                cx.notify();
                return;
            }
            s.session_expired = false;

            if let Some(u) = out.user.clone() {
                if !u.nickname.is_empty() {
                    s.session.update_nickname(&u.nickname);
                }
                if !u.pic.is_empty() {
                    s.session.update_pic(&u.pic);
                }
                s.user = Some(u);
            }
            if let Some(st) = out.vip.as_ref() {
                s.session.update_vip(st.vip_type, st.end_time);
            }

            if let Some(claim) = &out.claim {
                s.settings.update(|st| {
                    st.vip_claim_try_ts = now_ts();
                    st.vip_claim_date = date;
                    st.vip_claim_ok = claim.ok;
                });
                if let Some(st) = claim.vip.as_ref() {
                    s.session.update_vip(st.vip_type, st.end_time);
                }
                s.vip_note = vip_note_text(claim.vip.as_ref().or(out.vip.as_ref()));
                if claim.ok && !claim.already {
                    s.toast = "已自动领取今日VIP（超级VIP +1天）".into();
                }
            } else {
                s.vip_note = vip_note_text(out.vip.as_ref());
            }
            cx.notify();
        });
    })
    .detach();
}

/// 手动领取（资料页按钮）。
pub fn claim_now(store: &Entity<AppStore>, cx: &mut App) {
    claim(store, true, cx);
}

/// 播放中到期看门狗：每 60 秒看一次。
///
/// 两种情况会领：`vip_end_time` 已经过了（播放中会员失效），或今天还没领过
/// （含跨零点后补领）。
pub fn start_watchdog(store: &Entity<AppStore>, cx: &mut App) {
    let store = store.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            // 开局先等 30s，避免和启动引导抢着打接口
            smol::Timer::after(std::time::Duration::from_secs(30)).await;

            // ===== 登录态保活 =====
            // 只在启动时刷新是不够的：一直开着不关，token 到期就掉线。
            // 这里按「到期前 1 小时」或「距上次续期满 6 小时」续一次，
            // 续期成功后服务端会换一份新的 token + t1（必须一起落盘）。
            if store
                .read_with(cx, |s, _| token_refresh_due(s))
                .unwrap_or(false)
            {
                refresh_login(&store, cx).await;
            }

            let due = match store.read_with(cx, |s, _| {
                let session = s.session.get();
                if !session.is_valid() || s.session_expired || !can_auto_claim(s, 3600) {
                    return false;
                }
                let ended = session.vip_end_time > 0 && now_ts() >= session.vip_end_time;
                ended || session.vip_type == 0
            }) {
                Ok(v) => v,
                Err(_) => break,
            };
            if due {
                log::info!("VIP 看门狗：该领今天的了");
                let _ = cx.update(|app| claim(&store, false, app));
            }
        }
    })
    .detach();
}

/// 运行期续期的间隔：`/v5/login_by_token` 很轻，但也不宜过密（风控）。
const TOKEN_REFRESH_GAP: i64 = 6 * 3600;
/// 续期失败后的重试间隔（别太密：登录类接口调多了会被风控）。
const TOKEN_RETRY_GAP: i64 = 30 * 60;

/// 是否需要续期 token。
fn token_refresh_due(s: &AppStore) -> bool {
    let session = s.session.get();
    if !session.is_valid() || s.session_expired {
        return false;
    }
    let now = now_ts();
    // 服务端给的 `expires` 只有当它是**未来的时间戳**时才信：
    // 也可能是「有效时长」（几百万秒）或已经过去的值，那种按固定间隔续，
    // 否则会每 30 秒续一次（触发风控）。
    if session.expires > now {
        return now >= session.expires - 3600;
    }
    // 还没成功续过（比如刚补上 t1 的老会话）立刻续一次
    if s.token_refresh_at == 0 {
        return true;
    }
    now - s.token_refresh_at >= TOKEN_REFRESH_GAP
}

/// 续一次登录态：换新 token + 新 t1（+ 到期时间），失败不打断使用。
async fn refresh_login(store: &Entity<AppStore>, cx: &mut AsyncApp) {
    let client = match store.read_with(cx, |s, _| s.client.clone()) {
        Ok(c) => c,
        Err(_) => return,
    };
    let task = cx
        .background_executor()
        .spawn(async move { crate::ui::run_tokio(client.refresh_token()) });
    let res = task.await;
    let _ = store.update(cx, |s, cx| {
        let now = now_ts();
        match res {
            Ok(r) => {
                s.session.update(|x| {
                    x.token = r.token.clone();
                    if !r.t1.is_empty() {
                        x.t1 = r.t1.clone();
                    }
                    if r.expires > 0 {
                        x.expires = r.expires;
                    }
                });
                s.client.set_t1(&r.t1);
                s.token_refresh_at = now;
                log::info!("token 保活 OK expires={}", r.expires);
            }
            Err(e) if echo_api::is_auth_expired(&e) => {
                s.token_refresh_at = now;
                log::warn!("token 保活：登录态已失效 {e}");
                crate::actions::force_logout(s, "登录已过期，请重新登录");
            }
            Err(e) => {
                // 网络/风控之类的临时失败：半小时后再试，别把用户踢下线
                s.token_refresh_at = now - TOKEN_REFRESH_GAP + TOKEN_RETRY_GAP;
                log::warn!("token 保活失败（{TOKEN_RETRY_GAP}s 后重试）: {e}");
            }
        }
        cx.notify();
    });
}

fn claim(store: &Entity<AppStore>, force: bool, cx: &mut App) {
    let store = store.clone();
    log::info!("vip::claim 触发（force={force}）");
    let (client, valid, claiming) = {
        let s = store.read(cx);
        (s.client.clone(), s.session.get().is_valid(), s.vip_claiming)
    };
    if !valid {
        if force {
            store.update(cx, |s, cx| {
                s.toast = "请先登录后再领取VIP".into();
                cx.notify();
            });
        }
        return;
    }
    if claiming {
        return;
    }
    if !force && !can_auto_claim(store.read(cx), 300) {
        return;
    }

    store.update(cx, |s, cx| {
        s.vip_claiming = true;
        s.vip_note = "正在领取今日VIP…".into();
        cx.notify();
    });

    let date = today();
    let task = cx
        .background_executor()
        .spawn(async move { crate::ui::run_tokio(claim_loop(&client)) });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let claim = task.await;
        let _ = store.update(cx, |s, cx| {
            s.vip_claiming = false;
            s.settings.update(|st| {
                st.vip_claim_try_ts = now_ts();
                st.vip_claim_date = date;
                st.vip_claim_ok = claim.ok;
            });
            if claim.ok {
                if let Some(st) = claim.vip.as_ref() {
                    s.session.update_vip(st.vip_type, st.end_time);
                    if let Some(cur) = s.user.as_mut() {
                        cur.vip_type = st.vip_type;
                    }
                }
                s.session_expired = false;
                s.vip_note = vip_note_text(claim.vip.as_ref());
                s.toast = if claim.already {
                    "今天的VIP已经领过了".into()
                } else {
                    "今日VIP领取成功（超级VIP +1天）".into()
                };
            } else if claim.expired || s.session_expired {
                // 登录态已失效：直接退出登录（不是「领取失败」）
                crate::actions::force_logout(s, "登录已过期，请重新登录");
            } else {
                s.vip_note = if claim.err.is_empty() {
                    "今日VIP领取失败".into()
                } else {
                    format!("今日VIP领取失败：{}", claim.err)
                };
                if force {
                    s.toast = s.vip_note.clone();
                }
            }
            cx.notify();
        });
    })
    .detach();
}

// ==================== 后台流程 ====================

struct BootOutcome {
    /// 刷新 token 的完整结果（token / t1 / expires 都要回写，缺一样都续不上）
    refreshed: Option<echo_api::LoginResult>,
    user: Option<echo_core::User>,
    expired: bool,
    vip: Option<VipStatus>,
    claim: Option<Outcome>,
}

async fn run_bootstrap(client: Arc<KuGouClient>, want_claim: bool) -> BootOutcome {
    let mut out = BootOutcome {
        refreshed: None,
        user: None,
        expired: false,
        vip: None,
        claim: None,
    };

    // 续期失败**不直接判死**：20018/20017 也可能是风控/网关抽风，
    // 真正算数的是下面 `user_detail` 的结果（两个都说过期才退出登录）。
    let mut refresh_expired = false;
    match client.refresh_token().await {
        Ok(r) => {
            log::info!(
                "refresh_token OK userid={} vip_type={} t1={} expires={}",
                r.userid,
                r.vip_type,
                if r.t1.is_empty() { "无" } else { "有" },
                r.expires
            );
            out.refreshed = Some(r);
        }
        Err(e) => {
            if echo_api::is_auth_expired(&e) {
                // 带上原始错误码：20018 会话过期 / 20017 token 无效 / 20006 缺 token
                // 三者处理方式不同（20006 说明请求里根本没带上 token）
                log::warn!("refresh_token: 续期被拒（等 user_detail 复核）：{e}");
                refresh_expired = true;
            } else {
                log::warn!("refresh_token ERR {e}");
            }
        }
    }

    match client.user_detail().await {
        Ok(u) => {
            log::info!("user_detail vip_type={}", u.vip_type);
            if refresh_expired {
                // 续期失败但普通接口还能用：token 没过期，别把用户踢下线
                log::warn!("refresh_token 失败但 user_detail 正常 —— 保持登录态");
            }
            out.user = Some(u);
        }
        Err(e) => {
            if echo_api::is_auth_expired(&e) {
                log::warn!("user_detail: 登录态已过期（需要重新登录）");
                out.expired = true;
                return out;
            }
            log::warn!("user_detail ERR {e}");
        }
    }

    let st = fetch_vip(&client).await;
    log::info!(
        "vip_status: type={} end={} detail={}",
        st.vip_type,
        st.end_time,
        st.detail_text()
    );
    out.vip = Some(st);

    if want_claim {
        let claim = claim_loop(&client).await;
        log::info!(
            "bootstrap claim: ok={} already={} date={} err={}",
            claim.ok,
            claim.already,
            claim.date,
            claim.err
        );
        if claim.ok {
            if let Ok(u) = client.user_detail().await {
                out.user = Some(u);
            }
            if claim.vip.is_some() {
                out.vip = claim.vip.clone();
            }
        }
        out.claim = Some(claim);
    }
    out
}

fn truncate(s: &str) -> &str {
    let mut end = s.len().min(180);
    while end < s.len() && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
