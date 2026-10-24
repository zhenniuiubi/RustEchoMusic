//! 第二期页面：登录 / 探索 / 排行榜 / 每日推荐 / 歌手 / 专辑 / 榜单 / 设置 / 我的 / 收藏历史。
//!
//! 渲染入口由 `ui::Root::render_content` 分发；数据加载走 `ui::run_tokio`。

use crate::state::{AlbumView, ArtistView, AppStore, LoginMethod, Page};
use crate::text_input::TextInput;
use crate::theme::COVER_RADIUS;
use crate::theme::Theme;
use crate::ui::{centered_spinner, icon, play_from, run_tokio, PlaySource};
use std::sync::Arc;
use crate::Root;
use echo_core::Album;
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
fn input_shell(
    theme: &crate::theme::Theme,
    input: Entity<TextInput>,
    cx: &App,
) -> impl IntoElement + use<> {
    let handle = input.read(cx).focus();
    div()
        .h(px(40.))
        .w(px(312.))
        .px_3()
        .rounded(px(10.))
        .bg(theme.bg_hover)
        .border_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .cursor_text()
        .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, _| {
            window.focus(&handle);
        })
        .child(input)
}

/// 主按钮。
fn primary_button<F>(
    id: &'static str,
    label: impl Into<SharedString>,
    theme: &crate::theme::Theme,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let label: SharedString = label.into();
    div()
        .id(id)
        .h(px(40.))
        .w(px(312.))
        .rounded(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.accent)
        .text_color(gpui::white())
        .cursor_pointer()
        .hover(|s| s.opacity(0.88))
        .on_click(on_click)
        .child(label)
}

// ==================== 数据加载 ====================

/// 列表加载动画最短展示时长：加载太快时动画一闪而过，用户感知不到。
const MIN_SPINNER_MS: u64 = 500;

/// 确保加载动画至少展示 `MIN_SPINNER_MS`，在完成回调清 loading 前调用。
async fn min_spinner(start: std::time::Instant) {
    let min = std::time::Duration::from_millis(MIN_SPINNER_MS);
    let elapsed = start.elapsed();
    if elapsed < min {
        smol::Timer::after(min - elapsed).await;
    }
}

/// 探索页首屏（标签 + 推荐歌单 + 排行榜 + 新歌 + 新碟）。
pub(crate) fn load_explore(store: &Entity<AppStore>, cx: &mut App) {
    let store = store.clone();
    if store.read(cx).explore.loaded {
        return;
    }
    let t0 = std::time::Instant::now();
    store.update(cx, |s, cx| {
        s.explore.loaded = true;
        s.explore.loading = true;
        cx.notify();
    });
    let client = store.read(cx).client.clone();
    let (c2, c3, c4) = (client.clone(), client.clone(), client.clone());
    let task = cx.background_executor().spawn(async move {
        let tags = run_tokio(client.playlist_tags()).unwrap_or_default();
        let ranks = run_tokio(c2.rank_list()).unwrap_or_default();
        let songs = run_tokio(c3.top_songs(1, 30)).unwrap_or_default();
        let albums = run_tokio(c4.top_albums(1, 20)).unwrap_or_default();
        (tags, ranks, songs, albums)
    });
    let c5 = store.read(cx).client.clone();
    let pl_task = cx
        .background_executor()
        .spawn(async move { run_tokio(c5.recommend_playlists(0, 1, 24)) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (tags, ranks, songs, albums) = task.await;
        let playlists = pl_task.await.unwrap_or_default();
        min_spinner(t0).await;
        let _ = store.update(cx, |s, cx| {
            s.explore.tags = tags;
            s.explore.ranks = ranks;
            s.explore.new_songs = Arc::new(songs);
            s.explore.new_albums = albums;
            s.explore.playlists = playlists;
            s.explore.loading = false;
            cx.notify();
        });
    })
    .detach();
}

/// 按歌单标签刷新歌单列表。
pub(crate) fn load_explore_playlists(store: &Entity<AppStore>, tag_id: i64, cx: &mut App) {
    let store = store.clone();
    let client = store.read(cx).client.clone();
    let t0 = std::time::Instant::now();
    store.update(cx, |s, cx| {
        s.explore.loading = true;
        cx.notify();
    });
    let task = cx
        .background_executor()
        .spawn(async move { run_tokio(client.recommend_playlists(tag_id, 1, 24)) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let playlists = task.await.unwrap_or_default();
        min_spinner(t0).await;
        let _ = store.update(cx, |s, cx| {
            s.explore.playlists = playlists;
            s.explore.loading = false;
            cx.notify();
        });
    })
    .detach();
}

/// 每日推荐。
pub(crate) fn load_daily(store: &Entity<AppStore>, cx: &mut App) {
    let store = store.clone();
    if store.read(cx).daily_loaded {
        return;
    }
    let t0 = std::time::Instant::now();
    let client = store.read(cx).client.clone();
    store.update(cx, |s, cx| {
        s.daily_loaded = true;
        s.daily_loading = true;
        cx.notify();
    });
    let task = cx
        .background_executor()
        .spawn(async move { run_tokio(client.recommend_songs()) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let songs = task.await.unwrap_or_default();
        min_spinner(t0).await;
        let _ = store.update(cx, |s, cx| {
            s.daily_loading = false;
            s.daily = Arc::new(songs);
            cx.notify();
        });
    })
    .detach();
}

/// 榜单详情（左侧榜单列表 + 右侧歌曲）。
pub(crate) fn load_rank_detail(store: &Entity<AppStore>, rank_id: &str, cx: &mut App) {
    let store = store.clone();
    let client = store.read(cx).client.clone();
    let c2 = client.clone();
    let rid = rank_id.to_string();
    let t0 = std::time::Instant::now();
    store.update(cx, |s, cx| {
        s.rank_loading = true;
        s.rank_view.rank_id = rid.clone();
        s.rank_view.songs = Arc::new(Vec::new());
        cx.notify();
    });
    let task = cx.background_executor().spawn(async move {
        let ranks = run_tokio(c2.rank_list()).unwrap_or_default();
        // `/rank/audio` 要传 `rankid`（req_rank_id），传列表项 `id` 服务端只回 1 首
        let (req_id, cid) = ranks
            .iter()
            .find(|r| r.id == rid)
            .map(|r| {
                (
                    if r.req_rank_id.is_empty() {
                        rid.clone()
                    } else {
                        r.req_rank_id.clone()
                    },
                    r.rank_cid.parse::<i64>().ok().unwrap_or(0),
                )
            })
            .unwrap_or((rid.clone(), 0));
        let songs = run_tokio(client.rank_songs_with_cid(&req_id, cid, 1, 100)).unwrap_or_default();
        (ranks, songs)
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (ranks, songs) = task.await;
        min_spinner(t0).await;
        let _ = store.update(cx, |s, cx| {
            s.rank_loading = false;
            s.rank_view.ranks = ranks;
            s.rank_view.songs = Arc::new(songs);
            cx.notify();
        });
    })
    .detach();
}

/// 歌手详情。
pub(crate) fn load_artist(store: &Entity<AppStore>, artist_id: &str, cx: &mut App) {
    let store = store.clone();
    let client = store.read(cx).client.clone();
    let (c2, c3) = (client.clone(), client.clone());
    let aid = artist_id.to_string();
    let t0 = std::time::Instant::now();
    store.update(cx, |s, cx| {
        s.artist_loading = true;
        cx.notify();
    });
    let task = cx.background_executor().spawn(async move {
        let artist = run_tokio(c2.artist_detail(&aid)).unwrap_or_default();
        let songs = run_tokio(c3.artist_songs(&aid, 1, 50)).unwrap_or_default();
        (artist, songs)
    });
    let c4 = store.read(cx).client.clone();
    let aid2 = artist_id.to_string();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (artist, songs) = task.await;
        let albums = {
            let aid = aid2;
            cx.background_executor()
                .spawn(async move { run_tokio(c4.artist_albums(&aid)) })
                .await
                .unwrap_or_default()
        };
        min_spinner(t0).await;
        let _ = store.update(cx, |s, cx| {
            s.artist_loading = false;
            s.artist_view = Some(ArtistView { artist, songs: Arc::new(songs), albums });
            cx.notify();
        });
    })
    .detach();
}

/// 专辑详情。
pub(crate) fn load_album(store: &Entity<AppStore>, album_id: &str, cx: &mut App) {
    let store = store.clone();
    let client = store.read(cx).client.clone();
    let (c2, c3) = (client.clone(), client.clone());
    let aid = album_id.to_string();
    let t0 = std::time::Instant::now();
    store.update(cx, |s, cx| {
        s.album_loading = true;
        cx.notify();
    });
    let task = cx.background_executor().spawn(async move {
        let album = run_tokio(c2.album_detail(&aid)).ok().flatten();
        let songs = run_tokio(c3.album_songs_all(&aid));
        match &songs {
            Ok(v) => log::info!("[load_album] id={aid} detail_ok={} songs={}", album.is_some(), v.len()),
            Err(e) => log::warn!("[load_album] id={aid} songs ERR {e}"),
        }
        (album, songs.unwrap_or_default())
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (album, songs) = task.await;
        min_spinner(t0).await;
        let _ = store.update(cx, |s, cx| {
            s.album_loading = false;
            if let Some(album) = album {
                s.album_view = Some(AlbumView { album, songs: Arc::new(songs) });
            } else {
                s.toast = "专辑加载失败".into();
            }
            cx.notify();
        });
    })
    .detach();
}

/// 登录成功后写入会话 / 客户端登录态。
fn apply_login(store: &Entity<AppStore>, login: echo_api::LoginResult, cx: &mut App) {
    let need_claim = login.vip_type == 0;
    store.update(cx, |s, cx| {
        s.session.set(echo_core::Session {
            token: login.token.clone(),
            userid: login.userid,
            nickname: login.nickname.clone(),
            pic: login.pic.clone(),
            vip_type: login.vip_type,
            vip_end_time: 0,
            logged_in: true,
        });
        s.client.set_auth(&login.token, login.userid);
        s.user = Some(echo_core::User {
            userid: login.userid,
            token: login.token,
            nickname: login.nickname.clone(),
            username: login.nickname.clone(),
            pic: login.pic,
            vip_type: login.vip_type,
        });
        s.login.password.clear();
        s.login.code.clear();
        s.login.error.clear();
        s.login.busy = false;
        s.login.sms_busy = false;
        s.toast = "登录成功".into();
        s.session_expired = false;
        s.vip_note.clear();
        // 未登录时被拦下来的那个页面，登录后跳回去；否则停在个人中心
        let target = s.login_redirect.take().unwrap_or(Page::Profile);
        s.routes.navigate(target);
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
                    crate::views::force_logout(s, "登录已过期，请重新登录");
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
pub(crate) fn send_sms_code(store: &Entity<AppStore>, mobile: String, cx: &mut App) {
    let store = store.clone();
    if mobile.trim().len() < 11 {
        store.update(cx, |s, cx| {
            s.login.error = "请输入 11 位手机号".into();
            cx.notify();
        });
        return;
    }
    let client = store.read(cx).client.clone();
    store.update(cx, |s, cx| {
        s.login.sms_busy = true;
        s.login.error.clear();
        s.login.mobile = mobile.clone();
        cx.notify();
    });
    let task = cx
        .background_executor()
        .spawn(async move { run_tokio(client.send_sms_code(&mobile)) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        let _ = store.update(cx, |s, cx| {
            s.login.sms_busy = false;
            match res {
                Ok(()) => {
                    s.toast = "验证码已发送".into();
                    s.login.sms_countdown = 60;
                }
                Err(e) => s.login.error = format!("验证码发送失败：{e}"),
            }
            cx.notify();
        });
        // 倒计时
        loop {
            let left = store
                .update(cx, |s, cx| {
                    cx.notify();
                    if s.login.sms_countdown > 0 {
                        s.login.sms_countdown -= 1;
                    }
                    s.login.sms_countdown
                })
                .unwrap_or(0);
            if left == 0 {
                break;
            }
            smol::Timer::after(std::time::Duration::from_secs(1)).await;
        }
    })
    .detach();
}

/// 验证码登录。
pub(crate) fn submit_sms_login(
    store: &Entity<AppStore>,
    mobile: String,
    code: String,
    cx: &mut App,
) {
    let store = store.clone();
    let mobile = mobile.trim().to_string();
    let code = code.trim().to_string();
    if mobile.is_empty() || code.is_empty() {
        store.update(cx, |s, cx| {
            s.login.error = "请输入手机号与验证码".into();
            cx.notify();
        });
        return;
    }
    let client = store.read(cx).client.clone();
    store.update(cx, |s, cx| {
        s.login.busy = true;
        s.login.error.clear();
        cx.notify();
    });
    let task = cx
        .background_executor()
        .spawn(async move { run_tokio(client.login_by_sms(&mobile, &code)) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        match res {
            Ok(login) => {
                let _ = cx.update(|app| apply_login(&store, login, app));
            }
            Err(e) => {
                let _ = store.update(cx, |s, cx| {
                    s.login.busy = false;
                    s.login.error = format!("{e}");
                    cx.notify();
                });
            }
        }
    })
    .detach();
}

/// 登录（账号密码）。
pub(crate) fn submit_login(
    store: &Entity<AppStore>,
    username: String,
    password: String,
    cx: &mut App,
) {
    let store = store.clone();
    let username = username.trim().to_string();
    if username.is_empty() || password.is_empty() {
        store.update(cx, |s, cx| {
            s.login.error = "请输入账号与密码".into();
            cx.notify();
        });
        return;
    }
    let client = store.read(cx).client.clone();
    store.update(cx, |s, cx| {
        s.login.busy = true;
        s.login.error.clear();
        cx.notify();
    });
    let task = cx.background_executor().spawn(async move {
        run_tokio(client.login_by_pwd(&username, &password))
    });
    let store2 = store.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        match res {
            // 统一走 apply_login：写会话 + 拉用户详情 + 红心歌单 + 每日 VIP 领取
            Ok(login) => {
                let _ = cx.update(|app| apply_login(&store2, login, app));
            }
            Err(e) => {
                let _ = store.update(cx, |s, cx| {
                    s.login.busy = false;
                    s.login.error = format!("{e}");
                    cx.notify();
                });
            }
        }
    })
    .detach();
}

/// 退出登录。
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
    s.fav_filter.clear();
    s.fav_filtered = Arc::new(Vec::new());
}

/// 登录态失效（服务端 20018 等）：**自动退出登录**，并把当前页面导到登录页。
///
/// 用户看到的应该是「已经登出、请重新登录」，而不是一堆接口报错。
pub(crate) fn force_logout(s: &mut AppStore, msg: &str) {
    log::warn!("登录态失效，自动退出登录：{msg}");
    clear_login_state(s);
    s.session_expired = true;
    s.vip_note = msg.to_string();
    s.toast = msg.to_string();
    // 当前页需要登录就改道登录页；收藏页之类可以留下的不动
    let cur = s.routes.current().cloned();
    if let Some(p) = cur {
        if p.requires_login() || matches!(p, Page::Login) {
            if !matches!(p, Page::Login) {
                s.login_redirect = Some(p);
            }
            s.routes.navigate(Page::Login);
        }
    }
}

// ==================== 渲染 ====================

impl Root {
    /// 登录页。
    /// 登录页（扫码 / 验证码 / 密码）。
    pub(crate) fn render_login(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        let login = self.store.read(cx).login.clone();
        let store = self.store.clone();
        let busy = login.busy;
        let method = login.method;

        // ---- 登录方式切换 ----
        let tabs: Vec<AnyElement> = [
            (LoginMethod::Qr, "扫码登录"),
            (LoginMethod::Sms, "验证码登录"),
            (LoginMethod::Password, "账号密码"),
        ]
        .iter()
        .map(|(m, label)| {
            let active = *m == method;
            let m = *m;
            let st = store.clone();
            let accent = theme.accent;
            div()
                .id(SharedString::from(format!("login-tab-{label}")))
                .flex_1()
                .h(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .text_size(px(12.5))
                .cursor_pointer()
                .when(active, |el| el.bg(theme.bg_panel).text_color(accent))
                .when(!active, |el| {
                    el.text_color(theme.text_secondary)
                        .hover(move |s| s.text_color(theme.text))
                })
                .on_click(move |_, _, cx| {
                    st.update(cx, |s, cx| {
                        s.login.method = m;
                        s.login.error.clear();
                        cx.notify();
                    });
                    if m == LoginMethod::Qr {
                        start_qr_login(&st, cx);
                    }
                })
                .child(*label)
                .into_any_element()
        })
        .collect();

        // ---- 各登录方式内容 ----
        let body: AnyElement = match method {
            LoginMethod::Qr => {
                let accent = theme.accent;
                let status = login.qr_status;
                let loading = login.qr_loading;
                let url = login.qr_url.clone();
                let is_ok = status == 4;

                let hint = if loading {
                    "二维码加载中…".to_string()
                } else {
                    match status {
                        0 => "二维码已失效，点击刷新".to_string(),
                        2 => "已扫码，请在手机上确认登录".to_string(),
                        4 => "登录成功".to_string(),
                        _ => "请打开酷狗音乐 APP 扫一扫".to_string(),
                    }
                };

                let refresh_store = store.clone();
                div()
                    .w(px(312.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    // 二维码卡片（白底，保证任何主题下都可扫）
                    .child(
                        div()
                            .p_3()
                            .rounded(px(COVER_RADIUS))
                            .bg(gpui::white())
                            .when(url.is_empty() || status == 0, |el| {
                                el.child(
                                    div()
                                        .size(px(168.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(icon("icons/qr.svg", 56., gpui::rgb(0xd8d8d8).into())),
                                )
                            })
                            .when(!url.is_empty() && status != 0, |el| {
                                el.child(qr_widget(url.clone(), 168., gpui::rgb(0x111111).into()))
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(if is_ok { accent } else { theme.text_secondary })
                            .child(hint),
                    )
                    .child(
                        div()
                            .id("qr-refresh")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_3()
                            .py_1p5()
                            .rounded(px(8.))
                            .text_size(px(12.))
                            .text_color(theme.text_secondary)
                            .cursor_pointer()
                            .hover(move |s| s.bg(theme.bg_hover).text_color(accent))
                            .on_click(move |_, _, cx| start_qr_login(&refresh_store, cx))
                            .child(icon("icons/refresh.svg", 12., theme.text_secondary))
                            .child("刷新二维码"),
                    )
                    .into_any_element()
            }
            LoginMethod::Sms => {
                let phone_input = self.login_phone.clone();
                let code_input = self.login_code.clone();
                let code_store = store.clone();
                let phone_for_code = self.login_phone.clone();
                let submit_store = store.clone();
                let phone_for_submit = self.login_phone.clone();
                let code_for_submit = self.login_code.clone();
                let countdown = login.sms_countdown;
                let accent = theme.accent;
                let code_label = if countdown > 0 {
                    format!("{countdown}s 后重发")
                } else if login.sms_busy {
                    "发送中…".to_string()
                } else {
                    "获取验证码".to_string()
                };

                div()
                    .w(px(312.))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(input_shell(&theme, phone_input, cx))
                    .child(
                        div()
                            .w(px(312.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(input_shell(&theme, code_input, cx)))
                            .child(
                                div()
                                    .id("sms-send")
                                    .h(px(40.))
                                    .px_3()
                                    .flex_shrink_0()
                                    .rounded(px(10.))
                                    .border_1()
                                    .border_color(theme.border)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .when(countdown > 0, |el| el.text_color(theme.text_tertiary))
                                    .when(countdown == 0, |el| {
                                        el.text_color(accent).hover(move |s| s.bg(theme.bg_hover))
                                    })
                                    .on_click(move |_, _, cx| {
                                        if countdown > 0 {
                                            return;
                                        }
                                        let phone = phone_for_code.read(cx).text().to_string();
                                        send_sms_code(&code_store, phone, cx);
                                    })
                                    .child(code_label),
                            ),
                    )
                    .child(primary_button(
                        "sms-submit",
                        if busy { "登录中…" } else { "登录" },
                        &theme,
                        move |_, _, cx| {
                            let p = phone_for_submit.read(cx).text().to_string();
                            let c = code_for_submit.read(cx).text().to_string();
                            submit_sms_login(&submit_store, p, c, cx);
                        },
                    ))
                    .into_any_element()
            }
            LoginMethod::Password => {
                let user_input = self.login_user.clone();
                let pass_input = self.login_pass.clone();
                let submit_store = store.clone();
                let user_for_submit = self.login_user.clone();
                let pass_for_submit = self.login_pass.clone();
                div()
                    .w(px(312.))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(input_shell(&theme, user_input, cx))
                    .child(input_shell(&theme, pass_input, cx))
                    .child(primary_button(
                        "login-submit",
                        if busy { "登录中…" } else { "登 录" },
                        &theme,
                        move |_, _, cx| {
                            let u = user_for_submit.read(cx).text().to_string();
                            let p = pass_for_submit.read(cx).text().to_string();
                            submit_login(&submit_store, u, p, cx);
                        },
                    ))
                    .into_any_element()
            }
        };

        let error = login.error.clone();
        let accent = theme.accent;
        let tab_bg = if theme.dark {
            theme.bg_hover
        } else {
            theme.bg_sidebar
        };

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(360.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_6()
                    .rounded(px(16.))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_1()
                            .child(icon("icons/user.svg", 30., accent))
                            .child(div().text_size(px(18.)).child("登录酷狗账号"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child("登录后可播放、收藏、同步歌单"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .p_1()
                            .rounded(px(10.))
                            .bg(tab_bg)
                            .children(tabs),
                    )
                    .child(div().w(px(312.)).min_h(px(240.)).child(body))
                    .when(!error.is_empty(), |el| {
                        el.child(
                            div()
                                .text_size(px(12.))
                                .text_color(gpui::red())
                                .child(error.clone()),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.text_tertiary)
                            .child("登录信息仅保存在本机，不会上传到第三方服务器。"),
                    ),
            )
    }


    /// 探索页（歌单 / 排行榜 / 新歌 / 新碟）。
    pub(crate) fn render_explore(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        let explore = self.store.read(cx).explore.clone();
        // 加载态：居中旋转动画（填充内容区并垂直/水平居中）
        if explore.loading {
            return centered_spinner(theme);
        }
        let store = self.store.clone();
        let tabs = ["歌单", "排行榜", "新歌", "新碟"];

        let tab_bar: Vec<AnyElement> = tabs
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let st = store.clone();
                let active = i == explore.tab;
                let label = *name;
                div()
                    .id(SharedString::from(format!("ex-tab-{i}")))
                    .px_4()
                    .py_2()
                    .rounded_full()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .when(active, |el| {
                        el.bg(theme.accent).text_color(gpui::white())
                    })
                    .when(!active, |el| {
                        el.text_color(theme.text_secondary)
                            .hover(move |s| s.bg(theme.bg_hover))
                    })
                    .on_click(move |_, _, cx| {
                        st.update(cx, |s, cx| {
                            s.explore.tab = i;
                            cx.notify();
                        });
                    })
                    .child(label)
                    .into_any_element()
            })
            .collect();

        // 标签 chips（仅歌单 Tab）
        let tag_chips: Vec<AnyElement> = explore
            .tags
            .iter()
            .flat_map(|(_, tags)| tags.clone())
            .take(18)
            .map(|(id, name)| {
                let st = store.clone();
                let active = explore.active_tag.as_deref() == Some(id.as_str());
                let tag_id: i64 = id.parse().unwrap_or(0);
                div()
                    .id(SharedString::from(format!("ex-tag-{id}")))
                    .px_3()
                    .py(px(5.))
                    .rounded_full()
                    .border_1()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .when(active, |el| {
                        el.bg(theme.accent).border_color(theme.accent).text_color(gpui::white())
                    })
                    .when(!active, |el| {
                        el.border_color(theme.border)
                            .text_color(theme.text_secondary)
                            .hover(move |s| s.bg(theme.bg_hover))
                    })
                    .on_click(move |_, _, cx| {
                        st.update(cx, |s, cx| {
                            s.explore.active_tag = Some(tag_id.to_string());
                            cx.notify();
                        });
                        load_explore_playlists(&st, tag_id, cx);
                    })
                    .child(name)
                    .into_any_element()
            })
            .collect();

        let body: AnyElement = match explore.tab {
            0 => {
                let cards: Vec<AnyElement> = explore
                    .playlists
                    .iter()
                    .map(|pl| self.playlist_card(pl, cx).into_any_element())
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(div().flex().flex_wrap().gap_2().children(tag_chips))
                    .child(div().grid().grid_cols(4).gap_4().children(cards))
                    .into_any_element()
            }
            1 => {
                let cards: Vec<AnyElement> = explore
                    .ranks
                    .iter()
                    .map(|r| self.rank_card(r, cx).into_any_element())
                    .collect();
                div()
                    .grid()
                    .grid_cols(4)
                    .gap_4()
                    .children(cards)
                    .into_any_element()
            }
            2 => {
                let rows: Vec<AnyElement> = explore
                    .new_songs
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        self.song_row_with_source(s, i, PlaySource::NewSongs, cx)
                            .into_any_element()
                    })
                    .collect();
                div().flex().flex_col().children(rows).into_any_element()
            }
            _ => {
                let cards: Vec<AnyElement> = explore
                    .new_albums
                    .iter()
                    .map(|a| self.album_card(a, cx).into_any_element())
                    .collect();
                div()
                    .grid()
                    .grid_cols(4)
                    .gap_4()
                    .children(cards)
                    .into_any_element()
            }
        };

        div()
            .flex()
            .flex_col()
            .gap_5()
            .p_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(20.)).child("探索发现"))
                    .child(div().flex().gap_2().children(tab_bar)),
            )
            .child(body)
    }

    /// 榜单卡片。
    fn rank_card(
        &self,
        rank: &echo_core::RankMeta,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        let nav = self.store.clone();
        let page = Page::Rank(rank.id.clone(), rank.name.clone());
        let pic = rank.pic.clone();
        let name = rank.name.clone();
        let freq = rank.update_frequency.clone();
        div()
            .id(SharedString::from(format!("rank-{}", rank.id)))
            .w_full()
            .flex()
            .flex_col()
            .gap_1p5()
            .cursor_pointer()
            .on_click(move |_, _, cx| crate::ui::navigate(&nav, page.clone(), cx))
            .child({
                let mut cover = div()
                    .w_full()
                    .rounded(px(COVER_RADIUS))
                    .overflow_hidden()
                    .bg(theme.bg_hover)
                    .flex()
                    .items_center()
                    .justify_center();
                cover.style().aspect_ratio = Some(1.0);
                cover.when(!pic.is_empty(), |el| {
                    el.child(img(SharedUri::from(pic)).size_full().rounded(px(COVER_RADIUS)).object_fit(ObjectFit::Cover))
                })
            })
            .child(div().text_size(px(12.5)).truncate().child(name))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.text_tertiary)
                    .truncate()
                    .child(freq),
            )
    }

    /// 专辑卡片。
    pub(crate) fn album_card(&self, album: &Album, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        let nav = self.store.clone();
        let page = Page::Album(album.id.clone(), album.name.clone());
        let pic = album.pic.clone();
        let name = album.name.clone();
        let singer = album.singer_name.clone();
        div()
            .id(SharedString::from(format!("album-{}", album.id)))
            .w_full()
            .flex()
            .flex_col()
            .gap_1p5()
            .cursor_pointer()
            .on_click(move |_, _, cx| crate::ui::navigate(&nav, page.clone(), cx))
            .child({
                let mut cover = div()
                    .w_full()
                    .rounded(px(COVER_RADIUS))
                    .overflow_hidden()
                    .bg(theme.bg_hover)
                    .flex()
                    .items_center()
                    .justify_center();
                cover.style().aspect_ratio = Some(1.0);
                cover.when(!pic.is_empty(), |el| {
                    el.child(img(SharedUri::from(pic)).size_full().rounded(px(COVER_RADIUS)).object_fit(ObjectFit::Cover))
                })
            })
            .child(div().text_size(px(12.5)).truncate().child(name))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.text_secondary)
                    .truncate()
                    .child(singer),
            )
    }

    /// 排行榜页。
    pub(crate) fn render_ranking(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let explore = self.store.read(cx).explore.clone();
        let cards: Vec<AnyElement> = explore
            .ranks
            .iter()
            .map(|r| self.rank_card(r, cx).into_any_element())
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .p_6()
            .child(div().text_size(px(20.)).child("排行榜"))
            .child(div().grid().grid_cols(4).gap_4().children(cards))
    }

    /// 榜单详情。
    pub(crate) fn render_rank_detail(
        &mut self,
        id: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        // 加载态：居中旋转动画
        if self.store.read(cx).rank_loading {
            return centered_spinner(theme);
        }
        let songs = self.store.read(cx).rank_view.songs.clone();
        let store = self.store.clone();
        let list: Option<AnyElement> = if songs.is_empty() {
            None
        } else {
            Some(
                self.render_song_list(songs.clone(), PlaySource::RankSongs, cx)
                    .into_any_element(),
            )
        };
        let title = name.to_string();
        let _ = id;
        div()
            .flex()
            .flex_col()
            .h_full()
            .gap_4()
            .p_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(20.)).child(format!("榜单：{title}")))
                    .child(
                        div()
                            .id("rank-play-all")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_3()
                            .py(px(5.))
                            .rounded_full()
                            .bg(theme.accent)
                            .text_color(gpui::white())
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.88))
                            .on_click(move |_, _, cx| {
                                play_from(&store, PlaySource::RankSongs, 0, cx)
                            })
                            .child(icon("icons/play.svg", 11., gpui::white()))
                            .child("播放全部"),
                    ),
            )
            .when_some(list, |el, list| {
                el.child(div().flex_1().min_h_0().child(list))
            })
    }

    /// 私人 FM（参考酷狗「私人FM」：推荐来源 tabs + 频道卡片 + 当前播放）。
    pub(crate) fn render_fm(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.store.read(cx).theme();
        if self.store.read(cx).fm_loading {
            return centered_spinner(theme).into_any_element();
        }
        let songs = self.store.read(cx).fm.clone();
        let mode = self.store.read(cx).fm_mode.clone();
        let pool = self.store.read(cx).fm_pool;
        let playing_fm = self.store.read(cx).player.queue_title == "私人FM";
        let current = self.store.read(cx).player.current().cloned();

        let play_all = self.store.clone();
        let reload = self.store.clone();
        let empty_reload = self.store.clone();

        // 卡片上展示「当前这首」：正在放 FM 就放正在放的，否则放这批的第一首
        let featured = if playing_fm {
            current.clone().or_else(|| songs.first().cloned())
        } else {
            songs.first().cloned()
        };
        // 接下来几首：做成右侧的圆盘（点一下直接播）
        let circles: Vec<AnyElement> = songs
            .iter()
            .skip(1)
            .take(3)
            .enumerate()
            .map(|(i, song)| {
                let st = self.store.clone();
                let cover = song.cover_url.clone();
                let idx = i + 1;
                div()
                    .id(SharedString::from(format!("fm-disc-{i}")))
                    .w(px(126.))
                    .h(px(126.))
                    .ml(px(-26.))
                    .flex_shrink_0()
                    .rounded_full()
                    .overflow_hidden()
                    .border_2()
                    .border_color(theme.bg_main)
                    .bg(theme.bg_hover)
                    .cursor_pointer()
                    .hover(move |s| s.opacity(0.86))
                    .on_click(move |_, _, cx| play_from(&st, PlaySource::Fm, idx, cx))
                    .when(!cover.is_empty(), |el| {
                        el.child(
                            img(SharedUri::from(cover.clone()))
                                .size_full()
                                .rounded_full()
                                .object_fit(ObjectFit::Cover),
                        )
                    })
                    .when(cover.is_empty(), |el| {
                        el.flex().items_center().justify_center().child(icon(
                            "icons/music-note.svg",
                            30.,
                            theme.text_tertiary,
                        ))
                    })
                    .into_any_element()
            })
            .collect();

        // 推荐来源（口味 / 风格 / 探索）→ song_pool_id 0/1/2
        let pool_tab = |label: &'static str, pid: i64, active: bool, theme: &Theme| {
            let st = self.store.clone();
            let st2 = self.store.clone();
            div()
                .id(SharedString::from(format!("fm-pool-{pid}")))
                .px_4()
                .py(px(5.))
                .rounded_full()
                .text_size(px(12.))
                .cursor_pointer()
                .when(active, |el| el.bg(theme.accent).text_color(gpui::white()))
                .when(!active, |el| {
                    el.bg(theme.bg_hover)
                        .text_color(theme.text_secondary)
                        .hover(move |s| s.bg(theme.border))
                })
                .on_click(move |_, _, cx| {
                    let same = st.read(cx).fm_pool == pid;
                    if !same {
                        st.update(cx, |s, _| s.fm_pool = pid);
                        load_fm(&st2, true, cx);
                    }
                })
                .child(label)
        };

        // 频道（红心 / 小众 / 速览 / 电台）
        let mode_chip = |label: &'static str, m: &'static str, active: bool, theme: &Theme| {
            let st = self.store.clone();
            let st2 = self.store.clone();
            div()
                .id(SharedString::from(format!("fm-mode-{m}")))
                .px_2p5()
                .py(px(3.))
                .rounded_full()
                .text_size(px(11.))
                .cursor_pointer()
                .when(active, |el| {
                    el.bg(gpui::white()).text_color(theme.accent)
                })
                .when(!active, |el| {
                    el.bg(gpui::white().opacity(0.18))
                        .text_color(gpui::white().opacity(0.9))
                        .hover(move |s| s.bg(gpui::white().opacity(0.28)))
                })
                .on_click(move |_, _, cx| {
                    let same = st.read(cx).fm_mode == m;
                    if !same {
                        st.update(cx, |s, _| s.fm_mode = m.to_string());
                        load_fm(&st2, true, cx);
                    }
                })
                .child(label)
        };

        let artists = fm_top_artists(&songs);
        let mode_title = fm_mode_title(&mode);
        let pool_label = fm_pool_label(pool);

        // 当前播放卡片
        let featured_card: Option<AnyElement> = featured.clone().map(|song| {
            let cover = song.cover_url.clone();
            let dur = if song.duration > 0 {
                crate::ui::fmt_time(song.duration as f64)
            } else {
                String::new()
            };
            let quality = {
                let q = self.store.read(cx).settings.get().audio_quality;
                if q == "flac" || q == "320" {
                    "HQ".to_string()
                } else {
                    format!("{q}K")
                }
            };
            let similar = if song.similar.is_empty() {
                String::new()
            } else {
                format!("相似度{}", song.similar)
            };
            let chips: Vec<AnyElement> = [dur, quality, similar]
                .into_iter()
                .filter(|t| !t.is_empty())
                .map(|t| {
                    div()
                        .px_2()
                        .py(px(2.))
                        .rounded_full()
                        .bg(theme.bg_hover)
                        .text_size(px(10.))
                        .text_color(theme.text_secondary)
                        .child(t)
                        .into_any_element()
                })
                .collect();
            let reason = if song.rec_reason.is_empty() {
                format!("根据你的{pool_label}，为你推荐同类型歌曲")
            } else {
                song.rec_reason.clone()
            };
            div()
                .flex()
                .gap_4()
                .p_4()
                .rounded(px(14.))
                .bg(theme.bg_panel)
                .border_1()
                .border_color(theme.border)
                .child(
                    div()
                        .w(px(92.))
                        .h(px(92.))
                        .flex_shrink_0()
                        .rounded(px(10.))
                        .overflow_hidden()
                        .bg(theme.bg_hover)
                        .when(!cover.is_empty(), |el| {
                            el.child(
                                img(SharedUri::from(cover.clone()))
                                    .size_full()
                                    .rounded(px(10.))
                                    .object_fit(ObjectFit::Cover),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme.text_tertiary)
                                .child(if playing_fm { "当前播放" } else { "本批推荐" }),
                        )
                        .child(
                            div()
                                .text_size(px(19.))
                                .truncate()
                                .child(song.title.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.text_secondary)
                                .truncate()
                                .child(song.artist.clone()),
                        )
                        .when(!song.album.is_empty(), |el| {
                            el.child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.text_tertiary)
                                    .truncate()
                                    .child(song.album.clone()),
                            )
                        })
                        .child(
                            div()
                                .mt_1()
                                .text_size(px(11.))
                                .text_color(theme.accent)
                                .child(reason),
                        )
                        .child(div().flex().items_center().gap_2().mt_1().children(chips)),
                )
                .into_any_element()
        });

        div()
            .id("fm-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            // ===== 头部 =====
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .w(px(84.))
                            .h(px(84.))
                            .flex_shrink_0()
                            .rounded(px(20.))
                            .bg(theme.accent)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("icons/heart-sparkle.svg", 34., gpui::white())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(22.)).child("私人FM"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child("实时推送契合口味的音乐，随反馈动态更新专属歌单"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_size(px(11.))
                                    .text_color(theme.text_tertiary)
                                    .child(icon("icons/headphones.svg", 11., theme.text_tertiary))
                                    .child(div().child(format!(
                                        "{mode_title} · 根据{pool_label}{}",
                                        if artists.is_empty() {
                                            String::new()
                                        } else {
                                            format!(" · {artists}")
                                        }
                                    ))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .mt_1()
                                    .child(pool_tab("口味", 0, pool == 0, &theme))
                                    .child(pool_tab("风格", 1, pool == 1, &theme))
                                    .child(pool_tab("探索", 2, pool == 2, &theme)),
                            ),
                    ),
            )
            // ===== 电台卡片 + 圆盘 =====
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(300.))
                            .h(px(140.))
                            .flex_shrink_0()
                            .rounded(px(16.))
                            .bg(gpui::linear_gradient(
                                135.,
                                gpui::linear_color_stop(theme.accent, 0.),
                                gpui::linear_color_stop(theme.accent.opacity(0.62), 1.),
                            ))
                            .p_4()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(mode_chip("红心", "normal", mode == "normal", &theme))
                                    .child(mode_chip("小众", "small", mode == "small", &theme))
                                    .child(mode_chip("速览", "peak", mode == "peak", &theme))
                                    .child(mode_chip("电台", "radio", mode == "radio", &theme)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(21.))
                                            .text_color(gpui::white())
                                            .child(mode_title),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(gpui::white().opacity(0.8))
                                            .truncate()
                                            .child(if artists.is_empty() {
                                                "还没拿到推荐，点右下角换一批".to_string()
                                            } else {
                                                artists.clone()
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    // 装饰用音柱
                                    .child(
                                        div()
                                            .flex()
                                            .items_end()
                                            .gap(px(2.))
                                            .h(px(16.))
                                            .children([7., 14., 9., 16., 6.].into_iter().map(|h| {
                                                div()
                                                    .w(px(2.))
                                                    .h(px(h))
                                                    .rounded_full()
                                                    .bg(gpui::white().opacity(0.55))
                                                    .into_any_element()
                                            })),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .id("fm-refresh")
                                            .w(px(28.))
                                            .h(px(28.))
                                            .rounded_full()
                                            .bg(gpui::white().opacity(0.2))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .hover(move |s| s.bg(gpui::white().opacity(0.32)))
                                            .on_click(move |_, _, cx| load_fm(&reload, true, cx))
                                            .child(icon("icons/refresh.svg", 13., gpui::white())),
                                    )
                                    .child(
                                        div()
                                            .id("fm-play-all")
                                            .w(px(34.))
                                            .h(px(34.))
                                            .rounded_full()
                                            .bg(gpui::white())
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .hover(|s| s.opacity(0.88))
                                            .on_click(move |_, _, cx| {
                                                play_from(&play_all, PlaySource::Fm, 0, cx)
                                            })
                                            .child(icon("icons/play.svg", 14., theme.accent)),
                                    ),
                            ),
                    )
                    .children(circles),
            )
            // ===== 当前播放 =====
            .when_some(featured_card, |el, card| el.child(card))
            .when(songs.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .py_16()
                        .text_color(theme.text_tertiary)
                        .child(icon("icons/radio.svg", 34., theme.text_tertiary))
                        .child(div().text_size(px(13.)).child("还没有推荐"))
                        .child(
                            div()
                                .id("fm-reload")
                                .mt_2()
                                .px_4()
                                .py_2()
                                .rounded_full()
                                .bg(theme.accent)
                                .text_color(gpui::white())
                                .text_size(px(12.))
                                .cursor_pointer()
                                .hover(|s| s.opacity(0.88))
                                .on_click(move |_, _, cx| load_fm(&empty_reload, true, cx))
                                .child("换一批"),
                        ),
                )
            })
            .into_any_element()
    }

    /// 歌词页：整页歌词 + 当前行高亮（逐字歌词时逐字高亮）。
    pub(crate) fn render_lyric(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.store.read(cx).theme();
        let (lyric, loading, position, playing) = {
            let s = self.store.read(cx);
            (
                s.lyric.clone(),
                s.lyric_loading,
                s.player.position,
                s.player.playing,
            )
        };
        let Some(mut lyric) = lyric else {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(theme.text_tertiary)
                .child(icon("icons/music-note.svg", 34., theme.text_tertiary))
                .child(
                    div()
                        .text_size(px(13.))
                        .child(if loading { "歌词加载中…" } else { "暂无歌词" }),
                )
                .into_any_element();
        };

        let cur = lyric.index_at(position);
        let cur_word = cur.and_then(|i| lyric.word_index_at(i, position));
        let word_time = lyric.word_time;
        let offset_ms = lyric.offset_ms;
        // 行数据转成 owned：闭包要被打包进 AnyElement，不能借用局部变量
        let lines: Vec<echo_core::LyricLine> = std::mem::take(&mut lyric.lines);
        let store = self.store.clone();
        let song_title = self
            .store
            .read(cx)
            .player
            .current()
            .map(|s| s.title.clone())
            .unwrap_or_default();
        let song_artist = self
            .store
            .read(cx)
            .player
            .current()
            .map(|s| s.artist.clone())
            .unwrap_or_default();

        div()
            .id("lyric-page")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .px_8()
            .py_6()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .mb_4()
                    .child(div().text_size(px(18.)).child(song_title))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.text_tertiary)
                            .child(song_artist),
                    )
                    .child(
                        div()
                            .ml_2()
                            .text_size(px(11.))
                            .text_color(theme.text_tertiary)
                            .child(if word_time {
                                if playing {
                                    "逐字歌词"
                                } else {
                                    "逐字歌词 · 已暂停"
                                }
                            } else {
                                "LRC"
                            }),
                    ),
            )
            .children(lines.into_iter().enumerate().map(move |(i, line)| {
                let active = cur == Some(i);
                let wi = if active { cur_word } else { None };
                let secs = (line.time_ms as i64 + offset_ms).max(0) as f64 / 1000.0;
                let text = render_lyric_text(&line, wi, &theme);
                div()
                    .id(SharedString::from(format!("lyric-line-{i}")))
                    .py(px(7.))
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.92))
                    .on_click({
                        let store = store.clone();
                        move |_, _, cx| {
                            // 点歌词跳到这一行
                            store.update(cx, |s, cx| {
                                s.engine.seek(secs);
                                s.player.position = secs;
                                cx.notify();
                            });
                        }
                    })
                    .child(
                        div()
                            .text_size(if active { px(17.) } else { px(15.) })
                            .text_color(if active {
                                theme.text
                            } else {
                                theme.text_tertiary
                            })
                            .child(text),
                    )
                    .into_any_element()
            }))            .child(div().h(px(60.)).flex_shrink_0())
            .into_any_element()
    }

    /// 免费听（概念版推荐流）。
    pub(crate) fn render_free_listen(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.store.read(cx).theme();
        if self.store.read(cx).free_listen_loading {
            return centered_spinner(theme).into_any_element();
        }
        let songs = self.store.read(cx).free_listen.clone();
        let store = self.store.clone();
        let reload = self.store.clone();
        let list: Option<AnyElement> = if songs.is_empty() {
            None
        } else {
            Some(
                self.render_song_list(songs.clone(), PlaySource::FreeListen, cx)
                    .into_any_element(),
            )
        };
        div()
            .flex()
            .flex_col()
            .h_full()
            .gap_4()
            .p_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_size(px(20.)).child("免费听"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child("概念版专属：不登录也能听的推荐曲库"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("free-refresh")
                                    .px_3()
                                    .py(px(5.))
                                    .rounded_full()
                                    .bg(theme.bg_hover)
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .hover(move |s| s.bg(theme.border))
                                    .on_click(move |_, _, cx| load_free_listen(&reload, true, cx))
                                    .child("换一批"),
                            )
                            .when(!songs.is_empty(), |el| {
                                el.child(
                                    div()
                                        .id("free-play-all")
                                        .px_3()
                                        .py(px(5.))
                                        .rounded_full()
                                        .bg(theme.accent)
                                        .text_color(gpui::white())
                                        .text_size(px(12.))
                                        .cursor_pointer()
                                        .hover(|s| s.opacity(0.88))
                                        .on_click(move |_, _, cx| {
                                            play_from(&store, PlaySource::FreeListen, 0, cx)
                                        })
                                        .child("播放全部"),
                                )
                            }),
                    ),
            )
            .when_some(list, |el, list| {
                el.child(div().flex_1().min_h_0().child(list))
            })
            .when(songs.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .py_16()
                        .text_color(theme.text_tertiary)
                        .child(icon("icons/sparkles.svg", 34., theme.text_tertiary))
                        .child(div().text_size(px(13.)).child("还没有推荐")),
                )
            })
            .into_any_element()
    }

    /// 音乐云盘。
    pub(crate) fn render_cloud(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        if self.store.read(cx).cloud_loading {
            return centered_spinner(theme);
        }
        let songs = self.store.read(cx).cloud.clone();
        let store = self.store.clone();
        let reload = self.store.clone();
        let count = songs.len();
        let list: Option<AnyElement> = if songs.is_empty() {
            None
        } else {
            Some(
                self.render_song_list(songs.clone(), PlaySource::Cloud, cx)
                    .into_any_element(),
            )
        };
        let empty = div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .py_16()
            .text_color(theme.text_tertiary)
            .child(icon("icons/cloud.svg", 34., theme.text_tertiary))
            .child(div().text_size(px(13.)).child("云盘里还没有歌曲"))
            .child(
                div()
                    .text_size(px(11.))
                    .child("在酷狗手机端上传到云盘后，这里会自动出现"),
            );
        div()
            .flex()
            .flex_col()
            .h_full()
            .gap_4()
            .p_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_2()
                            .child(div().text_size(px(20.)).child("音乐云盘"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_tertiary)
                                    .child(format!("{count} 首")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("cloud-refresh")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_3()
                                    .py(px(5.))
                                    .rounded_full()
                                    .bg(theme.bg_hover)
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .hover(move |s| s.bg(theme.border))
                                    .on_click(move |_, _, cx| {
                                        load_cloud(&reload, true, cx);
                                    })
                                    .child(icon("icons/refresh.svg", 11., theme.text_secondary))
                                    .child("刷新"),
                            )
                            .when(count > 0, |el| {
                                el.child(
                                    div()
                                        .id("cloud-play-all")
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_3()
                                        .py(px(5.))
                                        .rounded_full()
                                        .bg(theme.accent)
                                        .text_color(gpui::white())
                                        .text_size(px(12.))
                                        .cursor_pointer()
                                        .hover(|s| s.opacity(0.88))
                                        .on_click(move |_, _, cx| {
                                            play_from(&store, PlaySource::Cloud, 0, cx)
                                        })
                                        .child(icon("icons/play.svg", 11., gpui::white()))
                                        .child("播放全部"),
                                )
                            }),
                    ),
            )
            .when_some(list, |el, list| {
                el.child(div().flex_1().min_h_0().child(list))
            })
            .when(count == 0, |el| el.child(empty))
    }

    /// 已购音乐（单曲 / 专辑）。
    pub(crate) fn render_purchased(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        if self.store.read(cx).purchased_loading {
            return centered_spinner(theme);
        }
        let tab = self.store.read(cx).purchased_tab;
        let songs = self.store.read(cx).purchased_songs.clone();
        let albums = self.store.read(cx).purchased_albums.clone();
        let store = self.store.clone();
        let tab_store = self.store.clone();
        let reload = self.store.clone();
        let song_count = songs.len();
        let album_count = albums.len();

        let list: Option<AnyElement> = if tab == 0 && !songs.is_empty() {
            Some(
                self.render_song_list(songs.clone(), PlaySource::Purchased, cx)
                    .into_any_element(),
            )
        } else if tab == 1 && !albums.is_empty() {
            let nav = self.store.clone();
            let cards: Vec<AnyElement> = albums
                .iter()
                .map(|a| {
                    let st = nav.clone();
                    let page = Page::Album(a.id.clone(), a.name.clone());
                    div()
                        .id(SharedString::from(format!("bought-album-{}", a.id)))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .cursor_pointer()
                        .hover(move |s| s.opacity(0.85))
                        .on_click(move |_, _, cx| crate::ui::navigate(&st, page.clone(), cx))
                        .child(
                            div()
                                .w_full()
                                .h(px(124.))
                                .rounded(px(10.))
                                .overflow_hidden()
                                .bg(theme.bg_hover)
                                .when(!a.pic.is_empty(), |el| {
                                    el.child(
                                        img(SharedUri::from(a.pic.clone()))
                                            .size_full()
                                            .object_fit(ObjectFit::Cover),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .truncate()
                                .child(a.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme.text_tertiary)
                                .truncate()
                                .child(a.singer_name.clone()),
                        )
                        .into_any_element()
                })
                .collect();
            Some(
                div()
                    .id("bought-albums")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(div().grid().grid_cols(5).gap_4().children(cards))
                    .into_any_element(),
            )
        } else {
            None
        };

        let tab_btn = |idx: usize, label: &'static str, count: usize, active: bool| {
            let st = tab_store.clone();
            div()
                .id(SharedString::from(format!("bought-tab-{idx}")))
                .px_3()
                .py(px(5.))
                .rounded_full()
                .text_size(px(12.))
                .cursor_pointer()
                .when(active, |el| el.bg(theme.accent).text_color(gpui::white()))
                .when(!active, |el| {
                    el.bg(theme.bg_hover)
                        .text_color(theme.text_secondary)
                        .hover(move |s| s.bg(theme.border))
                })
                .on_click(move |_, _, cx| {
                    st.update(cx, |s, cx| {
                        s.purchased_tab = idx;
                        cx.notify();
                    });
                })
                .child(format!("{label} {count}"))
        };

        let make_empty = |is_songs: bool| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .py_16()
                .text_color(theme.text_tertiary)
                .child(icon("icons/bag.svg", 34., theme.text_tertiary))
                .child(div().text_size(px(13.)).child(if is_songs {
                    "还没有已购单曲"
                } else {
                    "还没有已购专辑"
                }))
                .child(
                    div()
                        .text_size(px(11.))
                        .child("在酷狗购买数字专辑 / 单曲后会出现在这里"),
                )
        };

        div()
            .flex()
            .flex_col()
            .h_full()
            .gap_4()
            .p_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(20.)).child("已购音乐"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(tab_btn(0, "单曲", song_count, tab == 0))
                            .child(tab_btn(1, "专辑", album_count, tab == 1))
                            .child(
                                div()
                                    .id("bought-refresh")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_3()
                                    .py(px(5.))
                                    .rounded_full()
                                    .bg(theme.bg_hover)
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .hover(move |s| s.bg(theme.border))
                                    .on_click(move |_, _, cx| {
                                        load_purchased(&reload, true, cx);
                                    })
                                    .child(icon("icons/refresh.svg", 11., theme.text_secondary))
                                    .child("刷新"),
                            )
                            .when(tab == 0 && song_count > 0, |el| {
                                el.child(
                                    div()
                                        .id("bought-play-all")
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_3()
                                        .py(px(5.))
                                        .rounded_full()
                                        .bg(theme.accent)
                                        .text_color(gpui::white())
                                        .text_size(px(12.))
                                        .cursor_pointer()
                                        .hover(|s| s.opacity(0.88))
                                        .on_click(move |_, _, cx| {
                                            play_from(&store, PlaySource::Purchased, 0, cx)
                                        })
                                        .child(icon("icons/play.svg", 11., gpui::white()))
                                        .child("播放全部"),
                                )
                            }),
                    ),
            )
            .when_some(list, |el, list| {
                el.child(div().flex_1().min_h_0().child(list))
            })
            .when(tab == 0 && song_count == 0, |el| el.child(make_empty(true)))
            .when(tab == 1 && album_count == 0, |el| el.child(make_empty(false)))
    }

    /// 每日推荐。
    pub(crate) fn render_daily(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        // 加载态：居中旋转动画
        if self.store.read(cx).daily_loading {
            return centered_spinner(theme);
        }
        let songs = self.store.read(cx).daily.clone();
        let store = self.store.clone();
        // 虚拟化列表：只渲染可见行（长列表不再全量构建）
        let list: Option<AnyElement> = if songs.is_empty() {
            None
        } else {
            Some(
                self.render_song_list(songs.clone(), PlaySource::Daily, cx)
                    .into_any_element(),
            )
        };
        div()
            .flex()
            .flex_col()
            .h_full()
            .gap_4()
            .p_6()
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_size(px(20.)).child("每日推荐"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child(format!("共 {} 首 · 每日 6 点更新", songs.len())),
                            ),
                    )
                    .child(
                        div()
                            .id("daily-play-all")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_3()
                            .py(px(5.))
                            .rounded_full()
                            .bg(theme.accent)
                            .text_color(gpui::white())
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.88))
                            .on_click(move |_, _, cx| play_from(&store, PlaySource::Daily, 0, cx))
                            .child(icon("icons/play.svg", 11., gpui::white()))
                            .child("播放全部"),
                    ),
            )
            .when(songs.is_empty(), |el| {
                el.child(
                    div()
                        .py_10()
                        .text_size(px(12.))
                        .text_color(theme.text_tertiary)
                        .child("正在获取今日推荐…"),
                )
            })
            .when_some(list, |el, list| {
                el.child(div().flex_1().min_h_0().child(list))
            })
    }
}

impl Root {
    /// 歌手详情。
    pub(crate) fn render_artist(
        &mut self,
        _id: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        // 加载态：居中旋转动画
        if self.store.read(cx).artist_loading {
            return centered_spinner(theme);
        }
        let view = self.store.read(cx).artist_view.clone().unwrap_or_default();
        let store = self.store.clone();
        let songs = view.songs.clone();
        let albums = view.albums.clone();
        let title = if view.artist.name.is_empty() {
            name.to_string()
        } else {
            view.artist.name.clone()
        };
        let pic = view.artist.pic.clone();
        let intro = view.artist.intro.clone();
        let song_count = view.artist.song_count;
        let album_count = view.artist.album_count;

        let list: Option<AnyElement> = if songs.is_empty() {
            None
        } else {
            Some(
                self.render_song_list(songs.clone(), PlaySource::ArtistSongs, cx)
                    .into_any_element(),
            )
        };
        let album_cards: Vec<AnyElement> = albums
            .iter()
            .map(|a| self.album_card(a, cx).into_any_element())
            .collect();

        div()
            .flex()
            .flex_col()
            .gap_5()
            .p_6()
            .child(
                div()
                    .flex()
                    .gap_5()
                    .child(
                        div()
                            .size(px(140.))
                            .rounded_full()
                            .overflow_hidden()
                            .bg(theme.bg_hover)
                            .flex_shrink_0()
                            .when(!pic.is_empty(), |el| {
                                el.child(
                                    img(SharedUri::from(pic))
                                        .size_full()
                                        .rounded_full()
                                        .object_fit(ObjectFit::Cover),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(24.)).child(title))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child(format!("{song_count} 首单曲 · {album_count} 张专辑")),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_tertiary)
                                    .child(intro),
                            )
                            .child(
                                div()
                                    .id("artist-play-all")
                                    .mt_2()
                                    .w(px(120.))
                                    .h(px(32.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_1()
                                    .bg(theme.accent)
                                    .text_color(gpui::white())
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.88))
                                    .on_click(move |_, _, cx| {
                                        play_from(&store, PlaySource::ArtistSongs, 0, cx)
                                    })
                                    .child(icon("icons/play.svg", 11., gpui::white()))
                                    .child("播放全部"),
                            ),
                    ),
            )
            .child(div().text_size(px(16.)).child("热门歌曲"))
            .when_some(list, |el, list| {
                el.child(div().flex_1().min_h_0().child(list))
            })
            .when(!album_cards.is_empty(), |el| {
                el.child(div().text_size(px(16.)).child("专辑"))
                    .child(div().grid().grid_cols(4).gap_4().children(album_cards))
            })
    }

    /// 专辑详情。
    pub(crate) fn render_album(
        &mut self,
        _id: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        // 加载态：居中旋转动画
        if self.store.read(cx).album_loading {
            return centered_spinner(theme);
        }
        let view = self.store.read(cx).album_view.clone();
        let store = self.store.clone();
        let (album, songs) = match view {
            Some(v) => (v.album, v.songs),
            None => (
                Album {
                    name: name.to_string(),
                    ..Default::default()
                },
                Arc::new(Vec::new()),
            ),
        };
        let list: Option<AnyElement> = if songs.is_empty() {
            None
        } else {
            Some(
                self.render_song_list(songs.clone(), PlaySource::AlbumSongs, cx)
                    .into_any_element(),
            )
        };
        let pic = album.pic.clone();
        let title = album.name.clone();
        let singer = album.singer_name.clone();
        let publish = album.publish_time.clone();
        let intro = album.intro.clone();

        div()
            .flex()
            .flex_col()
            .h_full()
            .gap_5()
            .p_6()
            .child(
                div()
                    .flex()
                    .gap_5()
                    .child(
                        div()
                            .size(px(150.))
                            .rounded(px(COVER_RADIUS))
                            .overflow_hidden()
                            .bg(theme.bg_hover)
                            .flex_shrink_0()
                            .when(!pic.is_empty(), |el| {
                                el.child(
                                    img(SharedUri::from(pic))
                                        .size_full()
                                        .rounded(px(COVER_RADIUS))
                                        .object_fit(ObjectFit::Cover),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(22.)).child(title))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(theme.text_secondary)
                                    .child(singer),
                            )
                            .when(!publish.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme.text_tertiary)
                                        .child(format!("发行时间：{publish}")),
                                )
                            })
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_tertiary)
                                    .child(intro),
                            )
                            .child(
                                div()
                                    .id("album-play-all")
                                    .mt_2()
                                    .w(px(120.))
                                    .h(px(32.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_1()
                                    .bg(theme.accent)
                                    .text_color(gpui::white())
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.88))
                                    .on_click(move |_, _, cx| {
                                        play_from(&store, PlaySource::AlbumSongs, 0, cx)
                                    })
                                    .child(icon("icons/play.svg", 11., gpui::white()))
                                    .child("播放全部"),
                            ),
                    ),
            )
            .child(div().text_size(px(16.)).child("歌曲列表"))
            .when_some(list, |el, list| {
                el.child(div().flex_1().min_h_0().child(list))
            })
    }

    /// 设置页。
    pub(crate) fn render_settings(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        let settings = self.store.read(cx).settings.get();
        let diag = self.store.read(cx).audio_diag;
        let store = self.store.clone();
        let s2 = store.clone();
        let s3 = store.clone();
        let s4 = store.clone();

        // 主题模式
        let mode_btn = |label: &'static str, mode: echo_core::ThemeMode, cur: echo_core::ThemeMode, st: Entity<AppStore>| {
            let active = mode == cur;
            div()
                .id(SharedString::from(format!("mode-{label}")))
                .px_4()
                .py_2()
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(12.5))
                .when(active, |el| el.bg(theme.accent).text_color(gpui::white()))
                .when(!active, |el| {
                    el.bg(theme.bg_hover).text_color(theme.text_secondary)
                })
                .on_click(move |_, _, cx| {
                    crate::ui::set_theme_mode(&st, mode, cx);
                })
                .child(label)
                .into_any_element()
        };

        let mode_row = div()
            .flex()
            .gap_2()
            .child(mode_btn("浅色", echo_core::ThemeMode::Light, settings.theme_mode, s2.clone()))
            .child(mode_btn("深色", echo_core::ThemeMode::Dark, settings.theme_mode, s2.clone()))
            .child(mode_btn("跟随系统", echo_core::ThemeMode::System, settings.theme_mode, s2));

        // 主题色
        let colors = ["#f55e55", "#e8a33d", "#3fa66a", "#3d8ee8", "#8b5cf6", "#e05299"];
        let color_dots: Vec<AnyElement> = colors
            .iter()
            .map(|c| {
                let st = s3.clone();
                let value = c.to_string();
                let cur = settings.theme_color.clone();
                let is_active = cur.eq_ignore_ascii_case(c);
                div()
                    .id(SharedString::from(format!("color-{c}")))
                    .size(px(26.))
                    .rounded_full()
                    .cursor_pointer()
                    .bg(crate::theme::parse_color(c))
                    .border_2()
                    .border_color(if is_active { theme.text } else { theme.bg_panel })
                    .on_click(move |_, _, cx| {
                        let v = value.clone();
                        st.update(cx, |s, cx| {
                            s.settings.update(|st| st.theme_color = v.clone());
                            cx.notify();
                        });
                    })
                    .into_any_element()
            })
            .collect();

        // 音质
        let qualities = [("128", "标准"), ("320", "高品质"), ("flac", "无损")];
        let quality_row: Vec<AnyElement> = qualities
            .iter()
            .map(|(q, label)| {
                let st = s4.clone();
                let qv = q.to_string();
                let active = settings.audio_quality == *q;
                div()
                    .id(SharedString::from(format!("quality-{q}")))
                    .px_4()
                    .py_2()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(active, |el| el.bg(theme.accent).text_color(gpui::white()))
                    .when(!active, |el| {
                        el.bg(theme.bg_hover).text_color(theme.text_secondary)
                    })
                    .on_click(move |_, _, cx| {
                        let v = qv.clone();
                        st.update(cx, |s, cx| {
                            s.settings.update(|st| st.audio_quality = v.clone());
                            cx.notify();
                        });
                    })
                    .child(*label)
                    .into_any_element()
            })
            .collect();

        let clear_store = store.clone();

        div()
            .flex()
            .flex_col()
            .gap_6()
            .p_6()
            .child(div().text_size(px(20.)).child("设置"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .rounded(px(12.))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .child(div().text_size(px(14.)).child("外观"))
                    .child(mode_row)
                    .child(div().text_size(px(12.)).text_color(theme.text_secondary).child("主题色"))
                    .child(div().flex().gap_2().children(color_dots)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .rounded(px(12.))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .child(div().text_size(px(14.)).child("音质"))
                    .child(div().flex().gap_2().children(quality_row))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.text_tertiary)
                            .child("无损音质需要酷狗 VIP 账号。"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .rounded(px(12.))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .child(div().text_size(px(14.)).child("数据"))
                    .child(
                        div()
                            .id("clear-history")
                            .w(px(140.))
                            .h(px(32.))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme.bg_hover)
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(theme.bg_hover))
                            .on_click(move |_, _, cx| {
                                clear_store.update(cx, |s, cx| {
                                    s.library.clear_history();
                                    s.toast = "播放历史已清空".into();
                                    cx.notify();
                                });
                            })
                            .child("清空播放历史"),
                    ),
            )
            .child(
                // 音频诊断：数据来自引擎 1Hz 限频事件，渲染时才拼字符串
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .rounded(px(12.))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .child(div().text_size(px(14.)).child("音频诊断"))
                    .when_some(diag, |el, d| {
                        el.child(
                            div()
                                .flex()
                                .gap_6()
                                .text_size(px(12.))
                                .text_color(theme.text_secondary)
                                .child(format!("欠载 {} 次", d.underruns))
                                .child(format!("静音 {} 样本", d.silent_samples))
                                .child(format!("回调周期 {} 帧", d.cb_frames)),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_6()
                                .text_size(px(12.))
                                .text_color(theme.text_secondary)
                                .child(format!("最长回调 {} µs", d.max_cb_us))
                                .child(format!("预缓冲目标 {} ms", d.target_ms))
                                .child(format!(
                                    "实时优先级 {}",
                                    if d.rt_ok { "已启用" } else { "未启用" }
                                )),
                        )
                    })
                    .when(diag.is_none(), |el| {
                        el.child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.text_tertiary)
                                .child("暂无数据（播放后每秒刷新）"),
                        )
                    }),
            )
            // ===== 第三期：播放与系统 =====
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .rounded(px(12.))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .child(div().text_size(px(14.)).child("播放"))
                    // 倍速
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(72.))
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child("倍速"),
                            )
                            .children(
                                [0.75f64, 1.0, 1.25, 1.5, 2.0].into_iter().map(|v| {
                                    let st = store.clone();
                                    let active = (settings.speed - v).abs() < 0.001;
                                    div()
                                        .id(SharedString::from(format!("spd-{v}")))
                                        .px_3()
                                        .py(px(4.))
                                        .rounded(px(6.))
                                        .text_size(px(12.))
                                        .cursor_pointer()
                                        .when(active, |el| {
                                            el.bg(theme.accent).text_color(gpui::white())
                                        })
                                        .when(!active, |el| {
                                            el.bg(theme.bg_hover)
                                                .text_color(theme.text_secondary)
                                        })
                                        .on_click(move |_, _, cx| {
                                            st.update(cx, |s, cx| {
                                                s.settings.update(|st| st.speed = v);
                                                s.engine.set_speed(v);
                                                s.toast = format!("倍速 {}x", v);
                                                cx.notify();
                                            });
                                        })
                                        .child(format!("{}x", v))
                                        .into_any_element()
                                }),
                            )
                            .child(
                                div()
                                    .ml_2()
                                    .text_size(px(11.))
                                    .text_color(theme.text_tertiary)
                                    .child("（变速会改变音调）"),
                            ),
                    )
                    // 睡眠定时器
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(72.))
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child("睡眠定时"),
                            )
                            .children([0i64, 15, 30, 60].into_iter().map(|m| {
                                let st = store.clone();
                                let active = match settings.sleep_minutes {
                                    0 => m == 0,
                                    cur => cur == m,
                                };
                                div()
                                    .id(SharedString::from(format!("sleep-{m}")))
                                    .px_3()
                                    .py(px(4.))
                                    .rounded(px(6.))
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .when(active, |el| {
                                        el.bg(theme.accent).text_color(gpui::white())
                                    })
                                    .when(!active, |el| {
                                        el.bg(theme.bg_hover).text_color(theme.text_secondary)
                                    })
                                    .on_click(move |_, _, cx| {
                                        st.update(cx, |s, cx| {
                                            s.settings.update(|st| st.sleep_minutes = m);
                                            cx.notify();
                                        });
                                        crate::ui::set_sleep_timer(
                                            &st,
                                            if m == 0 { None } else { Some(m) },
                                            cx,
                                        );
                                    })
                                    .child(if m == 0 { "关".to_string() } else { format!("{m} 分") })
                                    .into_any_element()
                            })),
                    )
                    // 桌面歌词
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(72.))
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child("桌面歌词"),
                            )
                            .child({
                                let st = store.clone();
                                let on = settings.desktop_lyric;
                                div()
                                    .id("set-desktop-lyric")
                                    .px_3()
                                    .py(px(4.))
                                    .rounded(px(6.))
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .when(on, |el| el.bg(theme.accent).text_color(gpui::white()))
                                    .when(!on, |el| {
                                        el.bg(theme.bg_hover).text_color(theme.text_secondary)
                                    })
                                    .on_click(move |_, _, cx| {
                                        let next = !st.read(cx).settings.get().desktop_lyric;
                                        st.update(cx, |s, cx| {
                                            s.settings.update(|st| st.desktop_lyric = next);
                                            cx.notify();
                                        });
                                        crate::desktop::toggle_desktop_lyric(&st, cx);
                                    })
                                    .child(if on { "已开启" } else { "关" })
                            })
                            .child({
                                let st = store.clone();
                                div()
                                    .id("set-mini")
                                    .px_3()
                                    .py(px(4.))
                                    .rounded(px(6.))
                                    .bg(theme.bg_hover)
                                    .text_color(theme.text_secondary)
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .on_click(move |_, _, cx| {
                                        crate::desktop::open_mini_player(&st, cx);
                                    })
                                    .child("打开 mini 播放器")
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .rounded(px(12.))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .child(div().text_size(px(14.)).child("系统"))
                    // 开机自启
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(72.))
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child("开机自启"),
                            )
                            .child({
                                #[cfg(windows)]
                                {
                                    let st = store.clone();
                                    let on = crate::win::auto_start_enabled();
                                    div()
                                        .id("set-autostart")
                                        .px_3()
                                        .py(px(4.))
                                        .rounded(px(6.))
                                        .text_size(px(12.))
                                        .cursor_pointer()
                                        .when(on, |el| {
                                            el.bg(theme.accent).text_color(gpui::white())
                                        })
                                        .when(!on, |el| {
                                            el.bg(theme.bg_hover).text_color(theme.text_secondary)
                                        })
                                        .on_click(move |_, _, cx| {
                                            let next = !crate::win::auto_start_enabled();
                                            match crate::win::set_auto_start(next) {
                                                Ok(()) => {
                                                    st.update(cx, |s, cx| {
                                                        s.toast = if next {
                                                            "已开启开机自启".into()
                                                        } else {
                                                            "已关闭开机自启".into()
                                                        };
                                                        cx.notify();
                                                    });
                                                }
                                                Err(e) => {
                                                    st.update(cx, |s, cx| {
                                                        s.toast = format!("设置失败：{e}");
                                                        cx.notify();
                                                    });
                                                }
                                            }
                                        })
                                        .child(if on { "已开启" } else { "关" })
                                        .into_any_element()
                                }
                                #[cfg(not(windows))]
                                {
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme.text_tertiary)
                                        .child("仅 Windows 支持")
                                        .into_any_element()
                                }
                            }),
                    )
                    // 输出设备
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(72.))
                                    .pt(px(5.))
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child("输出设备"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .children({
                                        let mut names: Vec<String> = vec!["系统默认".into()];
                                        names.extend(echo_audio::output_devices());
                                        names.into_iter().map(|name| {
                                            let st = store.clone();
                                            let cur = settings.output_device.clone();
                                            let is_default = name == "系统默认";
                                            let active = if is_default {
                                                cur.is_empty()
                                            } else {
                                                cur == name
                                            };
                                            div()
                                                .id(SharedString::from(format!("dev-{name}")))
                                                .px_3()
                                                .py(px(4.))
                                                .rounded(px(6.))
                                                .text_size(px(12.))
                                                .cursor_pointer()
                                                .when(active, |el| {
                                                    el.bg(theme.accent)
                                                        .text_color(gpui::white())
                                                })
                                                .when(!active, |el| {
                                                    el.bg(theme.bg_hover)
                                                        .text_color(theme.text_secondary)
                                                })
                                                .on_click({
                                                    let name = name.clone();
                                                    move |_, _, cx| {
                                                        let value =
                                                            if is_default { String::new() } else { name.clone() };
                                                        st.update(cx, |s, cx| {
                                                            s.settings.update(|st| {
                                                                st.output_device = value
                                                            });
                                                            s.toast = "输出设备已保存，重启后生效".into();
                                                            cx.notify();
                                                        });
                                                    }
                                                })
                                                .child(name.clone())
                                                .into_any_element()
                                        })
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.text_tertiary)
                    .child("EchoMusic · Rust + GPUI 重写版 · 第三期"),
            )
    }

    /// 个人中心。
    pub(crate) fn render_profile(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        let session = self.store.read(cx).session.get();
        let vip_claiming = self.store.read(cx).vip_claiming;
        let vip_note = self.store.read(cx).vip_note.clone();
        let expired = self.store.read(cx).session_expired;
        let store = self.store.clone();
        let nav = self.store.clone();
        let claim_store = self.store.clone();

        let header = if session.is_valid() {
            let pic = session.pic.clone();
            let nick = if session.nickname.is_empty() {
                format!("用户 {}", session.userid)
            } else {
                session.nickname.clone()
            };
            let vip_type = session.vip_type;
            div()
                .flex()
                .items_center()
                .gap_4()
                .child(
                    div()
                        .size(px(64.))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(theme.bg_hover)
                        .when(!pic.is_empty(), |el| {
                            el.child(
                                img(SharedUri::from(pic))
                                    .size_full()
                                    .rounded_full()
                                    .object_fit(ObjectFit::Cover),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().text_size(px(18.)).child(nick))
                                .when_some(crate::ui::vip_tag(vip_type), |el, tag| {
                                    el.child(tag)
                                }),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.text_secondary)
                                .child(format!("ID {}", session.userid)),
                        )
                )
                .child(
                    // 每日畅听 VIP：自动领取兜底，也能手动点一次
                    div()
                        .id("claim-vip")
                        .ml_4()
                        .px_3()
                        .py_2()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme.accent)
                        .text_size(px(12.))
                        .cursor_pointer()
                        .when(vip_claiming, |el| el.text_color(theme.text_tertiary))
                        .when(!vip_claiming, |el| el.text_color(theme.accent))
                        .hover(move |s| s.bg(theme.bg_hover))
                        .on_click(move |_, _, cx| {
                            if !vip_claiming {
                                crate::vip::claim_now(&claim_store, cx);
                            }
                        })
                        .child(if vip_claiming { "领取中…" } else { "领取今日VIP" }),
                )
                .child(
                    div()
                        .id("logout")
                        .ml_4()
                        .px_4()
                        .py_2()
                        .rounded(px(8.))
                        .bg(theme.bg_hover)
                        .text_size(px(12.))
                        .cursor_pointer()
                        .on_click(move |_, _, cx| logout(&store, cx))
                        .child("退出登录"),
                )
                .into_any_element()
        } else {
            let nav2 = nav.clone();
            div()
                .flex()
                .items_center()
                .gap_4()
                .child(
                    div()
                        .size(px(64.))
                        .rounded_full()
                        .bg(theme.bg_hover)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon("icons/user.svg", 28., theme.text_tertiary)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_size(px(17.)).child("未登录"))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.text_secondary)
                                .child("登录后即可播放与同步收藏"),
                        ),
                )
                .child(
                    div()
                        .id("go-login")
                        .ml_4()
                        .px_5()
                        .py_2()
                        .rounded_full()
                        .bg(theme.accent)
                        .text_color(gpui::white())
                        .text_size(px(12.))
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.88))
                        .on_click(move |_, _, cx| crate::ui::navigate(&nav2, Page::Login, cx))
                        .child("去登录"),
                )
                .into_any_element()
        };

        let entries = [
            ("我最喜爱", "icons/heart.svg", Page::Favorites),
            ("播放历史", "icons/history.svg", Page::History),
            ("音乐云盘", "icons/cloud.svg", Page::Cloud),
            ("已购音乐", "icons/star.svg", Page::Purchased),
        ];
        let entry_store = self.store.clone();
        let entry_cards: Vec<AnyElement> = entries
            .iter()
            .map(|(label, icon_path, page)| {
                let st = entry_store.clone();
                let p = page.clone();
                div()
                    .id(SharedString::from(format!("profile-{label}")))
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_4()
                    .rounded(px(12.))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .hover(move |s| s.bg(theme.bg_hover))
                    .on_click(move |_, _, cx| crate::ui::navigate(&st, p.clone(), cx))
                    .child(icon(icon_path, 18., theme.accent))
                    .child(div().text_size(px(13.)).child(*label))
                    .into_any_element()
            })
            .collect();

        div()
            .flex()
            .flex_col()
            .gap_6()
            .p_6()
            .child(div().text_size(px(20.)).child("个人中心"))
            .child(
                div()
                    .p_5()
                    .rounded(px(14.))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(header)
                    // 会员状态：到期时间精确到秒（来自 kugouvip 会员接口）
                    .when(session.is_valid() && !vip_note.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .pt_3()
                                .border_t_1()
                                .border_color(theme.border)
                                .text_size(px(12.))
                                .child(
                                    div()
                                        .text_color(theme.text_tertiary)
                                        .child("会员状态"),
                                )
                                .child(
                                    div()
                                        .text_color(if expired {
                                            theme.accent
                                        } else {
                                            theme.text_secondary
                                        })
                                        .child(vip_note.clone()),
                                ),
                        )
                    }),
            )
            .child(div().grid().grid_cols(4).gap_4().children(entry_cards))
    }

    /// 收藏 / 历史列表页。
    pub(crate) fn render_library(
        &mut self,
        favorites: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        let lib = self.store.read(cx).library.get();
        let logged_in = self.store.read(cx).session.get().userid != 0;
        let remote_loading = self.store.read(cx).remote_liked_loading;
        let filter = self.store.read(cx).fav_filter.clone();
        // 云歌单（登录用户「我最喜爱」）加载态：居中旋转动画
        if favorites && logged_in && remote_loading {
            return centered_spinner(theme);
        }
        // 我的喜爱：登录用户用云歌单、未登录用本地收藏；
        // 关键词非空时用前端模糊搜索的结果（不打接口，纯本地过滤）。
        let songs = if favorites {
            self.store.read(cx).favorites_view()
        } else {
            lib.history.clone()
        };
        let source = if !favorites {
            PlaySource::History
        } else if filter.trim().is_empty() {
            PlaySource::Favorites
        } else {
            PlaySource::FavoritesFiltered
        };
        let store = self.store.clone();
        let list: Option<AnyElement> = if songs.is_empty() {
            None
        } else {
            Some(
                self.render_song_list(songs.clone(), source.clone(), cx)
                    .into_any_element(),
            )
        };
        let title = if favorites { "我最喜爱" } else { "播放历史" };
        let empty_hint = if favorites {
            if !filter.trim().is_empty() {
                "没有匹配的歌曲，换个关键词试试"
            } else if logged_in {
                "云歌单里还没有歌曲，点歌曲右侧的红心即可收藏"
            } else {
                "还没有收藏的歌曲，点歌曲右侧的红心即可收藏"
            }
        } else {
            "还没有播放记录"
        };
        // 空收藏时也留着搜索框，否则用户搜完清空后会「找不到入口」
        let show_search = favorites && (logged_in || !lib.favorites.is_empty() || !filter.is_empty());
        let clear_store = store.clone();
        let reload_store = store.clone();

        // ---- 听歌统计（仅「播放历史」）----
        let stats_panel: Option<AnyElement> = if favorites {
            None
        } else if lib.plays.is_empty() {
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_4()
                    .rounded(px(12.))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .child(div().text_size(px(14.)).child("听歌统计"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.text_tertiary)
                            .child("还没有统计：从这一版开始会记录每次播放的时间"),
                    )
                    .into_any_element(),
            )
        } else {
            let st = lib.listening_stats();
            let max_day = st.days.iter().copied().max().unwrap_or(1).max(1);
            let peak_hour = st
                .hours
                .iter()
                .enumerate()
                .max_by_key(|(_, c)| **c)
                .map(|(h, _)| h)
                .unwrap_or(0);
            let label = match peak_hour {
                0..=5 => "深夜",
                6..=10 => "清晨",
                11..=13 => "午间",
                14..=17 => "午后",
                18..=22 => "晚间",
                _ => "深夜",
            };
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .rounded(px(12.))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_2()
                            .child(div().text_size(px(14.)).child("听歌统计"))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.text_tertiary)
                                    .child(format!("累计播放 {} 次", st.total)),
                            ),
                    )
                    // 近 7 日柱状图
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap_2()
                            .h(px(64.))
                            .children(st.days.iter().enumerate().map(|(i, c)| {
                                let h = (*c as f32 / max_day as f32 * 48.0).max(3.0);
                                div()
                                    .w(px(26.))
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .justify_end()
                                    .gap_1()
                                    .child(
                                        div()
                                            .w(px(16.))
                                            .h(px(h))
                                            .rounded(px(3.))
                                            .bg(if i == 6 {
                                                theme.accent
                                            } else {
                                                theme.accent.opacity(0.35)
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(theme.text_tertiary)
                                            .child(if i == 6 {
                                                "今天".to_string()
                                            } else {
                                                format!("{}d", 6 - i)
                                            }),
                                    )
                                    .into_any_element()
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_6()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child(format!("偏好时段：{label}（{peak_hour} 点前后）")),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child(format!(
                                        "常听歌手：{}",
                                        if st.top_artists.is_empty() {
                                            "暂无".to_string()
                                        } else {
                                            st.top_artists
                                                .iter()
                                                .map(|(a, _)| a.clone())
                                                .collect::<Vec<_>>()
                                                .join("、")
                                        }
                                    )),
                            ),
                    )
                    .child({
                        let st2 = store.clone();
                        div()
                            .id("clear-stats")
                            .w(px(90.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .px_3()
                            .py(px(4.))
                            .rounded(px(6.))
                            .bg(theme.bg_hover)
                            .text_size(px(11.5))
                            .text_color(theme.text_secondary)
                            .cursor_pointer()
                            .hover(move |el| el.bg(theme.border))
                            .on_click(move |_, _, cx| {
                                st2.update(cx, |s, cx| {
                                    s.library.clear_plays();
                                    s.toast = "听歌统计已清零".into();
                                    cx.notify();
                                });
                            })
                            .child("清空统计")
                    })
                    .into_any_element(),
            )
        };

        // ---- 前端模糊搜索框（仅「我最喜爱」）----
        let search_box = if show_search {
            let fav_input = self.fav_search.clone();
            let fav_focus = self.fav_search_focus.clone();
            let bounds_slot = self.store.read(cx).fav_search_bounds.clone();
            let clear_btn_store = self.store.clone();
            let clear_btn_input = self.fav_search.clone();
            let hover_bg = theme.bg_hover;
            Some(
                div()
                    .id("fav-search-box")
                    .relative()
                    .flex()
                    .items_center()
                    .gap_1()
                    .w(px(220.))
                    .h(px(30.))
                    .px_2()
                    .rounded_full()
                    .bg(theme.bg_hover)
                    .border_1()
                    .border_color(theme.border)
                    .cursor_text()
                    .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, _: &mut App| {
                        window.focus(&fav_focus);
                    })
                    .child({
                        // 记录区域：点到外面时输入框失焦
                        canvas(
                            move |bounds: Bounds<Pixels>, _, _| {
                                *bounds_slot.lock() = Some(bounds);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                    })
                    .child(icon("icons/search.svg", 12., theme.text_tertiary))
                    .child(div().flex_1().min_w_0().child(fav_input))
                    .when(!filter.is_empty(), |el| {
                        el.child(
                            div()
                                .id("fav-search-clear")
                                .flex_shrink_0()
                                .size(px(18.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |s| s.bg(hover_bg))
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    clear_btn_input.update(cx, |i, cx| i.set_text("", cx));
                                    clear_btn_store.update(cx, |s, cx| {
                                        s.fav_filter.clear();
                                        s.refresh_fav_filter();
                                        cx.notify();
                                    });
                                })
                                .child(icon("icons/close.svg", 10., theme.text_tertiary)),
                        )
                    })
                    .into_any_element(),
            )
        } else {
            None
        };

        div()
            .flex()
            .flex_col()
            .h_full()
            .gap_4()
            .p_6()
            .when_some(stats_panel, |el, panel| el.child(panel))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_size(px(20.)).child(title))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child(if favorites && !filter.trim().is_empty() {
                                        format!("匹配 {} 首 / 共 {} 首", songs.len(), self.store.read(cx).favorites_source().len())
                                    } else {
                                        format!("共 {} 首", songs.len())
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when_some(search_box, |el, boxed| el.child(boxed))
                            .when(favorites && logged_in, |el| {
                                el.child(
                                    div()
                                        .id("lib-refresh")
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_3()
                                        .py(px(5.))
                                        .rounded_full()
                                        .bg(theme.bg_hover)
                                        .text_size(px(12.))
                                        .cursor_pointer()
                                        .hover(move |s| s.bg(theme.bg_panel))
                                        .on_click(move |_, _, cx| {
                                            crate::liked::load_remote_liked(
                                                &reload_store, true, cx,
                                            );
                                        })
                                        .child(icon(
                                            "icons/refresh.svg",
                                            11.,
                                            theme.text_secondary,
                                        ))
                                        .child("刷新"),
                                )
                            })
                            .child(
                                div()
                                    .id("lib-play-all")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_3()
                                    .py(px(5.))
                                    .rounded_full()
                                    .bg(theme.accent)
                                    .text_color(gpui::white())
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.88))
                                    .on_click({
                                        let store = store.clone();
                                        let source = source.clone();
                                        move |_, _, cx| play_from(&store, source.clone(), 0, cx)
                                    })
                                    .child(icon("icons/play.svg", 11., gpui::white()))
                                    .child("播放全部"),
                            )
                            .when(!favorites, |el| {
                                el.child(
                                    div()
                                        .id("lib-clear")
                                        .px_3()
                                        .py(px(5.))
                                        .rounded_full()
                                        .bg(theme.bg_hover)
                                        .text_size(px(12.))
                                        .cursor_pointer()
                                        .on_click(move |_, _, cx| {
                                            clear_store.update(cx, |s, cx| {
                                                s.library.clear_history();
                                                cx.notify();
                                            });
                                        })
                                        .child("清空"),
                                )
                            }),
                    ),
            )
            .when(songs.is_empty(), |el| {
                el.child(
                    div()
                        .py_10()
                        .text_size(px(12.))
                        .text_color(theme.text_tertiary)
                        .child(empty_hint),
                )
            })
            .when_some(list, |el, list| {
                el.child(div().flex_1().min_h_0().child(list))
            })
    }
}

// ==================== 音乐云盘 / 已购音乐 ====================

/// 加载音乐云盘（`mcloudservice` 加密接口）。
pub(crate) fn load_cloud(store: &Entity<AppStore>, force: bool, cx: &mut App) {
    let store_c = store.clone();
    {
        let s = store.read(cx);
        if !s.session.get().is_valid() {
            return;
        }
        if !force && (s.cloud_loaded || s.cloud_loading) {
            return;
        }
    }
    store.update(cx, |s, cx| {
        s.cloud_loading = true;
        cx.notify();
    });
    let client = store.read(cx).client.clone();
    let task = cx.background_executor().spawn(async move {
        run_tokio(async move {
            let mut all = Vec::new();
            for page in 1..=5 {
                match client.cloud_songs(page, 30).await {
                    Ok(batch) => {
                        let n = batch.len();
                        all.extend(batch);
                        if n < 30 {
                            break;
                        }
                    }
                    Err(e) => {
                        if page == 1 {
                            return Err(e);
                        }
                        log::warn!("cloud page{page} ERR {e}");
                        break;
                    }
                }
            }
            Ok::<Vec<echo_core::Song>, anyhow::Error>(all)
        })
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        let _ = store_c.update(cx, |s, cx| {
            s.cloud_loading = false;
            s.cloud_loaded = true;
            match res {
                Ok(songs) => {
                    log::info!("cloud: {} 首", songs.len());
                    s.cloud = Arc::new(songs);
                }
                Err(e) => {
                    log::warn!("cloud ERR {e}");
                    s.toast = format!("云盘加载失败：{e}");
                }
            }
            cx.notify();
        });
    })
    .detach();
}

/// 加载已购单曲 + 已购专辑。
pub(crate) fn load_purchased(store: &Entity<AppStore>, force: bool, cx: &mut App) {
    let store_c = store.clone();
    {
        let s = store.read(cx);
        if !s.session.get().is_valid() {
            return;
        }
        if !force && (s.purchased_loaded || s.purchased_loading) {
            return;
        }
    }
    store.update(cx, |s, cx| {
        s.purchased_loading = true;
        cx.notify();
    });
    let client = store.read(cx).client.clone();
    let task = cx.background_executor().spawn(async move {
        run_tokio(async move {
            let mut songs = Vec::new();
            for page in 1..=5 {
                match client.purchased_songs(page, 50).await {
                    Ok(batch) => {
                        let n = batch.len();
                        songs.extend(batch);
                        if n < 50 {
                            break;
                        }
                    }
                    Err(e) => {
                        log::warn!("purchased_songs page{page} ERR {e}");
                        break;
                    }
                }
            }
            let mut albums = Vec::new();
            for page in 1..=5 {
                match client.purchased_albums(page, 15).await {
                    Ok(batch) => {
                        let n = batch.len();
                        albums.extend(batch);
                        if n < 15 {
                            break;
                        }
                    }
                    Err(e) => {
                        log::warn!("purchased_albums page{page} ERR {e}");
                        break;
                    }
                }
            }
            (songs, albums)
        })
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (songs, albums) = task.await;
        let _ = store_c.update(cx, |s, cx| {
            s.purchased_loading = false;
            s.purchased_loaded = true;
            log::info!("purchased: {} 首 / {} 张专辑", songs.len(), albums.len());
            s.purchased_songs = Arc::new(songs);
            s.purchased_albums = Arc::new(albums);
            cx.notify();
        });
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

/// 频道标题（对齐酷狗：红心 / 小众 / 速览 / 电台 Radio）。
fn fm_mode_title(mode: &str) -> &'static str {
    match mode {
        "small" => "小众 Radio",
        "peak" => "速览 Radio",
        "radio" => "电台 Radio",
        _ => "红心 Radio",
    }
}

/// 推荐来源文案（对应 `song_pool_id` 0/1/2）。
fn fm_pool_label(pool: i64) -> &'static str {
    match pool {
        1 => "风格",
        2 => "探索",
        _ => "口味",
    }
}

/// 这一批里出现最多的几位歌手（最多 3 个），用于卡片副标题。
fn fm_top_artists(songs: &[echo_core::Song]) -> String {
    let mut names: Vec<String> = Vec::new();
    for s in songs {
        if s.artist.is_empty() {
            continue;
        }
        let first = s.artist.split('/').next().unwrap_or("").trim().to_string();
        if first.is_empty() || names.contains(&first) {
            continue;
        }
        names.push(first);
        if names.len() >= 3 {
            break;
        }
    }
    names.join("、")
}


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

// ==================== 免费听 ====================

pub(crate) fn load_free_listen(store: &Entity<AppStore>, force: bool, cx: &mut App) {
    let store_c = store.clone();
    {
        let s = store.read(cx);
        if !force && (s.free_listen_loaded || s.free_listen_loading) {
            return;
        }
    }
    store.update(cx, |s, cx| {
        s.free_listen_loading = true;
        cx.notify();
    });
    let client = store.read(cx).client.clone();
    let task = cx
        .background_executor()
        .spawn(async move { run_tokio(client.free_listen_songs(1, 30)) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        let _ = store_c.update(cx, |s, cx| {
            s.free_listen_loading = false;
            s.free_listen_loaded = true;
            match res {
                Ok(songs) => {
                    log::info!("free_listen: {} 首", songs.len());
                    s.free_listen = Arc::new(songs);
                }
                Err(e) => {
                    log::warn!("free_listen ERR {e}");
                    s.toast = format!("免费听加载失败：{e}");
                }
            }
            cx.notify();
        });
    })
    .detach();
}

// ==================== MV ====================

/// 查当前曲目的 MV（用于「查看 MV」入口）。
pub(crate) fn load_current_mv(store: &Entity<AppStore>, cx: &mut App) {
    let Some(song) = store.read(cx).player.current().cloned() else {
        return;
    };
    let aa = song.play_album_audio_id();
    if aa <= 0 {
        store.update(cx, |s, cx| {
            s.mv = None;
            s.toast = "这首歌没有 MV".into();
            cx.notify();
        });
        return;
    }
    store.update(cx, |s, cx| {
        s.mv = None;
        s.mv_loading = true;
        cx.notify();
    });
    let store_c = store.clone();
    let client = store.read(cx).client.clone();
    let task = cx
        .background_executor()
        .spawn(async move { run_tokio(client.audio_mv(aa)) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        let _ = store_c.update(cx, |s, cx| {
            s.mv_loading = false;
            match res {
                Ok(Some(mv)) => {
                    log::info!("mv: {} - {}", mv.title, mv.singer);
                    s.mv = Some(mv);
                }
                Ok(None) => {
                    s.mv = None;
                    s.toast = "这首歌没有 MV".into();
                }
                Err(e) => {
                    log::warn!("mv ERR {e}");
                    s.toast = format!("MV 查询失败：{e}");
                }
            }
            cx.notify();
        });
    })
    .detach();
}


/// 歌词一行：有逐字时间轴时把已经唱过的字点亮（用多个小 div 拼，不依赖富文本 API）。
fn render_lyric_text(line: &echo_core::LyricLine, word_index: Option<usize>, theme: &Theme) -> AnyElement {
    if line.words.is_empty() {
        return div().child(line.text.clone()).into_any_element();
    }
    div()
        .flex()
        .flex_wrap()
        .children(line.words.iter().enumerate().map(|(i, w)| {
            let done = word_index.map(|wi| i <= wi).unwrap_or(false);
            div()
                .text_color(if done { theme.accent } else { theme.text })
                .child(w.text.clone())
                .into_any_element()
        }))
        .into_any_element()
}


/// 拉取登录用户的自建歌单（侧栏「自建歌单」+ 歌单管理都用它）。
pub(crate) fn load_user_playlists(store: &Entity<AppStore>, cx: &mut App) {
    if !store.read(cx).session.get().is_valid() {
        return;
    }
    let store_c = store.clone();
    let client = store.read(cx).client.clone();
    let task = cx
        .background_executor()
        .spawn(async move { run_tokio(client.user_playlists(1, 100)) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        let _ = store_c.update(cx, |s, cx| {
            s.user_playlists_loaded = true;
            match res {
                Ok(list) => {
                    log::info!("user_playlists: {} 个", list.len());
                    s.user_playlists = list;
                }
                Err(e) => log::warn!("user_playlists ERR {e}"),
            }
            cx.notify();
        });
    })
    .detach();
}
