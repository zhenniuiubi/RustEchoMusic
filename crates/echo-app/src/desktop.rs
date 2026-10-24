//! 桌面歌词窗口与 mini 播放器（第三期）。
//!
//! 两个都是**独立 gpui 窗口**，共享同一个 `AppStore`，所以播放状态/歌词是同一份数据，
//! 不需要跨窗口同步。窗口句柄存在全局静态里，重复点击只是激活而不是再开一个。

use crate::state::AppStore;
use gpui::{prelude::*, *};
use std::sync::{Arc, Mutex, OnceLock};


/// 桌面歌词窗口句柄。
static LYRIC_WIN: OnceLock<Mutex<Option<WeakEntity<DesktopLyricView>>>> =
    OnceLock::new();
/// mini 播放器窗口句柄。
static MINI_WIN: OnceLock<Mutex<Option<WeakEntity<MiniPlayerView>>>> = OnceLock::new();

fn lyric_slot() -> &'static Mutex<Option<WeakEntity<DesktopLyricView>>> {
    LYRIC_WIN.get_or_init(|| Mutex::new(None))
}

fn mini_slot() -> &'static Mutex<Option<WeakEntity<MiniPlayerView>>> {
    MINI_WIN.get_or_init(|| Mutex::new(None))
}

/// 桌面歌词窗口的视图（只画当前/下一行歌词）。
pub struct DesktopLyricView {
    store: Entity<AppStore>,
    /// 置位后本窗口在下一帧自行关闭（`App` 侧没有按 entity 关窗的接口）
    closed: Arc<std::sync::atomic::AtomicBool>,
    /// 置位后本窗口请求激活
    focus: Arc<std::sync::atomic::AtomicBool>,
}

impl Render for DesktopLyricView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.closed.swap(false, std::sync::atomic::Ordering::Relaxed) {
            window.remove_window();
            return div().into_any_element();
        }
        if self.focus.swap(false, std::sync::atomic::Ordering::Relaxed) {
            window.activate_window();
        }
        let theme = self.store.read(cx).theme();
        let (lyric, position, title) = {
            let s = self.store.read(cx);
            (
                s.lyric.clone(),
                s.player.position,
                s.player.current().map(|c| c.title.clone()).unwrap_or_default(),
            )
        };

        let (cur_text, next_text) = match lyric.as_ref() {
            Some(l) => match l.index_at(position) {
                Some(i) => (
                    l.lines.get(i).map(|x| x.text.clone()).unwrap_or_default(),
                    l.lines.get(i + 1).map(|x| x.text.clone()).unwrap_or_default(),
                ),
                None => (String::new(), l.lines.first().map(|x| x.text.clone()).unwrap_or_default()),
            },
            None => (String::new(), "暂无歌词".to_string()),
        };

        div()
            .id("desktop-lyric")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .px_6()
            // 半透明底：锁在桌面最上层时才不会挡住背景太狠
            .bg(theme.bg_main.opacity(0.72))
            .child(
                div()
                    .text_size(px(20.))
                    .text_color(theme.text)
                    .child(cur_text),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(theme.text_tertiary)
                    .child(next_text),
            )
            .when(!title.is_empty(), |el| {
                el.child(
                    div()
                        .mt_1()
                        .text_size(px(10.))
                        .text_color(theme.text_tertiary)
                        .child(title),
                )
            })
            .into_any_element()
    }
}

/// mini 播放器视图：封面 + 标题 + 播放控制。
pub struct MiniPlayerView {
    store: Entity<AppStore>,
    focus: Arc<std::sync::atomic::AtomicBool>,
}

impl Render for MiniPlayerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus.swap(false, std::sync::atomic::Ordering::Relaxed) {
            window.activate_window();
        }
        let theme = self.store.read(cx).theme();
        let s = self.store.read(cx);
        let song = s.player.current().cloned();
        let playing = s.player.playing;
        let store = self.store.clone();

        div()
            .id("mini-player")
            .size_full()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .bg(theme.bg_main)
            .child(
                div()
                    .w(px(40.))
                    .h(px(40.))
                    .rounded(px(6.))
                    .overflow_hidden()
                    .bg(theme.bg_hover)
                    .when_some(song.as_ref(), |el, song| {
                        el.when(!song.cover_url.is_empty(), |el| {
                            el.child(
                                img(SharedUri::from(song.cover_url.clone()))
                                    .size_full()
                                    .rounded(px(6.))
                                    .object_fit(ObjectFit::Cover),
                            )
                        })
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
                            .text_size(px(12.))
                            .truncate()
                            .child(song.as_ref().map(|c| c.title.clone()).unwrap_or_default()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(theme.text_tertiary)
                            .truncate()
                            .child(song.as_ref().map(|c| c.artist.clone()).unwrap_or_default()),
                    ),
            )
            .child(
                div()
                    .id("mini-prev")
                    .w(px(26.))
                    .h(px(26.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.bg_hover)
                    .cursor_pointer()
                    .on_click({
                        let st = store.clone();
                        move |_, _, cx| crate::ui::prev_track(&st, cx)
                    })
                    .child(crate::ui::icon("icons/prev.svg", 12., theme.text)),
            )
            .child(
                div()
                    .id("mini-play")
                    .w(px(30.))
                    .h(px(30.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.accent)
                    .cursor_pointer()
                    .on_click({
                        let st = store.clone();
                        move |_, _, cx| crate::ui::toggle_play(&st, cx)
                    })
                    .child(crate::ui::icon(
                        if playing {
                            "icons/pause.svg"
                        } else {
                            "icons/play.svg"
                        },
                        13.,
                        gpui::white(),
                    )),
            )
            .child(
                div()
                    .id("mini-next")
                    .w(px(26.))
                    .h(px(26.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.bg_hover)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| crate::ui::next_track(&store, false, cx))
                    .child(crate::ui::icon("icons/next.svg", 12., theme.text)),
            )
    }
}

/// 打开/关闭桌面歌词窗口。
pub fn toggle_desktop_lyric(store: &Entity<AppStore>, cx: &mut App) {
    let on = store.read(cx).settings.get().desktop_lyric;
    if !on {
        // 关：直接关掉窗口
        if let Ok(mut slot) = lyric_slot().lock() {
            if let Some(weak) = slot.take() {
                if let Some(entity) = weak.upgrade() {
                    // 视图自己关窗口：gpui 的 App 侧没有「按 entity 关窗」的接口
                    let _ = entity.update(cx, |view, cx| {
                        view.closed.store(true, std::sync::atomic::Ordering::Relaxed);
                        cx.notify();
                    });
                }
            }
        }
        return;
    }

    // 已经开着就只刷新
    if let Ok(slot) = lyric_slot().lock() {
        if let Some(weak) = slot.as_ref() {
            if weak.upgrade().is_some() {
                return;
            }
        }
    }

    let store_c = store.clone();
    let window = cx.open_window(
        WindowOptions {
            titlebar: None,
            window_background: WindowBackgroundAppearance::Transparent,
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(600.), px(120.)),
                cx,
            ))),
            window_min_size: Some(size(px(320.), px(80.))),
            ..Default::default()
        },
        |_, cx| {
            cx.new(|_| DesktopLyricView {
                store: store_c,
                closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                focus: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            })
        },
    );
    match window {
        Ok(handle) => {
            let entity = handle
                .update(cx, |_view, window, cx| {
                    crate::win::apply_window_icon(window);
                    window.activate_window();
                    cx.entity()
                })
                .ok();
            if let Some(e) = entity {
                if let Ok(mut slot) = lyric_slot().lock() {
                    *slot = Some(e.downgrade());
                }
            }
        }
        Err(e) => log::warn!("桌面歌词窗口打开失败：{e}"),
    }
}

/// 打开 mini 播放器（已开则激活）。
pub fn open_mini_player(store: &Entity<AppStore>, cx: &mut App) {
    if let Ok(slot) = mini_slot().lock() {
        if let Some(weak) = slot.as_ref() {
            if let Some(e) = weak.upgrade() {
                let _ = e.update(cx, |view, cx| {
                    view.focus.store(true, std::sync::atomic::Ordering::Relaxed);
                    cx.notify();
                });
                return;
            }
        }
    }
    let store_c = store.clone();
    let window = cx.open_window(
        WindowOptions {
            titlebar: None,
            window_background: WindowBackgroundAppearance::Opaque,
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(320.), px(64.)),
                cx,
            ))),
            window_min_size: Some(size(px(280.), px(56.))),
            ..Default::default()
        },
        |_, cx| {
            cx.new(|_| MiniPlayerView {
                store: store_c,
                focus: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            })
        },
    );
    match window {
        Ok(handle) => {
            let entity = handle
                .update(cx, |_view, window, cx| {
                    crate::win::apply_window_icon(window);
                    window.activate_window();
                    cx.entity()
                })
                .ok();
            if let Some(e) = entity {
                if let Ok(mut slot) = mini_slot().lock() {
                    *slot = Some(e.downgrade());
                }
            }
        }
        Err(e) => log::warn!("mini 播放器窗口打开失败：{e}"),
    }
}

/// 主窗口刷新时同步刷新这两个浮窗（歌词要跟着进度走）。
pub fn refresh_float_windows(cx: &mut App) {
    if let Ok(slot) = lyric_slot().lock() {
        if let Some(weak) = slot.as_ref() {
            if let Some(e) = weak.upgrade() {
                let _ = e.update(cx, |_, cx| cx.notify());
            } else {
                drop(slot);
            }
        }
    }
    if let Ok(slot) = mini_slot().lock() {
        if let Some(weak) = slot.as_ref() {
            if let Some(e) = weak.upgrade() {
                let _ = e.update(cx, |_, cx| cx.notify());
            }
        }
    }
}
