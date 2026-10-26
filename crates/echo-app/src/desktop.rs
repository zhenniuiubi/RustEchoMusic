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
    /// 鼠标是否停在歌词上：停住才显示播放控制条（平时只有歌词，不挡视线）
    hovered: bool,
    /// 窗口 HWND（首帧记录）：hover 轮询只做坐标判断，拿不到 window 对象
    hwnd_raw: isize,
    /// 前几帧再补一次「去边框」：DWM 属性在窗口真正上屏后才生效，
    /// 创建时设的那一次会被覆盖，表现就是「没 hover 时仍有一圈边框」
    frame_fix_frames: u8,
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
        if self.hwnd_raw == 0 {
            self.hwnd_raw = crate::win::hwnd(window).map(|h| h as isize).unwrap_or(0);
        }
        if self.frame_fix_frames > 0 {
            self.frame_fix_frames -= 1;
            crate::win::remove_window_frame(window);
        }
        let theme = self.store.read(cx).theme();
        let accent = theme.accent;
        let (lyric, position, position_at, playing, volume) = {
            let s = self.store.read(cx);
            (
                s.lyric.clone(),
                s.player.position,
                s.position_at,
                s.player.playing,
                s.player.volume,
            )
        };
        // 未唱=**中性灰**、已唱=主题色。
        //
        // 不能用半透明白：桌面歌词浮在任何壁纸/窗口之上，白字落在浅色背景
        // （浏览器、资源管理器）上等于隐形。中性灰两头都看得见，也和主题色
        // 差得足够远，注水边界才清楚。
        let unsung: Hsla = gpui::rgb(0x9a9a9a).into();
        let cfg = crate::amll::AmllConfig::new(unsung, accent, px(24.));

        // 位置外推：引擎 Position 只有 4Hz，不外推逐字就是 250ms 一跳的阶梯
        let pos = if playing {
            position + position_at.elapsed().as_secs_f64().min(1.0)
        } else {
            position
        };

        // 当前行（逐字注水 + 换行弹性，统一走 amll::current_line）+ 下一行文本
        let (current, next_text) = match lyric.as_ref() {
            Some(l) => match l.index_at(pos) {
                Some(i) => {
                    let line = &l.lines[i];
                    let t_in_line = pos * 1000.0 + l.offset_ms as f64 - line.time_ms as f64;
                    let el = crate::amll::current_line(line, t_in_line, &cfg, "dl", i, None);
                    (
                        el,
                        l.lines
                            .get(i + 1)
                            .map(|x| x.text.clone())
                            .unwrap_or_default(),
                    )
                }
                // 前奏：先把第一行摆出来（全灰），不要让窗口空着
                None => match l.lines.first() {
                    Some(l0) => (
                        crate::amll::line(vec![(l0.text.clone(), 0.0)], &cfg)
                            .into_any_element(),
                        l.lines
                            .get(1)
                            .map(|x| x.text.clone())
                            .unwrap_or_default(),
                    ),
                    None => (div().into_any_element(), "暂无歌词".to_string()),
                },
            },
            None => (div().into_any_element(), "暂无歌词".to_string()),
        };

        let next_gray: Hsla = gpui::rgb(0x8c8c8c).into();

        // ===== hover 时才出现的控制条（半透明方块按钮，参考 QQ 音乐桌面歌词）=====
        let ctrl = |id: &'static str,
                    icon: &'static str,
                    color: Hsla,
                    action: Box<dyn Fn(&mut Window, &mut App) + 'static>| {
            div()
                .id(id)
                .size(px(26.))
                .rounded(px(7.))
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .cursor_pointer()
                .bg(gpui::black().opacity(0.32))
                .hover(|el| el.bg(gpui::black().opacity(0.52)))
                .on_click(move |_, window, cx| action(window, cx))
                .child(crate::ui::icon(icon, 14., color))
        };
        let icon_color = gpui::white().opacity(0.9);

        let prev_btn = {
            let st = self.store.clone();
            ctrl(
                "dl-prev",
                "icons/prev.svg",
                icon_color,
                Box::new(move |_, cx| crate::ui::prev_track(&st, cx)),
            )
        };
        let play_btn = {
            let st = self.store.clone();
            ctrl(
                "dl-play",
                if playing {
                    "icons/pause.svg"
                } else {
                    "icons/play.svg"
                },
                if playing { accent } else { icon_color },
                Box::new(move |_, cx| crate::ui::toggle_play(&st, cx)),
            )
        };
        let next_btn = {
            let st = self.store.clone();
            ctrl(
                "dl-next",
                "icons/next.svg",
                icon_color,
                Box::new(move |_, cx| crate::ui::next_track(&st, false, cx)),
            )
        };
        let vol_btn = {
            let st = self.store.clone();
            ctrl(
                "dl-vol",
                if volume <= 0.001 {
                    "icons/volume-mute.svg"
                } else {
                    "icons/volume.svg"
                },
                icon_color,
                Box::new(move |_, cx| {
                    let cur = st.read(cx).player.volume;
                    crate::ui::set_volume(&st, if cur > 0.0 { 0.0 } else { 0.8 }, cx);
                }),
            )
        };
        // 列表：把主界面叫回来（桌面歌词是浮窗，歌单在主窗口里）
        let list_btn = {
            let raw = cx
                .try_global::<crate::state::AppGlobal>()
                .and_then(|g| g.main_window)
                .and_then(|h| {
                    h.update(cx, |_, w, _| crate::win::hwnd(w).map(|x| x as isize))
                        .ok()
                        .flatten()
                })
                .unwrap_or(0);
            ctrl(
                "dl-list",
                "icons/queue.svg",
                icon_color,
                Box::new(move |_, cx| {
                    cx.defer(move |_| {
                        if raw != 0 {
                            crate::win::show_window(raw);
                        }
                    });
                }),
            )
        };
        let pin_btn = {
            let pinned = crate::win::is_topmost(window);
            ctrl(
                "dl-pin",
                "icons/pin.svg",
                if pinned { accent } else { icon_color },
                Box::new(|window, _| {
                    let next = !crate::win::is_topmost(window);
                    crate::win::set_topmost(window, next);
                }),
            )
        };
        let close_btn = div()
            .id("dl-close")
            .size(px(26.))
            .rounded(px(7.))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .cursor_pointer()
            .bg(gpui::black().opacity(0.32))
            .hover(|el| el.bg(gpui::black().opacity(0.52)))
            .on_click(cx.listener(
                |view: &mut DesktopLyricView,
                 _,
                 _,
                 cx: &mut Context<DesktopLyricView>| {
                    view.closed.store(true, std::sync::atomic::Ordering::Relaxed);
                    // 关窗同时把设置项落回关（下次启动不再自动恢复）
                    let st = view.store.clone();
                    cx.defer(move |cx| {
                        st.update(cx, |s, _| {
                            s.settings.update(|st| st.desktop_lyric = false);
                        });
                    });
                    cx.notify();
                },
            ))
            .child(crate::ui::icon("icons/close.svg", 14., icon_color));

        let hovered = self.hovered;

        div()
            .id("desktop-lyric")
            .size_full()
            .relative()
            // 默认**无背景**（透明，只浮歌词）；鼠标移入才铺一层 80% 透明深灰底，
            // 让控制条和文字有依托，浅色桌面上也不至于看不清。
            .when(hovered, |el| el.bg(gpui::black().opacity(0.8)).rounded(px(10.)))
            // 整窗的 hover 只用来切换控制条：控制条是子节点，鼠标落在上面也算 hover
            .on_hover(cx.listener(
                |view: &mut DesktopLyricView, hovered: &bool, _, cx: &mut Context<Self>| {
                    if view.hovered != *hovered {
                        view.hovered = *hovered;
                        cx.notify();
                    }
                },
            ))
            .child(
                // 拖动层：铺满窗口、只放非交互的歌词文本（按钮是兄弟节点，
                // 整层标记 Drag 才不会把按钮点击吃成标题栏拖动）
                div()
                    .id("dl-drag")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .px_6()
                    // 不挂 window_control_area：命中成功时系统和这里补发的
                    // WM_NCLBUTTONDOWN 会叠出两个移动循环，拖动会掉帧
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        crate::win::start_drag(window);
                    })
                    .child(div().id("dl-current").child(current))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(next_gray)
                            .child(next_text),
                    ),
            )
            .when(hovered, |el| {
                el.child(
                    div()
                        .id("dl-controls")
                        .absolute()
                        .top(px(6.))
                        .right(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(3.))
                        .occlude()
                        .child(prev_btn)
                        .child(play_btn)
                        .child(next_btn)
                        .child(vol_btn)
                        .child(list_btn)
                        .child(pin_btn)
                        .child(close_btn),
                )
            })
            .into_any_element()
    }
}

/// 浮窗的 hover 轮询。
///
/// gpui 0.2.2 的 Windows 端**没有处理 `WM_MOUSELEAVE`**：鼠标移出窗口后
/// `on_hover(false)` 不会回调，于是「hover 才出现的控制条」会一直挂在窗口上。
/// 这里每 250ms 用光标位置（`GetCursorPos` + `GetWindowRect`）校正一次。
///
/// 视图只需要提供 `hwnd_raw` / `hovered` 两个字段。
trait HoverPollable: 'static {
    fn poll_hwnd(&self) -> isize;
    fn poll_hovered(&self) -> bool;
    fn set_poll_hovered(&mut self, v: bool);
}

impl HoverPollable for DesktopLyricView {
    fn poll_hwnd(&self) -> isize {
        self.hwnd_raw
    }
    fn poll_hovered(&self) -> bool {
        self.hovered
    }
    fn set_poll_hovered(&mut self, v: bool) {
        self.hovered = v;
    }
}

impl HoverPollable for MiniPlayerView {
    fn poll_hwnd(&self) -> isize {
        self.hwnd_raw
    }
    fn poll_hovered(&self) -> bool {
        self.hovered
    }
    fn set_poll_hovered(&mut self, v: bool) {
        self.hovered = v;
    }
}

fn spawn_hover_poll<T: HoverPollable>(weak: WeakEntity<T>, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        cx.background_executor()
            .timer(std::time::Duration::from_millis(250))
            .await;
        let alive = cx
            .update(|cx| {
                weak.update(cx, |view, cx| {
                    let inside = crate::win::cursor_in_rect(view.poll_hwnd());
                    if inside != view.poll_hovered() {
                        view.set_poll_hovered(inside);
                        cx.notify();
                    }
                })
                .is_ok()
            })
            .unwrap_or(false);
        if !alive {
            break;
        }
    })
    .detach();
}

/// mini 窗口收尾：把全局槽位清掉。
///
/// 不清的话，动画泵每次 `refresh_float_windows` 还会升级这个 WeakEntity 并
/// `update` 一个已经 `remove_window` 掉的窗口，gpui 就会打 “window not found”。
fn forget_mini() {
    if let Ok(mut slot) = mini_slot().lock() {
        slot.take();
    }
}

/// 从 mini 播放器回到主窗口：先把主窗口显示出来（`maximize` 时再最大化）。
///
/// **不能在这里同步 ShowWindow**：它派发的 `WM_SIZE` 会让 gpui 的 resize 回调重入
/// `window.update(...)`，日志里就是 “RefCell already borrowed”，窗口铺开了布局却是旧的。
/// 所以只投递系统命令（SC_RESTORE / SC_MAXIMIZE），顺序执行交给窗口线程。
fn restore_main_window(cx: &mut Context<MiniPlayerView>, maximize: bool) {
    let handle = cx
        .try_global::<crate::state::AppGlobal>()
        .and_then(|g| g.main_window);
    let raw = handle
        .and_then(|h| {
            h.update(cx, |_, w, _| crate::win::hwnd(w).map(|x| x as isize))
                .ok()
                .flatten()
        })
        .unwrap_or(0);
    if raw == 0 {
        return;
    }
    crate::win::post_restore(raw);
    if maximize {
        crate::win::post_maximize(raw);
    }
}

// ==================== mini 播放器 ====================

/// mini 播放器窗口尺寸（展开队列时只加高）。
const MINI_W: f32 = 360.;
const MINI_H: f32 = 64.;
const MINI_QUEUE_H: f32 = 324.;
const MINI_ROW_H: f32 = 52.;
/// mini 进度条左端：`pl 6 + 窗口按钮列 24 + gap 8 + 封面 44 + gap 8`
/// （绝对定位的 left 相对 padding box，所以 padding 已经算在里面了）
const MINI_PROGRESS_LEFT: f32 = 90.;

/// mini 播放器视图：封面 + 标题 + 进度 + 播放控制/收藏/列表/桌面歌词/音量 + 置顶（整窗可拖动）。
pub struct MiniPlayerView {
    store: Entity<AppStore>,
    /// 置位后本窗口在下一帧自行关闭（与桌面歌词同一套关闭机制）
    closed: Arc<std::sync::atomic::AtomicBool>,
    focus: Arc<std::sync::atomic::AtomicBool>,
    /// 歌曲列表是否展开（展开时窗口加高）
    queue_open: bool,
    /// 队列列表滚动句柄（展开时滚到当前歌）
    queue_scroll: gpui::UniformListScrollHandle,
    /// 鼠标是否停在 mini 上：停住才浮出播放控制条
    hovered: bool,
    /// 窗口 HWND（首帧记录，hover 轮询用）
    hwnd_raw: isize,
}

impl Render for MiniPlayerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.closed.swap(false, std::sync::atomic::Ordering::Relaxed) {
            window.remove_window();
            return div().into_any_element();
        }
        if self.focus.swap(false, std::sync::atomic::Ordering::Relaxed) {
            window.activate_window();
        }
        if self.hwnd_raw == 0 {
            self.hwnd_raw = crate::win::hwnd(window).map(|h| h as isize).unwrap_or(0);
        }
        let hovered = self.hovered;
        let theme = self.store.read(cx).theme();
        let (song, playing, position, duration, count, queue_open, lyric_on) = {
            let s = self.store.read(cx);
            (
                s.player.current().cloned(),
                s.player.playing,
                s.player.position,
                s.player.duration,
                s.player.queue.len(),
                self.queue_open,
                s.settings.get().desktop_lyric,
            )
        };
        let fav = {
            let s = self.store.read(cx);
            song.as_ref()
                .map(|c| {
                    if s.session.get().userid != 0 {
                        crate::liked::is_remote_liked(&s, c)
                    } else {
                        s.library.is_favorite(c)
                    }
                })
                .unwrap_or(false)
        };
        let store = self.store.clone();
        let ratio = if duration > 0.01 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };

        // 小圆形图标按钮（hover 高亮）
        let btn = |id: &'static str,
                   icon_path: &'static str,
                   color: gpui::Hsla,
                   size: f32,
                   action: Box<dyn Fn(&mut Window, &mut App) + 'static>| {
            div()
                .id(SharedString::from(id))
                .size(px(size))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .cursor_pointer()
                .hover({
                    let bg = theme.bg_hover;
                    move |el| el.bg(bg)
                })
                .on_click(move |_, window, cx| action(window, cx))
                .child(crate::ui::icon(icon_path, size * 0.6, color))
        };

        // 播放圆钮稍大一点，用强调色填充
        let play_btn = {
            let st = store.clone();
            div()
                .id("mini-play")
                .size(px(26.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .cursor_pointer()
                .bg(theme.accent)
                .hover(|el| el.opacity(0.88))
                .on_click(move |_, _, cx| crate::ui::toggle_play(&st, cx))
                .child(crate::ui::icon(
                    if playing { "icons/pause.svg" } else { "icons/play.svg" },
                    15.,
                    gpui::white(),
                ))
        };

        let fav_btn = {
            let cur = song.clone();
            let st = store.clone();
            btn(
                "mini-fav",
                if fav { "icons/heart-filled.svg" } else { "icons/heart.svg" },
                if fav { theme.accent } else { theme.text_secondary },
                24.,
                Box::new(move |_, cx| {
                    if let Some(song) = cur.clone() {
                        crate::liked::toggle_like(&st, &song, cx);
                    }
                }),
            )
        };
        let prev_btn = {
            let st = store.clone();
            btn(
                "mini-prev",
                "icons/prev.svg",
                theme.text,
                24.,
                Box::new(move |_, cx| crate::ui::prev_track(&st, cx)),
            )
        };
        let next_btn = {
            let st = store.clone();
            btn(
                "mini-next",
                "icons/next.svg",
                theme.text,
                24.,
                Box::new(move |_, cx| crate::ui::next_track(&st, false, cx)),
            )
        };
        // 歌曲列表（展开/收起，窗口跟着加高）
        let queue_btn = btn(
            "mini-queue",
            "icons/queue.svg",
            if queue_open { theme.accent } else { theme.text_secondary },
            24.,
            Box::new(|_, _| {}), // 真动作走下面的 on_click（要 window 改尺寸）
        )
        .on_click(cx.listener(
            |view: &mut MiniPlayerView, _, window, cx: &mut Context<MiniPlayerView>| {
                view.queue_open = !view.queue_open;
                let h = if view.queue_open {
                    MINI_H + MINI_QUEUE_H
                } else {
                    MINI_H
                };
                window.resize(size(px(MINI_W), px(h)));
                if view.queue_open {
                    if let Some(ix) = view.store.read(cx).player.index {
                        view.queue_scroll
                            .scroll_to_item(ix, gpui::ScrollStrategy::Center);
                    }
                }
                cx.notify();
            },
        ));
        // 桌面歌词开关（那个 A）
        let lyric_btn = {
            let st = store.clone();
            btn(
                "mini-lyric",
                "icons/lyric.svg",
                if lyric_on { theme.accent } else { theme.text_secondary },
                24.,
                Box::new(move |_, cx| {
                    let next = !st.read(cx).settings.get().desktop_lyric;
                    st.update(cx, |s, cx| {
                        s.settings.update(|st| st.desktop_lyric = next);
                        cx.notify();
                    });
                    crate::desktop::toggle_desktop_lyric(&st, cx);
                }),
            )
        };
        // 音量按钮已从 mini 去掉（图里那排按钮到「A」为止）
        let pin_btn = {
            let pinned = crate::win::is_topmost(window);
            btn(
                "mini-pin",
                "icons/pin.svg",
                if pinned { theme.accent } else { theme.text_secondary },
                24.,
                Box::new(|window, _| {
                    let next = !crate::win::is_topmost(window);
                    crate::win::set_topmost(window, next);
                }),
            )
        };

        // 歌曲列表（展开时才渲染）
        let queue_panel: Option<AnyElement> = if queue_open {
            let rows_store = store.clone();
            let handle = self.queue_scroll.clone();
            // 拖拽起点：按下时写进这个 Arc，容器 on_mouse_move 继续跟手
            let drag: std::sync::Arc<parking_lot::Mutex<Option<(f32, f32)>>> =
                std::sync::Arc::new(parking_lot::Mutex::new(None));
            let move_handle = handle.clone();
            let move_drag = drag.clone();
            let up_drag = drag.clone();

            let list = gpui::uniform_list("mini-queue-list", count, move |range, _w, cx: &mut App| {
                range
                    .map(|ix| mini_queue_row(&rows_store, ix, cx))
                    .collect::<Vec<_>>()
            })
            .track_scroll(handle.clone())
            .size_full();

            let sb_handle = handle.clone();
            let sb_drag = drag.clone();
            let overlay = crate::ui::uniform_scrollbar_overlay(
                "mini-queue-sb",
                sb_handle,
                move |_, start| {
                    *sb_drag.lock() = Some(start);
                },
            );

            Some(
                div()
                    .id("mini-queue-panel")
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .border_t_1()
                    .border_color(theme.border)
                    .pt_1()
                    .on_mouse_move(move |ev: &gpui::MouseMoveEvent, _, _| {
                        if let Some(start) = *move_drag.lock() {
                            crate::ui::apply_scrollbar_drag(
                                &move_handle,
                                start,
                                f32::from(ev.position.y),
                            );
                        }
                    })
                    .on_mouse_up(gpui::MouseButton::Left, move |_, _, _| {
                        *up_drag.lock() = None;
                    })
                    .child(list)
                    .child(overlay)
                    .into_any_element(),
            )
        } else {
            None
        };

        div()
            .id("mini-player")
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme.bg_main)
            // 底层拖动层：满幅只做命中测试。前景没有 hitbox 的区域（封面/文字）
            // 点击穿透到这一层 → 拖动窗口；按钮 occlude 后不受影响
            .child(
                div()
                    .id("mini-drag")
                    .absolute()
                    .size_full()
                    // 与拖动条同款处理：只走 win::start_drag，不挂 window_control_area
                    // （命中成功时它会和补发的 WM_NCLBUTTONDOWN 叠成两个移动循环）
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        crate::win::start_drag(window);
                    }),
            )
            // ===== 第一行：窗口按钮 + 封面 + 文案/进度 + 控制 =====
            .child(
                div()
                    .id("mini-row")
                    .h(px(MINI_H))
                    .flex_shrink_0()
                    .relative()
                    .flex()
                    .items_center()
                    .pl(px(6.))
                    .pr(px(6.))
                    .gap_2()
                    // 左上：关闭（上）/ 恢复主窗口（下）
                    .child(
                        div()
                            .id("mini-win-controls")
                            .w(px(24.))
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(2.))
                            .occlude()
                            .child(
                                div()
                                    .id("mini-close")
                                    .size(px(22.))
                                    .rounded(px(6.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover({
                                        let bg = theme.bg_hover;
                                        move |el| el.bg(bg)
                                    })
                                    .on_click(cx.listener(
                                        |view: &mut MiniPlayerView,
                                         _,
                                         _,
                                         cx: &mut Context<MiniPlayerView>| {
                                            view.closed.store(
                                                true,
                                                std::sync::atomic::Ordering::Relaxed,
                                            );
                                            forget_mini();
                                            restore_main_window(cx, false);
                                            cx.notify();
                                        },
                                    ))
                                    .child(crate::ui::icon(
                                        "icons/close.svg",
                                        11.,
                                        theme.text_tertiary,
                                    )),
                            )
                            .child(
                                // 「最大化」图标其实是「恢复主窗口」：mini 模式下主窗口已隐藏，
                                // 点它把主窗口带回来（不是把 mini 窗口最大化）
                                div()
                                    .id("mini-max")
                                    .size(px(22.))
                                    .rounded(px(6.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover({
                                        let bg = theme.bg_hover;
                                        move |el| el.bg(bg)
                                    })
                                    .on_click(cx.listener(
                                        |view: &mut MiniPlayerView,
                                         _,
                                         _,
                                         cx: &mut Context<MiniPlayerView>| {
                                            view.closed.store(
                                                true,
                                                std::sync::atomic::Ordering::Relaxed,
                                            );
                                            forget_mini();
                                            restore_main_window(cx, false);
                                            cx.notify();
                                        },
                                    ))
                                    .child(crate::ui::icon(
                                        "icons/restore.svg",
                                        11.,
                                        theme.text_tertiary,
                                    )),
                            ),
                    )
                    // 封面
                    .child(
                        div()
                            .w(px(44.))
                            .h(px(44.))
                            .flex_shrink_0()
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
                    // 中段：默认「歌名/歌手」，hover 换成「上一曲/播放/下一曲」
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .relative()
                            .h(px(44.))
                            .flex()
                            .items_center()
                            // 默认：歌名 + 歌手（hover 时隐藏，给控制键让位）
                            .when(!hovered, |el| {
                                el.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .justify_center()
                                        .gap(px(2.))
                                        .child(
                                            div()
                                                .text_size(px(12.5))
                                                .truncate()
                                                .child(
                                                    song.as_ref()
                                                        .map(|c| c.title.clone())
                                                        .unwrap_or_default(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.5))
                                                .text_color(theme.text_tertiary)
                                                .truncate()
                                                .child(
                                                    song.as_ref()
                                                        .map(|c| c.artist.clone())
                                                        .unwrap_or_default(),
                                                ),
                                        ),
                                )
                            })
                            // hover：上一曲 / 播放 / 下一曲（盖住歌名歌手）
                            .when(hovered, |el| {
                                el.child(
                                    div()
                                        .flex_1()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .occlude()
                                        .child(prev_btn)
                                        .child(play_btn)
                                        .child(next_btn),
                                )
                            }),
                    )
                    // 固定控制：收藏 / 列表 / 桌面歌词 / 置顶（一直显示）
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .flex_shrink_0()
                            .occlude()
                            .child(fav_btn)
                            .child(queue_btn)
                            .child(lyric_btn)
                            .child(pin_btn),
                    )
                    // 进度条：与封面**底边对齐**（封面 44 居中 → 距底 10px），
                    // 从封面右缘一路拉到窗口右缘
                    .child(
                        div()
                            .id("mini-progress")
                            .absolute()
                            .bottom(px(10.))
                            .left(px(MINI_PROGRESS_LEFT))
                            .right(px(6.))
                            .h(px(3.))
                            .rounded_full()
                            .overflow_hidden()
                            .child(
                                canvas(
                                    |_, _, _| {},
                                    move |bounds: Bounds<Pixels>, _, window, _| {
                                        window.paint_quad(fill(bounds, theme.progress_track));
                                        let w = bounds.size.width * ratio;
                                        if w > px(0.) {
                                            window.paint_quad(fill(
                                                Bounds::new(
                                                    bounds.origin,
                                                    size(w, bounds.size.height),
                                                ),
                                                theme.accent,
                                            ));
                                        }
                                    },
                                )
                                .size_full(),
                            ),
                    )
            )
            .when_some(queue_panel, |el, panel| el.child(panel))
            .into_any_element()
    }
}

/// mini 播放器里的一行队列（封面 + 歌名 + 歌手，当前播放高亮）。
fn mini_queue_row(store: &Entity<AppStore>, ix: usize, cx: &App) -> AnyElement {
    let theme = store.read(cx).theme();
    let (song, active) = {
        let s = store.read(cx);
        match s.player.queue.get(ix) {
            Some(song) => (song.clone(), s.player.index == Some(ix)),
            None => return div().h(px(MINI_ROW_H)).into_any_element(),
        }
    };
    let st = store.clone();
    div()
        .id(SharedString::from(format!("mq-{ix}")))
        .h(px(MINI_ROW_H))
        .w_full()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .cursor_pointer()
        .when(active, |el| el.bg(theme.accent.opacity(0.10)))
        .when(!active, |el| {
            let bg = theme.bg_hover;
            el.hover(move |s| s.bg(bg))
        })
        .on_click(move |_, _, cx| crate::ui::play_index(&st, ix, cx))
        .child(
            div()
                .w(px(34.))
                .h(px(34.))
                .flex_shrink_0()
                .rounded(px(4.))
                .overflow_hidden()
                .bg(theme.bg_hover)
                .when(!song.cover_url.is_empty(), |el| {
                    el.child(
                        img(SharedUri::from(song.cover_url.clone()))
                            .size_full()
                            .object_fit(ObjectFit::Cover),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .text_size(px(12.))
                        .truncate()
                        .text_color(if active { theme.accent } else { theme.text })
                        .child(song.title),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .truncate()
                        .text_color(theme.text_tertiary)
                        .child(song.artist),
                ),
        )
        .into_any_element()
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
            is_resizable: false,
            // PopUp = WS_EX_TOOLWINDOW：任务栏 / Alt-Tab 里不会出现「桌面歌词」
            // 这个独立窗口（它只是挂在桌面上的一层文字，不该当成一个应用窗口）。
            // 顺带去掉 WS_THICKFRAME/WS_SYSMENU，透明浮窗四周也不会有系统边框。
            kind: WindowKind::PopUp,
            ..Default::default()
        },
        |_, cx| {
            cx.new(|_| DesktopLyricView {
                store: store_c,
                closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                focus: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                hovered: false,
                hwnd_raw: 0,
                frame_fix_frames: 5,
            })
        },
    );
    match window {
        Ok(handle) => {
            let entity = handle
                .update(cx, |_view, window, cx| {
                    crate::win::apply_window_icon(window);
                    // 桌面歌词常驻置顶、无系统边框（透明浮窗，只留文字）
                    crate::win::set_topmost(window, true);
                    crate::win::remove_window_frame(window);
                    window.activate_window();
                    cx.entity()
                })
                .ok();
            if let Some(e) = &entity {
                if let Ok(mut slot) = lyric_slot().lock() {
                    *slot = Some(e.downgrade());
                }
                spawn_hover_poll(e.downgrade(), cx);
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
                size(px(MINI_W), px(MINI_H)),
                cx,
            ))),
            window_min_size: Some(size(px(210.), px(44.))),
            is_resizable: false,
            ..Default::default()
        },
        |_, cx| {
            cx.new(|_| MiniPlayerView {
                store: store_c,
                closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                focus: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                queue_open: false,
                queue_scroll: gpui::UniformListScrollHandle::new(),
                hovered: false,
                hwnd_raw: 0,
            })
        },
    );
    match window {
        Ok(handle) => {
            let entity = handle
                .update(cx, |_view, window, cx| {
                    crate::win::apply_window_icon(window);
                    // mini 播放器常驻置顶
                    crate::win::set_topmost(window, true);
                    // 任务栏不显示 mini 窗口（mini 模式只留主窗口/托盘）
                    crate::win::hide_from_taskbar(window);
                    window.activate_window();
                    cx.entity()
                })
                .ok();
            if let Some(e) = &entity {
                if let Ok(mut slot) = mini_slot().lock() {
                    *slot = Some(e.downgrade());
                }
                spawn_hover_poll(e.downgrade(), cx);
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

/// 桌面歌词窗口是否开着（动画泵判断要不要刷它）。
pub fn lyric_window_open() -> bool {
    lyric_slot()
        .lock()
        .map(|s| s.as_ref().and_then(|w| w.upgrade()).is_some())
        .unwrap_or(false)
}
