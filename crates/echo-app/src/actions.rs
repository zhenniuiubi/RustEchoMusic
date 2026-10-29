//! Lite 版的动作层：登录（扫码/登录态刷新）、私人 FM、歌词、自建歌单。
//!
//! 原来这些都是 `views.rs`（主窗口页面）里的函数，Lite 只保留播放条之后，
//! 把还需要的逻辑摘出来放这里，页面渲染部分整体删掉。

use crate::state::AppStore;
use crate::ui::run_tokio;
use std::sync::Arc;
use gpui::prelude::*;
use gpui::*;

// ==================== 小部件 ====================

/// 用 canvas 直接绘制二维码矩阵（避免生成图片文件 / 依赖图床）。
pub(crate) fn qr_widget(content: String, size_px: f32, fg: Hsla) -> impl IntoElement {
    let code = qrcode::QrCode::new(content.as_bytes()).ok();
    let (n, cells) = match code {
        Some(c) => (c.width(), c.to_colors()),
        None => (0usize, Vec::new()),
    };
    canvas(
        move |bounds, _, _| bounds,
        move |bounds, _state, window, _| {
            if n == 0 {
                return;
            }
            let quiet = 2usize;
            let total = (n + quiet * 2) as f32;
            let cell = f32::from(bounds.size.width) / total;
            for y in 0..n {
                for x in 0..n {
                    if cells[y * n + x] == qrcode::types::Color::Dark {
                        let origin = point(
                            bounds.origin.x + px(cell * (x + quiet) as f32),
                            bounds.origin.y + px(cell * (y + quiet) as f32),
                        );
                        window.paint_quad(fill(Bounds::new(origin, size(px(cell), px(cell))), fg));
                    }
                }
            }
        },
    )
    .size(px(size_px))
}

/// 表单输入框外框（点击整框即可聚焦并输入）。
fn apply_login(store: &Entity<AppStore>, login: echo_api::LoginResult, cx: &mut App) {
    let need_claim = login.vip_type == 0;
    store.update(cx, |s, cx| {
        s.session.set(echo_core::Session {
            token: login.token.clone(),
            userid: login.userid,
            // t1 / expires 必须落盘：少了 t1，下次 `/v5/login_by_token` 续不上，
            // token 到期就只能重新登录（「昨天登录今天过期」就是这个）
            t1: login.t1.clone(),
            expires: login.expires,
            nickname: login.nickname.clone(),
            pic: login.pic.clone(),
            vip_type: login.vip_type,
            vip_end_time: 0,
            logged_in: true,
        });
        s.client.set_auth(&login.token, login.userid);
        s.client.set_t1(&login.t1);
        s.user = Some(echo_core::User {
            userid: login.userid,
            token: login.token,
            nickname: login.nickname.clone(),
            username: login.nickname.clone(),
            pic: login.pic,
            vip_type: login.vip_type,
        });
        s.login.error.clear();
        s.toast = "登录成功".into();
        s.session_expired = false;
        s.vip_note.clear();
        cx.notify();
    });
    // 登录后：拉取用户详情（含真实 vip_type）+ 远端红心歌单
    refresh_user_detail(store, cx);
    crate::liked::load_remote_liked(store, true, cx);
    // 登录的账号不是 VIP：顺手把今天的畅听 VIP 领了，否则 VIP 歌曲直接听不了
    if need_claim {
        crate::vip::claim_now(store, cx);
    }
}

/// 拉取用户详情（昵称 / 头像 / VIP 等级）。
pub(crate) fn refresh_user_detail(store: &Entity<AppStore>, cx: &mut App) {
    let store = store.clone();
    if store.read(cx).session.get().userid == 0 {
        return;
    }
    let client = store.read(cx).client.clone();
    let task = cx.background_executor().spawn(async move {
        run_tokio(async move {
            let detail = client.user_detail().await;
            // 到期时间只有会员接口给（user_detail 只有等级）
            let vip = crate::vip::fetch_vip(&client).await;
            (detail, vip)
        })
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (res, vip) = task.await;
        let _ = store.update(cx, |s, cx| {
            match res {
                Ok(u) => {
                    if !u.nickname.is_empty() {
                        s.session.update_nickname(&u.nickname);
                    }
                    if !u.pic.is_empty() {
                        s.session.update_pic(&u.pic);
                    }
                    s.session.update_vip(u.vip_type, vip.end_time);
                    s.user = Some(u);
                    s.session_expired = false;
                    s.vip_note = crate::vip::vip_note_text(Some(&vip));
                }
                // 20018：token 已被服务端判过期 —— 自动退出登录
                Err(e) if echo_api::is_auth_expired(&e) => {
                    log::warn!("user_detail: 登录态已过期 {e}");
                    crate::actions::force_logout(s, "登录已过期，请重新登录");
                }
                Err(_) => {}
            }
            cx.notify();
        });
    })
    .detach();
}

/// 发起扫码登录：取二维码 key，然后开始轮询。
pub(crate) fn start_qr_login(store: &Entity<AppStore>, cx: &mut App) {
    let store = store.clone();
    let qr_ver = store.update(cx, |s, cx| {
        s.login.qr_gen += 1;
        s.login.qr_loading = true;
        s.login.qr_status = 1;
        s.login.qr_key.clear();
        s.login.qr_url.clear();
        s.login.error.clear();
        cx.notify();
        s.login.qr_gen
    });

    let client = store.read(cx).client.clone();
    let task = cx
        .background_executor()
        .spawn(async move { run_tokio(client.qr_key()) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        let go_on = store
            .update(cx, |s, cx| {
                cx.notify();
                // 已被新一轮会话取代
                if s.login.qr_gen != qr_ver {
                    return false;
                }
                s.login.qr_loading = false;
                match res {
                    Ok((key, url)) => {
                        s.login.qr_key = key;
                        s.login.qr_url = url;
                        s.login.qr_status = 1;
                        true
                    }
                    Err(e) => {
                        s.login.error = format!("二维码获取失败：{e}");
                        s.login.qr_status = 0;
                        false
                    }
                }
            })
            .unwrap_or(false);
        if go_on {
            poll_qr_login(&store, qr_ver, cx);
        }
    })
    .detach();
}

/// 轮询扫码状态（3 秒一次，成功后自动登录）。
fn poll_qr_login(store: &Entity<AppStore>, qr_ver: u64, cx: &mut AsyncApp) {
    let store = store.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            let (key, still) = match store.read_with(cx, |s, _| (s.login.qr_key.clone(), s.login.qr_gen == qr_ver)) {
                Ok(v) => v,
                Err(_) => break,
            };
            if !still || key.is_empty() {
                break;
            }
            let client = store.read_with(cx, |s, _| s.client.clone()).ok();
            let Some(client) = client else { break };
            let task = cx
                .background_executor()
                .spawn(async move { run_tokio(client.qr_check(&key)) });

            let mut done = false;
            let mut logged: Option<echo_api::LoginResult> = None;
            let res = task.await;
            let _ = store.update(cx, |s, cx| {
                cx.notify();
                if s.login.qr_gen != qr_ver {
                    done = true;
                    return;
                }
                match res {
                    Ok(state) => {
                        s.login.qr_status = state.status;
                        if state.status == 4 && !state.token.is_empty() {
                            logged = Some(echo_api::LoginResult {
                                token: state.token,
                                userid: state.userid,
                                t1: state.t1,
                                expires: 0,
                                nickname: state.nickname,
                                pic: state.pic,
                                vip_type: state.vip_type,
                            });
                            done = true;
                        } else if state.status == 0 {
                            done = true;
                        }
                    }
                    Err(e) => {
                        s.login.error = format!("扫码状态获取失败：{e}");
                        s.login.qr_status = 0;
                        done = true;
                    }
                }
            });
            if let Some(login) = logged {
                let _ = cx.update(|app| apply_login(&store, login, app));
                break;
            }
            if done {
                break;
            }
            smol::Timer::after(std::time::Duration::from_secs(3)).await;
        }
    })
    .detach();
}

/// 发送短信验证码。
pub(crate) fn logout(store: &Entity<AppStore>, cx: &mut App) {
    store.update(cx, |s, cx| {
        clear_login_state(s);
        s.session_expired = false;
        s.vip_note.clear();
        s.toast = "已退出登录".into();
        cx.notify();
    });
}

/// 清掉所有登录相关状态（含远端红心缓存与 VIP 信息）。
fn clear_login_state(s: &mut AppStore) {
    s.session.clear();
    s.user = None;
    s.client.set_auth("", 0);
    s.login.error.clear();
    s.vip_claiming = false;
    // 清空远端红心缓存（下次登录重新拉取）
    s.remote_liked = Arc::new(Vec::new());
    s.remote_liked_keys.clear();
    s.remote_liked_fileids.clear();
    s.remote_liked_loaded = false;
    s.remote_liked_listid.clear();
}

/// 登录态失效（服务端 20018 等）：**自动退出登录**，账号面板会自己变成未登录。
///
/// 用户看到的应该是「已经登出、请重新登录」，而不是一堆接口报错。
pub(crate) fn force_logout(s: &mut AppStore, msg: &str) {
    log::warn!("登录态失效，自动退出登录：{msg}");
    clear_login_state(s);
    s.session_expired = true;
    s.vip_note = msg.to_string();
    s.toast = msg.to_string();
}

pub(crate) fn note_fm_heard(store: &Entity<AppStore>, song: &echo_core::Song, cx: &mut App) {
    let _ = store.update(cx, |s, _| {
        if !s.player.fm_queue || s.fm_batch.is_empty() {
            return false;
        }
        // 队列里插了别的歌（非 FM）时不算数
        if !s.fm_batch.iter().any(|h| h == &song.hash) {
            return false;
        }
        s.fm_heard.insert(song.hash.clone());
        s.fm_batch.iter().all(|h| s.fm_heard.contains(h))
    });
}

/// 私人 FM 这一轮的歌是不是都听过了（切歌时据此决定「清空队列换下一轮」）。
pub(crate) fn fm_round_finished(s: &AppStore) -> bool {
    s.player.fm_queue
        && !s.fm_batch.is_empty()
        && s.fm_batch.iter().all(|h| s.fm_heard.contains(h))
}

/// 私人 FM 换一轮：带着上一轮的 `cur_mark` 再要一批，**清空队列换成新一轮**，
/// 从新批第一首开始播。
///
/// 接口语义是「一批一批推」：`cur_mark` 不带回去服务端会认为刚推过、直接给空列表
/// （见 `api.rs::personal_fm`），所以游标必须一路存下来（`AppStore::fm_cur_mark`）。
/// 返回的批次可能和已播的重叠，这里按 hash 去重；整批都是重复的话就用新的游标再要
/// 一次，最多 3 轮。
pub(crate) fn fm_extend(store: &Entity<AppStore>, cx: &mut App) {
    let (client, mode, pool, mut mark, seen) = {
        let s = store.read(cx);
        if s.fm_extending {
            return;
        }
        // 用当前队列的 hash 去重：新一轮要「不一样的歌」，同一批再拿一次没意义
        let seen: std::collections::HashSet<String> =
            s.player.queue.iter().map(|x| x.hash.clone()).collect();
        (s.client.clone(), s.fm_mode.clone(), s.fm_pool, s.fm_cur_mark, seen)
    };
    store.update(cx, |s, cx| {
        s.fm_extending = true;
        s.fm_loading = true;
        cx.notify();
    });

    let store_c = store.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let mut picked: Vec<echo_core::Song> = Vec::new();
        for round in 0..3 {
            let client_c = client.clone();
            let mode_c = mode.clone();
            let res = cx
                .background_executor()
                .spawn(async move {
                    crate::ui::run_tokio(client_c.personal_fm(0, mark, "login", &mode_c, pool))
                })
                .await;
            match res {
                Ok(batch) => {
                    mark = batch.cur_mark;
                    let total = batch.songs.len();
                    let fresh: Vec<echo_core::Song> = batch
                        .songs
                        .into_iter()
                        .filter(|x| {
                            !seen.contains(&x.hash) && !picked.iter().any(|p| p.hash == x.hash)
                        })
                        .collect();
                    log::info!(
                        "fm extend round {round}: 收到 {} 首，去重后剩 {} 首（cur_mark={mark}）",
                        total,
                        fresh.len()
                    );
                    if !fresh.is_empty() {
                        picked = fresh;
                        break;
                    }
                }
                Err(e) => {
                    log::warn!("fm extend ERR {e}");
                    break;
                }
            }
        }

        // 清空旧队列，换成新一轮，再从第一首开始播
        let ok = store_c.update(cx, |s, cx| {
            s.fm_extending = false;
            s.fm_loading = false;
            s.fm_cur_mark = mark;
            if picked.is_empty() {
                s.player.playing = false;
                s.player.loading = false;
                s.toast = "私人FM 暂时没有更多推荐".into();
                cx.notify();
                return false;
            }
            let added = picked.len();
            // 新一批成为「本批」：已播记录清空，等下一轮再判断
            s.fm_batch = picked.iter().map(|x| x.hash.clone()).collect();
            s.fm_heard.clear();
            s.fm = Arc::new(picked.clone());
            // 关键在于「清空」而不是像以前那样挂到队尾：队列不会越滚越长
            s.player.set_queue("私人FM", picked, 0);
            s.player.fm_queue = true;
            s.player.position = 0.0;
            s.player.loading = true;
            s.player.request_id += 1;
            s.persist_queue();
            log::info!("fm: 换一轮，{added} 首，队列已重置");
            cx.notify();
            true
        });
        if let Ok(true) = ok {
            let _ = cx.update(|app| crate::ui::play_index(&store_c, 0, app));
        }
    })
    .detach();
}

/// 加载私人 FM（`/v2/personal_recommend`）。
pub(crate) fn load_fm(store: &Entity<AppStore>, force: bool, cx: &mut App) {
    let store_c = store.clone();
    {
        let s = store.read(cx);
        if !force && (s.fm_loaded || s.fm_loading) {
            return;
        }
    }
    store.update(cx, |s, cx| {
        s.fm_loading = true;
        cx.notify();
    });
    let client = store.read(cx).client.clone();
    // 用 action=login（开一个新的 FM 会话）：
    // action=play 需要带上「正在播的那首歌」的 hash/songid/cur_mark 才能续推，
    // 不带上下文再要一次会直接返回空列表（实测）。
    let mode = store.read(cx).fm_mode.clone();
    let pool = store.read(cx).fm_pool;
    let task = cx.background_executor().spawn(async move {
        run_tokio(client.personal_fm(0, 0, "login", &mode, pool))
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        let _ = store_c.update(cx, |s, cx| {
            s.fm_loading = false;
            s.fm_loaded = true;
            match res {
                Ok(batch) if !batch.songs.is_empty() => {
                    log::info!("fm: {} 首（cur_mark={}）", batch.songs.len(), batch.cur_mark);
                    s.fm_cur_mark = batch.cur_mark;
                    s.fm = Arc::new(batch.songs);
                }
                Ok(_) => {
                    log::warn!("fm: 接口没返回歌曲");
                    s.toast = "私人FM 暂时没有推荐".into();
                }
                Err(e) => {
                    log::warn!("fm ERR {e}");
                    s.toast = format!("私人FM 加载失败：{e}");
                }
            }
            cx.notify();
        });
    })
    .detach();
}


// ==================== 私人FM 文案 ====================

// ==================== 歌词 ====================

/// 加载当前曲目的歌词（切歌时调用）。
pub(crate) fn load_lyric(store: &Entity<AppStore>, cx: &mut App) {
    let Some(song) = store.read(cx).player.current().cloned() else {
        store.update(cx, |s, cx| {
            s.lyric = None;
            s.lyric_key.clear();
            cx.notify();
        });
        return;
    };
    let key = song.key();
    if store.read(cx).lyric_key == key && store.read(cx).lyric.is_some() {
        return;
    }
    store.update(cx, |s, cx| {
        s.lyric_key = key.clone();
        s.lyric = None;
        s.lyric_loading = true;
        cx.notify();
    });
    let store_c = store.clone();
    let client = store.read(cx).client.clone();
    let task = cx
        .background_executor()
        .spawn(async move { run_tokio(client.lyric(&song)) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        let _ = store_c.update(cx, |s, cx| {
            s.lyric_loading = false;
            // 只接受「当前曲目」的歌词，慢请求回来时可能已经切歌了
            if s.lyric_key != key_clone(&s.player) {
                cx.notify();
                return;
            }
            match res {
                Ok(lyric) => {
                    log::info!("lyric: {} 行（逐字={}）", lyric.lines.len(), lyric.word_time);
                    s.lyric = Some(lyric);
                }
                Err(e) => {
                    log::info!("lyric ERR {e}");
                    s.lyric = None;
                }
            }
            cx.notify();
        });
    })
    .detach();
}

fn key_clone(p: &crate::player::PlayerState) -> String {
    p.current().map(|s| s.key()).unwrap_or_default()
}

// ==================== MV ====================

