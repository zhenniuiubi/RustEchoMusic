//! 主界面：布局（侧栏 / 顶栏 / 内容区 / 播放条）与交互动作。
//!
//! 布局对齐原项目 `layouts/MainLayout.vue + Sidebar.vue + TitleBar.vue`、
//! 首页对齐 `views/Home.vue`、播放条对齐 `components/PlayerBar.vue`。

use crate::state::{AppStore, Page};
use crate::theme::{CARD_RADIUS, CONTROL_RADIUS, COVER_RADIUS};
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

// ==================== 播放动作 ====================

/// 解析指定歌曲的播放地址并加载引擎（在 update 之外调用）。
pub fn spawn_resolve(store: &Entity<AppStore>, request_id: u64, song: Song, cx: &mut App) {
    let store = store.clone();
    // 私人 FM 的「听过没」在这里打点：所有播放入口（play_from / play_index /
    // Ended 自动下一首 / 重启还原后首次播放）最终都会走到解析地址。
    crate::actions::note_fm_heard(&store, &song, cx);
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
                    crate::actions::force_logout(s, "登录已过期，请重新登录");
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
    // 私人 FM：把这一批的 hash 记下来，后面判断「整批都播过」用
    let fm_hashes: Vec<String> = if matches!(source, PlaySource::Fm) {
        songs.iter().map(|x| x.hash.clone()).collect()
    } else {
        Vec::new()
    };
    // 设置队列状态
    let current_song = store.update(cx, |s, cx| {
        let mode = s.player.play_mode;
        let volume = s.player.volume;
        s.player.set_queue(&title, songs, start);
        s.player.fm_queue = matches!(source, PlaySource::Fm);
        if s.player.fm_queue {
            s.fm_batch = fm_hashes;
            s.fm_heard.clear();
        }
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
        crate::actions::load_lyric(&store, cx);
        start_with_fade(&store, rid, song, cx);
    }
}

/// 「播放全部」：随机模式下从**随机一首**开始，而不是永远固定第一首。
///
/// 目前用在「我最喜爱」页；点单曲仍然走 `play_from(.., index)`，从点的那首开始。
pub fn play_all(store: &Entity<AppStore>, source: PlaySource, cx: &mut App) {
    let start = {
        let s = store.read(cx);
        let len = match source {
            PlaySource::Favorites | PlaySource::FavoritesFiltered => s.favorites_view().len(),
            PlaySource::History => s.library.get().history.len(),
            _ => 0,
        };
        if s.player.play_mode == echo_core::PlayMode::Random && len > 0 {
            use rand::Rng as _;
            let start = rand::rng().random_range(0..len);
            log::info!("play_all: 随机模式，从第 {start}/{len} 首开始");
            start
        } else {
            0
        }
    };
    play_from(store, source, start, cx);
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
        crate::actions::load_lyric(&store, cx);
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
    // 私人 FM：这一轮的歌都听过了 → 清空队列换下一轮（不再往下接老队列）
    if crate::actions::fm_round_finished(store.read(cx)) {
        crate::actions::fm_extend(store, cx);
        return;
    }
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
            // 私人 FM 的「一轮听完」不能靠 `next_index()` 判断：列表循环/随机模式下
            // 它会绕回队首，永远碰不到队尾，于是同一批 5 首来回放、再也不去要新歌。
            // 这里只要「这一轮都听过了」或者「已经是队列最后一首」就换一轮
            // （单曲循环除外，那是用户明确要求重播）。
            let at_tail = s
                .player
                .index
                .map(|i| i + 1 >= s.player.queue.len())
                .unwrap_or(false);
            let fm_round_done = s.player.fm_queue
                && s.player.play_mode != echo_core::PlayMode::Single
                && (crate::actions::fm_round_finished(s) || at_tail);
            if fm_round_done {
                s.player.loading = true;
                s.fm_loading = true;
                cx.notify();
                let store = cx.entity();
                cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                    let _ = cx.update(|app| crate::actions::fm_extend(&store, app));
                })
                .detach();
                None
            } else if let Some(i) = s.player.next_index() {
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
        // 系统材质（Mica/Acrylic）的明暗跟着主题走，切完主题要重新下发一次属性，
        // 否则深色 UI 底下还垫着浅色 Mica（或者反过来）
        let _ = cx.update(|app| apply_backdrop(&anim_store, app));
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
    let dark = store.read(cx).settings.get().theme_mode == echo_core::ThemeMode::Dark;
    let handle = cx.try_global::<crate::state::AppGlobal>()
        .and_then(|g| g.main_window);
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_view, window, _cx| {
            crate::windows_backdrop::apply(window, mode, dark);
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


/// 滚动条轨道上下留白（px）：拇指不会贴到列表最上/最下沿。
pub(crate) const SB_INSET: f32 = 4.0;
/// 滚动条拇指宽度（px）。
const SB_W: f32 = 6.0;

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
                    hsla(0., 0., 0., 0.30),
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
