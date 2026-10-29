//! 桌面歌词窗口与 mini 播放器（第三期）。
//!
//! 它与 Lite 播放条共享同一个 `AppStore`，所以播放状态/歌词是同一份数据，
//! 不需要跨窗口同步。窗口句柄存在全局静态里，重复点击只是激活而不是再开一个。

use crate::state::AppStore;
use gpui::{prelude::*, *};
use std::sync::{Arc, Mutex, OnceLock};


/// 桌面歌词窗口句柄。
static LYRIC_WIN: OnceLock<Mutex<Option<WeakEntity<DesktopLyricView>>>> =
    OnceLock::new();

fn lyric_slot() -> &'static Mutex<Option<WeakEntity<DesktopLyricView>>> {
    LYRIC_WIN.get_or_init(|| Mutex::new(None))
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
    /// 当前窗口尺寸是按哪个字号倍数量出来的（倍数变了就重新量一次）
    sized_for: f32,
}

/// 桌面歌词基准字号（1 倍 = 24px）。
pub const LYRIC_BASE_PX: f32 = 24.;
/// 字号倍数取值范围。
pub const LYRIC_SCALE_MIN: f32 = 2.0;
pub const LYRIC_SCALE_MAX: f32 = 5.0;

/// 读取并夹取字号倍数（老配置没有这个字段时 serde 会给默认 2.0）。
fn lyric_scale_of(settings: &echo_core::Settings) -> f32 {
    let v = if settings.lyric_scale > 0.0 {
        settings.lyric_scale
    } else {
        2.0
    };
    v.clamp(LYRIC_SCALE_MIN, LYRIC_SCALE_MAX)
}

/// 字号倍数 → 窗口尺寸。
///
/// 字号变大必须同步放开窗口：文字排版靠窗口宽度换行，窗口不涨就会被裁掉。
/// 宽度随倍数增长（2 倍 840，4/5 倍封顶 1500），高度按「当前行 + 下一行 + 边距」算。
fn lyric_window_size(scale: f32) -> (f32, f32) {
    let font = LYRIC_BASE_PX * scale;
    let next = 13. * scale;
    let w = (420. * scale).clamp(700., 1500.);
    let h = font * 1.7 + next * 1.5 + 30.;
    (w, h)
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
        // 字号倍数（2–5 倍，默认 2）：窗口尺寸要跟着走，否则大字会被裁掉
        let scale = lyric_scale_of(&self.store.read(cx).settings.get());
        if (self.sized_for - scale).abs() > 0.01 {
            let (mut w, h) = lyric_window_size(scale);
            let dpi = window.scale_factor();
            let (screen_w, _) = crate::win::screen_size_logical(dpi);
            // 5 倍字号（120px）在 1080p 上按公式要 1500 宽，别超出屏幕
            w = w.min((screen_w - 40.0).max(320.0));
            let cur = window.bounds();
            if (f32::from(cur.size.width) - w).abs() > 2.0
                || (f32::from(cur.size.height) - h).abs() > 2.0
            {
                window.resize(size(px(w), px(h)));
                // resize 以左上角为锚点，宽度变大就会伸出屏幕右边；这里保持
                // 「窗口中心不动」重新摆一次位（gpui 0.2.2 没有 set_position）
                if let Some(hwnd) = crate::win::hwnd(window) {
                    let old_cx = f32::from(cur.origin.x) + f32::from(cur.size.width) / 2.0;
                    let nx = (old_cx - w / 2.0).clamp(4.0, (screen_w - w - 4.0).max(4.0));
                    crate::win::move_window(hwnd as isize, nx, f32::from(cur.origin.y), dpi);
                }
            }
            self.sized_for = scale;
        }
        let unsung: Hsla = gpui::rgb(0x9a9a9a).into();
        let cfg = crate::amll::AmllConfig::new(unsung, accent, px(LYRIC_BASE_PX * scale));

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
            // 整窗的 hover 只用来**点亮**控制条/背景。
            //
            // 这里只接受 `hovered == true`，绝不在这里关掉：`dl-controls` 用了
            // `occlude()`（挡住下面的拖动层），鼠标移到按钮上时父节点的 hover 会被
            // 判成 false，于是背景 hide → 250ms 轮询又把它 show → 再移一下又 hide，
            // 表现就是**背景闪烁**。关闭统一交给 `spawn_hover_poll` 按光标位置收敛。
            .on_hover(cx.listener(
                |view: &mut DesktopLyricView, hovered: &bool, _, cx: &mut Context<Self>| {
                    if *hovered && !view.hovered {
                        view.hovered = true;
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
                            .text_size(px(13. * scale))
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
    /// 光标是否算「停在浮窗上」。默认只看整窗矩形；mini 会收窄成
    /// 「歌名/控制键那一块」，鼠标落在收藏/队列/歌词/图钉上不算。
    fn poll_inside(&self) -> bool {
        crate::win::cursor_in_rect(self.poll_hwnd())
    }
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

fn spawn_hover_poll<T: HoverPollable>(weak: WeakEntity<T>, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        cx.background_executor()
            .timer(std::time::Duration::from_millis(120))
            .await;
        let alive = cx
            .update(|cx| {
                weak.update(cx, |view, cx| {
                    let inside = view.poll_inside();
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
    // 初始尺寸按当前字号倍数量出来（默认 2 倍 → 840×147）
    let scale = lyric_scale_of(&store.read(cx).settings.get());
    let (ly_w, ly_h) = lyric_window_size(scale);
    let window = cx.open_window(
        WindowOptions {
            titlebar: None,
            window_background: WindowBackgroundAppearance::Transparent,
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(ly_w), px(ly_h)),
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
                sized_for: scale,
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

/// 主窗口刷新时同步刷新这两个浮窗（歌词要跟着进度走）。
pub fn refresh_float_windows(cx: &mut App) {
    crate::lite::refresh(cx);
    if let Ok(slot) = lyric_slot().lock() {
        if let Some(weak) = slot.as_ref() {
            if let Some(e) = weak.upgrade() {
                let _ = e.update(cx, |_, cx| cx.notify());
            } else {
                drop(slot);
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
