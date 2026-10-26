//! 主界面：布局（侧栏 / 顶栏 / 内容区 / 播放条）与交互动作。
//!
//! 布局对齐原项目 `layouts/MainLayout.vue + Sidebar.vue + TitleBar.vue`、
//! 首页对齐 `views/Home.vue`、播放条对齐 `components/PlayerBar.vue`。

use crate::state::{AppStore, Page};
use crate::theme::{CARD_RADIUS, CONTROL_RADIUS, COVER_RADIUS};
use crate::Root;
use echo_core::Song;
use chrono::Timelike;
use gpui::prelude::*;
use gpui::*;
use std::sync::{Arc, OnceLock};

/// tokio 运行时（reqwest(async) 需要 tokio reactor；gpui 是 smol）。
static TOKIO: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

pub fn init_tokio() {
    let _ = TOKIO.set(tokio::runtime::Runtime::new().unwrap());
}

/// 在 gpui 的 smol 执行器里安全地驱动 reqwest(async) 请求。
pub fn run_tokio<F: std::future::Future>(fut: F) -> F::Output {
    TOKIO.get().expect("tokio runtime not initialized").block_on(fut)
}

// ==================== 通用小部件 ====================

pub(crate) fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// 居中旋转加载动画：12 个圆点围成一圈，按墙钟时间推进「头部」位置，
/// 尾部渐隐形成追逐旋转效果。配合 `any_loading` 重绘泵可持续旋转。
/// 注：gpui 0.2.2 的 `paint_path` 在 Windows 后端不渲染，故只用 `paint_quad` 绘制。
pub(crate) fn loading_spinner(size: f32, color: Hsla) -> impl IntoElement {
    canvas(
        move |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let w = bounds.size.width;
            let h = bounds.size.height;
            let cx = bounds.origin.x + w / 2.0;
            let cy = bounds.origin.y + h / 2.0;
            let radius = (w.min(h) / 2.0) - px(4.);
            if radius <= px(0.) {
                return;
            }
            let dots = 12u32;
            let dot = px(4.);
            let half = dot / 2.0;
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f32();
            // 头部沿环推进，形成旋转
            let head = (t * 1.4 * dots as f32).rem_euclid(dots as f32);
            for i in 0..dots {
                let a = std::f32::consts::TAU * (i as f32 / dots as f32)
                    - std::f32::consts::FRAC_PI_2;
                let d = point(cx + radius * a.cos(), cy + radius * a.sin());
                // 与头部的环形距离：0 = 头部最亮，越远越淡（拖尾）
                let rel = (i as f32 - head).rem_euclid(dots as f32) / dots as f32;
                let mut c = color;
                c.a = color.a * (1.0 - rel * 0.88);
                window.paint_quad(
                    fill(
                        Bounds::new(point(d.x - half, d.y - half), gpui::size(dot, dot)),
                        c,
                    )
                    .corner_radii(half),
                );
            }
        },
    )
    .size(px(size))
}

/// 居中铺满（垂直 + 水平居中）的加载动画，用于列表加载态。
/// 作为内容区的直接子节点时（overflow_y_scroll 容器内）可填充并居中。
/// 返回具体 `Div`（而非 `impl IntoElement`），以便调用方将其与主体 `Div()`
/// 放在同一 `impl IntoElement` 返回分支里而不产生类型不匹配。
pub(crate) fn centered_spinner(theme: crate::theme::Theme) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .child(loading_spinner(40., theme.accent))
}

/// 自绘滚动条 thumb 浮层（gpui 0.2.2 不绘制滚动条，`scrollbar_width` 只预留布局空间）。
/// 用 canvas 依据 ScrollHandle 实时状态画 thumb；内容不可滚动时不绘制。
/// 支持按住 thumb 拖拽（`ScrollHandle::set_offset`），点击轨道直接跳转。
/// 悬浮在滚动容器右缘：外层包一个 `.relative()` 容器，叠加 `absolute` 定位的窄条即可。
pub(crate) fn scrollbar_overlay(id: &'static str, handle: &ScrollHandle) -> impl IntoElement {
    let paint_handle = handle.clone();
    let down_handle = handle.clone();
    let move_handle = handle.clone();
    // 拖拽起点：(按下时的鼠标 y, 按下时的 offset.y, 每像素对应的滚动量)
    let drag: std::sync::Arc<parking_lot::Mutex<Option<(f32, f32, f32)>>> =
        std::sync::Arc::new(parking_lot::Mutex::new(None));
    let down_drag = drag.clone();
    let move_drag = drag.clone();
    let up_drag = drag.clone();

    let bar = canvas(
        move |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let viewport_h = paint_handle.bounds().size.height;
            let max_off = paint_handle.max_offset().height;
            if max_off <= px(0.) || viewport_h <= px(0.) {
                return;
            }
            let track_h = bounds.size.height;
            let ratio = viewport_h / (viewport_h + max_off);
            let thumb_h = (track_h * ratio).max(px(30.));
            let progress = (-paint_handle.offset().y / max_off).clamp(0.0, 1.0);
            let thumb_y = bounds.origin.y + (track_h - thumb_h) * progress;
            // thumb 画在浮层右侧 5px 内，浮层本身留 10px 便于点中
            let thumb_w = px(5.);
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(bounds.origin.x + bounds.size.width - thumb_w, thumb_y),
                        gpui::size(thumb_w, thumb_h),
                    ),
                    hsla(0., 0., 0., 0.20),
                )
                .corner_radii(px(3.)),
            );
        },
    )
    .size_full();

    div()
        .id(id)
        .size_full()
        .cursor_pointer()
        // 按下：记录起点；点在 thumb 外则先按比例跳转过去
        .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
            let max_off = down_handle.max_offset().height;
            if max_off <= px(0.) {
                return;
            }
            let viewport_h = down_handle.bounds().size.height;
            let track_h = (viewport_h - px(8.)).max(px(1.));
            let thumb_h = (track_h * (viewport_h / (viewport_h + max_off))).max(px(30.));
            let travel = (track_h - thumb_h).max(px(1.));
            let per_px = max_off / travel;

            // 点击位置到 thumb 中心的距离：若点在 thumb 外，先把 thumb 移过去
            let track_top = down_handle.bounds().origin.y + px(4.);
            let thumb_top = track_top + (-down_handle.offset().y / per_px);
            let click_y = ev.position.y;
            let half = thumb_h / 2.0;
            if click_y < thumb_top || click_y > thumb_top + thumb_h {
                let want_top = click_y - half;
                let off = -((want_top - track_top) * per_px)
                    .clamp(px(0.), max_off);
                down_handle.set_offset(point(down_handle.offset().x, off));
                window.refresh();
                cx.stop_propagation();
            }
            *down_drag.lock() = Some((
                f32::from(ev.position.y),
                f32::from(down_handle.offset().y),
                f32::from(per_px),
            ));
        })
        .on_mouse_move(move |ev, window, _| {
            if ev.pressed_button != Some(MouseButton::Left) {
                return;
            }
            let Some((start_y, start_off, per_px)) = *move_drag.lock() else {
                return;
            };
            let dy = f32::from(ev.position.y) - start_y;
            let max_off = f32::from(move_handle.max_offset().height);
            let new_off = (start_off + dy * per_px).clamp(-max_off, 0.0);
            move_handle.set_offset(point(move_handle.offset().x, px(new_off)));
            window.refresh();
        })
        .on_mouse_up(MouseButton::Left, move |_, _, _| {
            *up_drag.lock() = None;
        })
        .child(bar)
}

pub(crate) fn icon(path: &str, size: f32, color: Hsla) -> impl IntoElement {
    svg()
        .path(SharedString::from(path.to_string()))
        .size(px(size))
        .text_color(color)
        .flex_shrink_0()
}


/// 用户 VIP 标签（vip_type：0 普通 / >0 VIP / 8+ 更高等级）。
pub(crate) fn vip_tag(vip_type: i64) -> Option<AnyElement> {
    if vip_type <= 0 {
        return None;
    }
    let label = match vip_type {
        1..=3 => "VIP",
        4..=7 => "超级 VIP",
        _ => "黑胶 VIP",
    };
    Some(
        div()
            .flex_shrink_0()
            .px_1p5()
            .py_0p5()
            .rounded(px(4.))
            .bg(Hsla { h: 0.11, s: 0.75, l: 0.55, a: 0.18 })
            .text_size(px(10.))
            .line_height(px(13.))
            .text_color(Hsla { h: 0.11, s: 0.75, l: 0.55, a: 1.0 })
            .child(label)
            .into_any_element(),
    )
}

/// 歌曲权限标签（VIP / 付费 / 版权 / 音源）。
pub(crate) fn song_tag(song: &Song, theme: &crate::theme::Theme) -> Option<AnyElement> {
    let (label, color) = match song.privilege_tag()? {
        echo_core::SongTag::Vip => ("VIP", theme.accent),
        echo_core::SongTag::Paid => ("付费", gpui::rgb(0xef4444).into()),
        echo_core::SongTag::NoCopyright => ("版权", gpui::rgb(0x8b5cf6).into()),
        echo_core::SongTag::Unavailable => ("音源", gpui::rgb(0x6b7280).into()),
    };
    Some(
        div()
            .flex_shrink_0()
            .px_1()
            .py_0p5()
            .rounded(px(4.))
            .bg(color.opacity(0.12))
            .text_size(px(9.5))
            .line_height(px(12.))
            .text_color(color)
            .child(label)
            .into_any_element(),
    )
}

/// 图标按钮（悬停高亮）。
struct IconButton {
    path: &'static str,
    size: f32,
    color: Hsla,
    hover_bg: Hsla,
    action: Box<dyn Fn(&mut Root, &MouseDownEvent, &mut Window, &mut Context<Root>) + 'static>,
}

impl IconButton {
    fn render(self, id: impl Into<ElementId>, cx: &Context<Root>) -> Stateful<Div> {
        let color = self.color;
        let hover_bg = self.hover_bg;
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .size(px(self.size + 12.))
            .rounded(px(8.))
            .hover(move |s| s.bg(hover_bg))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(move |root, ev, window, cx| {
                (self.action)(root, ev, window, cx);
            }))
            .child(icon(self.path, self.size, color))
    }
}

// ==================== 播放动作 ====================

/// 解析指定歌曲的播放地址并加载引擎（在 update 之外调用）。
pub fn spawn_resolve(store: &Entity<AppStore>, request_id: u64, song: Song, cx: &mut App) {
    let store = store.clone();
    let (client, engine, quality) = {
        let s = store.read(cx);
        (s.client.clone(), s.engine.clone(), s.settings.get().audio_quality)
    };
    // 取播放地址属于网络 IO，放到**专用线程**跑：
    // 不占 GPUI 的执行器（前台/后台都不该被网络请求占住），
    // 结果通过 smol channel 回灌到 UI 任务里。
    // 播放被 VIP 挡住时，兜底领一天畅听 VIP 再重试（见 `vip.rs`）
    let allow_vip_claim = crate::vip::should_claim_for_playback(store.read(cx));
    let (tx, rx) = smol::channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("song-resolve".into())
        .spawn(move || {
            // 必须用 MixSongID 这一侧的 id：传 audio_id 会让服务端解析成别的歌
            let aa: i64 = song.play_album_audio_id();
            let hash = song.hash.clone();
            // 云盘里的歌是用户自己上传的，走云盘专用取地址接口（返回原始文件）
            let mut out = if song.from_cloud {
                match run_tokio(client.cloud_song_url(&song)) {
                    Ok(url) => Ok(vec![url]),
                    Err(e) => {
                        log::warn!("云盘取地址失败，回退普通接口: {e}");
                        run_tokio(async { client.song_url(&hash, &quality, aa).await })
                    }
                }
            } else {
                run_tokio(async { client.song_url(&hash, &quality, aa).await })
            };
            let mut claim = None;
            // 免费歌/网络错误不领；只有「要 VIP/付费」才值得花一次请求
            if allow_vip_claim && matches!(&out, Err(e) if crate::vip::is_vip_block(e)) {
                // 先查真实身份：session 里的 vip_type 可能是过期前的快照，
                // 「播放过程中会员到期」正是靠这一步发现的。
                let st = run_tokio(crate::vip::fetch_vip(&client));
                let not_vip = st.vip_type <= 0 || (st.end_time > 0 && st.end_time <= chrono::Local::now().timestamp());
                if not_vip {
                    let o = run_tokio(crate::vip::claim_loop(&client));
                    log::info!(
                        "播放被VIP挡住，自动领取: ok={} date={} err={}",
                        o.ok, o.date, o.err
                    );
                    if o.ok {
                        out = run_tokio(async { client.song_url(&hash, &quality, aa).await });
                    }
                    claim = Some(o);
                } else {
                    log::info!(
                        "播放失败但当前仍是VIP（type={} end={}），不领取",
                        st.vip_type, st.end_time
                    );
                }
            }
            let _ = smol::block_on(tx.send((out, claim)));
        });
    if spawned.is_err() {
        log::error!("spawn_resolve: 无法启动 song-resolve 线程");
        return;
    }
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (res, claim) = match rx.recv().await {
            Ok(r) => r,
            Err(_) => return,
        };
        let _ = store.update(cx, |s, cx| {
            if s.player.request_id != request_id {
                return; // 过期请求
            }
            // 记下这次兜底领取的结果，避免同一天反复打接口
            if let Some(c) = claim {
                let date = chrono::Local::now().format("%Y-%m-%d").to_string();
                s.settings.update(|st| {
                    st.vip_claim_date = date;
                    st.vip_claim_ok = c.ok;
                });
                if c.ok {
                    if let Some(st) = c.vip.as_ref() {
                        s.session.update_vip(st.vip_type, st.end_time);
                        if let Some(cur) = s.user.as_mut() {
                            cur.vip_type = st.vip_type;
                        }
                    }
                    s.vip_note = crate::vip::vip_note_text(c.vip.as_ref());
                } else if c.expired {
                    // 播放中才发现登录失效：直接退出登录，别让用户对着报错发呆
                    crate::views::force_logout(s, "登录已过期，请重新登录");
                }
                log::info!("播放兜底领取: ok={} date={} err={}", c.ok, c.date, c.err);
            }
            match res {
                Ok(urls) if !urls.is_empty() => {
                    // 多个候选 URL（主地址 + CDN 备份），下载失败自动回退
                    log::info!("spawn_resolve: OK n={} url0={}", urls.len(), &urls[0][..60.min(urls[0].len())]);
                    engine.load_multi(&urls);
                    s.player.loading = false;
                    s.player.loaded = true;
                    s.player.playing = true;
                    s.player.position = 0.0;
                    if let Some(cur) = s.player.current_mut() {
                        cur.audio_url = urls[0].clone();
                    }
                }
                Ok(_) => {
                    log::warn!("spawn_resolve: empty urls");
                    s.player.loading = false;
                    s.player.playing = false;
                    s.toast = "未能获取播放地址（可能需要登录或版权受限）".into();
                }
                Err(e) => {
                    log::warn!("spawn_resolve: ERR {e}");
                    s.player.loading = false;
                    s.player.playing = false;
                    s.toast = format!("播放失败：{e}");
                }
            }
            cx.notify();
        });
    })
    .detach();
}

/// 播放来源（决定从哪个列表取队列）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaySource {
    Search,
    StyleSongs,
    PlaylistDetail,
    Daily,
    NewSongs,
    RankSongs,
    ArtistSongs,
    AlbumSongs,
    Favorites,
    History,
    /// 「我最喜爱」带前端模糊搜索的视图（下标基于过滤后的列表）
    FavoritesFiltered,
    /// 私人 FM
    Fm,
    /// 音乐云盘
    Cloud,
    /// 已购单曲
    Purchased,
}

/// 从来源列表设置队列并从 index 开始播放。
pub fn play_from(store: &Entity<AppStore>, source: PlaySource, index: usize, cx: &mut App) {
    let store = store.clone();
    let (title, songs) = store.update(cx, |s, _| {
        let (title, songs): (String, Vec<Song>) = match &source {
            PlaySource::Search => ("搜索结果".into(), s.search.songs.as_ref().clone()),
            PlaySource::StyleSongs => ("风格推荐".into(), s.home.style_songs.as_ref().clone()),
            PlaySource::PlaylistDetail => s
                .playlist_detail
                .as_ref()
                .map(|(pl, songs)| (pl.name.clone(), songs.as_ref().clone()))
                .unwrap_or_default(),
            PlaySource::Daily => ("每日推荐".into(), s.daily.as_ref().clone()),
            PlaySource::NewSongs => ("新歌速递".into(), s.explore.new_songs.as_ref().clone()),
            PlaySource::RankSongs => (
                s.rank_view
                    .ranks
                    .iter()
                    .find(|r| r.id == s.rank_view.rank_id)
                    .map(|r| r.name.clone())
                    .unwrap_or_else(|| "排行榜".into()),
                s.rank_view.songs.as_ref().clone(),
            ),
            PlaySource::ArtistSongs => s
                .artist_view
                .as_ref()
                .map(|v| (v.artist.name.clone(), v.songs.as_ref().clone()))
                .unwrap_or_default(),
            PlaySource::AlbumSongs => s
                .album_view
                .as_ref()
                .map(|v| (v.album.name.clone(), v.songs.as_ref().clone()))
                .unwrap_or_default(),
            // 登录用户的「我最喜爱」页面展示的是云歌单（remote_liked），
            // 队列必须取同一份数据，否则下标对不上、本地收藏为空时点了没反应。
            // 两种来源都取「当前视图」：有搜索词时就是过滤后的列表。
            PlaySource::Favorites | PlaySource::FavoritesFiltered => (
                "我最喜爱".into(),
                s.favorites_view().as_ref().clone(),
            ),
            PlaySource::History => ("播放历史".into(), s.library.get().history.as_ref().clone()),
            PlaySource::Fm => ("私人FM".into(), s.fm.as_ref().clone()),
            PlaySource::Cloud => ("音乐云盘".into(), s.cloud.as_ref().clone()),
            PlaySource::Purchased => ("已购音乐".into(), s.purchased_songs.as_ref().clone()),
        };
        (title, songs)
    });
    if songs.is_empty() {
        return;
    }
    let start = index.min(songs.len() - 1);
    // 设置队列状态
    let current_song = store.update(cx, |s, cx| {
        let mode = s.player.play_mode;
        let volume = s.player.volume;
        s.player.set_queue(&title, songs, start);
        s.player.play_mode = mode;
        s.player.volume = volume;
        s.player.position = 0.0;
        s.player.duration = s.player.current().map(|c| c.duration as f64).unwrap_or(0.0);
        s.player.request_id += 1;
        s.player.loading = true;
        // 队列已换：把新快照落盘（关掉 App 再打开能还原）
        s.persist_queue();
        let out = s.player.current().cloned().map(|c| (s.player.request_id, c));
        cx.notify();
        out
    });
    if let Some((rid, song)) = current_song {
        // 记录播放历史（去重，最多 300 条）
        store.update(cx, |s, _| s.library.push_history(&song));
        // 歌词跟着新曲目走
        crate::views::load_lyric(&store, cx);
        start_with_fade(&store, rid, song, cx);
    }
}

/// 切歌淡出：正在播就把旧音源淡出，再解析新地址（避免硬切爆音）。
///
/// 「淡出」必须由引擎在回调里走完，所以这里要等一小会儿再 `load`；
/// 等待期间 UI 状态已经切到新歌（`loading=true`），看起来就是「缓冲中」。
pub(crate) fn start_with_fade(store: &Entity<AppStore>, rid: u64, song: Song, cx: &mut App) {
    const FADE_MS: u64 = 130;
    let playing = store.read(cx).player.playing;
    if !playing {
        spawn_resolve(store, rid, song, cx);
        return;
    }
    store.read(cx).engine.fade_out(FADE_MS as f32 / 1000.0);
    let store_c = store.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        smol::Timer::after(std::time::Duration::from_millis(FADE_MS)).await;
        let _ = cx.update(|app| {
            store_c.update(app, |s, _| s.engine.fade_reset());
            spawn_resolve(&store_c, rid, song, app);
        });
    })
    .detach();
}

/// 播放队列中指定下标。
pub fn play_index(store: &Entity<AppStore>, index: usize, cx: &mut App) {
    let data = store.update(cx, |s, cx| {
        s.player.index = Some(index);
        s.player.position = 0.0;
        s.player.duration = s.player.queue.get(index).map(|q| q.duration as f64).unwrap_or(0.0);
        s.player.request_id += 1;
        s.player.loading = true;
        s.persist_queue();
        let out = s.player.current().cloned().map(|c| (s.player.request_id, c));
        cx.notify();
        out
    });
    if let Some((rid, song)) = data {
        store.update(cx, |s, _| s.library.push_history(&song));
        crate::views::load_lyric(&store, cx);
        start_with_fade(&store, rid, song, cx);
    }
}

pub fn toggle_play(store: &Entity<AppStore>, cx: &mut App) {
    // 引擎里没有音频时（重启还原队列后的第一次播放）必须先解析地址再播
    let (need_resolve, paused) = store.update(cx, |s, cx| {
        if s.player.current().is_none() {
            return (None, false);
        }
        if s.player.playing {
            // 暂停前先淡出，避免直接掐断产生「咔哒」声；
            // 真正的 `pause()` 等淡出走完再发（见下面的延迟任务）
            s.engine.fade_out(0.12);
            s.player.playing = false;
            let paused = true;
            cx.notify();
            return (None, paused);
        }
        if s.player.loaded {
            s.engine.fade_reset();
            s.engine.play();
            s.engine.fade_in(0.12);
            s.player.playing = true;
            cx.notify();
            return (None, false);
        }
        s.player.request_id += 1;
        s.player.loading = true;
        let out = s.player.current().cloned().map(|c| (s.player.request_id, c));
        cx.notify();
        (out, false)
    });
    if paused {
        let store_c = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            smol::Timer::after(std::time::Duration::from_millis(140)).await;
            let _ = cx.update(|app| {
                store_c.update(app, |s, _| {
                    s.engine.pause();
                    s.engine.fade_reset();
                });
            });
        })
        .detach();
    }
    if let Some((rid, song)) = need_resolve {
        spawn_resolve(store, rid, song, cx);
    }
}

/// 清空播放队列（停止播放并清掉落盘快照）。
pub fn clear_queue(store: &Entity<AppStore>, cx: &mut App) {
    store.update(cx, |s, cx| {
        if s.player.queue.is_empty() {
            return;
        }
        // 让在途的解析结果作废，避免刚清空又被塞回来
        s.player.request_id += 1;
        s.engine.stop();
        s.player.clear_queue();
        s.queue_store.clear();
        s.toast = "已清空播放队列".into();
        s.queue_drag = None;
        cx.notify();
    });
}

pub fn next_track(store: &Entity<AppStore>, auto: bool, cx: &mut App) {
    let next = store.read(cx).player.next_index();
    match next {
        Some(i) => play_index(store, i, cx),
        None => {
            if !auto {
                store.update(cx, |s, cx| {
                    s.toast = "已经是最后一首了".into();
                    cx.notify();
                });
            }
        }
    }
}

pub fn prev_track(store: &Entity<AppStore>, cx: &mut App) {
    if let Some(i) = store.read(cx).player.prev_index() {
        play_index(store, i, cx);
    }
}

pub(crate) fn cycle_play_mode(store: &Entity<AppStore>, cx: &mut App) {
    store.update(cx, |s, cx| {
        s.player.play_mode = s.player.play_mode.next();
        let mode = s.player.play_mode;
        s.settings.update(|st| st.play_mode = mode);
        s.toast = mode.label().into();
        cx.notify();
    });
}

/// 倍速档位（与播放条的循环一致）。
pub(crate) const SPEED_STEPS: [f64; 5] = [0.75, 1.0, 1.25, 1.5, 2.0];

/// 按档位加减倍速（播放页 `[` / `]` 用）。
pub(crate) fn step_speed(store: &Entity<AppStore>, dir: i32, cx: &mut App) {
    store.update(cx, |s, cx| {
        let cur = s.settings.get().speed;
        let mut ix = SPEED_STEPS
            .iter()
            .position(|v| (*v - cur).abs() < 0.001)
            .unwrap_or(1) as i32;
        ix = (ix + dir).clamp(0, SPEED_STEPS.len() as i32 - 1);
        let next = SPEED_STEPS[ix as usize];
        s.settings.update(|st| st.speed = next);
        s.engine.set_speed(next);
        s.toast = format!("倍速 {next}x");
        cx.notify();
    });
}

pub(crate) fn set_volume(store: &Entity<AppStore>, v: f64, cx: &mut App) {
    store.update(cx, |s, cx| {
        s.player.volume = v.clamp(0.0, 1.0);
        s.engine.set_volume(s.player.volume);
        s.settings.update(|st| st.volume = s.player.volume);
        cx.notify();
    });
}

/// 播放器事件处理 —— 状态部分（在 store.update 内调用）。
/// 返回需要解析并加载的 (request_id, song)。
pub fn handle_player_event_state(
    s: &mut AppStore,
    ev: echo_audio::PlayerEvent,
    cx: &mut gpui::Context<AppStore>,
) -> Option<(u64, Song)> {
    // 歌词兜底：不管从哪条路径开始播放（列表点击 / 播放条 / 自动下一首 / 重启还原后第一次播放），
    // 进度事件每 250ms 一次，这里比对一次曲目 key 就够了。
    if let Some(song) = s.player.current() {
        let key = song.key();
        if !key.is_empty() && s.lyric_key != key && !s.lyric_loading {
            s.lyric_key = key;
            s.lyric = None;
            cx.notify();
            let client = s.client.clone();
            let store = cx.entity();
            let song = song.clone();
            cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                let res = cx
                    .background_executor()
                    .spawn(async move { run_tokio(client.lyric(&song)) })
                    .await;
                let _ = store.update(cx, |s, cx| {
                    s.lyric_loading = false;
                    match res {
                        Ok(lyric) => {
                            log::info!(
                                "lyric: {} 行（逐字={}）",
                                lyric.lines.len(),
                                lyric.word_time
                            );
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
            s.lyric_loading = true;
        }
    }
    match ev {
        echo_audio::PlayerEvent::Position(p) => {
            s.player.position = p;
            s.position_at = std::time::Instant::now();
            cx.notify();
            None
        }
        echo_audio::PlayerEvent::Ended => {
            if let Some(i) = s.player.next_index() {
                s.player.index = Some(i);
                s.player.position = 0.0;
                s.player.loaded = false;
                s.player.duration = s.player.queue.get(i).map(|q| q.duration as f64).unwrap_or(0.0);
                s.player.request_id += 1;
                s.player.loading = true;
                s.persist_queue();
                let out = s.player.current().cloned().map(|c| (s.player.request_id, c));
                cx.notify();
                out
            } else {
                s.player.playing = false;
                cx.notify();
                None
            }
        }
        echo_audio::PlayerEvent::Error(e) => {
            s.player.loading = false;
            s.player.playing = false;
            s.toast = format!("播放错误：{e}");
            cx.notify();
            None
        }
        // 音频诊断（1Hz 限频）：只存快照，渲染时才拼字符串。
        // 只有停在设置页才重绘，否则每秒白刷一帧。
        echo_audio::PlayerEvent::Diagnostics(d) => {
            s.audio_diag = Some(d);
            if matches!(s.routes.current(), Some(crate::state::Page::Settings)) {
                cx.notify();
            }
            None
        }
        echo_audio::PlayerEvent::StateChanged(_) => None,
    }
}

/// 切换浅色/深色主题（带过渡动画，并落盘）。
pub(crate) fn cycle_theme(
    store: &Entity<AppStore>,
    origin: Option<gpui::Point<gpui::Pixels>>,
    cx: &mut App,
) {
    let next = match store.read(cx).settings.get().theme_mode {
        echo_core::ThemeMode::Dark => echo_core::ThemeMode::Light,
        _ => echo_core::ThemeMode::Dark,
    };
    set_theme_mode(store, next, origin, cx);
}

/// 指定主题模式：从 `origin`（通常是开关位置）开始的「开关灯」圆形揭示。
pub(crate) fn set_theme_mode(
    store: &Entity<AppStore>,
    mode: echo_core::ThemeMode,
    origin: Option<gpui::Point<gpui::Pixels>>,
    cx: &mut App,
) {
    if store.read(cx).settings.get().theme_mode == mode {
        return;
    }
    store.update(cx, |s, cx| {
        // 揭示期间保持旧主题；圆盖满窗口的那一帧切换为新主题
        let hold = s.theme();
        s.theme_anim = None;
        s.theme_reveal = Some(crate::state::ThemeReveal {
            origin: origin.unwrap_or_else(|| gpui::point(px(64.), px(600.))),
            hold,
            started: std::time::Instant::now(),
        });
        s.settings.update(|st| st.theme_mode = mode);
        s.toast = match mode {
            echo_core::ThemeMode::Dark => "已切换到深色主题".into(),
            echo_core::ThemeMode::Light => "已切换到浅色主题".into(),
            echo_core::ThemeMode::System => "已切换到跟随系统".into(),
        };
        cx.notify();
    });

    // 逐帧驱动：圆形扩散 ~550ms（ease-out cubic），结束后收尾换装
    let anim_store = store.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let steps = (crate::theme::THEME_REVEAL_SECS / 0.016).ceil() as usize + 1;
        for _ in 0..steps {
            smol::Timer::after(std::time::Duration::from_millis(16)).await;
            let _ = anim_store.update(cx, |_, cx| cx.notify());
        }
        let _ = anim_store.update(cx, |s, cx| {
            s.theme_reveal = None;
            s.finish_theme_anim();
            cx.notify();
        });
    })
    .detach();
}

/// 指定背景特效模式（Mica / Acrylic / 跟随系统 / 关闭），落盘并立即应用。
pub(crate) fn set_backdrop_mode(store: &Entity<AppStore>, mode: echo_core::BackdropMode, cx: &mut App) {
    // 生效模式与持久化设置都要看：ECHO_BACKDROP 环境变量覆盖时两者可能不同
    let persisted = store.read(cx).settings.get().backdrop;
    let effective = store.read(cx).backdrop_effective;
    if persisted == mode && effective == mode {
        return;
    }
    store.update(cx, |s, cx| {
        s.backdrop_effective = mode;
        if s.settings.get().backdrop != mode {
            s.settings.update(|st| st.backdrop = mode);
        }
        s.toast = format!("背景特效：{}", mode.label());
        cx.notify();
    });
    apply_backdrop(store, cx);
}

/// 把当前设置的背景特效模式应用到主窗口。
///
/// 点击回调里只有 `&mut App`（没有 `&mut Window`），所以从 `AppGlobal`
/// 取出主窗口句柄回到窗口上下文再调 DWM。
pub(crate) fn apply_backdrop(store: &Entity<AppStore>, cx: &mut App) {
    let mode = store.read(cx).backdrop_effective;
    let handle = cx.try_global::<crate::state::AppGlobal>()
        .and_then(|g| g.main_window);
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_view, window, _cx| {
            crate::windows_backdrop::apply(window, mode);
        });
        // 材质可见性可能变了（比如关掉特效），主窗口背景要重新配色
        let _ = store.update(cx, |_, cx| cx.notify());
    }
}

/// 最大化 / 还原（点窗口按钮、双击标题栏都走这里）。
///
/// **不能在事件派发或绘制过程中同步改窗口**：`ShowWindow`/`SetWindowPos` 会同步派发
/// `WM_SIZE`，而 gpui 的 resize 回调是 `window.update(...)`，重入时会失败并被
/// `log_err` 吞掉 —— 结果就是窗口铺开了、布局还是旧的（用户看到的「只是挪到了左上角」）。
/// 所以延到下一轮消息循环再交给系统处理。
pub(crate) fn spawn_toggle_maximize(handle: AnyWindowHandle, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        smol::Timer::after(std::time::Duration::from_millis(60)).await;
        let before = handle
            .update(cx, |_, window, _| crate::win::is_maximized(window))
            .unwrap_or(false);
        let want = !before;
        let _ = handle.update(cx, |_, window, _| {
            crate::win::set_maximized(window, want)
        });
        // 万一这条消息没被处理（极少见），按同一个目标补一次：
        // 注意必须补发「同一个命令」，不能重新取反，否则会把刚生效的操作再翻回去
        smol::Timer::after(std::time::Duration::from_millis(400)).await;
        let now = handle
            .update(cx, |_, window, _| crate::win::is_maximized(window))
            .unwrap_or(want);
        if now != want {
            log::warn!("最大化/还原没生效，补发一次");
            let _ = handle.update(cx, |_, window, _| {
                crate::win::set_maximized(window, want)
            });
        }
    })
    .detach();
}

/// 路由后退一步（顶栏后退按钮与歌词页返回共用）。
#[allow(dead_code)] // 播放条上的「切回播放页」已移除，暂时没有调用点，保留备用
pub fn go_back(store: &Entity<AppStore>, cx: &mut App) {
    let moved = store.update(cx, |s, cx| {
        let ok = s.routes.back();
        if ok {
            s.list_drag = None;
            s.song_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
            cx.notify();
        }
        ok
    });
    if moved {
        let page = store.read(cx).routes.current().cloned();
        if let Some(page) = page {
            load_page(store, &page, cx);
        }
    }
}

pub fn navigate(store: &Entity<AppStore>, page: Page, cx: &mut App) {
    // 需要登录的页面：未登录（或登录已失效）直接改道登录页，登录成功后再跳回来
    let need_login = {
        let s = store.read(cx);
        page.requires_login() && !s.session.get().is_valid()
    };
    let page = if need_login {
        log::info!("navigate: 未登录，改道登录页（目标 {:?}）", page);
        let target = page.clone();
        store.update(cx, |st, _| st.login_redirect = Some(target));
        Page::Login
    } else {
        page
    };
    store.update(cx, |s, cx| {
        // 上一个页面的大列表不再需要，直接释放（否则每开一个列表都多留一份）
        release_inactive_data(s, &page);
        s.routes.navigate(page.clone());
        s.list_drag = None;
        // 新页面从顶部开始
        s.song_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        cx.notify();
    });
    load_page(store, &page, cx);
}

/// 释放与目标页面无关的大列表，避免浏览过的每个列表都常驻内存。
///
/// 队列（`player.queue`）在 `play_from` 时已经拷走，释放页面数据不影响正在播放的歌。
fn release_inactive_data(s: &mut AppStore, keep: &Page) {
    if !matches!(keep, Page::Rank(..)) && !s.rank_view.songs.is_empty() {
        s.rank_view.songs = Arc::new(Vec::new());
    }
    if !matches!(keep, Page::Artist(..))
        && s
            .artist_view
            .as_ref()
            .is_some_and(|v| !v.songs.is_empty() || !v.albums.is_empty())
    {
        s.artist_view = None;
    }
    if !matches!(keep, Page::Album(..)) && s.album_view.is_some() {
        s.album_view = None;
    }
    if !matches!(keep, Page::Playlist(..)) && s.playlist_detail.is_some() {
        s.playlist_detail = None;
    }
    if !matches!(keep, Page::Search(_)) && !s.search.songs.is_empty() {
        s.search.songs = Arc::new(Vec::new());
        s.search.playlists.clear();
        s.search.artists.clear();
        s.search.albums.clear();
        s.search.loaded_tabs.clear();
        s.search.loaded = false;
    }
}

/// 按页面加载数据（navigate 与前进/后退共用）。
pub fn load_page(store: &Entity<AppStore>, page: &Page, cx: &mut App) {
    match page.clone() {
        Page::Home => load_home(store, cx),
        Page::Explore | Page::Ranking => crate::views::load_explore(store, cx),
        Page::DailyRecommend => crate::views::load_daily(store, cx),
        Page::Search(kw) => {
            load_search(store, &kw, cx);
            // 顺带拉取热门搜索词（空关键词时展示）
            load_search_hot(store, cx);
        }
        Page::Playlist(id, name) => load_playlist(store, &id, &name, cx),
        Page::Artist(id, _) => crate::views::load_artist(store, &id, cx),
        Page::Album(id, _) => crate::views::load_album(store, &id, cx),
        Page::Rank(id, _) => crate::views::load_rank_detail(store, &id, cx),
        Page::Login => {
            // 进入登录页默认展示扫码登录，自动拉取二维码
            let need = store.read(cx).login.method == crate::state::LoginMethod::Qr;
            if need {
                crate::views::start_qr_login(store, cx);
            }
        }
        Page::Favorites => {
            // 登录用户「我最喜爱」来自云歌单
            crate::liked::load_remote_liked(store, false, cx);
        }
        Page::Fm => crate::views::load_fm(store, false, cx),
        Page::Lyric => {
            // 歌词页直接渲染当前曲目的歌词（没有就现拉一次）
            crate::views::load_lyric(store, cx);
        }
        Page::Cloud => crate::views::load_cloud(store, false, cx),
        Page::Purchased => crate::views::load_purchased(store, false, cx),
        _ => {}
    }
}

// ==================== 数据加载 ====================

fn load_home(store: &Entity<AppStore>, cx: &mut App) {
    let store = store.clone();
    if store.read(cx).home.loaded {
        return;
    }
    store.update(cx, |s, cx| {
        s.home.loaded = true; // 防重复加载
        cx.notify();
    });
    let client = store.read(cx).client.clone();
    let c2 = client.clone();
    let c3 = client.clone();
    let task = cx.background_executor().spawn(async move {
        let playlists = run_tokio(client.recommend_playlists(0, 1, 24));
        let editor = run_tokio(c2.top_ip());
        let style = run_tokio(c3.style_recommend(""));
        (playlists, editor, style)
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (playlists, editor, style) = task.await;
        let _ = store.update(cx, |s, cx| {
            s.home.recommend_playlists = playlists.unwrap_or_default();
            s.home.editor_playlists = editor.unwrap_or_default();
            match style {
                Ok(st) => {
                    s.home.style_categories = st.categories;
                    s.home.style_songs = Arc::new(st.songs);
                    // 默认选中各分类下 default=1 的标签
                    s.home.selected_tags = s
                        .home
                        .style_categories
                        .iter()
                        .flat_map(|c| c.tags.iter().filter(|t| t.is_default).map(|t| t.id.clone()))
                        .collect();
                    s.home.style_loading = false;
                }
                Err(e) => {
                    s.home.style_loading = false;
                    log::warn!("style_recommend failed: {e}");
                }
            }
            cx.notify();
        });
    })
    .detach();
}

/// 按选中标签重新拉取风格推荐歌曲。
fn load_style(store: &Entity<AppStore>, cx: &mut App) {
    let store = store.clone();
    let (client, tagids) = {
        let s = store.read(cx);
        (s.client.clone(), s.home.selected_tags.join(","))
    };
    store.update(cx, |s, cx| {
        s.home.style_loading = true;
        cx.notify();
    });
    let task = cx.background_executor().spawn(async move {
        run_tokio(client.style_recommend(&tagids))
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        let _ = store.update(cx, |s, cx| {
            s.home.style_loading = false;
            if let Ok(st) = res {
                s.home.style_songs = Arc::new(st.songs);
                if !st.categories.is_empty() {
                    s.home.style_categories = st.categories;
                }
            }
            cx.notify();
        });
    })
    .detach();
}

fn load_search(store: &Entity<AppStore>, kw: &str, cx: &mut App) {
    let store = store.clone();
    let kw = kw.trim().to_string();
    if kw.is_empty() {
        return;
    }
    let t0 = std::time::Instant::now();
    store.update(cx, |s, cx| {
        s.search.keyword = kw.clone();
        s.search.loaded = false;
        s.search.loading = true;
        s.search.songs = Arc::new(Vec::new());
        s.search.playlists.clear();
        s.search.artists.clear();
        s.search.albums.clear();
        s.search.loaded_tabs.clear();
        s.search.tab = 0;
        cx.notify();
    });
    let client = store.read(cx).client.clone();
    let task = cx.background_executor().spawn(async move {
        run_tokio(async { client.search_songs(&kw, 1, 30).await })
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let songs = task.await.unwrap_or_default();
        // 与 views::min_spinner 相同：保证加载动画至少可见
        let min = std::time::Duration::from_millis(500);
        let elapsed = t0.elapsed();
        if elapsed < min {
            smol::Timer::after(min - elapsed).await;
        }
        let _ = store.update(cx, |s, cx| {
            s.search.loading = false;
            s.search.loaded = true;
            s.search.songs = Arc::new(songs);
            cx.notify();
        });
    })
    .detach();
}

fn load_playlist(store: &Entity<AppStore>, id: &str, name: &str, cx: &mut App) {
    let store = store.clone();
    let client = store.read(cx).client.clone();
    let client2 = client.clone();
    let pid = id.to_string();
    let fallback_name = name.to_string();
    // 列表里已有的元信息（封面/创建者）作为兜底
    let known = store.read(cx).home.recommend_playlists.iter()
        .chain(store.read(cx).home.editor_playlists.iter())
        .find(|p| p.id == pid)
        .cloned();

    let pid_task = pid.clone();
    let task = cx.background_executor().spawn(async move {
        run_tokio(async move {
            let detail = client.playlist_detail(&pid_task).await.ok().flatten();
            let songs = client2.playlist_tracks(&pid_task, 1, 100).await.unwrap_or_default();
            (detail, songs)
        })
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (detail, songs) = task.await;
        let _ = store.update(cx, |s, cx| {
            let mut pl = detail
                .or(known)
                .unwrap_or_else(|| echo_core::Playlist {
                    id: pid.clone(),
                    name: fallback_name.clone(),
                    ..Default::default()
                });
            // 封面兜底：用首曲封面
            if pl.pic.is_empty() {
                if let Some(first) = songs.first() {
                    pl.pic = first.cover_url.clone();
                }
            }
            if pl.count == 0 {
                pl.count = songs.len() as u64;
            }
            if songs.is_empty() {
                s.playlist_detail = None;
                s.toast = "歌单暂无歌曲".into();
            } else {
                s.playlist_detail = Some((pl, Arc::new(songs)));
            }
            s.playlists_loaded.insert(pid.clone());
            cx.notify();
        });
    })
    .detach();
}

/// 搜索分类（歌单 / 歌手 / 专辑）按需加载。
pub(crate) fn load_search_tab(store: &Entity<AppStore>, kw: &str, tab: usize, cx: &mut App) {
    if tab == 0 {
        return;
    }
    let store = store.clone();
    let kw = kw.trim().to_string();
    if kw.is_empty() {
        return;
    }
    if store.read(cx).search.loaded_tabs.contains(&tab) {
        return;
    }
    let t0 = std::time::Instant::now();
    store.update(cx, |s, cx| {
        s.search.tab = tab;
        s.search.loading = true;
        s.search.loaded_tabs.push(tab);
        cx.notify();
    });
    let client = store.read(cx).client.clone();
    let task = cx.background_executor().spawn(async move {
        run_tokio(async move {
            match tab {
                1 => client.search_playlists(&kw, 1, 30).await.map(SearchHit::Playlists),
                2 => client.search_artists(&kw, 1, 30).await.map(SearchHit::Artists),
                3 => client.search_albums(&kw, 1, 30).await.map(SearchHit::Albums),
                4 => client
                    .lyric_candidates(&echo_core::Song {
                        title: kw.clone(),
                        ..Default::default()
                    })
                    .await
                    .map(|v| v.into_iter().take(30).collect::<Vec<_>>())
                    .map(SearchHit::Lyrics),
                _ => client.search_mvs(&kw, 1, 30).await.map(SearchHit::Mvs),
            }
        })
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        // 保证加载动画至少可见
        let min = std::time::Duration::from_millis(500);
        let elapsed = t0.elapsed();
        if elapsed < min {
            smol::Timer::after(min - elapsed).await;
        }
        let _ = store.update(cx, |s, cx| {
            s.search.loading = false;
            match res {
                Ok(SearchHit::Playlists(v)) => s.search.playlists = v,
                Ok(SearchHit::Artists(v)) => s.search.artists = v,
                Ok(SearchHit::Albums(v)) => s.search.albums = v,
                Ok(SearchHit::Lyrics(v)) => s.search.lyrics = v,
                Ok(SearchHit::Mvs(v)) => s.search.mvs = v,
                Err(e) => s.toast = format!("搜索失败：{e}"),
            }
            cx.notify();
        });
    })
    .detach();
}

/// 分类搜索结果。
enum SearchHit {
    Playlists(Vec<echo_core::Playlist>),
    Artists(Vec<echo_core::Artist>),
    Albums(Vec<echo_core::Album>),
    /// 歌词搜索候选（title/artist/duration）
    Lyrics(Vec<echo_api::lyrics::LyricCandidate>),
    Mvs(Vec<echo_core::MvMeta>),
}

fn load_search_hot(store: &Entity<AppStore>, cx: &mut App) {
    let store = store.clone();
    // 已加载过则跳过（守卫条件曾写反，导致永不执行）
    if !store.read(cx).search_hot.is_empty() {
        return;
    }
    let client = store.read(cx).client.clone();
    let task = cx.background_executor().spawn(async move {
        run_tokio(client.search_hot())
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let hot = task.await.unwrap_or_default();
        let _ = store.update(cx, |s, cx| {
            s.search_hot = hot;
            cx.notify();
        });
    })
    .detach();
}

/// 画一个实心圆（圆角=半径的方块）：主题揭示层的「圆盘」用它叠出径向羽化。
fn paint_disc(window: &mut Window, cx: f32, cy: f32, r: f32, color: Hsla) {
    if r <= 0.0 {
        return;
    }
    let side = r * 2.0;
    window.paint_quad(
        fill(
            Bounds::new(point(px(cx - r), px(cy - r)), size(px(side), px(side))),
            color,
        )
        .corner_radii(gpui::Corners::all(px(r))),
    );
}

/// 画一圈圆环（外圆挖掉内圆）。
///
/// gpui 0.2.2 的 `ContentMask` 只有矩形 bounds，元素层面做不了圆形裁剪；但
/// `PathBuilder` 走的是 lyon 的 NonZero 填充规则——外圈顺时针 + 内圈逆时针两个
/// 多边形叠在一起，中间就被挖空，得到真正的环带。
///
/// 环带之间**互不重叠**，所以每一圈都能独立指定 alpha，可以拼出任意径向分布
/// （「中心淡、波前浓、外沿归零」这种双向羽化，靠同心圆盘叠加是做不出来的——
/// 叠加只能越叠越实）。
fn paint_ring(window: &mut Window, cx: f32, cy: f32, r_out: f32, r_in: f32, color: Hsla) {
    if r_out <= 0.5 || r_in >= r_out {
        return;
    }
    const SEG: usize = 56;
    let step = std::f32::consts::TAU / SEG as f32;
    let mut outer = Vec::with_capacity(SEG);
    let mut inner = Vec::with_capacity(SEG);
    for i in 0..SEG {
        let a = i as f32 * step;
        outer.push(point(px(cx + r_out * a.cos()), px(cy + r_out * a.sin())));
    }
    // 内圈反向：NonZero 规则下才会被当成「洞」
    for i in (0..SEG).rev() {
        let a = i as f32 * step;
        inner.push(point(px(cx + r_in * a.cos()), px(cy + r_in * a.sin())));
    }
    let mut b = gpui::PathBuilder::fill();
    b.add_polygon(&outer, true);
    b.add_polygon(&inner, true);
    match b.build() {
        Ok(path) => window.paint_path(path, color),
        Err(e) => log::warn!("波前环构建失败：{e}"),
    }
}

// ==================== 主布局 Root ====================

impl Root {
    pub fn render_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.store.read(cx).theme();
        // 系统材质（Mica/Acrylic）垫在窗口后面时，基础背景画成半透明透出材质
        let backdrop = self.store.read(cx).backdrop_active();
        // 这些页面用虚拟化列表（gpui::list 自滚），不需要整页滚动容器
        let list_page = {
            let s = self.store.read(cx);
            match s.routes.current() {
                Some(Page::DailyRecommend)
                | Some(Page::Rank(..))
                | Some(Page::Playlist(..))
                | Some(Page::Album(..))
                | Some(Page::Artist(..))
                | Some(Page::Favorites)
                | Some(Page::History) => true,
                Some(Page::Search(_)) => s.search.tab == 0,
                _ => false,
            }
        };

        div()
            .id("root")
            .size_full()
            .flex()
            .flex_col()
            .bg(if backdrop {
                // 内容区基底层：透明部分透出系统材质（Mica/Acrylic）。
                // 不能太实，否则材质被盖住只剩灰底（用户看到的「云母无效」）
                theme.bg_main.opacity(0.72)
            } else {
                theme.bg_main
            })
            .text_color(theme.text)
            .text_size(px(14.))
            .font_family("Segoe UI")
            // 键盘默认落点（没有输入框占用时）：空格 = 播放/暂停
            .track_focus(&self.root_focus)
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _window, cx| {
                // 只在根节点持有焦点时生效：焦点在输入框里时空格要用来打空格
                if ev.keystroke.key == "space" && ev.keystroke.modifiers.number_of_modifiers() == 0 {
                    log::info!("空格：播放/暂停");
                    toggle_play(&this.store, cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &gpui::MouseDownEvent, window, cx| {
                // 输入框失焦：点在输入框之外就移除焦点（光标跟着消失）。
                //
                // 只在「当前焦点仍是输入框」时才处理：点到别的输入框时，
                // 子元素已经先把焦点移过去了，这里不能抢着 blur，否则输入框点不进去。
                if this.search_focus.is_focused(window) {
                    let inside = this
                        .store
                        .read(cx)
                        .search_bounds
                        .lock()
                        .as_ref()
                        .map(|b| b.contains(&ev.position))
                        .unwrap_or(true);
                    if !inside {
                        window.focus(&this.root_focus);
                    }
                } else if this.fav_search_focus.is_focused(window) {
                    let inside = this
                        .store
                        .read(cx)
                        .fav_search_bounds
                        .lock()
                        .as_ref()
                        .map(|b| b.contains(&ev.position))
                        .unwrap_or(true);
                    if !inside {
                        window.focus(&this.root_focus);
                    }
                }
                // 点击空白处清除 toast；点击播放队列外部则关闭队列
                this.store.update(cx, |s, cx| {
                    let mut changed = false;
                    if !s.toast.is_empty() {
                        s.toast.clear();
                        changed = true;
                    }
                    if s.queue_open {
                        let inside = s
                            .queue_bounds
                            .lock()
                            .as_ref()
                            .map(|b| b.contains(&ev.position))
                            .unwrap_or(false);
                        if !inside {
                            s.queue_open = false;
                            changed = true;
                        }
                    }
                    if changed {
                        cx.notify();
                    }
                });
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(self.render_titlebar(window, cx))
                            .child(
                                // relative 包一层：叠加自绘滚动条（gpui 0.2.2 不绘制滚动条）
                                div().relative().flex_1().min_h_0().flex().flex_col()
                                    .child(
                                        div()
                                            .id("content")
                                            .flex_1()
                                            .min_h_0()
                                            // 虚拟化列表页自己滚动（gpui::list 需要确定高度），
                                            // 其它页面（首页/探索/设置等）继续整页滚动。
                                            .when(!list_page, |el| {
                                                el.overflow_y_scroll()
                                                    .overflow_x_hidden()
                                                    .track_scroll(&self.content_scroll)
                                            })
                                            .child(self.render_content(cx)),
                                    )
                                    .when(!list_page, |el| {
                                        el.child(
                                            div()
                                                .absolute()
                                                .top(px(4.))
                                                .bottom(px(4.))
                                                .right(px(0.))
                                                .w(px(10.))
                                                .child(scrollbar_overlay("content-sb", &self.content_scroll)),
                                        )
                                    }),
                            ),
                    ),
            )
            .child(self.render_player_bar(cx))
            .child(self.render_player_menu(cx))
            .child(self.render_queue_panel(cx))
            .child(self.render_mv_card(cx))
            .child(self.render_theme_reveal(cx))
            .child(self.render_toast(cx))
    }

    /// 主题「开关灯」的波前光圈：新主题色的一道圆形波从开关位置扩散铺满窗口。
    ///
    /// Web 上那套 View Transitions 是「新主题整页快照 + `clip-path: circle()`」，
    /// gpui 0.2.2 做不到同样的效果，原因有二：
    /// 1. 没有整页快照 API（`grep screenshot` 无结果）；
    /// 2. `ContentMask` 只有矩形 `bounds`，元素层面**没有圆形裁剪**，所以也拿不到
    ///    「圆内显示新 UI、圆外显示旧 UI」的双层。
    ///
    /// 于是改成：**底层 UI 自己从旧主题插值到新主题**（见 `AppStore::theme`），
    /// 上层只画一圈半透明的「波前」——已扫过的区域只压很淡的一层新主题色，
    /// 波前那一圈稍浓，外沿渐隐到 0。这样内容全程都看得见（最浓处也只有 50%
    /// 覆盖），同时保留了「从点击处扩散」的动线。
    fn render_theme_reveal(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(r) = self.store.read(cx).theme_reveal.clone() else {
            return div().into_any_element();
        };
        let target_bg = self.store.read(cx).target_theme().bg_main;
        let origin = r.origin;
        let started = r.started;
        canvas(
            move |_, _, _| {},
            move |bounds, _, window, _| {
                let t =
                    (started.elapsed().as_secs_f32() / crate::theme::THEME_REVEAL_SECS).min(1.0f32);
                let grow = (t / crate::theme::THEME_REVEAL_GROW_FRAC).min(1.0);
                let e = 1.0 - (1.0 - grow).powi(3); // ease-out cubic：起步快、铺满稳
                // 圆心到最远角的距离 = 波前铺满窗口所需的半径
                let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let ox = f32::from(origin.x) - f32::from(bounds.origin.x);
                let oy = f32::from(origin.y) - f32::from(bounds.origin.y);
                let dx = (ox).max(w - ox);
                let dy = (oy).max(h - oy);
                let max_r = (dx * dx + dy * dy).sqrt();
                let rad = e * (max_r + 48.);
                if rad <= 0.5 {
                    return;
                }
                // 收尾：波前铺满后整体淡出（比底色渐变更早收，剩下的交给底色补上）
                let fade = ((t - crate::theme::THEME_REVEAL_GROW_FRAC)
                    / crate::theme::THEME_REVEAL_FADE_FRAC)
                    .clamp(0.0, 1.0);
                let alpha = 1.0 - fade * fade * (3.0 - 2.0 * fade); // smoothstep
                if alpha <= 0.002 {
                    return;
                }

                let (cx0, cy0) = (f32::from(origin.x), f32::from(origin.y));
                // 已扫过区压一层新主题色（浓度要够看出色差，又不能盖住内容）；
                // 波前那一圈更浓，外沿渐隐到 0，形成一道「扫过去」的波。
                const A_IN: f32 = 0.34;
                const A_PEAK: f32 = 0.60;
                const RINGS: usize = 14;
                let ring_w = (max_r * 0.30).clamp(56.0, 190.0);
                let inner_r = rad - ring_w;
                if inner_r > 0.5 {
                    paint_disc(window, cx0, cy0, inner_r, target_bg.opacity(A_IN * alpha));
                }
                let step = ring_w / RINGS as f32;
                for k in 0..RINGS {
                    let r_out = rad - step * k as f32;
                    if r_out <= 0.5 {
                        break;
                    }
                    // u: 0 = 波前外沿（还没扫到），1 = 内沿（已扫过）
                    let u = (k as f32 + 0.5) / RINGS as f32;
                    let a = if u <= 0.5 {
                        A_PEAK * (u / 0.5)
                    } else {
                        A_PEAK + (A_IN - A_PEAK) * ((u - 0.5) / 0.5)
                    };
                    let color = target_bg.opacity(a * alpha);
                    let r_in = r_out - step;
                    if r_in <= 0.5 {
                        paint_disc(window, cx0, cy0, r_out, color);
                    } else {
                        paint_ring(window, cx0, cy0, r_out, r_in, color);
                    }
                }
            },
        )
        .absolute()
        .size_full()
        .into_any_element()
    }

    // ---------- 播放队列面板 ----------
    /// 右侧浮层：当前播放队列（可点击切歌 / 清空 / 拖拽滚动条）。
    ///
    /// 用 `uniform_list`（行高固定 `QUEUE_ROW_H`）虚拟化：队列动辄几百上千首，
    /// 全量构建 div 每帧都要重建一遍，既卡又吃内存；虚拟化后只渲染可见的十几行，
    /// 而且内容高度可精确推算，滚动条能做到 1:1 跟手。
    fn render_queue_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.store.read(cx).theme();
        let s = self.store.read(cx);
        if !s.queue_open {
            return div().id("queue-panel").into_any_element();
        }
        let count = s.player.queue.len();
        let title = s.player.queue_title.clone();
        let handle = s.queue_scroll.clone();
        let bounds_slot = s.queue_bounds.clone();
        let store = self.store.clone();

        // 行数据在闭包里按需从 store 读，避免每帧深拷贝整个队列
        let rows_store = store.clone();
        let list = gpui::uniform_list("queue-rows", count, move |range, _window, cx: &mut App| {
            range
                .map(|ix| queue_row(&rows_store, ix, cx))
                .collect::<Vec<_>>()
        })
        .track_scroll(handle.clone())
        .size_full();

        // 滚动条浮层：点轨道跳转 + 按住拖动（拖拽起点存 store，跨帧保留）
        let drag_store = store.clone();
        let overlay = uniform_scrollbar_overlay("queue-sb", handle, move |cx, start| {
            drag_store.update(cx, |s, _| s.queue_drag = Some(start));
        });

        let hover_bg = theme.bg_hover;
        let clear_store = store.clone();
        let close_store = store.clone();

        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                // 标题 + 来源：来源可能很长（歌单名/搜索词），必须 truncate，
                // 否则会把右侧的按钮挤出面板
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(13.))
                            .child("播放队列"),
                    )
                    .when(!title.is_empty(), |el| {
                        el.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(theme.text_tertiary)
                                .child(ellipsize(&format!("· {title}"), 18)),
                        )
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(theme.text_tertiary)
                            .child(format!("{count} 首")),
                    ),
            )
            .child(
                div()
                    .id("queue-clear")
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .h(px(22.))
                    .px_2()
                    .rounded(px(6.))
                    .text_size(px(11.))
                    .text_color(theme.text_secondary)
                    .cursor_pointer()
                    .hover(move |el| el.bg(hover_bg))
                    // 用 on_mouse_down 而非 on_click：面板注册了 on_mouse_up（滚动条拖拽收尾），
                    // 在这种容器里子元素的 click 收不到 up 事件，按钮会「点不动」。
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        clear_queue(&clear_store, cx);
                        // 别让外层（root）把刚弹出的 toast 又清掉
                        cx.stop_propagation();
                    })
                    .child("清空"),
            )
            .child(
                div()
                    .id("queue-close")
                    .flex_shrink_0()
                    .size(px(22.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |el| el.bg(hover_bg))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        close_store.update(cx, |s, cx| {
                            s.queue_open = false;
                            cx.notify();
                        });
                    })
                    .child(icon("icons/close.svg", 12., theme.text_secondary)),
            );

        div()
            .id("queue-panel")
            // occlude：把整块面板设为命中测试终点，点击不会穿透到下方内容列表
            .occlude()
            .child({
                // 记录面板边界：点击外部时据此关闭（canvas 不绘制，也不参与命中测试）
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
            .absolute()
            .right(px(16.))
            .bottom(px(90.))
            .w(px(380.))
            .h(px(380.))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            // 拦截滚轮：避免滚动穿透到下方内容列表
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            // 拖拽监听挂在整块面板上：只挂在 10px 细条上，鼠标一拖出条就断
            .on_mouse_move(cx.listener(|root: &mut Root, ev: &MouseMoveEvent, _, cx: &mut Context<Root>| {
                let Some(start) = root.store.read(cx).queue_drag else {
                    return;
                };
                if ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let handle = root.store.read(cx).queue_scroll.clone();
                if apply_scrollbar_drag(&handle, start, f32::from(ev.position.y)) {
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|root: &mut Root, _, _, cx: &mut Context<Root>| {
                root.store.update(cx, |s, cx| {
                    if s.queue_drag.take().is_some() {
                        cx.notify();
                    }
                });
            }))
            .child(header)
            .child(if count == 0 {
                // 清空后的空态：否则只剩一块白板，看起来像坏了
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(theme.text_tertiary)
                    .child("队列是空的，去播放一首歌吧")
                    .into_any_element()
            } else {
                div()
                    .id("queue-list")
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(list)
                    .child(
                        div()
                            .absolute()
                            .top(px(0.))
                            .bottom(px(0.))
                            .right(px(0.))
                            .w(px(10.))
                            .child(overlay),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    // ---------- 侧边栏 ----------
    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.store.read(cx).theme();
        let backdrop = self.store.read(cx).backdrop_active();
        let collapsed = self.store.read(cx).sidebar_collapsed;
        let found_open = self.store.read(cx).sidebar_found_open;
        let lib_open = self.store.read(cx).sidebar_lib_open;
        let tab = self.store.read(cx).sidebar_tab;
        let user = self.store.read(cx).user.clone();
        let store = self.store.clone();
        let width = if collapsed { 64. } else { 230. };

        // ---- 账号区 ----
        let (nickname, subtitle) = match &user {
            Some(u) if !u.nickname.is_empty() => (u.nickname.clone(), "已登录酷狗账号".to_string()),
            Some(_) => ("酷狗用户".to_string(), "已登录".to_string()),
            None => ("未登录".to_string(), "点击登录账号".to_string()),
        };
        let account_store = store.clone();
        let logged_in = user.is_some();
        let avatar_pic = user.as_ref().map(|u| u.pic.clone()).filter(|p| !p.is_empty());
        let account = div()
            .id("account")
            .flex()
            .items_center()
            .gap_2()
            .mx_2()
            .px_2()
            .py_2()
            .rounded(px(10.))
            .cursor_pointer()
            .hover(move |s| s.bg(theme.bg_hover))
            .on_click(move |_, _, cx| {
                if logged_in {
                    navigate(&account_store, Page::Profile, cx);
                } else {
                    navigate(&account_store, Page::Login, cx);
                }
            })
            .child(
                div()
                    .size(px(36.))
                    .rounded_full()
                    .overflow_hidden()
                    .flex_shrink_0()
                    .bg(theme.bg_hover)
                    .flex()
                    .items_center()
                    .justify_center()
                    .when_some(avatar_pic, |el, pic| {
                        el.child(img(SharedUri::from(pic)).size_full().rounded_full().object_fit(ObjectFit::Cover))
                    })
                    .when(user.as_ref().map(|u| u.pic.is_empty()).unwrap_or(true), |el| {
                        el.child(icon("icons/user.svg", 18., theme.text_secondary))
                    }),
            )
            .when(!collapsed, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .min_w_0()
                                .child(div().text_size(px(13.)).truncate().child(nickname))
                                // VIP 标签
                                .when_some(
                                    user.as_ref().and_then(|u| vip_tag(u.vip_type)),
                                    |el, tag| el.child(tag),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme.text_tertiary)
                                .truncate()
                                .child(subtitle),
                        ),
                )
            });

        // ---- 分组标题（可折叠） ----
        let section_header = |store: &Entity<AppStore>,
                              label: &'static str,
                              open: bool,
                              toggle: fn(&mut AppStore),
                              cx: &Context<Root>| {
            let st = store.clone();
            let theme = store.read(cx).theme();
            div()
                .id(SharedString::from(format!("section-{label}")))
                .flex()
                .items_center()
                .justify_between()
                .mx_2()
                .px_2()
                .py_1p5()
                .cursor_pointer()
                .text_size(px(11.))
                .text_color(theme.text_tertiary)
                .hover(move |s| s.text_color(theme.text_secondary))
                .on_click(move |_, _, cx| {
                    st.update(cx, |s, cx| {
                        toggle(s);
                        cx.notify();
                    });
                })
                .child(div().child(label))
                .child(icon(
                    if open { "icons/chevron-up.svg" } else { "icons/chevron-down.svg" },
                    13.,
                    theme.text_tertiary,
                ))
        };

        // ---- 歌单 Tab 行 ----
        let tab_row = |store: &Entity<AppStore>, cx: &Context<Root>| {
            let theme = store.read(cx).theme();
            let active = store.read(cx).sidebar_tab;
            let make_tab = |idx: usize, label: &'static str, st: Entity<AppStore>| {
                let is_active = active == idx;
                div()
                    .id(SharedString::from(format!("pl-tab-{idx}")))
                    .text_size(px(11.5))
                    .cursor_pointer()
                    .when(is_active, |el| el.text_color(theme.accent))
                    .when(!is_active, |el| {
                        el.text_color(theme.text_tertiary)
                            .hover(move |s| s.text_color(theme.text_secondary))
                    })
                    .on_click(move |_, _, cx| {
                        st.update(cx, |s, cx| {
                            s.sidebar_tab = idx;
                            cx.notify();
                        });
                    })
                    .child(label)
            };
            let toast_store = store.clone();
            div()
                .flex()
                .items_center()
                .gap_2()
                .mx_2()
                .px_1()
                .pt_3()
                .pb_1()
                .child(make_tab(0, "自建歌单", store.clone()))
                .child(div().w(px(1.)).h(px(9.)).bg(theme.border))
                .child(make_tab(1, "收藏歌单", store.clone()))
                .child(div().flex_1())
                .child(div().child(icon("icons/sort.svg", 13., theme.text_tertiary)))
                .child(
                    div()
                        .id("pl-refresh")
                        .cursor_pointer()
                        .hover(move |s| s.opacity(0.7))
                        .on_click({
                            let st = store.clone();
                            move |_, _, cx| navigate(&st, Page::Home, cx)
                        })
                        .child(icon("icons/refresh.svg", 13., theme.text_tertiary)),
                )
                .child(
                    div()
                        .id("pl-plus")
                        .cursor_pointer()
                        .hover(move |s| s.opacity(0.7))
                        .on_click(move |_, _, cx| {
                            // 第三期：真的建一个歌单（酷狗云歌单接口）
                            crate::liked::create_playlist(&toast_store, cx);
                        })
                        .child(icon("icons/plus.svg", 14., theme.text_tertiary)),
                )
        };

        // ---- 本地歌单条目 ----
        let local_entry = |store: &Entity<AppStore>,
                           icon_path: &'static str,
                           name: &'static str,
                           page: Page,
                           cx: &Context<Root>| {
            let theme = store.read(cx).theme();
            let st = store.clone();
            let active = store.read(cx).routes.current() == Some(&page);
            div()
                .id(SharedString::from(format!("local-{name}")))
                .flex()
                .items_center()
                .gap_2()
                .mx_1()
                .px_2()
                .py_1()
                .rounded(px(8.))
                .cursor_pointer()
                .when(active, |el| el.bg(theme.bg_hover))
                .hover(move |s| s.bg(theme.bg_hover))
                .on_click(move |_, _, cx| navigate(&st, page.clone(), cx))
                .child(
                    div()
                        .size(px(28.))
                        .rounded(px(COVER_RADIUS))
                        .flex_shrink_0()
                        .bg(theme.bg_hover)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(icon_path, 15., theme.text_secondary)),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme.text_secondary)
                        .truncate()
                        .child(name),
                )
        };

        div()
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            // 材质激活时侧栏不再自画背景：它叠在根基层上会变成一块
            // 「灰色补丁」（两层半透明叠出 ~85% 不透明度，把材质全盖住），
            // 与内容区统一透出材质，靠右侧 1px 边框区分层级
            .when(!backdrop, |el| el.bg(theme.bg_sidebar))
            .border_r_1()
            .border_color(theme.border)
            // 顶部拖动区：拖动可移动窗口
            .child(
                div()
                    .id("sidebar-drag")
                    .w_full()
                    .h(px(8.))
                    .flex_shrink_0()
                    .on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, cx| {
                        if ev.click_count == 2 {
                            spawn_toggle_maximize(window.window_handle(), cx);
                        } else {
                            crate::win::start_drag(window);
                        }
                    }),
            )
            .child(div().pt_1().child(account))
            // ===== 发现音乐 =====
            .when(!collapsed, |el| {
                el.child(section_header(
                    &store,
                    "发现音乐",
                    found_open,
                    |s| s.sidebar_found_open = !s.sidebar_found_open,
                    cx,
                ))
            })
            .when(!collapsed && found_open || collapsed, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .px_1()
                        .child(self.quick_nav_item(&store, "icons/sparkles.svg", "为您推荐", Page::Home, cx))
                        .child(self.quick_nav_item(&store, "icons/compass.svg", "探索发现", Page::Explore, cx)),
                )
            })
            // ===== 我的乐库 =====
            .when(!collapsed, |el| {
                el.child(section_header(
                    &store,
                    "我的乐库",
                    lib_open,
                    |s| s.sidebar_lib_open = !s.sidebar_lib_open,
                    cx,
                ))
            })
            .when(!collapsed && lib_open || collapsed, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .px_1()
                        .child(self.quick_nav_item(&store, "icons/heart.svg", "我最喜爱", Page::Favorites, cx))
                        .child(self.quick_nav_item(&store, "icons/radio.svg", "私人FM", Page::Fm, cx))
                        .child(self.quick_nav_item(&store, "icons/cloud.svg", "音乐云盘", Page::Cloud, cx))
                        .child(self.quick_nav_item(&store, "icons/clock.svg", "播放历史", Page::History, cx))
                        .child(self.quick_nav_item(&store, "icons/bag.svg", "已购音乐", Page::Purchased, cx))
                )
            })
            // ===== 歌单 =====
            .when(!collapsed, |el| el.child(tab_row(&store, cx)))
            .child(
                // relative 包一层：叠加自绘滚动条
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("sidebar-playlists")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.sidebar_scroll)
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .pt_1()
                            .when(!collapsed && tab == 0, |el| {
                                el.child(local_entry(
                                    &store,
                                    "icons/heart.svg",
                                    "默认收藏",
                                    Page::Favorites,
                                    cx,
                                ))
                                // 登录用户的自建歌单（第三期：可点开、可删除）
                                .children(
                                    self.store
                                        .read(cx)
                                        .user_playlists
                                        .iter()
                                        .map(|pl| {
                                            let theme = self.store.read(cx).theme();
                                            let st = self.store.clone();
                                            let del_store = self.store.clone();
                                            let pid = pl.id.clone();
                                            let name = pl.name.clone();
                                            let page = Page::Playlist(pl.id.clone(), pl.name.clone());
                                            let active =
                                                self.store.read(cx).routes.current() == Some(&page);
                                            div()
                                                .id(SharedString::from(format!("upl-{pid}")))
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .mx_1()
                                                .px_2()
                                                .py_1()
                                                .rounded(px(8.))
                                                .cursor_pointer()
                                                .when(active, |el| el.bg(theme.bg_hover))
                                                .hover(move |s| s.bg(theme.bg_hover))
                                                .on_click(move |_, _, cx| {
                                                    navigate(&st, page.clone(), cx)
                                                })
                                                .child(icon(
                                                    "icons/list-music.svg",
                                                    12.,
                                                    theme.text_tertiary,
                                                ))
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .text_size(px(12.))
                                                        .truncate()
                                                        .child(name.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .id(SharedString::from(format!("upl-del-{pid}")))
                                                        .w(px(14.))
                                                        .h(px(14.))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .rounded(px(4.))
                                                        .text_size(px(11.))
                                                        .text_color(theme.text_tertiary)
                                                        .hover(move |s| {
                                                            s.bg(theme.bg_hover)
                                                                .text_color(gpui::red())
                                                        })
                                                        .on_click(move |ev, _, cx| {
                                                            // 别让点击冒泡到歌单条目（否则会先跳转再删除）
                                                            let _ = ev;
                                                            crate::liked::delete_playlist(
                                                                &del_store,
                                                                &pid,
                                                                &name,
                                                                cx,
                                                            );
                                                        })
                                                        .child("×"),
                                                )
                                                .into_any_element()
                                        })
                                        .collect::<Vec<_>>(),
                                )
                            })
                            .when(!collapsed && tab == 1, |el| {
                                el.child(
                                    div()
                                        .px_3()
                                        .py_3()
                                        .text_size(px(11.5))
                                        .text_color(theme.text_tertiary)
                                        .child(if user.is_some() {
                                            "暂未获取到收藏的歌单"
                                        } else {
                                            "登录后可查看收藏歌单"
                                        }),
                                )
                            }),
                    )
                    .child(
                        // 自绘滚动条浮层
                        div()
                            .absolute()
                            .top(px(4.))
                            .bottom(px(4.))
                            .right(px(0.))
                            .w(px(10.))
                            .child(scrollbar_overlay("sidebar-sb", &self.sidebar_scroll)),
                    ),
            )
            .child(
                // 底部工具
                div()
                    .flex()
                    .items_center()
                    .justify_around()
                    .px_3()
                    .py_3()
                    .child(
                        IconButton {
                            path: "icons/settings.svg",
                            size: 16.,
                            color: theme.text_secondary,
                            hover_bg: theme.bg_hover,
                            action: Box::new(|root: &mut Root, _, _, cx: &mut Context<Root>| {
                                navigate(&root.store.clone(), Page::Settings, cx);
                            }),
                        }
                        .render("tool-settings", cx),
                    )
                    .child(
                        // 深色显示太阳（点击切浅色），浅色显示月亮
                        IconButton {
                            path: if theme.dark { "icons/sun.svg" } else { "icons/moon.svg" },
                            size: 16.,
                            color: theme.text_secondary,
                            hover_bg: theme.bg_hover,
                            action: Box::new(|root: &mut Root, ev: &MouseDownEvent, _, cx: &mut Context<Root>| {
                                // 开光灯从开关位置开始扩散
                                cycle_theme(&root.store.clone(), Some(ev.position), cx);
                            }),
                        }
                        .render("tool-theme", cx),
                    )
            )
    }

    fn quick_nav_item(
        &self,
        store: &Entity<AppStore>,
        icon_path: &'static str,
        label: &str,
        page: Page,
        cx: &Context<Root>,
    ) -> impl IntoElement {
        let theme = store.read(cx).theme();
        let active = store.read(cx).routes.current() == Some(&page);
        let label = label.to_string();
        let store = store.clone();
        let collapsed = store.read(cx).sidebar_collapsed;
        let nav_store = store.clone();
        let nav_page = page.clone();
        div()
            .id(SharedString::from(format!("quick-{label}")))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(px(CONTROL_RADIUS))
            .cursor_pointer()
            .when(active, |el| {
                el.bg(theme.accent.opacity(0.10)).text_color(theme.accent)
            })
            .when(!active, |el| {
                el.text_color(theme.text_secondary)
                    .hover(move |s| s.bg(theme.bg_hover).text_color(theme.text))
            })
            .on_click(move |_, _, cx| navigate(&nav_store, nav_page.clone(), cx))
            .child(icon(icon_path, 16., if active { theme.accent } else { theme.text_secondary }))
            .when(!collapsed, |el| el.child(div().text_size(px(13.)).child(label)))
            .when(collapsed, |el| el.child(div()))
    }

    // ---------- 顶栏 ----------
    fn render_titlebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.store.read(cx).theme();
        // 材质激活时标题栏不再自画背景（与根基层同色，避免叠出一块深斑）
        let backdrop = self.store.read(cx).backdrop_active();
        let store = self.store.clone();
        let can_back = store.read(cx).routes.can_back();
        let can_forward = store.read(cx).routes.can_forward();
        let search_input = self.search_input.clone();
        let focus = self.search_focus.clone();
        // 直接问系统：最大化状态下按钮要变成「还原」
        let maximized = crate::win::is_maximized(window);

        div()
            .h(px(52.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .when(!backdrop, |el| el.bg(theme.bg_main))
            // 顶部细条：整行宽可拖动窗口
            .child(
                div()
                    .id("titlebar-top-drag")
                    .w_full()
                    .h(px(8.))
                    .flex_shrink_0()
                    // 不用 window_control_area：它靠「上一次鼠标移动算出的 hitbox」
                    // 做命中，鼠标刚从别的窗口移进来时匹配不到 → 拖不动；命中成功时
                    // 又会和这里补发的 WM_NCLBUTTONDOWN 叠出第二个系统移动循环 →
                    // 拖动掉帧。统一走 win::start_drag（见 win.rs 的说明）。
                    .on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, cx| {
                        if ev.click_count == 2 {
                            spawn_toggle_maximize(window.window_handle(), cx);
                        } else {
                            crate::win::start_drag(window);
                        }
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_3()
            // 后退 / 前进 / 刷新
            .child(
                IconButton {
                    path: "icons/arrow-left.svg",
                    size: 15.,
                    color: if can_back { theme.text } else { theme.text_tertiary },
                    hover_bg: theme.bg_hover,
                    action: Box::new(|root: &mut Root, _, _, cx: &mut Context<Root>| {
                        let moved = root.store.update(cx, |s, cx| {
                            let ok = s.routes.back();
                            if ok {
                                // 后退到的页面数据可能已被释放，需要重新加载
                                s.list_drag = None;
                                s.song_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
                                cx.notify();
                            }
                            ok
                        });
                        if moved {
                            let page = root.store.read(cx).routes.current().cloned();
                            if let Some(page) = page {
                                load_page(&root.store.clone(), &page, cx);
                            }
                        }
                    }),
                }
                .render("tb-back", cx),
            )
            .child(
                IconButton {
                    path: "icons/arrow-right.svg",
                    size: 15.,
                    color: if can_forward { theme.text } else { theme.text_tertiary },
                    hover_bg: theme.bg_hover,
                    action: Box::new(|root: &mut Root, _, _, cx: &mut Context<Root>| {
                        let moved = root.store.update(cx, |s, cx| {
                            let ok = s.routes.forward();
                            if ok {
                                s.list_drag = None;
                                s.song_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
                                cx.notify();
                            }
                            ok
                        });
                        if moved {
                            let page = root.store.read(cx).routes.current().cloned();
                            if let Some(page) = page {
                                load_page(&root.store.clone(), &page, cx);
                            }
                        }
                    }),
                }
                .render("tb-forward", cx),
            )
            .child(
                IconButton {
                    path: "icons/refresh.svg",
                    size: 15.,
                    color: theme.text,
                    hover_bg: theme.bg_hover,
                    action: Box::new(|root: &mut Root, _, _, cx: &mut Context<Root>| {
                        let page = root.store.read(cx).routes.current().cloned();
                        if let Some(page) = page {
                            navigate(&root.store.clone(), page, cx);
                        }
                    }),
                }
                .render("tb-refresh", cx),
            )
            .child(
                // 搜索框
                div()
                    .id("search-box")
                    .relative()
                    .flex()
                    .items_center()
                    .gap_1()
                    .w(px(340.))
                    .h(px(34.))
                    .px_2()
                    .rounded_full()
                    .bg(theme.bg_hover)
                    .border_1()
                    .border_color(theme.border)
                    .cursor_text()
                    .on_mouse_down(MouseButton::Left, {
                        let focus = focus.clone();
                        move |_: &MouseDownEvent, window: &mut Window, _: &mut App| {
                            window.focus(&focus);
                        }
                    })
                    .child({
                        // 记录输入框区域：点到区域外时让输入框失焦（光标消失）
                        let slot = self.store.read(cx).search_bounds.clone();
                        canvas(
                            move |bounds: Bounds<Pixels>, _, _| {
                                *slot.lock() = Some(bounds);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                    })
                    .child(icon("icons/search.svg", 13., theme.text_tertiary))
                    .child(search_input),
            )
            .child(
                IconButton {
                    path: "icons/mic.svg",
                    size: 15.,
                    color: theme.text,
                    hover_bg: theme.bg_hover,
                    action: Box::new(|_, _, _, _| {}),
                }
                .render("tb-mic", cx),
            )
            // 拖动区：单击拖动窗口、双击最大化/还原
            .child(
                div()
                    .id("titlebar-drag")
                    .flex_1()
                    .h_full()
                    .on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, cx| {
                        if ev.click_count == 2 {
                            // 双击最大化/还原
                            spawn_toggle_maximize(window.window_handle(), cx);
                        } else {
                            crate::win::start_drag(window);
                        }
                    }),
            )
            // 窗口控制
            .child(
                div()
                    .flex()
                    .items_center()
                    .mr_2()
                    // mini 模式：开 mini 播放器并隐藏主窗口（播放不受影响）
                    .child(
                        IconButton {
                            path: "icons/pip.svg",
                            size: 14.,
                            color: theme.text_secondary,
                            hover_bg: theme.bg_hover,
                            action: Box::new(|root: &mut Root, _, window, cx| {
                                crate::desktop::open_mini_player(&root.store.clone(), cx);
                                crate::win::hide_window(window);
                            }),
                        }
                        .render("tb-mini", cx),
                    ),
            )
            .child(
                IconButton {
                    path: "icons/minimize.svg",
                    size: 14.,
                    color: theme.text,
                    hover_bg: theme.bg_hover,
                    action: Box::new(|_, _, window, _| window.minimize_window()),
                }
                .render("tb-min", cx),
            )
            .child(
                IconButton {
                    // 最大化后换成「还原」图标（两个叠起来的方框）
                    path: if maximized {
                        "icons/restore.svg"
                    } else {
                        "icons/maximize.svg"
                    },
                    size: 13.,
                    color: theme.text,
                    hover_bg: theme.bg_hover,
                    // gpui 的 zoom_window 只会 SW_MAXIMIZE（点第二次回不去），
                    // 这里自己判断状态做 最大化/还原 切换。
                    //
                    // 必须「跳出去」再做：在 gpui 的事件派发/绘制过程中改窗口，会因为
                    // resize 回调重入 `window.update` 失败（被 log_err 吞掉）而只改窗口
                    // 尺寸、不改布局 —— 表现就是「只是挪到了左上角」。放到定时任务里
                    // 执行，等价于用户自己按 Win+Up。
                    action: Box::new(|_, _, window, cx: &mut Context<Root>| {
                        spawn_toggle_maximize(window.window_handle(), cx);
                    }),
                }
                .render("tb-max", cx),
            )
            .child(
                IconButton {
                    path: "icons/close.svg",
                    size: 14.,
                    color: theme.text,
                    hover_bg: gpui::red().opacity(0.7),
                    // 关闭只隐藏窗口，App 继续在托盘里跑；退出走托盘菜单
                    action: Box::new(|_, _, window, _| crate::win::hide_window(window)),
                }
                .render("tb-close", cx),
            )
            )
    }

    // ---------- 内容分发 ----------
    fn render_content(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.store.read(cx).routes.current().cloned();
        match page {
            Some(Page::Home) => self.render_home(cx).into_any_element(),
            Some(Page::Explore) => self.render_explore(cx).into_any_element(),
            Some(Page::Ranking) => self.render_ranking(cx).into_any_element(),
            Some(Page::DailyRecommend) => self.render_daily(cx).into_any_element(),
            Some(Page::Login) => self.render_login(cx).into_any_element(),
            Some(Page::Profile) => self.render_profile(cx).into_any_element(),
            Some(Page::Settings) => self.render_settings(cx).into_any_element(),
            Some(Page::Favorites) => self.render_library(true, cx).into_any_element(),
            Some(Page::Fm) => self.render_fm(cx).into_any_element(),
            Some(Page::Lyric) => div().into_any_element(),
            Some(Page::Cloud) => self.render_cloud(cx).into_any_element(),
            Some(Page::Purchased) => self.render_purchased(cx).into_any_element(),
            Some(Page::History) => self.render_library(false, cx).into_any_element(),
            Some(Page::Artist(id, name)) => self.render_artist(&id, &name, cx).into_any_element(),
            Some(Page::Album(id, name)) => self.render_album(&id, &name, cx).into_any_element(),
            Some(Page::Rank(id, name)) => self.render_rank_detail(&id, &name, cx).into_any_element(),
            Some(Page::Search(kw)) => self.render_search(&kw, cx).into_any_element(),
            Some(Page::Playlist(_, name)) => self.render_playlist(&name, cx).into_any_element(),
            Some(Page::Placeholder(title)) => render_placeholder(title, &self.store.read(cx).theme()).into_any_element(),
            None => div().into_any_element(),
        }
    }

    // ---------- 首页 ----------
    fn render_home(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.store.read(cx).theme();
        let home = self.store.read(cx).home.clone();
        let greeting = {
            let hour = chrono::Local::now().hour();
            match hour {
                0..=5 => "凌晨好",
                6..=8 => "早上好",
                9..=11 => "上午好",
                12 => "中午好",
                13..=17 => "下午好",
                _ => "晚上好",
            }
        };

        div().flex().flex_col().gap_4().p_6()
            // 问候
            .child(
                div().flex().flex_col().gap_1()
                    .child(div().text_size(px(24.)).child(format!("Hi, {greeting}")))
                    .child(div().text_size(px(13.)).text_color(theme.text_secondary).child("由此开启好心情～")),
            )
            // 快捷卡
            .child(
                div().grid().grid_cols(2).gap_4()
                    .child(self.quick_card("每日推荐", "为你量身定制", "icons/calendar.svg", theme.accent, Page::DailyRecommend, cx))
                    .child(self.quick_card("排行榜", "实时热门趋势", "icons/star.svg", gpui::blue(), Page::Ranking, cx)),
            )
            // 风格推荐（分类 tab + 标签 chips + 歌曲网格）
            .child(self.render_style_section(&home, cx))
            // 推荐歌单
            .child(
                div().flex().flex_col().gap_3()
                    .child(div().text_size(px(17.)).child("推荐歌单"))
                    .child(
                        div().grid().grid_cols(4).gap_4()
                            .children(home.recommend_playlists.iter().map(|pl| {
                                self.playlist_card(pl, cx)
                            })),
                    )
                    .when(home.recommend_playlists.is_empty(), |el| {
                        el.child(self.empty_hint("推荐歌单加载中…", &theme))
                    }),
            )
            // 编辑精选
            .when(!home.editor_playlists.is_empty(), |el| {
                el.child(
                    div().flex().flex_col().gap_3()
                        .child(div().text_size(px(17.)).child("编辑精选"))
                        .child(
                            div().grid().grid_cols(4).gap_4()
                                .children(home.editor_playlists.iter().map(|pl| {
                                    self.playlist_card(pl, cx)
                                })),
                        ),
                )
            })
    }

    fn quick_card(&self, title: &str, subtitle: &str, icon_path: &'static str, color: Hsla, page: Page, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        let nav_store = self.store.clone();
        div()
            .id(SharedString::from(format!("qcard-{title}")))
            .flex()
            .items_center()
            .gap_3()
            .p_3()
            .rounded(px(12.))
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border)
            .cursor_pointer()
            .hover(move |s| s.bg(theme.bg_hover))
            .on_click({
                let nav_store = nav_store.clone();
                let page = page.clone();
                move |_, _, cx| navigate(&nav_store, page.clone(), cx)
            })
            .child(
                div()
                    .size(px(44.))
                    .rounded(px(10.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(color.opacity(0.9))
                    .text_color(gpui::white())
                    .child(div().text_size(px(16.)).child(icon_path_short(icon_path))),
            )
            .child(
                div().flex().flex_col().gap_0p5()
                    .child(div().text_size(px(14.)).child(title.to_string()))
                    .child(div().text_size(px(11.)).text_color(theme.text_secondary).child(subtitle.to_string())),
            )
    }

    /// 风格推荐区块：分类切换 + 多选标签 + 歌曲网格。
    fn render_style_section(&self, home: &crate::state::HomeData, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        let store = self.store.clone();
        let categories = home.style_categories.clone();
        let active = home.active_category;
        let selected = home.selected_tags.clone();
        let songs = home.style_songs.clone();
        let loading = home.style_loading;
        let tags: Vec<echo_api::StyleTag> = home.active_tags().to_vec();

        // 分类 tabs
        let cat_store = store.clone();
        let cat_tabs: Vec<AnyElement> = categories
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let st = cat_store.clone();
                let name = c.name.clone();
                let is_active = i == active;
                div()
                    .id(SharedString::from(format!("style-cat-{name}")))
                    .px_3()
                    .py(px(6.))
                    .rounded_full()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(is_active, |el| {
                        el.bg(theme.accent.opacity(0.12)).text_color(theme.accent)
                    })
                    .when(!is_active, |el| {
                        el.text_color(theme.text_secondary).hover(move |s| s.bg(theme.bg_hover))
                    })
                    .on_click(move |_, _, cx| {
                        st.update(cx, |s, cx| {
                            s.home.active_category = i;
                            cx.notify();
                        });
                    })
                    .child(name)
                    .into_any_element()
            })
            .collect();

        // 标签 chips
        let tag_store = store.clone();
        let selected_set: Vec<String> = selected.clone();
        let tag_chips: Vec<AnyElement> = tags
            .iter()
            .map(|tag| {
                let st = tag_store.clone();
                let id = tag.id.clone();
                let name = tag.name.clone();
                let on = selected_set.contains(&id);
                div()
                    .id(SharedString::from(format!("style-tag-{id}")))
                    .px_3()
                    .py(px(5.))
                    .rounded_full()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .border_1()
                    .when(on, |el| {
                        el.bg(theme.accent).border_color(theme.accent).text_color(gpui::white())
                    })
                    .when(!on, |el| {
                        el.border_color(theme.border)
                            .text_color(theme.text_secondary)
                            .hover(move |s| s.bg(theme.bg_hover))
                    })
                    .on_click(move |_, _, cx| {
                        let tag_id = id.clone();
                        st.update(cx, |s, cx| {
                            if let Some(pos) = s.home.selected_tags.iter().position(|t| *t == tag_id) {
                                s.home.selected_tags.remove(pos);
                            } else {
                                s.home.selected_tags.push(tag_id);
                            }
                            cx.notify();
                        });
                        load_style(&st, cx);
                    })
                    .child(name)
                    .into_any_element()
            })
            .collect();

        // 歌曲网格
        let song_cards: Vec<AnyElement> = songs
            .iter()
            .enumerate()
            .map(|(i, song)| self.song_card(song, i, cx).into_any_element())
            .collect();

        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(px(14.))
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(icon("icons/sparkles.svg", 15., theme.accent))
                            .child(div().text_size(px(16.)).child("风格推荐")),
                    )
                    .child(
                        div()
                            .id("play-all-style")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_3()
                            .py(px(5.))
                            .rounded_full()
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(gpui::white())
                            .bg(theme.accent)
                            .hover(|s| s.opacity(0.88))
                            .on_click({
                                let st = store.clone();
                                move |_, _, cx| play_from(&st, PlaySource::StyleSongs, 0, cx)
                            })
                            .child(icon("icons/play.svg", 11., gpui::white()))
                            .child("播放全部"),
                    ),
            )
            .when(!cat_tabs.is_empty(), |el| {
                el.child(div().flex().flex_wrap().gap_2().children(cat_tabs))
            })
            .when(!tag_chips.is_empty(), |el| {
                el.child(div().flex().flex_wrap().gap_2().children(tag_chips))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .when(songs.is_empty(), |el| {
                        el.child(
                            div()
                                .py_6()
                                .text_size(px(12.))
                                .text_color(theme.text_tertiary)
                                .child(if loading { "风格推荐加载中…" } else { "暂无推荐歌曲" }),
                        )
                    })
                    .when(!songs.is_empty(), |el| {
                        el.child(div().grid().grid_cols(3).gap_2().children(song_cards))
                    }),
            )
    }

    /// 歌曲卡片（封面 + 标题 + 歌手）。
    fn song_card(&self, song: &Song, index: usize, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        let playing = self.store.read(cx).player.is_current(song);
        let store = self.store.clone();
        div()
            .id(SharedString::from(format!("song-{}-{index}", song.key())))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded(px(10.))
            .cursor_pointer()
            .when(playing, |el| el.bg(theme.accent.opacity(0.08)))
            .when(!playing, |el| el.hover(move |s| s.bg(theme.bg_hover)))
            .on_click({
                let store = store.clone();
                move |_, _, cx| play_from(&store, PlaySource::StyleSongs, index, cx)
            })
            .child(
                div()
                    .size(px(44.))
                    .rounded(px(COVER_RADIUS))
                    .overflow_hidden()
                    .flex_shrink_0()
                    .bg(theme.bg_hover)
                    .when(!song.cover_url.is_empty(), |el: Div| {
                        let cover = song.cover_url.clone();
                        el.child(img(SharedUri::from(cover)).size_full().rounded(px(COVER_RADIUS)).object_fit(ObjectFit::Cover))
                    }),
            )
            .child(
                div().flex().flex_col().gap_0p5().min_w_0().flex_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .min_w_0()
                            .child(
                                div().text_size(px(13.)).truncate()
                                    .text_color(if playing { theme.accent } else { theme.text })
                                    .child(song.title.clone()),
                            )
                            .when_some(song_tag(song, &theme), |el, tag| el.child(tag)),
                    )
                    .child(
                        div().text_size(px(11.)).text_color(theme.text_secondary).truncate()
                            .child(song.artist.clone()),
                    ),
            )
    }

    pub(crate) fn playlist_card(&self, pl: &echo_core::Playlist, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = self.store.read(cx).theme();
        let nav_store = self.store.clone();
        let page = Page::Playlist(pl.id.clone(), pl.name.clone());
        div()
            .id(SharedString::from(format!("plc-{}", pl.id)))
            .w_full()
            .flex()
            .flex_col()
            .gap_1p5()
            .cursor_pointer()
            .on_click(move |_, _, cx| navigate(&nav_store, page.clone(), cx))
            .child({
                // 正方形封面盒：ObjectFit::Cover 会溢出容器，只有正方形才能让圆角可见
                let mut cover = div()
                    .w_full()
                    .rounded(px(COVER_RADIUS))
                    .overflow_hidden()
                    .bg(theme.bg_hover)
                    .flex()
                    .items_center()
                    .justify_center();
                cover.style().aspect_ratio = Some(1.0);
                cover.when(!pl.pic.is_empty(), |el: Div| {
                    let pic = pl.pic.clone();
                    el.child(img(SharedUri::from(pic)).size_full().rounded(px(COVER_RADIUS)).object_fit(ObjectFit::Cover))
                })
            })
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .truncate()
                    .child(pl.name.clone()),
            )
    }

    pub(crate) fn empty_hint(&self, text: &str, theme: &crate::theme::Theme) -> impl IntoElement + use<> {
        div()
            .p_8()
            .text_color(theme.text_tertiary)
            .text_size(px(13.))
            .child(text.to_string())
    }

    // ---------- 搜索页 ----------
    fn render_search(&mut self, kw: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.store.read(cx).theme();
        let search = self.store.read(cx).search.clone();
        let hot = self.store.read(cx).search_hot.clone();
        let store = self.store.clone();
        let tab = search.tab;

        // 加载态：居中旋转动画
        if search.loading {
            return centered_spinner(theme);
        }

        // 分类 Tab
        let tabs: Vec<AnyElement> = ["单曲", "歌单", "歌手", "专辑", "歌词", "MV"]
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let active = i == tab;
                let st = store.clone();
                let kw = kw.to_string();
                div()
                    .id(SharedString::from(format!("search-tab-{i}")))
                    .px_3()
                    .py(px(4.))
                    .rounded_full()
                    .text_size(px(12.5))
                    .cursor_pointer()
                    .when(active, |el| el.bg(theme.accent).text_color(gpui::white()))
                    .when(!active, |el| {
                        el.text_color(theme.text_secondary)
                            .hover(move |s| s.bg(theme.bg_hover).text_color(theme.text))
                    })
                    .on_click(move |_, _, cx| {
                        st.update(cx, |s, cx| {
                            s.search.tab = i;
                            cx.notify();
                        });
                        load_search_tab(&st, &kw, i, cx);
                    })
                    .child(*label)
                    .into_any_element()
            })
            .collect();

        let body: AnyElement = match tab {
            1 => {
                let cards: Vec<AnyElement> = search
                    .playlists
                    .iter()
                    .map(|pl| self.playlist_card(pl, cx).into_any_element())
                    .collect();
                if cards.is_empty() {
                    self.empty_hint("没有找到相关歌单", &theme).into_any_element()
                } else {
                    div().grid().grid_cols(5).gap_4().children(cards).into_any_element()
                }
            }
            2 => {
                let cards: Vec<AnyElement> = search
                    .artists
                    .iter()
                    .map(|ar| {
                        let nav = self.store.clone();
                        let page = Page::Artist(ar.id.clone(), ar.name.clone());
                        let pic = ar.pic.clone();
                        let name = ar.name.clone();
                        div()
                            .id(SharedString::from(format!("sr-artist-{}", ar.id)))
                            .w_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .on_click(move |_, _, cx| navigate(&nav, page.clone(), cx))
                            .child(
                                div()
                                    .size(px(96.))
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
                            .child(div().text_size(px(12.5)).truncate().child(name))
                            .into_any_element()
                    })
                    .collect();
                if cards.is_empty() {
                    self.empty_hint("没有找到相关歌手", &theme).into_any_element()
                } else {
                    div().grid().grid_cols(5).gap_4().children(cards).into_any_element()
                }
            }
            3 => {
                let cards: Vec<AnyElement> = search
                    .albums
                    .iter()
                    .map(|a| self.album_card(a, cx).into_any_element())
                    .collect();
                if cards.is_empty() {
                    self.empty_hint("没有找到相关专辑", &theme).into_any_element()
                } else {
                    div().grid().grid_cols(5).gap_4().children(cards).into_any_element()
                }
            }
            // ===== 歌词搜索结果（第三期搜索补全）=====
            4 => {
                if search.lyrics.is_empty() {
                    self.empty_hint("没有找到相关歌词", &theme).into_any_element()
                } else {
                    let rows: Vec<AnyElement> = search
                        .lyrics
                        .iter()
                        .map(|c| {
                            let nav_store = store.clone();
                            let title = c.title.clone();
                            let click_title = title.clone();
                            let artist = c.artist.clone();
                            let click_artist = artist.clone();
                            let dur = c.duration;
                            div()
                                .id(SharedString::from(format!(
                                    "lyric-{}-{}",
                                    c.id,
                                    title.chars().take(12).collect::<String>()
                                )))
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_3()
                                .py_2()
                                .rounded(px(8.))
                                .cursor_pointer()
                                .hover(move |el| el.bg(theme.bg_hover))
                                .on_click(move |_, _, cx| {
                                    // 歌词本身不可独立播放：点击跳去搜这首歌
                                    let kw = if click_artist.is_empty() {
                                        click_title.clone()
                                    } else {
                                        format!("{click_artist} {click_title}")
                                    };
                                    navigate(&nav_store, Page::Search(kw), cx);
                                })
                                .child(
                                    div()
                                        .w(px(18.))
                                        .text_size(px(13.))
                                        .text_color(theme.accent)
                                        .child(icon("icons/list-music.svg", 14., theme.accent)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(px(13.5))
                                                .truncate()
                                                .child(title),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .text_color(theme.text_tertiary)
                                                .child(artist),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .text_color(theme.text_tertiary)
                                        .child(format!("{}:{:02}", dur / 60, dur % 60)),
                                )
                                .into_any_element()
                        })
                        .collect();
                    div().flex().flex_col().gap_1().children(rows).into_any_element()
                }
            }
            // ===== MV 搜索结果（第三期搜索补全）=====
            5 => {
                if search.mvs.is_empty() {
                    self.empty_hint("没有找到相关 MV", &theme).into_any_element()
                } else {
                    let rows: Vec<AnyElement> = search
                        .mvs
                        .iter()
                        .map(|m| {
                            let url = format!(
                                "https://www.kugou.com/mvweb/html/mv_{}.html",
                                m.mvhash.to_lowercase()
                            );
                            let title = m.title.clone();
                            let singer = m.singer.clone();
                            let quality = m.quality.clone();
                            let heat = m.heat;
                            let dur = m.duration;
                            let mv_store = store.clone();
                            div()
                                .id(SharedString::from(format!(
                                    "mv-{}",
                                    m.mvhash.chars().take(10).collect::<String>()
                                )))
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_3()
                                .py_2()
                                .rounded(px(8.))
                                .cursor_pointer()
                                .hover(move |el| el.bg(theme.bg_hover))
                                .on_click(move |_, _, cx| {
                                    // 视频播放不在本 app 能力范围内：交给浏览器（与 MV 卡片一致）
                                    #[cfg(windows)]
                                    {
                                        let _ = std::process::Command::new("cmd")
                                            .args(["/c", "start", "", &url])
                                            .spawn();
                                    }
                                    #[cfg(not(windows))]
                                    {
                                        let _ = std::process::Command::new("open")
                                            .arg(&url)
                                            .spawn();
                                    }
                                    mv_store.update(cx, |s, cx| {
                                        s.toast = "已在浏览器中打开 MV".into();
                                        cx.notify();
                                    });
                                })
                                .child(
                                    div()
                                        .w(px(18.))
                                        .text_size(px(13.))
                                        .text_color(theme.accent)
                                        .child(icon("icons/play.svg", 14., theme.accent)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(px(13.5))
                                                .truncate()
                                                .child(title),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .text_color(theme.text_tertiary)
                                                .child(singer),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(if quality.is_empty() {
                                            theme.text_tertiary
                                        } else {
                                            theme.accent
                                        })
                                        .when(!quality.is_empty(), |el| el.child(quality)),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .text_color(theme.text_tertiary)
                                        .child(format!("{}:{:02}", dur / 60, dur % 60)),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .text_color(theme.text_tertiary)
                                        .child(format_heat(heat)),
                                )
                                .into_any_element()
                        })
                        .collect();
                    div().flex().flex_col().gap_1().children(rows).into_any_element()
                }
            }
            _ => {
                if search.songs.is_empty() && !search.loaded {
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(theme.text_secondary)
                                .child("热门搜索"),
                        )
                        .child(
                            div().flex().flex_wrap().gap_2().children(hot.iter().take(20).map(|w| {
                                let theme = self.store.read(cx).theme();
                                let nav_store = self.store.clone();
                                let label = w.clone();
                                let kw = w.clone();
                                div()
                                    .id(SharedString::from(format!("hot-{w}")))
                                    .px_3()
                                    .py_1p5()
                                    .rounded_full()
                                    .bg(theme.bg_panel)
                                    .border_1()
                                    .border_color(theme.border)
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .hover(move |s| s.bg(theme.bg_hover))
                                    .on_click(move |_, _, cx| {
                                        navigate(&nav_store, Page::Search(kw.clone()), cx)
                                    })
                                    .child(label)
                            })),
                        )
                        .into_any_element()
                } else if search.songs.is_empty() {
                    self.empty_hint("没有找到相关歌曲", &theme).into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(self.render_song_list(search.songs.clone(), PlaySource::Search, cx))
                        .into_any_element()
                }
            }
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            // 单曲 tab 用虚拟化列表（撑满高度、自己滚动）
            .when(tab == 0, |el| el.h_full())
            .child(div().text_size(px(20.)).child(format!("搜索：{kw}")))
            .child(div().flex().items_center().gap_2().children(tabs))
            .child(body)
    }

    fn render_playlist(&mut self, name: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.store.read(cx).theme();
        let detail = self.store.read(cx).playlist_detail.clone();
        let store = self.store.clone();
        let list: Option<AnyElement> = detail.as_ref().and_then(|(_, songs)| {
            if songs.is_empty() {
                None
            } else {
                Some(
                    self.render_song_list(songs.clone(), PlaySource::PlaylistDetail, cx)
                        .into_any_element(),
                )
            }
        });

        div().flex().flex_col().h_full().gap_4().p_6()
            .child(
                div().flex().items_end().gap_4()
                    .child(
                        div()
                            .size(px(140.))
                            .rounded(px(CARD_RADIUS))
                            .overflow_hidden()
                            .bg(theme.bg_hover)
                            .when(detail.as_ref().map(|(pl, _)| !pl.pic.is_empty()).unwrap_or(false), |el: Div| {
                                let pic = detail.as_ref().unwrap().0.pic.clone();
                                el.child(img(SharedUri::from(pic)).size_full().rounded(px(CARD_RADIUS)).object_fit(ObjectFit::Cover))
                            }),
                    )
                    .child(
                        div().flex().flex_col().gap_2().pb_2().flex_1().min_w_0()
                            .child(div().text_size(px(22.)).truncate().child(format!("歌单：{name}")))
                            .when_some(detail.as_ref().map(|(pl, _)| pl.nickname.clone()), |el, nickname| {
                                el.child(div().text_size(px(12.)).text_color(theme.text_secondary).child(format!("创建者：{nickname}")))
                            })
                            .when_some(detail.as_ref().map(|(_, songs)| songs.len()), |el, count| {
                                el.child(div().text_size(px(12.)).text_color(theme.text_secondary).child(format!("共 {count} 首")))
                            }),
                    )
                    .child(
                        div()
                            .id("playlist-play-all")
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .px_3()
                            .py(px(5.))
                            .mb_2()
                            .rounded_full()
                            .bg(theme.accent)
                            .text_color(gpui::white())
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.88))
                            .on_click(move |_, _, cx| {
                                play_from(&store, PlaySource::PlaylistDetail, 0, cx)
                            })
                            .child(icon("icons/play.svg", 11., gpui::white()))
                            .child("播放全部"),
                    ),
            )
            .when_some(list, |el, list| {
                el.child(div().flex_1().min_h_0().child(list))
            })
            .when(detail.is_none(), |el: Div| {
                el.child(self.empty_hint("歌单加载中…", &theme))
            })
    }

    pub(crate) fn song_row_with_source(
        &self,
        song: &Song,
        index: usize,
        source: PlaySource,
        cx: &Context<Self>,
    ) -> AnyElement {
        song_row(&self.store, song, index, &source, cx)
    }

    /// 虚拟化歌曲列表：用 `uniform_list` 只渲染视口内的行（长列表不再全量构建）。
    /// 调用方必须给该元素确定高度（放在 `flex_1().min_h_0()` 容器里）。
    pub(crate) fn render_song_list(
        &self,
        songs: Arc<Vec<Song>>,
        source: PlaySource,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let handle = self.store.read(cx).song_scroll.clone();
        let count = songs.len();
        let st = self.store.clone();
        let sb_store = self.store.clone();
        // uniform_list：行高统一（SONG_ROW_H），只渲染可见区间。
        // 相比 gpui::list，它不依赖逐项测量，内容高度 = 行数 × 行高，
        // 因此滚动条位置/比例是精确的，拖拽能 1:1 跟手。
        let list = gpui::uniform_list(
            "song-list",
            count,
            move |range, _window, cx: &mut App| {
                range
                    .map(|ix| song_row(&st, &songs[ix], ix, &source, cx))
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(handle.clone())
        // uniform_list 需要确定高度，否则不渲染
        .size_full();
        div()
            .size_full()
            .relative()
            // 拖拽监听挂在整块列表上（挂在 10px 细条上拖出边界就断）
            .on_mouse_move(cx.listener(|root: &mut Root, ev: &MouseMoveEvent, _, cx: &mut Context<Root>| {
                let Some(start) = root.store.read(cx).list_drag else {
                    return;
                };
                if ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let handle = root.store.read(cx).song_scroll.clone();
                if apply_scrollbar_drag(&handle, start, f32::from(ev.position.y)) {
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|root: &mut Root, _, _, cx: &mut Context<Root>| {
                root.store.update(cx, |s, cx| {
                    if s.list_drag.take().is_some() {
                        cx.notify();
                    }
                });
            }))
            .child(list)
            .child(
                // 整条右侧 10px 作为滚动条热区（和列表同高，便于和 bounds 精确对齐）
                div()
                    .absolute()
                    .top(px(0.))
                    .bottom(px(0.))
                    .right(px(0.))
                    .w(px(10.))
                    .child(uniform_scrollbar_overlay("song-sb", handle, move |cx, start| {
                        sb_store.update(cx, |s, _| s.list_drag = Some(start));
                    })),
            )
    }

    // ---------- 占位页 ----------
    // render_placeholder 在模块底部独立函数

    // ---------- 播放条 ----------
    fn render_player_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.store.read(cx).theme();
        let backdrop = self.store.read(cx).backdrop_active();
        let s = self.store.read(cx);
        let current = s.player.current().cloned();
        let playing = s.player.playing;
        let loading = s.player.loading;
        let position = s.player.position;
        let duration = s.player.duration.max(0.001);
        let volume = s.player.volume;
        let mode = s.player.play_mode;
        let mode_icon = match mode {
            echo_core::PlayMode::Sequential => "icons/queue.svg",
            echo_core::PlayMode::List => "icons/repeat.svg",
            echo_core::PlayMode::Random => "icons/shuffle.svg",
            echo_core::PlayMode::Single => "icons/repeat-one.svg",
        };
        let store = self.store.clone();
        let accent = theme.accent;
        let search_input = self.search_input.clone();

        let progress_ratio = (position / duration).clamp(0.0, 1.0);
        let progress_bounds_slot = self.progress_bounds.clone();
        let volume_bounds_slot = self.volume_bounds.clone();
        let progress_paint = (progress_ratio, accent, theme.progress_track);

        div()
            .h(px(84.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px_4()
            .gap_4()
            .bg(if backdrop {
                // 播放条最浓，给底部一个稳定的锚（文字/控件都落在这里）
                theme.bg_panel.opacity(0.90)
            } else {
                theme.bg_panel
            })
            .border_t_1()
            .border_color(theme.border)
            // 拖拽（进度条 / 音量条）统一挂在播放条上：
            // gpui 0.2.2 的 on_mouse_move 要求 hitbox 仍 hover，
            // 挂在细条上拖出边界就断，挂在整条播放条上更稳。
            // 注意：拖拽过程中只更新 UI 目标位置，不调用 seek —— 连续 seek 会
            // 阻塞音频控制线程并产生杂音；真正的 seek 在松手时执行一次。
            .on_mouse_move(cx.listener(|root: &mut Root, ev: &MouseMoveEvent, _, cx: &mut Context<Root>| {
                if ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                root.store.update(cx, |s, cx| {
                    if s.seeking {
                        if let Some(b) = s.progress_bounds_slot.lock().clone() {
                            let ratio = f32::from(ev.position.x - b.left()).max(0.0)
                                / f32::from(b.size.width).max(1.0);
                            let target = (ratio as f64).clamp(0.0, 1.0) * s.player.duration;
                            s.seek_target = target;
                            s.player.position = target;
                        }
                    } else if s.volume_open {
                        if let Some(b) = s.volume_bounds_slot.lock().clone() {
                            let ratio = f32::from(ev.position.x - b.left()).max(0.0)
                                / f32::from(b.size.width).max(1.0);
                            s.player.volume = (ratio as f64).clamp(0.0, 1.0);
                            s.engine.set_volume(s.player.volume);
                            s.settings.update(|st| st.volume = s.player.volume);
                        }
                    }
                    cx.notify();
                });
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|root: &mut Root, _, _, cx: &mut Context<Root>| {
                root.store.update(cx, |s, cx| {
                    if s.seeking {
                        s.seeking = false;
                        s.engine.seek(s.seek_target);
                    }
                    s.volume_open = false;
                    cx.notify();
                });
            }))
            // 左：歌曲信息
            .child(
                div().w(px(260.)).flex().items_center().gap_3()
                    .child(
                        div()
                            .id("pb-cover")
                            .size(px(52.))
                            .rounded(px(COVER_RADIUS))
                            .overflow_hidden()
                            .flex_shrink_0()
                            .cursor_pointer()
                            .hover(move |el| el.opacity(0.85))
                            // 点击封面进入歌词页（对齐原 EchoMusic）
                            .on_click({
                                let st = store.clone();
                                move |_, _, cx| {
                                    navigate(&st, Page::Lyric, cx);
                                }
                            })
                            .bg(theme.bg_hover)
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(current.as_ref().map(|c| !c.cover_url.is_empty()).unwrap_or(false), |el| {
                                let url = current.as_ref().unwrap().cover_url.clone();
                                el.child(img(SharedUri::from(url)).size_full().rounded(px(COVER_RADIUS)).object_fit(ObjectFit::Cover))
                            })
                            .when(current.as_ref().map(|c| c.cover_url.is_empty()).unwrap_or(true), |el| {
                                el.child(icon("icons/music-note.svg", 20., theme.text_tertiary))
                            }),
                    )
                    .child(
                        div().flex().flex_col().gap_0p5().min_w_0().flex_1()
                            .child(
                                div().text_size(px(13.)).truncate()
                                    .text_color(if current.is_some() { theme.text } else { theme.text_tertiary })
                                    .child(current.as_ref().map(|c| c.title.clone()).unwrap_or_else(|| "未在播放".into())),
                            )
                            .child({
                                // 歌手名可点击：快捷搜索该歌手（多歌手取第一个）
                                let artist_full = current
                                    .as_ref()
                                    .map(|c| c.artist.clone())
                                    .unwrap_or_default();
                                let kw = artist_full
                                    .split('/')
                                    .next()
                                    .unwrap_or(&artist_full)
                                    .trim()
                                    .to_string();
                                let nav_store = self.store.clone();
                                let si = search_input.clone();
                                let has = !kw.is_empty();
                                let hover_color = theme.accent;
                                let kw_click = kw.clone();
                                let nav_click = nav_store.clone();
                                let si_click = si.clone();
                                div()
                                    .id("pb-artist")
                                    .h(px(16.))
                                    .flex()
                                    .items_center()
                                    .text_size(px(11.))
                                    .text_color(theme.text_secondary)
                                    .truncate()
                                    .when(has, |el| el.cursor_pointer())
                                    .hover(move |st| st.text_color(hover_color))
                                    // 用 mouse_down 而非 click：播放条容器上的 up/move 监听器会
                                    // 干扰 click 判定，mouse_down 命中即触发更可靠。
                                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                        if kw_click.is_empty() {
                                            return;
                                        }
                                        log::info!("[pb-artist] quick search {}", kw_click);
                                        si_click.update(cx, |ti, cx| ti.set_text(&kw_click, cx));
                                        navigate(
                                            &nav_click,
                                            Page::Search(kw_click.clone()),
                                            cx,
                                        );
                                    })
                                    .child(artist_full)
                            }),
                    ),
            )
            // 中：控制 + 进度
            .child(
                div().flex().flex_col().items_center().gap_1().flex_1().min_w_0()
                    .child(
                        div().flex().items_center().gap_3()
                            .child(
                                IconButton {
                                    path: mode_icon,
                                    size: 15.,
                                    color: theme.text_secondary,
                                    hover_bg: theme.bg_hover,
                                    action: Box::new(|root: &mut Root, _, _, cx: &mut Context<Root>| {
                                        cycle_play_mode(&root.store.clone(), cx);
                                    }),
                                }
                                .render("pb-mode", cx),
                            )
                            .child(
                                IconButton {
                                    path: "icons/prev.svg",
                                    size: 16.,
                                    color: theme.text,
                                    hover_bg: theme.bg_hover,
                                    action: Box::new(|root: &mut Root, _, _, cx: &mut Context<Root>| {
                                        prev_track(&root.store.clone(), cx);
                                    }),
                                }
                                .render("pb-prev", cx),
                            )
                            .child(
                                // 主播放按钮
                                div()
                                    .id("pb-play")
                                    .size(px(40.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_color(gpui::white())
                                    .bg(accent)
                                    .hover(|s| s.opacity(0.88))
                                    .on_click({
                                        let store = self.store.clone();
                                        move |_, _, cx| toggle_play(&store, cx)
                                    })
                                    // 切歌/缓冲时显示旋转加载圈（不是下载图标）
                                    .child(if loading {
                                        loading_spinner(17., gpui::white()).into_any_element()
                                    } else {
                                        icon(
                                            if playing { "icons/pause.svg" } else { "icons/play.svg" },
                                            17.,
                                            gpui::white(),
                                        )
                                        .into_any_element()
                                    }),
                            )
                            .child(
                                IconButton {
                                    path: "icons/next.svg",
                                    size: 16.,
                                    color: theme.text,
                                    hover_bg: theme.bg_hover,
                                    action: Box::new(|root: &mut Root, _, _, cx: &mut Context<Root>| {
                                        next_track(&root.store.clone(), false, cx);
                                    }),
                                }
                                .render("pb-next", cx),
                            )
                            // 音量：图标（点击静音）+ 可拖动滑条
                            .child({
                                let vol = volume;
                                div()
                                    .id("pb-volume")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        // 只有图标响应点击（静音切换），滑条独立处理
                                        div()
                                            .id("pb-volume-icon")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(20.))
                                            .rounded(px(6.))
                                            .cursor_pointer()
                                            .hover(move |el| el.bg(theme.bg_hover))
                                            .on_mouse_down(MouseButton::Left, {
                                                let st = store.clone();
                                                move |_, _, cx| {
                                                    let cur = st.read(cx).player.volume;
                                                    set_volume(
                                                        &st.clone(),
                                                        if cur > 0.0 { 0.0 } else { 0.8 },
                                                        cx,
                                                    );
                                                }
                                            })
                                            .child(icon(
                                                if vol <= 0.001 {
                                                    "icons/volume-mute.svg"
                                                } else {
                                                    "icons/volume.svg"
                                                },
                                                15.,
                                                theme.text_secondary,
                                            )),
                                    )
                                    .child({
                                        // 用 canvas 记录滑条 bounds，避免用绝对屏幕坐标换算
                                        let vslot = volume_bounds_slot.clone();
                                        let vratio = vol as f32;
                                        let vtrack = theme.progress_track;
                                        div()
                                            .id("pb-volume-bar")
                                            .w(px(64.))
                                            .h(px(12.))
                                            .flex()
                                            .items_center()
                                            .cursor_pointer()
                                            .child(
                                                canvas(
                                                    {
                                                        let slot = vslot.clone();
                                                        move |bounds: Bounds<Pixels>, _, _| {
                                                            *slot.lock() = Some(bounds);
                                                        }
                                                    },
                                                    move |bounds, _, window, _| {
                                                        window.paint_quad(fill(bounds, vtrack));
                                                        let w = bounds.size.width * vratio;
                                                        if w > px(0.) {
                                                            window.paint_quad(fill(
                                                                Bounds::new(
                                                                    bounds.origin,
                                                                    size(w, bounds.size.height),
                                                                ),
                                                                accent,
                                                            ));
                                                        }
                                                    },
                                                )
                                                .h(px(4.))
                                                .w_full()
                                                .rounded_full(),
                                            )
                                            .on_mouse_down(MouseButton::Left, cx.listener(
                                                |root: &mut Root, ev: &MouseDownEvent, _, cx: &mut Context<Root>| {
                                                    root.store.update(cx, |s, cx| {
                                                        let b = s.volume_bounds_slot.lock().clone();
                                                        if let Some(b) = b {
                                                            s.volume_open = true;
                                                            let ratio = f32::from(
                                                                ev.position.x - b.left(),
                                                            )
                                                            .max(0.0)
                                                                / f32::from(b.size.width).max(1.0);
                                                            s.player.volume = (ratio as f64).clamp(0.0, 1.0);
                                                            s.engine.set_volume(s.player.volume);
                                                            s.settings.update(|st| st.volume = s.player.volume);
                                                        }
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                            .on_mouse_move(cx.listener(
                                                |root: &mut Root, ev: &MouseMoveEvent, _, cx: &mut Context<Root>| {
                                                    if ev.pressed_button != Some(MouseButton::Left) {
                                                        return;
                                                    }
                                                    root.store.update(cx, |s, cx| {
                                                        if !s.volume_open {
                                                            return;
                                                        }
                                                        let b = s.volume_bounds_slot.lock().clone();
                                                        if let Some(b) = b {
                                                            let ratio = f32::from(
                                                                ev.position.x - b.left(),
                                                            )
                                                            .max(0.0)
                                                                / f32::from(b.size.width).max(1.0);
                                                            s.player.volume = (ratio as f64).clamp(0.0, 1.0);
                                                            s.engine.set_volume(s.player.volume);
                                                            s.settings.update(|st| st.volume = s.player.volume);
                                                        }
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                            .on_mouse_up(MouseButton::Left, cx.listener(
                                                |root: &mut Root, _, _, cx: &mut Context<Root>| {
                                                    root.store.update(cx, |s, cx| {
                                                        s.volume_open = false;
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                    })
                            }),
                    )
                    .child(
                        div().w_full().flex().items_center().gap_2()
                            .child(div().text_size(px(10.)).text_color(theme.text_secondary).child(fmt_time(position)))
                            .child(
                                div()
                                    .id("pb-progress")
                                    .flex_1()
                                    .h(px(12.))
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .child(
                                        canvas(
                                            {
                                                let slot = progress_bounds_slot.clone();
                                                move |bounds: Bounds<Pixels>, _, _| {
                                                    *slot.lock() = Some(bounds);
                                                }
                                            },
                                            move |bounds, _, window, _| {
                                                let (ratio, accent, track) = progress_paint;
                                                let ratio = ratio as f32;
                                                window.paint_quad(fill(bounds, track));
                                                let w = bounds.size.width * ratio;
                                                if w > px(0.) {
                                                    window.paint_quad(fill(
                                                        Bounds::new(bounds.origin, size(w, bounds.size.height)),
                                                        accent,
                                                    ));
                                                }
                                            },
                                        )
                                        .h(px(4.))
                                        .w_full()
                                        .rounded_full(),
                                    )
                                    .on_mouse_down(MouseButton::Left, cx.listener(|root: &mut Root, ev: &MouseDownEvent, _, cx: &mut Context<Root>| {
                                        // 只记录目标位置，seek 统一在松手时执行（避免杂音）
                                        root.store.update(cx, |s, cx| {
                                            if let Some(b) = s.progress_bounds_slot.lock().clone() {
                                                let ratio = f32::from(ev.position.x - b.left()).max(0.0)
                                                    / f32::from(b.size.width).max(1.0);
                                                let target = (ratio as f64).clamp(0.0, 1.0) * s.player.duration;
                                                s.seeking = true;
                                                s.seek_target = target;
                                                s.player.position = target;
                                                cx.notify();
                                            }
                                        });
                                    })),
                            )
                            .child(div().text_size(px(10.)).text_color(theme.text_secondary).child(fmt_time(duration))),
                    ),
            )
            // 右：功能占位
            .child(
                div().w(px(200.)).flex().items_center().justify_end().gap_1()
                    .child({
                        let cur = current.clone();
                        let logged_in = s.session.get().userid != 0;
                        let fav = cur
                            .as_ref()
                            .map(|c| {
                                if logged_in {
                                    crate::liked::is_remote_liked(&s, c)
                                } else {
                                    s.library.is_favorite(c)
                                }
                            })
                            .unwrap_or(false);
                        let st = store.clone();
                        div()
                            .id("pb-fav")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(27.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(move |el| el.bg(theme.bg_hover))
                            .on_click(move |_, _, cx| {
                                if let Some(song) = cur.clone() {
                                    crate::liked::toggle_like(&st, &song, cx);
                                }
                            })
                            .child(icon(
                                if fav { "icons/heart-filled.svg" } else { "icons/heart.svg" },
                                15.,
                                if fav { theme.accent } else { theme.text_secondary },
                            ))
                    })
                    .child({
                        // 播放队列：点击展开/收起面板
                        let st = store.clone();
                        let open = self.store.read(cx).queue_open;
                        div()
                            .id("pb-queue")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(27.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(move |el| el.bg(theme.bg_hover))
                            .on_click(move |_, window, cx| {
                                let target = st.update(cx, |s, cx| {
                                    s.queue_open = !s.queue_open;
                                    let idx = if s.queue_open { s.player.index } else { None };
                                    cx.notify();
                                    idx
                                });
                                // 面板首帧绘制后再定位：此时列表已完成测量，
                                // scroll_to_reveal_item 才能依据真实 item 高度算准位置。
                                if target.is_some() {
                                    let st2 = st.clone();
                                    window.on_next_frame(move |_, app| {
                                        let _ = st2.update(app, |s, _| {
                                            if let Some(i) = s.player.index {
                                                s.queue_scroll.scroll_to_item(
                                                    i,
                                                    gpui::ScrollStrategy::Center,
                                                );
                                            }
                                        });
                                        let _ = st2.update(app, |_, cx| cx.notify());
                                    });
                                }
                            })
                            .child(icon(
                                "icons/queue.svg",
                                15.,
                                if open { theme.accent } else { theme.text_secondary },
                            ))
                    })
                    // 倍速：点一下在 0.75/1.0/1.25/1.5/2.0 之间循环
                    .child({
                        let st = self.store.clone();
                        let speed = self.store.read(cx).settings.get().speed;
                        div()
                            .id("pb-speed")
                            .px(px(6.))
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .bg(if (speed - 1.0).abs() > 0.001 {
                                theme.accent
                            } else {
                                theme.bg_hover
                            })
                            .text_color(if (speed - 1.0).abs() > 0.001 {
                                gpui::white()
                            } else {
                                theme.text_secondary
                            })
                            .text_size(px(11.))
                            .cursor_pointer()
                            .hover(move |el| el.opacity(0.88))
                            .on_click(move |_, _, cx| {
                                const STEPS: [f64; 5] = [0.75, 1.0, 1.25, 1.5, 2.0];
                                let cur = st.read(cx).settings.get().speed;
                                let next = STEPS
                                    .iter()
                                    .find(|s| **s > cur + 0.001)
                                    .copied()
                                    .unwrap_or(STEPS[0]);
                                st.update(cx, |s, cx| {
                                    s.settings.update(|st| st.speed = next);
                                    s.engine.set_speed(next);
                                    s.toast = format!("倍速 {}x", next);
                                    cx.notify();
                                });
                            })
                            .child(format!("{}x", format_speed(speed)))
                    })
                    .child(
                        IconButton {
                            path: "icons/more.svg",
                            size: 15.,
                            color: theme.text_secondary,
                            hover_bg: theme.bg_hover,
                            action: Box::new(|root: &mut Root, _, window: &mut Window, cx: &mut Context<Root>| {
                                // 更多菜单：睡眠定时器 / 桌面歌词 / MV / mini 播放器
                                root.open_player_menu(window, cx);
                            }),
                        }
                        .render("pb-more", cx),
                    ),
            )
    }

    /// 打开/关闭「更多」菜单（睡眠定时器 / 桌面歌词 / MV / mini 播放器）。
    fn open_player_menu(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| {
            s.player_menu = !s.player_menu;
            s.queue_open = false;
            cx.notify();
        });
    }

    /// 播放条「更多」菜单浮层。
    fn render_player_menu(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.store.read(cx).theme();
        let open = self.store.read(cx).player_menu;
        if !open {
            return div().into_any_element();
        }
        let store = self.store.clone();
        let sleep_until = self.store.read(cx).sleep_until;
        let desktop = self.store.read(cx).settings.get().desktop_lyric;
        let speed = self.store.read(cx).settings.get().speed;

        // 菜单项：图标 + 标题 + 右侧状态
        let item = |icon_name: &'static str,
                    title: &'static str,
                    tail: String,
                    active: bool,
                    on_click: Box<dyn Fn(&mut App) + 'static>|
         -> AnyElement {
            div()
                .id(SharedString::from(format!("pm-{title}")))
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py(px(7.))
                .rounded(px(6.))
                .text_size(px(12.))
                .cursor_pointer()
                .hover(move |el| el.bg(theme.bg_hover))
                .on_click(move |_, _, cx| on_click(cx))
                .child(icon(
                    icon_name,
                    14.,
                    if active {
                        theme.accent
                    } else {
                        theme.text_secondary
                    },
                ))
                .child(div().flex_1().child(title))
                .when(!tail.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(px(11.))
                            .text_color(if active {
                                theme.accent
                            } else {
                                theme.text_tertiary
                            })
                            .child(tail.clone()),
                    )
                })
                .into_any_element()
        };

        let sleep_tail = match sleep_until {
            Some(until) => {
                let left = (until - chrono::Local::now().timestamp()).max(0);
                format!("{} 分后停止", (left + 59) / 60)
            }
            None => "未设置".to_string(),
        };

        div()
            .id("player-menu")
            .absolute()
            .right(px(16.))
            .bottom(px(84.))
            .w(px(200.))
            .p_1()
            .rounded(px(10.))
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .child(item(
                "icons/timer.svg",
                "睡眠定时器",
                sleep_tail,
                sleep_until.is_some(),
                Box::new({
                    let st = store.clone();
                    move |cx: &mut App| {
                        const OPTIONS: [i64; 4] = [15, 30, 60, 0];
                        let cur = st.read(cx).sleep_until;
                        let next = match cur {
                            None => Some(15),
                            Some(_) => {
                                // 依次切到 30 / 60 / 关闭
                                let idx = OPTIONS
                                    .iter()
                                    .position(|_| false)
                                    .unwrap_or(0);
                                let _ = idx;
                                Some(30)
                            }
                        };
                        set_sleep_timer(&st, next, cx);
                    }
                }),
            ))
            .child(item(
                "icons/list-music.svg",
                "桌面歌词",
                if desktop { "已开启".into() } else { "关".into() },
                desktop,
                Box::new({
                    let st = store.clone();
                    move |cx: &mut App| {
                        let on = !st.read(cx).settings.get().desktop_lyric;
                        st.update(cx, |s, cx| {
                            s.settings.update(|st| st.desktop_lyric = on);
                            s.player_menu = false;
                            s.toast = if on {
                                "桌面歌词已开启".into()
                            } else {
                                "桌面歌词已关闭".into()
                            };
                            cx.notify();
                        });
                        crate::desktop::toggle_desktop_lyric(&st, cx);
                    }
                }),
            ))
            .child(item(
                "icons/play.svg",
                "查看 MV",
                String::new(),
                false,
                Box::new({
                    let st = store.clone();
                    move |cx: &mut App| {
                        st.update(cx, |s, cx| {
                            s.player_menu = false;
                            cx.notify();
                        });
                        crate::views::load_current_mv(&st, cx);
                    }
                }),
            ))
            .child(item(
                "icons/headphones.svg",
                "倍速",
                format!("{}x", format_speed(speed)),
                (speed - 1.0).abs() > 0.001,
                Box::new({
                    let st = store.clone();
                    move |cx: &mut App| {
                        const STEPS: [f64; 5] = [0.75, 1.0, 1.25, 1.5, 2.0];
                        let cur = st.read(cx).settings.get().speed;
                        let next = STEPS
                            .iter()
                            .find(|s| **s > cur + 0.001)
                            .copied()
                            .unwrap_or(STEPS[0]);
                        st.update(cx, |s, cx| {
                            s.settings.update(|st| st.speed = next);
                            s.engine.set_speed(next);
                            s.player_menu = false;
                            s.toast = format!("倍速 {}x", next);
                            cx.notify();
                        });
                    }
                }),
            ))
            .child(item(
                "icons/shirt.svg",
                "mini 播放器",
                String::new(),
                false,
                Box::new({
                    let st = store.clone();
                    move |cx: &mut App| {
                        st.update(cx, |s, cx| {
                            s.player_menu = false;
                            s.toast = "mini 播放器：请用托盘菜单或设置页开启".into();
                            cx.notify();
                        });
                        crate::desktop::open_mini_player(&st, cx);
                    }
                }),
            ))
            .into_any_element()
    }

    // ---------- MV ----------
    /// 查到 MV 后弹一个小卡片：封面 + 信息 + 「用浏览器播放」。
    fn render_mv_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.store.read(cx).theme();
        let (mv, loading) = {
            let s = self.store.read(cx);
            (s.mv.clone(), s.mv_loading)
        };
        let store = self.store.clone();

        let Some(mv) = mv else {
            return div()
                .when(loading, |el| {
                    el.child(
                        div()
                            .absolute()
                            .bottom(px(150.))
                            .left(px(0.))
                            .right(px(0.))
                            .flex()
                            .justify_center()
                            .child(
                                div()
                                    .px_4()
                                    .py_2()
                                    .rounded(px(10.))
                                    .bg(hsla(0., 0., 0., 0.7))
                                    .text_color(gpui::white())
                                    .text_size(px(12.))
                                    .child("正在查询 MV…"),
                            ),
                    )
                })
                .child(div())
                .into_any_element();
        };

        let url = format!(
            "https://www.kugou.com/mvweb/html/mv_{}.html",
            mv.mvhash.to_lowercase()
        );
        div()
            .absolute()
            .bottom(px(96.))
            .right(px(16.))
            .w(px(260.))
            .p_3()
            .rounded(px(12.))
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mb_2()
                    .child(div().text_size(px(12.5)).child("相关 MV"))
                    .child(
                        div()
                            .id("mv-close")
                            .px_2()
                            .py(px(1.))
                            .rounded(px(4.))
                            .text_size(px(11.))
                            .text_color(theme.text_tertiary)
                            .cursor_pointer()
                            .hover(move |el| el.bg(theme.bg_hover))
                            .on_click({
                                let st = store.clone();
                                move |_, _, cx| {
                                    st.update(cx, |s, cx| {
                                        s.mv = None;
                                        cx.notify();
                                    });
                                }
                            })
                            .child("关闭"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .w(px(84.))
                            .h(px(52.))
                            .flex_shrink_0()
                            .rounded(px(6.))
                            .overflow_hidden()
                            .bg(theme.bg_hover)
                            .when(!mv.thumb.is_empty(), |el| {
                                el.child(
                                    img(SharedUri::from(mv.thumb.clone()))
                                        .size_full()
                                        .rounded(px(6.))
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
                                    .text_size(px(12.))
                                    .truncate()
                                    .child(mv.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.text_tertiary)
                                    .truncate()
                                    .child(mv.singer.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(theme.text_tertiary)
                                    .child(format!("播放 {} 次", mv.hit)),
                            ),
                    ),
            )
            .child(
                div()
                    .id("mv-open")
                    .mt_3()
                    .py(px(6.))
                    .rounded(px(8.))
                    .bg(theme.accent)
                    .text_color(gpui::white())
                    .text_size(px(12.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|el| el.opacity(0.88))
                    .on_click(move |_, _, cx| {
                        // 视频播放不在本 app 的能力范围内：交给浏览器
                        #[cfg(windows)]
                        {
                            let _ = std::process::Command::new("cmd")
                                .args(["/c", "start", "", &url])
                                .spawn();
                        }
                        #[cfg(not(windows))]
                        {
                            let _ = std::process::Command::new("open").arg(&url).spawn();
                        }
                        store.update(cx, |s, cx| {
                            s.toast = "已在浏览器中打开 MV".into();
                            cx.notify();
                        });
                    })
                    .child("用浏览器播放"),
            )
            .into_any_element()
    }

    // ---------- Toast ----------
    fn render_toast(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let toast = self.store.read(cx).toast.clone();
        div()
            .absolute()
            .bottom(px(100.))
            .left(px(0.))
            .right(px(0.))
            .flex()
            .justify_center()
            .when(toast.is_empty(), |el| el.child(div()))
            .when(!toast.is_empty(), |el| {
                el.child(
                    div()
                        .px_4()
                        .py_2()
                        .rounded(px(10.))
                        .bg(hsla(0., 0., 0., 0.75))
                        .text_color(gpui::white())
                        .text_size(px(12.))
                        .child(toast),
                )
            })
    }
}

/// quick_card 里的色块字符。
fn icon_path_short(path: &str) -> &'static str {
    match path {
        "icons/calendar.svg" => "日",
        "icons/star.svg" => "TOP",
        _ => "♪",
    }
}

/// 占位页（第二期功能入口）。
fn render_placeholder(title: &str, theme: &crate::theme::Theme) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .child(icon("icons/music-note.svg", 40., theme.text_tertiary))
        .child(div().text_size(px(16.)).child(title.to_string()))
        .child(
            div()
                .text_size(px(12.))
                .text_color(theme.text_tertiary)
                .child("该功能将在后续版本实现"),
        )
}


// ==================== 歌曲行（虚拟化列表用） ====================

/// 单行歌曲。做成自由函数是为了能在 `uniform_list` 的闭包里调用（闭包不能借用 `self`）。
pub(crate) fn song_row(
    store: &Entity<AppStore>,
    song: &Song,
    index: usize,
    source: &PlaySource,
    cx: &App,
) -> AnyElement {
    let theme = store.read(cx).theme();
    let playing = store.read(cx).player.is_current(song);
    let st = store.clone();
    let source = source.clone();
    let cover = song.cover_url.clone();
    let title = song.title.clone();
    let artist = song.artist.clone();
    let album = song.album.clone();
    let duration = song.duration;
    let row_key = format!("row-{}-{index}", song.key());

    // 收藏状态（登录用户以云歌单为准）
    let logged_in = store.read(cx).session.get().userid != 0;
    let fav = if logged_in {
        crate::liked::is_remote_liked(store.read(cx), song)
    } else {
        store.read(cx).library.is_favorite(song)
    };
    let fav_store = store.clone();
    let song_owned = song.clone();
    let fav_key = format!("fav-{}-{index}", song.key());
    let fav_icon = if fav {
        "icons/heart-filled.svg"
    } else {
        "icons/heart.svg"
    };
    let fav_color = if fav { theme.accent } else { theme.text_tertiary };
    let accent = theme.accent;
    let text = theme.text;
    let text_secondary = theme.text_secondary;
    let text_tertiary = theme.text_tertiary;
    let bg_hover = theme.bg_hover;
    let tag = song_tag(song, &theme);

    div()
        .id(SharedString::from(row_key))
        // 虚拟列表的 item 不会被父级 stretch，必须显式撑满，否则右侧留白
        .w_full()
        .h(px(52.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .rounded(px(8.))
        .cursor_pointer()
        .when(playing, |el| el.bg(accent.opacity(0.08)))
        .when(!playing, |el| el.hover(move |s| s.bg(bg_hover)))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            play_from(&st, source.clone(), index, cx)
        })
        .child(
            div()
                .w(px(20.))
                .text_size(px(12.))
                .text_color(text_tertiary)
                .child((index + 1).to_string()),
        )
        .child(
            div()
                .size(px(40.))
                .rounded(px(COVER_RADIUS))
                .overflow_hidden()
                .flex_shrink_0()
                .bg(bg_hover)
                .when(!cover.is_empty(), |el| {
                    el.child(
                        img(SharedUri::from(cover.clone()))
                            .size_full()
                            .rounded(px(COVER_RADIUS))
                            .object_fit(ObjectFit::Cover),
                    )
                })
                .when(cover.is_empty(), |el| {
                    el.child(icon("icons/music-note.svg", 16., text_tertiary))
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(13.))
                                .truncate()
                                .text_color(if playing { accent } else { text })
                                .child(title),
                        )
                        .when_some(tag, |el, tag| el.child(tag)),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(text_secondary)
                        .truncate()
                        .child(artist),
                ),
        )
        .child(
            // 专辑列与标题列均分剩余宽度（固定 160px 会让右侧留白、观感不匀）
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.))
                .text_color(text_secondary)
                .text_align(TextAlign::Center)
                .truncate()
                .child(album),
        )
        .child(
            div()
                .w(px(48.))
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(text_tertiary)
                .text_align(TextAlign::Right)
                .child(fmt_time(duration as f64)),
        )
        .child(
            div()
                .id(SharedString::from(fav_key))
                .size(px(26.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |s| s.bg(bg_hover))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    crate::liked::toggle_like(&fav_store, &song_owned, cx);
                    // 阻止冒泡：否则外层的行点击会顺带把这首歌播起来
                    cx.stop_propagation();
                })
                .child(icon(fav_icon, 14., fav_color)),
        )
        .into_any_element()
}

/// 滚动条轨道上下内边距（px）。
pub(crate) const SB_INSET: f32 = 4.0;
/// 滚动条拇指宽度（px）。
const SB_W: f32 = 5.0;
/// 播放队列行高（uniform_list 要求所有行等高）。
const QUEUE_ROW_H: f32 = 42.;

/// 播放队列单行：序号 | 歌名 | 歌手 | 时长（三列，歌手在中间）。
///
/// 数据在渲染时从 store 按需读取，避免每帧把整个队列深拷贝一遍。
pub(crate) fn queue_row(store: &Entity<AppStore>, ix: usize, cx: &App) -> AnyElement {
    let theme = store.read(cx).theme();
    let (song, active) = {
        let s = store.read(cx);
        match s.player.queue.get(ix) {
            Some(song) => (song.clone(), s.player.index == Some(ix)),
            // 队列刚被清空、列表还没重排完时兜底
            None => return div().h(px(QUEUE_ROW_H)).into_any_element(),
        }
    };
    let st = store.clone();
    let accent = theme.accent;
    let text = theme.text;
    let text_secondary = theme.text_secondary;
    let text_tertiary = theme.text_tertiary;
    let bg_hover = theme.bg_hover;

    div()
        .id(SharedString::from(format!("q-{ix}")))
        .w_full()
        .h(px(QUEUE_ROW_H))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .rounded(px(6.))
        .cursor_pointer()
        .when(active, |el| el.bg(accent.opacity(0.10)))
        .when(!active, |el| el.hover(move |s| s.bg(bg_hover)))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            play_index(&st, ix, cx);
        })
        .child(
            div()
                .w(px(18.))
                .flex_shrink_0()
                .text_align(TextAlign::Center)
                .text_size(px(11.))
                .text_color(if active { accent } else { text_tertiary })
                .child((ix + 1).to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.5))
                .text_color(if active { accent } else { text })
                .child(song.title),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_align(TextAlign::Center)
                .text_size(px(11.))
                .text_color(text_secondary)
                .child(song.artist),
        )
        .child(
            div()
                .w(px(38.))
                .flex_shrink_0()
                .text_align(TextAlign::Right)
                .text_size(px(10.5))
                .text_color(text_tertiary)
                .child(fmt_time(song.duration as f64)),
        )
        .into_any_element()
}

/// 取出 uniform_list 内部真正的滚动状态句柄。
///
/// `UniformListScrollHandle` 本身只暴露 `scroll_to_item`，
/// 偏移量/边界/可滚动距离都挂在它内部的 `base_handle`（`ScrollHandle`）上。
pub(crate) fn list_scroll_handle(h: &gpui::UniformListScrollHandle) -> gpui::ScrollHandle {
    h.0.borrow().base_handle.clone()
}

/// 拇指高度：视口占内容的比例 × 轨道高，最小 28px。
///
/// `max` 是最大滚动偏移（= 内容高 - 视口高），所以内容高 = vp_h + max。
pub(crate) fn thumb_height(vp_h: f32, max: f32) -> f32 {
    let content = vp_h + max;
    if content <= 0.0 {
        return vp_h;
    }
    let track = (vp_h - SB_INSET * 2.0).max(1.0);
    (track * (vp_h / content)).clamp(28.0, track)
}


/// 应用一次滚动条拖拽位移（返回是否真的滚动过）。
///
/// `start` = (按下时的鼠标 y, 按下时的滚动偏移)；按拇指的可视行程做线性映射，
/// 这样鼠标和拇指是 1:1 跟手，而不是「按歌数跳」。
pub(crate) fn apply_scrollbar_drag(
    handle: &gpui::UniformListScrollHandle,
    start: (f32, f32),
    mouse_y: f32,
) -> bool {
    let h = list_scroll_handle(handle);
    let max = f32::from(h.max_offset().height).max(0.0);
    if max <= 0.0 {
        return false;
    }
    let vp_h = f32::from(h.bounds().size.height).max(1.0);
    let thumb_h = thumb_height(vp_h, max);
    let span = (vp_h - SB_INSET * 2.0 - thumb_h).max(1.0);
    let dy = mouse_y - start.0;
    let off = (start.1 + dy * (max / span)).clamp(0.0, max);
    h.set_offset(point(px(0.), px(-off)));
    true
}

/// `uniform_list` 的自绘滚动条浮层（通用版）。
///
/// uniform_list 的内容高度是精确的（行数 × 行高），所以 `max_offset`
/// 就是真实可滚动距离，拇指位置可以直接 1:1 映射到鼠标位移。
/// `on_down` 负责把拖拽起点写回状态（各列表的字段不同），
/// 后续拖动由列表容器的 `on_mouse_move` + `apply_scrollbar_drag` 完成。
pub(crate) fn uniform_scrollbar_overlay<F>(
    id: &'static str,
    handle: gpui::UniformListScrollHandle,
    on_down: F,
) -> impl IntoElement
where
    F: Fn(&mut App, (f32, f32)) + 'static,
{
    let paint_handle = handle.clone();

    let bar = canvas(
        move |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let h = list_scroll_handle(&paint_handle);
            let max = f32::from(h.max_offset().height).max(0.0);
            if max <= 0.0 {
                return;
            }
            let vp_h = f32::from(bounds.size.height).max(1.0);
            let thumb_h = thumb_height(vp_h, max);
            let span = (vp_h - SB_INSET * 2.0 - thumb_h).max(1.0);
            let off = (-f32::from(h.offset().y)).clamp(0.0, max);
            let thumb_y = bounds.origin.y + px(SB_INSET) + px(span * (off / max));
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(bounds.origin.x + bounds.size.width - px(SB_W), thumb_y),
                        gpui::size(px(SB_W), px(thumb_h)),
                    ),
                    hsla(0., 0., 0., 0.22),
                )
                .corner_radii(px(3.)),
            );
        },
    )
    .size_full();

    div()
        .id(id)
        .size_full()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |ev, window, cx: &mut App| {
            // 别让点击穿透到下面的行（否则点滚动条会顺带播放歌曲）
            cx.stop_propagation();
            let h = list_scroll_handle(&handle);
            let max = f32::from(h.max_offset().height).max(0.0);
            if max <= 0.0 {
                return;
            }
            let list_bounds = h.bounds();
            let vp_h = f32::from(list_bounds.size.height).max(1.0);
            let thumb_h = thumb_height(vp_h, max);
            let span = (vp_h - SB_INSET * 2.0 - thumb_h).max(1.0);
            // 让拇指中心落在鼠标位置
            let y = f32::from(ev.position.y - list_bounds.top()) - SB_INSET - thumb_h / 2.0;
            let off = (y / span * max).clamp(0.0, max);
            h.set_offset(point(px(0.), px(-off)));
            // 记录起点，交给容器上的 on_mouse_move 继续跟手拖动
            on_down(cx, (f32::from(ev.position.y), off));
            window.refresh();
        })
        .child(bar)
}

/// 按字符数截断并补省略号。
///
/// gpui 的 `truncate()` 在临界宽度下可能连「…」一起裁掉（看起来像硬切），
/// 队列标题这种要紧挨按钮的位置先按字符数截一刀，视觉上更稳。
pub(crate) fn ellipsize(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out = String::with_capacity(max_chars * 3 + 3);
    for ch in s.chars().take(max_chars) {
        out.push(ch);
    }
    out.push('…');
    out
}

/// 播放量格式化（MV 搜索结果）：105060666 → 1.1亿、9580000 → 958万、12345 → 1.2万。
fn format_heat(n: u64) -> String {
    if n >= 100_000_000 {
        format!("{:.1}亿", n as f64 / 100_000_000.0)
    } else if n >= 10_000 {
        format!("{:.0}万", n as f64 / 10_000.0)
    } else {
        n.to_string()
    }
}

/// 模糊匹配：先做子串匹配，不中再按「字符顺序出现」判定（支持跳字，如 `jx` 命中 `江南`…）。
///
/// `needle` 必须是小写且已 trim（调用方统一处理）。中文按 char 比较，
/// 所以「晴 天」「晴天」这种带空格的输入也能命中。
pub(crate) fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let hay = haystack.to_lowercase();
    if hay.contains(needle) {
        return true;
    }
    // 子序列匹配：needle 的每个字符需按顺序出现在 hay 中
    let mut chars = hay.chars();
    needle
        .chars()
        .all(|n| chars.any(|c| c == n))
}


/// 倍速显示文本（去掉尾随 0：`1.0` → `1`）。
fn format_speed(s: f64) -> String {
    let t = format!("{s:.2}");
    let t = t.trim_end_matches('0').trim_end_matches('.').to_string();
    if t.is_empty() {
        "1".to_string()
    } else {
        t
    }
}


/// 设置睡眠定时器（分钟；`None`/0 = 取消）。
pub fn set_sleep_timer(store: &Entity<AppStore>, minutes: Option<i64>, cx: &mut App) {
    let until = minutes
        .filter(|m| *m > 0)
        .map(|m| chrono::Local::now().timestamp() + m * 60);
    store.update(cx, |s, cx| {
        s.sleep_until = until;
        s.player_menu = false;
        s.toast = match minutes.unwrap_or(0) {
            0 => "睡眠定时器已取消".into(),
            m => format!("{m} 分钟后停止播放"),
        };
        cx.notify();
    });
    let Some(until) = until else {
        return;
    };
    // 到点淡出并暂停
    let store_c = store.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            let left = until - chrono::Local::now().timestamp();
            if left <= 0 {
                break;
            }
            smol::Timer::after(std::time::Duration::from_secs(left.min(5) as u64)).await;
        }
        let _ = cx.update(|app| {
            let _ = store_c.update(app, |s, cx| {
                s.sleep_until = None;
                s.engine.fade_out(0.6);
                s.player.playing = false;
                s.toast = "睡眠定时器到点，已停止播放".into();
                cx.notify();
            });
            let store2 = store_c.clone();
            app.spawn(async move |cx: &mut AsyncApp| {
                smol::Timer::after(std::time::Duration::from_millis(650)).await;
                let _ = cx.update(|app| {
                    let _ = store2.update(app, |s, _| {
                        s.engine.pause();
                        s.engine.fade_reset();
                    });
                });
            })
            .detach();
        });
    })
    .detach();
}
