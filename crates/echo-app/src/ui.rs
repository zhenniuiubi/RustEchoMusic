//! 播放动作与通用小件（Lite 版只剩这些）。
//!
//! 播放队列只有两个来源：私人 FM 与我喜爱；歌单页 / 搜索页那套已经移除。

use crate::state::AppStore;
use echo_core::Song;
use gpui::prelude::*;
use gpui::*;
use std::sync::OnceLock;

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

pub(crate) fn icon(path: &str, size: f32, color: Hsla) -> impl IntoElement {
    svg()
        .path(SharedString::from(path.to_string()))
        .size(px(size))
        .text_color(color)
        .flex_shrink_0()
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

/// 播放来源（Lite 只有这两条「整批换队列」的路径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaySource {
    /// 我最喜爱（未登录用本地收藏，登录后用云端红心）
    Favorites,
    /// 私人 FM
    Fm,
}

/// 从来源列表设置队列并从 index 开始播放。
pub fn play_from(store: &Entity<AppStore>, source: PlaySource, index: usize, cx: &mut App) {
    let (title, songs) = store.update(cx, |s, _| match source {
        PlaySource::Favorites => ("我最喜爱".to_string(), s.favorites_source().as_ref().clone()),
        PlaySource::Fm => ("私人FM".to_string(), s.fm.as_ref().clone()),
    });
    if songs.is_empty() {
        return;
    }
    let start = index.min(songs.len() - 1);
    play_list(store, title, songs, start, matches!(source, PlaySource::Fm), cx);
}

/// 播放整份「我最喜爱」：**随机顺序**，从洗好的第一首开始。
///
/// 从 FM 切到收藏时用这个 —— 收藏往往几十上百首，按列表顺序从头放太死板。
/// 返回 false 表示收藏是空的（调用方据此不要把 UI 切到收藏模式）。
pub fn play_favorites_shuffled(store: &Entity<AppStore>, cx: &mut App) -> bool {
    let mut songs = store.read(cx).favorites_source().as_ref().clone();
    if songs.is_empty() {
        store.update(cx, |s, cx| {
            s.toast = "还没有收藏的歌曲".into();
            cx.notify();
        });
        return false;
    }
    shuffle(&mut songs);
    play_list(store, "我最喜爱".to_string(), songs, 0, false, cx);
    true
}

/// Fisher-Yates 洗牌。
fn shuffle<T>(v: &mut [T]) {
    use rand::Rng;
    let mut rng = rand::rng();
    for i in (1..v.len()).rev() {
        let j = rng.random_range(0..=i);
        v.swap(i, j);
    }
}

/// 用一个现成的列表换掉队列并从 `start` 开始播。
///
/// `fm` = true 时把这一批的 hash 记进 `fm_batch`/`fm_heard`（「播完换一轮」靠它判断）。
fn play_list(
    store: &Entity<AppStore>,
    title: String,
    songs: Vec<Song>,
    start: usize,
    fm: bool,
    cx: &mut App,
) {
    let store = store.clone();
    if songs.is_empty() {
        return;
    }
    let start = start.min(songs.len() - 1);
    // 私人 FM：把这一批的 hash 记下来，后面判断「整批都播过」用
    let fm_hashes: Vec<String> = if fm {
        songs.iter().map(|x| x.hash.clone()).collect()
    } else {
        Vec::new()
    };
    // 设置队列状态
    let current_song = store.update(cx, |s, cx| {
        let mode = s.player.play_mode;
        let volume = s.player.volume;
        s.player.set_queue(&title, songs, start);
        s.player.fm_queue = fm;
        if fm {
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

pub fn next_track(store: &Entity<AppStore>, auto: bool, cx: &mut App) {
    // FM：只在「这一轮没播过的」里挑下一首；都播过就清空队列换下一轮
    // （不能等 `next_index()` 返回 None：列表循环到队尾会绕回第 1 首，永远不换轮）
    let (next, fm) = {
        let s = store.read(cx);
        (crate::actions::fm_next_index(s), s.player.fm_queue)
    };
    match next {
        Some(i) => play_index(store, i, cx),
        None => {
            if fm {
                crate::actions::fm_extend(store, cx);
            } else if !auto {
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
            // 这一轮都播过了 → 换下一轮（单曲循环是用户明确要求的重播，除外）
            let next = crate::actions::fm_next_index(s);
            let fm_round_done = next.is_none()
                && s.player.fm_queue
                && s.player.play_mode != echo_core::PlayMode::Single;
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
            } else if let Some(i) = next {
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
            None
        }
        echo_audio::PlayerEvent::StateChanged(_) => None,
    }
}

/// 指定主题模式：落盘并立刻换装。
///
/// Lite 只有一条播放条，没有页面级过渡，原来那套「开光灯圆形揭示」动画
/// （全屏 canvas + 逐帧插值）一并去掉了。
pub(crate) fn set_theme_mode(store: &Entity<AppStore>, mode: echo_core::ThemeMode, cx: &mut App) {
    if store.read(cx).settings.get().theme_mode == mode {
        return;
    }
    store.update(cx, |s, cx| {
        s.settings.update(|st| st.theme_mode = mode);
        s.toast = match mode {
            echo_core::ThemeMode::Dark => "已切换到深色主题".into(),
            echo_core::ThemeMode::Light => "已切换到浅色主题".into(),
            echo_core::ThemeMode::System => "已切换到跟随系统".into(),
        };
        cx.notify();
    });
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

