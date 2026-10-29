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

/// 播放页顶栏的图标按钮（小圆角 + hover 高亮）。
///
/// `occlude()` 很关键：顶栏按钮和整行拖动条是重叠的兄弟节点，
/// 不阻断命中测试的话点按钮会同时触发下面的「拖窗口」。
fn pp_icon_btn(
    id: &'static str,
    path: &'static str,
    size: f32,
    color: Hsla,
    hover_bg: Hsla,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(size))
        .rounded(px(7.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .occlude()
        .hover(move |el| el.bg(hover_bg))
        .on_click(move |_, window, cx| action(window, cx))
        .child(icon(path, size * 0.5, color))
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

/// 登录/验证码相关业务错误 → 用户看得懂的中文提示。
///
/// `ApiError::Business` 的 Display 是 `api status=0 error_code=20021 msg=` 这种调试串，
/// 直接丢给用户既看不懂也没法定位，这里按错误码换成人话（未知码仍带上错误码）。
fn login_error_text(err: &anyhow::Error, prefix: &str) -> String {
    match echo_api::business_code(err) {
        Some(20021) => "验证码错误或已失效，请重新获取".into(),
        Some(20028) => "当前设备被风控拦截，请稍后再试，或改用扫码登录".into(),
        Some(20006) | Some(20017) | Some(20018) => "登录态已失效，请重新登录".into(),
        Some(code) => format!("{prefix}（错误码 {code}）"),
        None => format!("{prefix}：{err}"),
    }
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
                Err(e) => {
                    // 20028 = 风控「请先通过验证」：接口侧拿 dfid 判的，换设备也没用
                    s.login.error = login_error_text(&e, "验证码发送失败");
                }
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
                let text = login_error_text(&e, "登录失败");
                let _ = store.update(cx, |s, cx| {
                    s.login.busy = false;
                    s.login.error = text;
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
                let text = login_error_text(&e, "登录失败");
                let _ = store.update(cx, |s, cx| {
                    s.login.busy = false;
                    s.login.error = text;
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
                                    .h(px(32.))
                                    .px_2()
                                    .mr_1()
                                    .flex_shrink_0()
                                    .rounded(px(9.))
                                    .border_1()
                                    .border_color(theme.border)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(11.5))
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
        let songs = self.store.read(cx).fm.clone();
        // 只有「首次、还没有任何内容」才整页转圈。
        // 切推荐来源（口味/风格/探索）或切频道时保留旧内容原地等新数据，
        // 否则整页白闪一下，观感上就像刷新了页面。
        if self.store.read(cx).fm_loading && songs.is_empty() {
            return centered_spinner(theme).into_any_element();
        }
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
            // 封面上的控制按钮：点它就是「播这首 / 暂停继续」
            let cover_store = self.store.clone();
            let cover_hash = song.hash.clone();
            let this_playing = playing_fm
                && current.as_ref().map(|c| c.hash == song.hash).unwrap_or(false)
                && self.store.read(cx).player.playing;
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
                        .relative()
                        .w(px(136.))
                        .h(px(136.))
                        .flex_shrink_0()
                        .rounded(px(12.))
                        .overflow_hidden()
                        .bg(theme.bg_hover)
                        .when(!cover.is_empty(), |el| {
                            el.child(
                                img(SharedUri::from(cover.clone()))
                                    .absolute()
                                    .size_full()
                                    .object_fit(ObjectFit::Cover),
                            )
                        })
                        // 播放控制**叠在封面图上**：封面走绝对定位铺底，
                        // 控制层走正常流 + size_full 居中（gpui 0.2.2 里
                        // 「absolute 容器 + flex 居中子元素」实测不居中，反着来才稳）
                        .child(
                            div()
                                .id("fm-now-toggle")
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|s| s.bg(gpui::black().opacity(0.14)))
                                .on_click(move |_, _, cx| {
                                    let same = {
                                        let st = cover_store.read(cx);
                                        st.player.fm_queue
                                            && st
                                                .player
                                                .current()
                                                .map(|c| c.hash == cover_hash)
                                                .unwrap_or(false)
                                    };
                                    if same {
                                        crate::ui::toggle_play(&cover_store, cx);
                                    } else {
                                        crate::ui::play_from(
                                            &cover_store,
                                            crate::ui::PlaySource::Fm,
                                            0,
                                            cx,
                                        );
                                    }
                                })
                                .child(
                                    div()
                                        .w(px(44.))
                                        .h(px(44.))
                                        .rounded_full()
                                        .bg(gpui::white().opacity(0.92))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(icon(
                                            if this_playing {
                                                "icons/pause.svg"
                                            } else {
                                                "icons/play.svg"
                                            },
                                            18.,
                                            theme.accent,
                                        )),
                                ),
                        ),
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
                            .child(icon("icons/radio.svg", 34., gpui::white())),
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

    /// 播放页（沉浸式）：铺满整个窗口，盖住侧栏 / 标题栏 / 播放条。
    ///
    /// 布局参考 QQ 音乐「正在播放」整页：
    ///
    /// ```text
    /// ┌───────────────────────────────────────────────────────────┐
    /// │ ⌄                                               ⤢ ─ □ ✕   │ ← 左：收起播放页；右：全屏/最小化/最大化/关闭
    /// │                                                           │
    /// │        ┌──────────┐                       当前行（最亮）  │
    /// │        │   封面    │                       上一行（暗）   │
    /// │        └──────────┘                       下一行（更暗）  │
    /// │            歌名                                           │
    /// │            歌手                                           │
    /// │        ▬▬▬▬▬▬▬▬▬                                          │
    /// │          ◀   ▶   ▶                                        │
    /// └───────────────────────────────────────────────────────────┘
    /// ```
    ///
    /// 分层要点：
    /// - 根容器只负责「铺满 + 定位」，**不带 padding**：背景层要真正整窗铺满；
    /// - 中间 `#pp-content` 才带 padding：左列封面/信息/控制，右列歌词；
    /// - 顶部拖动条 + 顶栏按钮在最上层：播放页没有标题栏，拖窗口和窗口按钮只能自己做。
    pub(crate) fn render_lyric(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // 「最大化/还原」要按系统真实状态出图标（Win+Up、双击、贴边吸附都算）
        let maximized = crate::win::is_maximized(window);
        let (lyric, loading, position, playing, position_at, duration, song, play_mode) = {
            let s = self.store.read(cx);
            (
                s.lyric.clone(),
                s.lyric_loading,
                s.player.position,
                s.player.playing,
                s.position_at,
                s.player.duration,
                s.player.current().cloned(),
                s.player.play_mode,
            )
        };
        let cover = song.as_ref().map(|c| c.cover_url.clone()).unwrap_or_default();
        // 264pt 的封面用 120 的缩略图拉上去是糊的，这里换高清版。
        // 没有 hd 字段（老快照/老数据）时不能直接回退到 120 的缩略图，而是把
        // 缩略图 URL 里的尺寸段改写成 480 —— 同一个 CDN 路径换个数字就是高清图。
        let cover_hd = song
            .as_ref()
            .map(|c| {
                if c.cover_url_hd.is_empty() {
                    echo_api::normalize::normalize_cover(&c.cover_url, 480)
                } else {
                    echo_api::normalize::normalize_cover(&c.cover_url_hd, 480)
                }
            })
            .unwrap_or_default();
        let title = song.as_ref().map(|c| c.title.clone()).unwrap_or_default();
        let artist = song.as_ref().map(|c| c.artist.clone()).unwrap_or_default();
        let fav = song
            .as_ref()
            .map(|c| {
                if self.store.read(cx).session.get().userid != 0 {
                    crate::liked::is_remote_liked(&self.store.read(cx), c)
                } else {
                    self.store.read(cx).library.is_favorite(c)
                }
            })
            .unwrap_or(false);

        // ===== 背景层：封面高斯模糊铺满 + 压暗 =====
        //
        // gpui 0.2.2 没有元素级模糊，模糊图由 `imgcache::blurred_cover` 在后台把封面
        // 缩小 + 高斯模糊后给出（按 URL 缓存）。它第一帧还没算完时会返回 None，
        // 这时先用原图顶一下，算完会通知重绘自动换成模糊版。
        // 背景用**高清封面**做底：120 的缩略图放大到整窗只有几块色斑，
        // 连压暗之后都发灰；高清图缩下去再模糊，颜色才立得住。
        let blurred = crate::imgcache::blurred_cover(&cover_hd, window, cx);
        // 背景层四周各外扩 10px：模糊图的边缘无论如何都比中间淡一点，正好贴边
        // 铺会出现「两边一条黑」；外扩之后多出来的部分被 `#player-page` 的
        // overflow_hidden 裁掉，边缘就干净了。
        let mut bg_layer = div()
            .id("pp-bg")
            .absolute()
            .top(px(-10.))
            .left(px(-10.))
            .right(px(-10.))
            .bottom(px(-10.));
        bg_layer = match blurred {
            Some(b) => bg_layer.child(
                img(b)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            ),
            None if !cover_hd.is_empty() => bg_layer.child(
                img(SharedUri::from(cover_hd.clone()))
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .opacity(0.45),
            ),
            None => bg_layer,
        };
        // 模糊后的背景比原图「实」得多，压暗可以轻一点，颜色反而更出得来
        let bg_layer = bg_layer.bg(gpui::black().opacity(0.62));

        // ===== 时间轴 =====
        // 位置外推：引擎的 Position 事件只有 4Hz，逐字动画要按帧连续推进，
        // 不外推的话进度是 250ms 一跳的阶梯，"逐渐变化"就成了"整字跳变"。
        let pos = if playing {
            position + position_at.elapsed().as_secs_f64().min(1.0)
        } else {
            position
        };
        let (cur, offset_ms, lines) = match lyric {
            Some(mut l) => {
                let cur = l.index_at(pos);
                let lines = std::mem::take(&mut l.lines);
                (cur, l.offset_ms, Some(lines))
            }
            None => (None, 0, None),
        };
        let store = self.store.clone();
        let theme = self.store.read(cx).theme();
        let accent = theme.accent;

        // 当前正在唱的那句**始终居中**，而且换行时是**弹性滚上去**（不是瞬移）。
        //
        // 为什么不用 `scroll_to_item(Center)`：
        // - 它是**非严格**滚动——目标行只要「还看得见」就完全不动，于是当前句
        //   会一路漂到视口底边才被拉一次，看着就是「没居中」；
        // - 换成 strict 版又是硬跳，没有弹性。
        // 所以这里直接用 uniform list 的底层 `ScrollHandle` 自己做缓动：
        // 目标偏移 = 让当前行中心落在视口中心，逐帧 ease-out-back 逼近，
        // 顺带 `request_animation_frame` 驱动（暂停播放也能滚完）。
        //
        // 用户手动滚过之后让位 5 秒（这段时间不抢鼠标），5 秒没动再弹性回弹到当前句。
        const LYRIC_IDLE: std::time::Duration = std::time::Duration::from_secs(5);
        if let Some(c) = cur {
            let _ = c;
            let base = crate::ui::list_scroll_handle(&self.lyric_scroll);
            let real_off = -f32::from(base.offset().y);
            // 程序没在动画、真实偏移却和上次自己设的不一样 → 用户滚的
            if self.lyric_scroll_anim.is_none()
                && (real_off - self.lyric_auto_off).abs() > 4.0
            {
                self.lyric_user_scroll_at = Some(std::time::Instant::now());
                self.lyric_restore_pending = true;
                self.lyric_auto_off = real_off;
            }
        }
        let user_browsing = self
            .lyric_user_scroll_at
            .map(|t| t.elapsed() < LYRIC_IDLE)
            .unwrap_or(false);
        if user_browsing {
            // 正在看别处：停掉自动滚动，别跟用户抢
            self.lyric_scroll_anim = None;
            // 记住「当前句」，等看完了再对齐
            self.lyric_follow = cur;
        } else if let Some(c) = cur {
            const ROW_H: f32 = 76.;
            const SCROLL_SECS: f32 = 0.46;
            let base = crate::ui::list_scroll_handle(&self.lyric_scroll);
            let vp_h = f32::from(base.bounds().size.height);
            let max_off = f32::from(base.max_offset().height);
            // 目标：当前行中心 → 视口中心（内容不够高时自动夹到 0）
            let want =
                (c as f32 * ROW_H + ROW_H * 0.5 - vp_h * 0.5).clamp(0.0, max_off.max(0.0));

            // 换行，或者用户滚完 5 秒要回弹，都走同一段动画
            if self.lyric_follow != Some(c) || self.lyric_restore_pending {
                self.lyric_follow = Some(c);
                self.lyric_restore_pending = false;
                // 列表还没排完版（vp_h == 0）时先不启动，下一帧再来
                if vp_h > 0.0 {
                    let from = -f32::from(base.offset().y);
                    self.lyric_scroll_anim = Some((from, want, std::time::Instant::now()));
                }
            }

            if let Some((from, to0, start)) = self.lyric_scroll_anim {
                let t = (start.elapsed().as_secs_f32() / SCROLL_SECS).clamp(0.0, 1.0);
                // ease-out-back：冲过头一点点再回弹，就是「弹性」的手感
                let e = {
                    let c1 = 1.70158f32;
                    let c3 = c1 + 1.0;
                    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
                };
                // 视口/列表高度这一帧才量准，用当帧目标值（不用启动时那份）
                let to = if max_off > 0.0 { want } else { to0 };
                let off = from + (to - from) * e;
                base.set_offset(point(px(0.), px(-off)));
                self.lyric_auto_off = off;
                if t >= 1.0 {
                    self.lyric_scroll_anim = None;
                } else {
                    window.request_animation_frame();
                }
            }
        }

        // ===== 左列：封面 + 歌名 + 歌手 + 进度 + 播放控制 =====
        let controls = {
            let slot = self.store.read(cx).progress_bounds_slot.clone();
            let ratio = if duration > 0.01 {
                (position / duration).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            let st_p = store.clone();
            let st_prev = store.clone();
            let st_play = store.clone();
            let st_next = store.clone();
            let st_mode = store.clone();
            let st_fav = store.clone();
            let fav_song = song.clone();
            let mode_icon = match play_mode {
                echo_core::PlayMode::Sequential => "icons/queue.svg",
                echo_core::PlayMode::List => "icons/repeat.svg",
                echo_core::PlayMode::Random => "icons/shuffle.svg",
                echo_core::PlayMode::Single => "icons/repeat-one.svg",
            };
            div()
                .id("pp-controls")
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                // 进度条：canvas 绘制 + bounds 记录（与主播放条共用槽位）
                .child(
                    div()
                        .id("pp-progress")
                        .w_full()
                        .h(px(14.))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .child(
                            canvas(
                                move |bounds: Bounds<Pixels>, _, _| {
                                    *slot.lock() = Some(bounds);
                                },
                                move |bounds, _, window, _| {
                                    window.paint_quad(fill(bounds, gpui::white().opacity(0.25)));
                                    let w = bounds.size.width * ratio;
                                    if w > px(0.) {
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                bounds.origin,
                                                size(w, bounds.size.height),
                                            ),
                                            gpui::white(),
                                        ));
                                    }
                                },
                            )
                            .h(px(4.))
                            .w_full()
                            .rounded_full(),
                        )
                        .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, _, cx| {
                            let b = st_p.read(cx).progress_bounds_slot.lock().clone();
                            if let Some(b) = b {
                                let r = f32::from(ev.position.x - b.left()).max(0.0)
                                    / f32::from(b.size.width).max(1.0);
                                let target =
                                    (r as f64).clamp(0.0, 1.0) * st_p.read(cx).player.duration;
                                st_p.update(cx, |s, cx| {
                                    s.engine.seek(target);
                                    s.player.position = target;
                                    s.position_at = std::time::Instant::now();
                                    cx.notify();
                                });
                            }
                        }),
                )
                // 时间行
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .w_full()
                        .text_size(px(10.))
                        .text_color(gpui::white().opacity(0.6))
                        .child(div().child(crate::ui::fmt_time(position)))
                        .child(div().child(crate::ui::fmt_time(duration))),
                )
                // 歌曲模式 / 上一首 / 播放暂停 / 下一首 / 收藏
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_4()
                        // 歌曲模式（顺序/列表循环/随机/单曲循环）
                        .child(
                            div()
                                .id("pp-mode")
                                .size(px(30.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |el| el.bg(gpui::white().opacity(0.12)))
                                .on_click(move |_, _, cx| crate::ui::cycle_play_mode(&st_mode, cx))
                                .child(icon(mode_icon, 14., gpui::white().opacity(0.82))),
                        )
                        .child(
                            div()
                                .id("pp-prev")
                                .size(px(34.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .bg(gpui::white().opacity(0.12))
                                .hover(move |el| el.bg(gpui::white().opacity(0.22)))
                                .on_click(move |_, _, cx| crate::ui::prev_track(&st_prev, cx))
                                .child(icon("icons/prev.svg", 14., gpui::white().opacity(0.9))),
                        )
                        .child(
                            div()
                                .id("pp-play")
                                .size(px(46.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .bg(gpui::white())
                                .hover(move |el| el.opacity(0.85))
                                .on_click(move |_, _, cx| crate::ui::toggle_play(&st_play, cx))
                                .child(icon(
                                    if playing { "icons/pause.svg" } else { "icons/play.svg" },
                                    17.,
                                    gpui::black(),
                                )),
                        )
                        .child(
                            div()
                                .id("pp-next")
                                .size(px(34.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .bg(gpui::white().opacity(0.12))
                                .hover(move |el| el.bg(gpui::white().opacity(0.22)))
                                .on_click(move |_, _, cx| crate::ui::next_track(&st_next, false, cx))
                                .child(icon("icons/next.svg", 14., gpui::white().opacity(0.9))),
                        )
                        // 收藏
                        .child(
                            div()
                                .id("pp-fav")
                                .size(px(30.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |el| el.bg(gpui::white().opacity(0.12)))
                                .on_click(move |_, _, cx| {
                                    if let Some(song) = fav_song.clone() {
                                        crate::liked::toggle_like(&st_fav, &song, cx);
                                    }
                                })
                                .child(icon(
                                    if fav { "icons/heart-filled.svg" } else { "icons/heart.svg" },
                                    14.,
                                    if fav { accent } else { gpui::white().opacity(0.82) },
                                )),
                        ),
                )
        };

        let cover_col = div()
            .id("pp-cover-col")
            .w(px(300.))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                div()
                    .w(px(264.))
                    .h(px(264.))
                    .flex_shrink_0()
                    .rounded(px(12.))
                    .overflow_hidden()
                    .bg(gpui::black().opacity(0.35))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!cover_hd.is_empty(), |el| {
                        el.child(
                            img(SharedUri::from(cover_hd.clone()))
                                .size_full()
                                .rounded(px(12.))
                                .object_fit(ObjectFit::Cover),
                        )
                    })
                    .when(cover.is_empty(), |el| {
                        el.child(icon("icons/music-note.svg", 44., gpui::white().opacity(0.4)))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(gpui::white())
                            .truncate()
                            .max_w(px(288.))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(gpui::white().opacity(0.6))
                            .truncate()
                            .max_w(px(288.))
                            .child(artist),
                    ),
            )
            .child(div().w(px(264.)).child(controls));

        // ===== 右列：歌词（左对齐、当前行居中跟随、逐字渐变）=====
        let lyric_col: AnyElement = match lines {
            None => div()
                .w(px(440.))
                .flex_shrink_0()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(gpui::white().opacity(0.45))
                .child(
                    div()
                        .text_size(px(14.))
                        .child(if loading { "歌词加载中…" } else { "暂无歌词" }),
                )
                .into_any_element(),
            Some(lines) => {
                let count = lines.len();
                let lines = Arc::new(lines);
                // 未唱：半透明白（灰）；已唱：主题色。两色差距够大，逐字渐变才看得见
                // （早先「灰白 → 纯白」几乎无差别，等于没有效果）
                let cfg = crate::amll::AmllConfig::new(
                    gpui::white().opacity(0.34),
                    accent,
                    px(26.),
                );
                let list = gpui::uniform_list(
                    "pp-lyric-list",
                    count,
                    move |range, _window, _cx: &mut App| {
                        range
                            .map(|i| {
                                let line = &lines[i];
                                // d<0：已经唱过（在上方）；d>0：还没唱到（在下方）
                                let d = cur.map(|c| i as i64 - c as i64).unwrap_or(9);
                                let active = d == 0;
                                let opacity = if active {
                                    1.0
                                } else if d < 0 {
                                    (0.52 + d as f32 * 0.07).max(0.22)
                                } else {
                                    (0.42 - d as f32 * 0.05).max(0.18)
                                };
                                let text: AnyElement = if active && !line.words.is_empty() {
                                    // 逐字注水 + 换行弹性（统一入口，和桌面歌词共用）
                                    let t_in_line = pos * 1000.0 + offset_ms as f64
                                        - line.time_ms as f64;
                                    crate::amll::current_line(
                                        line,
                                        t_in_line,
                                        &cfg,
                                        &format!("ppl{i}"),
                                        i,
                                        Some(px(440.)),
                                    )
                                } else {
                                    div()
                                        .text_color(gpui::white().opacity(opacity))
                                        .child(line.text.clone())
                                        .into_any_element()
                                };
                                div().h(px(76.)).flex().items_center().child(
                                    div()
                                        // 给确定宽度：文字才会折行而不是顶出列宽被裁
                                        .w(px(440.))
                                        .text_size(px(if active { 26. } else { 21. }))
                                        .when(active, |el| {
                                            el.font_weight(gpui::FontWeight::BOLD)
                                        })
                                        .child(text),
                                )
                            })
                            .collect::<Vec<_>>()
                    },
                )
                .track_scroll(self.lyric_scroll.clone())
                .size_full();
                div()
                    .w(px(440.))
                    .flex_shrink_0()
                    .h_full()
                    .child(list)
                    .into_any_element()
            }
        };

        // ===== 顶栏：左「收起播放页」/ 右「全屏 · 最小化 · 最大化 · 关闭」=====
        let hover_bg = gpui::white().opacity(0.16);
        let icon_color = gpui::white().opacity(0.82);
        let st_collapse = store.clone();
        let top_bar = div()
            .id("pp-top")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(44.))
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            // 收起：退出播放页回主界面（Esc 同效）
            .child(pp_icon_btn(
                "pp-collapse",
                "icons/chevron-down.svg",
                26.,
                icon_color,
                hover_bg,
                move |_, cx| crate::ui::navigate(&st_collapse, Page::Home, cx),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(pp_icon_btn(
                        "pp-fullscreen",
                        "icons/expand.svg",
                        26.,
                        icon_color,
                        hover_bg,
                        |window, _| window.toggle_fullscreen(),
                    ))
                    .child(pp_icon_btn(
                        "pp-minimize",
                        "icons/minimize.svg",
                        26.,
                        icon_color,
                        hover_bg,
                        |window, _| window.minimize_window(),
                    ))
                    .child(pp_icon_btn(
                        "pp-maximize",
                        if maximized {
                            "icons/restore.svg"
                        } else {
                            "icons/maximize.svg"
                        },
                        26.,
                        icon_color,
                        hover_bg,
                        |window, cx| crate::ui::spawn_toggle_maximize(window.window_handle(), cx),
                    ))
                    // 关闭：与标题栏一致，只隐藏窗口，App 继续在托盘里放歌
                    .child(pp_icon_btn(
                        "pp-close",
                        "icons/close.svg",
                        26.,
                        icon_color,
                        gpui::red().opacity(0.75),
                        |window, _| crate::win::hide_window(window),
                    )),
            );

        div()
            .id("player-page")
            .size_full()
            .relative()
            .overflow_hidden()
            // 键盘：Esc 退出播放页；空格 播放/暂停（与主界面一致）
            .track_focus(&self.root_focus)
            .on_key_down(cx.listener(
                |root: &mut Root, ev: &gpui::KeyDownEvent, _window, cx| {
                    match ev.keystroke.key.as_str() {
                        "escape" => {
                            crate::ui::navigate(&root.store.clone(), crate::state::Page::Home, cx);
                        }
                        "space" if ev.keystroke.modifiers.number_of_modifiers() == 0 => {
                            crate::ui::toggle_play(&root.store.clone(), cx);
                            cx.stop_propagation();
                        }
                        // 播放页里直接调速：`[` 变慢 / `]` 变快（0.5~2.0，含 0.75x）。
                        // 慢速是为了能看清逐字填充的推进过程。
                        "[" => {
                            crate::ui::step_speed(&root.store.clone(), -1, cx);
                            cx.stop_propagation();
                        }
                        "]" => {
                            crate::ui::step_speed(&root.store.clone(), 1, cx);
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                },
            ))
            // ① 背景层：根容器的直接子节点（内容层的兄弟），整窗铺满不留边
            .child(bg_layer)
            // ② 内容层：唯一带 padding 的一层
            .child(
                div()
                    .id("pp-content")
                    .size_full()
                    .flex()
                    .gap_8()
                    .px(px(56.))
                    .pt(px(56.))
                    .pb(px(32.))
                    .child(cover_col)
                    .child(div().flex_1())
                    .child(lyric_col),
            )
            // ③ 顶部拖动条：整行可拖（双击最大化），与主标题栏同款机制。
            //    必须在顶栏按钮之前，命中测试才让按钮优先。
            .child(
                div()
                    .id("pp-drag")
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(44.))
                    // 不用 window_control_area：命中成功时它会让系统先起一个移动循环，
                    // 和这里补发的 WM_NCLBUTTONDOWN 叠成两个循环 → 拖动掉帧；命中失败时
                    // 又退化成普通点击 → 拖不动。只走 win::start_drag。
                    .on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, cx| {
                        if ev.click_count == 2 {
                            crate::ui::spawn_toggle_maximize(window.window_handle(), cx);
                        } else {
                            crate::win::start_drag(window);
                        }
                    }),
            )
            // ④ 顶栏按钮（排在拖动条之后，命中测试优先）
            .child(top_bar)
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
                .on_click(move |ev: &gpui::ClickEvent, _, cx| {
                    let origin = if let gpui::ClickEvent::Mouse(m) = ev {
                        Some(m.down.position)
                    } else {
                        None
                    };
                    crate::ui::set_theme_mode(&st, mode, origin, cx);
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

        // 背景特效（Windows Mica / Acrylic，其他系统自动降级磨砂或关闭）
        let backdrop_btn = |label: &'static str,
                            mode: echo_core::BackdropMode,
                            cur: echo_core::BackdropMode,
                            st: Entity<AppStore>| {
            let active = mode == cur;
            div()
                .id(SharedString::from(format!("backdrop-{label}")))
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
                    crate::ui::set_backdrop_mode(&st, mode, cx);
                })
                .child(label)
                .into_any_element()
        };
        let s5 = store.clone();
        let backdrop_row = div()
            .flex()
            .gap_2()
            .child(backdrop_btn(
                "跟随系统",
                echo_core::BackdropMode::System,
                settings.backdrop,
                s5.clone(),
            ))
            .child(backdrop_btn(
                "云母",
                echo_core::BackdropMode::Mica,
                settings.backdrop,
                s5.clone(),
            ))
            .child(backdrop_btn(
                "亚克力",
                echo_core::BackdropMode::Acrylic,
                settings.backdrop,
                s5.clone(),
            ))
            .child(backdrop_btn(
                "关闭",
                echo_core::BackdropMode::Off,
                settings.backdrop,
                s5,
            ));

        // 染色浓度：同一种系统材质，染色越浓越像 Win11 开始菜单
        // （底层确实是亚克力，但几乎不透明、只泛一点壁纸的色调）。
        let dense_chip = |label: &'static str,
                          dense: bool,
                          cur: bool,
                          st: Entity<AppStore>| {
            div()
                .id(SharedString::from(format!("backdrop-dense-{label}")))
                .px_4()
                .py_2()
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(12.5))
                .when(cur, |el| el.bg(theme.accent).text_color(gpui::white()))
                .when(!cur, |el| {
                    el.bg(theme.bg_hover).text_color(theme.text_secondary)
                })
                .on_click(move |_, _, cx| {
                    st.update(cx, |s, cx| {
                        s.settings.update(|st| st.backdrop_dense = dense);
                        s.toast = if dense {
                            "材质染色：浓（开始菜单风格）".into()
                        } else {
                            "材质染色：通透".into()
                        };
                        cx.notify();
                    });
                })
                .child(label)
                .into_any_element()
        };
        let s6 = store.clone();
        let dense_row = div()
            .flex()
            .gap_2()
            .child(dense_chip(
                "通透",
                false,
                !settings.backdrop_dense,
                s6.clone(),
            ))
            .child(dense_chip("浓染色（开始菜单风格）", true, settings.backdrop_dense, s6));

        // 桌面歌词字号 chips（2×/3×/4×/5×）：收集成 Vec 再 children，
        // 免得在 div 链里套闭包导致括号层级太深
        let lyric_scale_row = {
            let cur = settings.lyric_scale;
            let mut chips: Vec<AnyElement> = Vec::new();
            for n in 2..=5u32 {
                let st = store.clone();
                let active = (cur - n as f32).abs() < 0.01;
                chips.push(
                    div()
                        .id(SharedString::from(format!("lyric-scale-{n}")))
                        .px_3()
                        .py(px(4.))
                        .rounded(px(6.))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .when(active, |el| el.bg(theme.accent).text_color(gpui::white()))
                        .when(!active, |el| {
                            el.bg(theme.bg_hover).text_color(theme.text_secondary)
                        })
                        .on_click(move |_, _, cx| {
                            st.update(cx, |s, cx| {
                                s.settings.update(|st| st.lyric_scale = n as f32);
                                cx.notify();
                            });
                            // 只改字号不会触发浮窗重绘（主窗口改设置时下一帧才
                            // refresh），这里立即刷一次，让窗口马上按新字号改尺寸
                            cx.defer(move |cx| {
                                crate::desktop::refresh_float_windows(cx);
                            });
                        })
                        .child(format!("{n}×"))
                        .into_any_element(),
                );
            }
            div().flex().gap_2().children(chips)
        };

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
                    .child(div().flex().gap_2().children(color_dots))
                    .child(div().text_size(px(12.)).text_color(theme.text_secondary).child("背景特效"))
                    .child(backdrop_row)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.text_secondary)
                            .child("染色浓度"),
                    )
                    .child(dense_row)
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.text_tertiary)
                            .child(
                                "「浓染色」= Win11 开始菜单的观感：底层仍是系统材质，\
                                 但染色很浓、只泛一点壁纸的色调；「通透」能明显看出壁纸被模糊透出来。\
                                 云母 / 亚克力需要 Windows 11 22H2 及以上；\
                                 Win10 及更早系统自动降级为磨砂，无磨砂能力时回退普通不透明背景。",
                            ),
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
                    )
                    // 桌面歌词字号（2–5 倍，基准 24px；默认 2 倍）
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
                                    .child("歌词字号"),
                            )
                            .child(lyric_scale_row)
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.text_tertiary)
                                    .child("桌面歌词放大倍数，窗口会自动跟着变"),
                            ),
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
                                                            // 运行中直接切换输出流（第三期：不再重启生效）
                                                            s.engine.switch_device(
                                                                if value.is_empty() { None } else { Some(value.as_str()) },
                                                            );
                                                            s.settings.update(|st| {
                                                                st.output_device = value.clone()
                                                            });
                                                            s.toast = if value.is_empty() {
                                                                "输出设备：系统默认".into()
                                                            } else {
                                                                format!("输出设备：{value}")
                                                            };
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
                                        // 随机模式下从随机一首开始（不要固定第一首）
                                        move |_, _, cx| {
                                            crate::ui::play_all(&store, source.clone(), cx)
                                        }
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


/// 私人 FM：记一首「已经播放过」。
///
/// 只打点，不在这里触发换歌 —— 换批的时机是**切歌**（见 [`fm_round_finished`] 与
/// `ui::next_track` / `PlayerEvent::Ended`）。之所以在**开始播放**（解析地址）时就记，
/// 是因为用户要求「不必每首都播完，听过就算」。
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
