//! Lite 版应用主体：只有一条 330×70 的迷你播放条。
//!
//! 参考原型（网页版 mini player）就是这个 app 的**全部界面**，这里按同一套
//! 几何与配色在 gpui 里重建，并接上真实播放/登录/收藏：
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │ ×  ╷  ┌────┐  歌名                ♥  ☰  A          📌    │
//! │ 👤 ╵  │ 封面│  歌手                                    │
//! │       └────┘  ～～～～～～～～～～～～～～～～～～          │
//! └──────────────────────────────────────────────────────────┘
//! ```
//!
//! 关键几何（与原型一一对应）：
//! - 窗口固定 330×70，圆角 8；
//! - 左栏 24（关闭 18 + 头像 24 竖排、底对齐）；
//! - 封面 50×50、左边距 8、圆角 6；
//! - 文字/进度条左端 96 = 8 + 24 + 8 + 50 + 6；
//! - 进度条贴底（距底 10、高 9），底边与封面底边严格同一条线；
//! - 右侧三个图标 22×22、间距 12、右内边距 14。
//!
//! 面板（队列 / 歌词 / 账号 / 我最喜爱 / 扫码登录）从播放条正下方展开，
//! 与播放条拼成一张完整卡片：窗口 resize 加高，播放条下半圆角收平。

use crate::state::AppStore;
use gpui::{prelude::*, *};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

// ==================== 几何 ====================

/// 播放条宽（窗口宽）。
pub const LITE_W: f32 = 330.;
/// 播放条高（收起面板时窗口高）。
pub const LITE_H: f32 = 70.;
/// 卡片圆角（原型 rounded-lg）。
const RADIUS: f32 = 8.;
const PAD_L: f32 = 8.;
const PAD_R: f32 = 14.;
/// 左栏宽度：关闭 + 头像竖排。
const RAIL_W: f32 = 24.;
const COVER: f32 = 50.;
/// 封面左边距（相对左栏）。
const COVER_ML: f32 = 8.;
const META_ML: f32 = 6.;
/// 文字与进度条的左端（相对窗口左边）。
const CONTENT_LEFT: f32 = PAD_L + RAIL_W + COVER_ML + COVER + META_ML;
/// 进度条距窗口底边、自身高度。
const PROG_BOTTOM: f32 = 10.;
const PROG_H: f32 = 9.;
/// 进度条宽度。
const PROG_W: f32 = LITE_W - CONTENT_LEFT - PAD_R;
/// 右侧图标：按钮边长（22 的可点区 + 两侧各 6 的留白）/ 整组宽度 / 整组左端。
///
/// 间距不用 `gap` / `margin`（在 gpui 0.2.2 的这个盒子里实测都不生效，三块会贴成一条），
/// 直接把留白算进按钮自身宽度 —— 几何就完全可预测了。
const ACT_W: f32 = 34.;
const ACTIONS_W: f32 = ACT_W * 3.;
const ACTIONS_LEFT: f32 = LITE_W - PAD_R - ACTIONS_W;
/// 封面左上角（hover 判定 + 鼠标命中）。
const COVER_X: f32 = PAD_L + RAIL_W + COVER_ML;
const COVER_Y: f32 = (LITE_H - COVER) / 2.;

// ==================== 面板高度 ====================

const PANEL_QUEUE_H: f32 = 264.;
const PANEL_FAV_H: f32 = 264.;
const PANEL_LYRIC_H: f32 = 214.;
const PANEL_ACC_H: f32 = 244.;
const PANEL_ACC_GUEST_H: f32 = 82.;
const PANEL_LOGIN_H: f32 = 196.;
/// 队列行高（原型 padding 7 + 单行 20）。
const ROW_H: f32 = 32.;

// ==================== 状态 ====================

/// 当前展开的下方面板（互斥）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    None,
    Queue,
    Lyric,
    Account,
    Fav,
    Login,
}

/// 进度条形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bars {
    /// 波浪线（默认）
    Wave,
    /// 4px 圆角胶囊
    Plain,
    /// 彩虹 + 像素猫
    Nyan,
}

impl Bars {
    fn from_settings(s: &str) -> Self {
        match s {
            "plain" => Bars::Plain,
            "nyan" => Bars::Nyan,
            _ => Bars::Wave,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Bars::Wave => "wave",
            Bars::Plain => "plain",
            Bars::Nyan => "nyan",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Bars::Wave => "波浪",
            Bars::Plain => "正常",
            Bars::Nyan => "彩虹猫",
        }
    }
    /// 这种形态是否需要持续重绘（有动画）。
    fn animated(self) -> bool {
        !matches!(self, Bars::Plain)
    }
}

/// Lite 播放条视图（也是这个 app 的唯一窗口）。
pub struct LiteView {
    store: Entity<AppStore>,
    /// 置位后本窗口在下一帧自行关闭
    closed: Arc<AtomicBool>,
    /// 置位后请求激活窗口
    focus: Arc<AtomicBool>,
    /// 窗口 HWND（首帧记录，hover 轮询用）
    hwnd_raw: isize,
    /// 鼠标停在「歌名 / 播放控制」那块
    hovered: bool,
    /// 鼠标停在封面上（中央的「不喜欢」只在此时浮出）
    hover_cover: bool,
    panel: Panel,
    /// 队列数据源：false = 私人 FM（当前播放队列）/ true = 我最喜爱
    fav_mode: bool,
    /// 按在封面上的位置（判断「点击」还是「拖动窗口」）
    press_at: Option<Point<Pixels>>,
    /// 队列列表滚动句柄
    queue_scroll: gpui::UniformListScrollHandle,
    /// 队列滚动条拖拽起点（`y, offset`）
    queue_drag: Option<(f32, f32)>,
    /// 进度条拖拽中
    seeking: bool,
    /// 建立时刻（波浪流动 / 彩虹猫跳动的相位基准）
    born: Instant,
    /// 扫码登录：key / 内容 URL / 状态（0 过期 1 等待 2 待确认 4 成功）
    qr_key: String,
    qr_url: String,
    qr_status: i32,
    /// 轮询代次：取消 / 重新获取时自增，旧轮询看到代次变了就退出
    qr_gen: u64,
    qr_busy: bool,
    qr_note: String,
}

static LITE_WIN: OnceLock<Mutex<Option<WeakEntity<LiteView>>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<WeakEntity<LiteView>>> {
    LITE_WIN.get_or_init(|| Mutex::new(None))
}

/// 窗口收尾：清掉全局槽位（否则刷新泵会 update 一个已关掉的窗口）。
fn forget() {
    if let Ok(mut s) = slot().lock() {
        s.take();
    }
}

/// 主循环里每次播放事件都调它：进度/歌名/登录态变了要重绘。
pub fn refresh(cx: &mut App) {
    if let Ok(s) = slot().lock() {
        if let Some(weak) = s.as_ref() {
            if let Some(e) = weak.upgrade() {
                let _ = e.update(cx, |_, cx| cx.notify());
            }
        }
    }
}

/// Lite 窗口是否开着。
pub fn is_open() -> bool {
    slot()
        .lock()
        .map(|s| s.as_ref().and_then(|w| w.upgrade()).is_some())
        .unwrap_or(false)
}

/// 把 Lite 窗口显示出来（托盘「显示」走这里；窗口没开过就新建）。
pub fn show(store: &Entity<AppStore>, cx: &mut App) {
    if let Ok(s) = slot().lock() {
        if let Some(weak) = s.as_ref() {
            if let Some(e) = weak.upgrade() {
                let _ = e.update(cx, |v, cx| {
                    v.closed.store(false, Ordering::Relaxed);
                    v.focus.store(true, Ordering::Relaxed);
                    cx.notify();
                });
                return;
            }
        }
    }
    open(store, cx);
}

/// 打开 Lite 播放条窗口（已开则只激活），返回窗口句柄供托盘 / SMTC 使用。
pub fn open(store: &Entity<AppStore>, cx: &mut App) -> Option<AnyWindowHandle> {
    if is_open() {
        if let Ok(s) = slot().lock() {
            if let Some(e) = s.as_ref().and_then(|w| w.upgrade()) {
                let _ = e.update(cx, |v, cx| {
                    v.focus.store(true, Ordering::Relaxed);
                    cx.notify();
                });
            }
        }
        return None;
    }
    let store_c = store.clone();
    let window = cx.open_window(
        WindowOptions {
            titlebar: None,
            // 透明窗口：圆角由根节点画，四角之外透出桌面
            window_background: WindowBackgroundAppearance::Transparent,
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(LITE_W), px(LITE_H)),
                cx,
            ))),
            window_min_size: Some(size(px(LITE_W), px(LITE_H))),
            is_resizable: false,
            ..Default::default()
        },
        |_, cx| {
            cx.new(|_| LiteView {
                store: store_c,
                closed: Arc::new(AtomicBool::new(false)),
                focus: Arc::new(AtomicBool::new(false)),
                hwnd_raw: 0,
                hovered: false,
                hover_cover: false,
                panel: Panel::None,
                fav_mode: false,
                press_at: None,
                queue_scroll: gpui::UniformListScrollHandle::new(),
                queue_drag: None,
                seeking: false,
                born: Instant::now(),
                qr_key: String::new(),
                qr_url: String::new(),
                qr_status: 1,
                qr_gen: 0,
                qr_busy: false,
                qr_note: String::new(),
            })
        },
    );
    match window {
        Ok(handle) => {
            let entity = handle
                .update(cx, |_v, window, cx| {
                    crate::win::apply_window_icon(window);
                    // 去掉 DWM 那圈 1px 描边，否则失焦状态下外面会多一条深边
                    crate::win::remove_window_frame(window);
                    crate::win::set_topmost(window, true);
                    window.activate_window();
                    cx.entity()
                })
                .ok();
            if let Some(e) = &entity {
                if let Ok(mut s) = slot().lock() {
                    *s = Some(e.downgrade());
                }
                spawn_hover_poll(e.downgrade(), cx);
                spawn_anim_pump(e.downgrade(), cx);
            }
            Some(handle.into())
        }
        Err(e) => {
            log::warn!("Lite 播放条打开失败：{e}");
            None
        }
    }
}

// ==================== 轮询 / 动画 ====================

/// gpui 0.2.2 的 Windows 端不处理 `WM_MOUSELEAVE`，鼠标移出窗口后
/// `on_hover(false)` 不会回调 → 每 120ms 用光标位置校正一次。
fn spawn_hover_poll(weak: WeakEntity<LiteView>, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        cx.background_executor()
            .timer(std::time::Duration::from_millis(120))
            .await;
        let alive = cx
            .update(|cx| {
                weak.update(cx, |v, cx| {
                    let on_meta = v.cursor_on_meta();
                    let on_cover = v.cursor_on_cover();
                    if on_meta != v.hovered || on_cover != v.hover_cover {
                        v.hovered = on_meta;
                        v.hover_cover = on_cover;
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

/// 波浪 / 彩虹猫要连续动，需要一个自己的帧泵（主循环的刷新跟着播放事件走，
/// 暂停时就没有了）。40ms 一帧足够。
fn spawn_anim_pump(weak: WeakEntity<LiteView>, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        cx.background_executor()
            .timer(std::time::Duration::from_millis(40))
            .await;
        let alive = cx
            .update(|cx| {
                weak.update(cx, |v, cx| {
                    let bars = Bars::from_settings(&v.store.read(cx).settings.get().lite_bar);
                    let playing = v.store.read(cx).player.playing;
                    if bars.animated() || playing {
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

// ==================== 小工具 ====================

/// `0xRRGGBB` → Hsla（`gpui::rgb` 返回的 Rgba 没有 `opacity`）。
fn hsla(rgb: u32) -> Hsla {
    gpui::rgb(rgb).into()
}

/// 带透明度的颜色。
fn alpha(rgb: u32, a: f32) -> Hsla {
    let c: Hsla = gpui::rgb(rgb).into();
    c.opacity(a)
}

fn fmt_time(sec: u64) -> String {
    format!("{}:{:02}", sec / 60, sec % 60)
}

impl LiteView {
    /// 光标是否落在「歌名 / 播放控制」那块（左端 = 文字左端，右端 = 右侧图标组左端）。
    ///
    /// 鼠标放到喜欢 / 队列 / 歌词 / 图钉上时**不算** —— 挑按钮时把歌名换走会闪。
    fn cursor_on_meta(&self) -> bool {
        crate::win::cursor_in_client_rect(
            self.hwnd_raw,
            LITE_W,
            CONTENT_LEFT,
            PROG_BOTTOM,
            ACTIONS_LEFT - CONTENT_LEFT,
            LITE_H - PROG_BOTTOM,
        )
    }

    /// 光标是否停在封面上。
    fn cursor_on_cover(&self) -> bool {
        crate::win::cursor_in_client_rect(self.hwnd_raw, LITE_W, COVER_X, COVER_Y, COVER, COVER)
    }

    /// 按进度条位置 seek：拖拽只改 UI 目标，松手才真正 `engine.seek`。
    fn seek_to(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let ratio = ((f32::from(x) - CONTENT_LEFT) / PROG_W).clamp(0., 1.);
        self.store.update(cx, |s, _| {
            let target = ratio as f64 * s.player.duration;
            s.seeking = true;
            s.seek_target = target;
            s.player.position = target;
        });
        cx.notify();
    }

    /// 波浪相位（px，0..26）。61.5 px/s ≈ 原型 1.6s 走 26px。
    fn phase(&self) -> f32 {
        (self.born.elapsed().as_secs_f32() * 61.5) % 26.0
    }

    /// 面板高度。
    fn panel_h(&self, cx: &App) -> f32 {
        match self.panel {
            Panel::None => 0.,
            Panel::Queue => PANEL_QUEUE_H,
            Panel::Fav => PANEL_FAV_H,
            Panel::Lyric => PANEL_LYRIC_H,
            Panel::Login => PANEL_LOGIN_H,
            Panel::Account => {
                if self.store.read(cx).session.get().userid != 0 {
                    PANEL_ACC_H
                } else {
                    PANEL_ACC_GUEST_H
                }
            }
        }
    }

    /// 换高度：面板互斥切换 + 窗口 resize（resize 以左上角为锚，向下长）。
    fn set_panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        self.panel = panel;
        if panel == Panel::Queue {
            if let Some(ix) = self.store.read(cx).player.index {
                self.queue_scroll
                    .scroll_to_item(ix, gpui::ScrollStrategy::Center);
            }
        }
        let h = LITE_H + self.panel_h(cx);
        window.resize(size(px(LITE_W), px(h)));
        cx.notify();
    }

    /// 点同一个按钮就收起。
    fn toggle_panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.panel == panel {
            Panel::None
        } else {
            panel
        };
        self.set_panel(next, window, cx);
    }
}

// ==================== 渲染 ====================

impl Render for LiteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.closed.swap(false, Ordering::Relaxed) {
            window.remove_window();
            return div().into_any_element();
        }
        if self.focus.swap(false, Ordering::Relaxed) {
            window.activate_window();
        }
        if self.hwnd_raw == 0 {
            self.hwnd_raw = crate::win::hwnd(window).map(|h| h as isize).unwrap_or(0);
        }

        let theme = self.store.read(cx).theme();
        let (song, playing, position, duration, logged, _nickname) = {
            let s = self.store.read(cx);
            let ses = s.session.get();
            (
                s.player.current().cloned(),
                s.player.playing,
                s.player.position,
                s.player.duration,
                ses.userid != 0,
                ses.nickname.clone(),
            )
        };
        let fav = {
            let s = self.store.read(cx);
            song.as_ref()
                .map(|c| {
                    if logged {
                        crate::liked::is_remote_liked(&s, c)
                    } else {
                        s.library.is_favorite(c)
                    }
                })
                .unwrap_or(false)
        };
        let bars = Bars::from_settings(&self.store.read(cx).settings.get().lite_bar);
        let ratio = if duration > 0.01 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let phase = self.phase();
        let hovered = self.hovered;
        let hover_cover = self.hover_cover;
        let panel = self.panel;
        let panel_open = panel != Panel::None;
        let store = self.store.clone();

        // ---------- 左栏：关闭（收起到托盘）+ 头像 ----------
        let close_btn = div()
            .id("lite-close")
            .size(px(18.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover({
                let bg = theme.bg_hover;
                move |el| el.bg(bg)
            })
            .on_click(cx.listener(|v: &mut LiteView, _, window, cx| {
                // 「收起播放条」：面板一起收起（否则下次出来还带着面板），
                // 然后隐藏到托盘 —— 托盘菜单可再叫出来
                v.panel = Panel::None;
                window.resize(size(px(LITE_W), px(LITE_H)));
                crate::win::hide_window(window);
                cx.notify();
            }))
            .child(crate::ui::icon("icons/close.svg", 13., theme.text_tertiary));

        let avatar_btn = {
            let active = panel == Panel::Account || panel == Panel::Login;
            let tint = if active {
                theme.accent
            } else {
                theme.text_secondary
            };
            div()
                .id("lite-avatar")
                .size(px(24.))
                .relative()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover({
                    let c = theme.text;
                    move |el| el.text_color(c)
                })
                .on_click(cx.listener(|v: &mut LiteView, _, window, cx| {
                    v.toggle_panel(Panel::Account, window, cx);
                }))
                .child(crate::ui::icon("icons/user.svg", 17., tint))
                .when(logged, |el| {
                    el.child(
                        div()
                            .absolute()
                            .right(px(1.))
                            .bottom(px(5.))
                            .size(px(6.))
                            .rounded_full()
                            .bg(hsla(0x10b981))
                            .border_1()
                            .border_color(theme.bg_panel),
                    )
                })
        };

        let rail = div()
            .id("lite-rail")
            .w(px(RAIL_W))
            .h(px(LITE_H))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_end()
            .gap(px(6.))
            .pb(px(7.))
            .occlude()
            .child(close_btn)
            .child(avatar_btn);

        // ---------- 封面 ----------
        let cover = {
            let cur = song.clone();
            div()
                .id("lite-cover")
                .size(px(COVER))
                .ml(px(COVER_ML))
                .flex_shrink_0()
                .relative()
                .rounded(px(6.))
                .overflow_hidden()
                .bg(theme.bg_hover)
                .cursor_pointer()
                // 点封面 = 播放/暂停；按住拖动 = 移动窗口（位移超过 4px 就不算点击）
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|v: &mut LiteView, ev: &MouseDownEvent, _, cx| {
                        v.press_at = Some(ev.position);
                        cx.notify();
                    }),
                )
                .when(true, |el| {
                    let st = store.clone();
                    el.on_mouse_up(
                        MouseButton::Left,
                        cx.listener(
                            move |v: &mut LiteView, ev: &MouseUpEvent, _, cx: &mut Context<LiteView>| {
                                let moved = v
                                    .press_at
                                    .map(|p| {
                                        (f32::from(ev.position.x - p.x).powi(2)
                                            + f32::from(ev.position.y - p.y).powi(2))
                                        .sqrt()
                                    })
                                    .unwrap_or(99.);
                                v.press_at = None;
                                if moved < 4. {
                                    crate::ui::toggle_play(&st, cx);
                                }
                                cx.notify();
                            },
                        ),
                    )
                })
                .when_some(cur, |el, c| {
                    el.when(!c.cover_url.is_empty(), |el| {
                        el.child(
                            img(SharedUri::from(c.cover_url.clone()))
                                .size_full()
                                .object_fit(ObjectFit::Cover),
                        )
                    })
                })
                // 播放中才有的「不喜欢」：悬停封面浮出，点了跳过这首
                .when(hover_cover, |el| {
                    el.child(
                        div()
                            .id("lite-dislike")
                            .absolute()
                            .left(px(13.))
                            .top(px(13.))
                            .size(px(24.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(alpha(0x201c19, 0.5))
                            .hover(move |el| el.bg(alpha(0x201c19, 0.72)))
                            .cursor_pointer()
                            .occlude()
                            .on_click(cx.listener(|v: &mut LiteView, _, _, cx| {
                                let st = v.store.clone();
                                crate::ui::next_track(&st, false, cx);
                                cx.notify();
                            }))
                            .child(crate::ui::icon("icons/dislike.svg", 14., hsla(0xffffff))),
                    )
                })
        };

        // ---------- 中间：歌名歌手 / hover 播放控制 ----------
        let ctl = |id: &'static str,
                   path: &'static str,
                   size: f32,
                   icon_size: f32,
                   action: Box<dyn Fn(&mut Window, &mut App) + 'static>| {
            div()
                .id(SharedString::from(id))
                .size(px(size))
                .flex_shrink_0()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover({
                    let c = theme.accent;
                    move |el| el.text_color(c)
                })
                .on_click(move |_, window, cx| action(window, cx))
                .child(crate::ui::icon(path, icon_size, theme.text))
        };

        let play_btn = {
            let st = store.clone();
            div()
                .id("lite-play")
                .size(px(30.))
                .flex_shrink_0()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .bg(theme.accent)
                .hover(|el| el.opacity(0.9))
                .on_click(move |_, _, cx| crate::ui::toggle_play(&st, cx))
                .child(crate::ui::icon(
                    if playing {
                        "icons/mini-pause.svg"
                    } else {
                        "icons/mini-play.svg"
                    },
                    13.,
                    hsla(0xffffff),
                ))
        };

        let prev_btn = {
            let st = store.clone();
            ctl(
                "lite-prev",
                "icons/mini-prev.svg",
                24.,
                14.,
                Box::new(move |_, cx| crate::ui::prev_track(&st, cx)),
            )
        };
        let next_btn = {
            let st = store.clone();
            ctl(
                "lite-next",
                "icons/mini-next.svg",
                24.,
                14.,
                Box::new(move |_, cx| crate::ui::next_track(&st, false, cx)),
            )
        };
        // 模式切换：FM（私人漫游）/ 收藏 —— 决定队列面板的数据源
        let fav_mode = self.fav_mode;
        let mode_btn = div()
            .id("lite-mode")
            .size(px(22.))
            .flex_shrink_0()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover({
                let c = theme.accent;
                move |el| el.text_color(c)
            })
            .on_click(cx.listener(|v: &mut LiteView, _, window, cx| {
                v.fav_mode = !v.fav_mode;
                // 队列面板开着就同步换源（标题里的 FM / 收藏 也会跟着变）
                if v.panel == Panel::Queue {
                    v.set_panel(Panel::Queue, window, cx);
                } else {
                    cx.notify();
                }
            }))
            .child(crate::ui::icon(
                if fav_mode {
                    "icons/star.svg"
                } else {
                    "icons/radio.svg"
                },
                13.,
                if fav_mode {
                    theme.accent
                } else {
                    theme.text_secondary
                },
            ));

        let meta = div()
            .id("lite-meta")
            .flex_1()
            .min_w_0()
            .ml(px(META_ML))
            .h(px(LITE_H))
            .relative()
            .flex()
            .items_center()
            // 内容整体上抬 7.5：给底下那条波浪线留呼吸位
            .pb(px(15.))
            .when(!hovered, |el| {
                el.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .gap(px(1.))
                        .child(
                            div()
                                .text_size(px(15.))
                                .line_height(px(20.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .truncate()
                                .text_color(theme.text)
                                .child(
                                    song.as_ref()
                                        .map(|c| c.title.clone())
                                        .unwrap_or_else(|| "未在播放".into()),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .line_height(px(16.))
                                .truncate()
                                .text_color(theme.text_secondary)
                                .child(
                                    song.as_ref()
                                        .map(|c| c.artist.clone())
                                        .unwrap_or_else(|| "选一首歌开始".into()),
                                ),
                        ),
                )
            })
            .when(hovered, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(px(0.))
                        .right(px(0.))
                        .top(px(0.))
                        .bottom(px(0.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .pb(px(12.))
                        .occlude()
                        .child(prev_btn)
                        .child(play_btn)
                        .child(next_btn)
                        .child(
                            div()
                                .w(px(1.))
                                .h(px(14.))
                                .flex_shrink_0()
                                .mx(px(1.))
                                .bg(theme.border),
                        )
                        .child(mode_btn),
                )
            });

        // ---------- 右侧：喜欢 / 队列 / 歌词 ----------
        // 只做外观：事件由调用方挂（同一个元素挂两次 on_click 只有最后一次生效）
        let act = |id: &'static str, content: AnyElement, color: Hsla| {
            div()
                .id(SharedString::from(id))
                .size(px(ACT_W))
                .flex_shrink_0()
                .rounded(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(color)
                .hover({
                    let c = theme.text;
                    move |el| el.text_color(c)
                })
                .child(content)
        };

        let like_btn = {
            let st = store.clone();
            let cur = song.clone();
            act(
                "lite-like",
                crate::ui::icon(
                    if fav {
                        "icons/heart-filled.svg"
                    } else {
                        "icons/heart.svg"
                    },
                    19.,
                    if fav { theme.accent } else { theme.text_tertiary },
                )
                .into_any_element(),
                if fav { theme.accent } else { theme.text_tertiary },
            )
            .on_click(move |_, _, cx| {
                if let Some(s) = cur.clone() {
                    crate::liked::toggle_like(&st, &s, cx);
                }
            })
        };
        let list_btn = {
            let open = panel == Panel::Queue;
            act(
                "lite-list",
                crate::ui::icon(
                    "icons/playlist.svg",
                    18.,
                    if open { theme.accent } else { theme.text_secondary },
                )
                .into_any_element(),
                if open { theme.accent } else { theme.text_secondary },
            )
            .on_click(cx.listener(|v: &mut LiteView, _, window, cx| {
                v.toggle_panel(Panel::Queue, window, cx);
            }))
        };
        let lyric_btn = {
            let open = panel == Panel::Lyric;
            act(
                "lite-lyric",
                crate::ui::icon(
                    "icons/lyric.svg",
                    19.,
                    if open { theme.accent } else { theme.text_secondary },
                )
                .into_any_element(),
                if open { theme.accent } else { theme.text_secondary },
            )
            .on_click(cx.listener(|v: &mut LiteView, _, window, cx| {
                v.toggle_panel(Panel::Lyric, window, cx);
            }))
        };

        let actions = div()
            .flex()
            .items_center()
            .ml(px(10.))
            .mb(px(12.))
            .flex_shrink_0()
            .occlude()
            .child(like_btn)
            .child(list_btn)
            .child(lyric_btn);

        // ---------- 右上角图钉 ----------
        let pinned = crate::win::is_topmost(window);
        let pin = div()
            .id("lite-pin")
            .absolute()
            .top(px(3.))
            .right(px(6.))
            .size(px(16.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .occlude()
            .text_color(if pinned {
                theme.accent
            } else {
                theme.text_tertiary
            })
            .hover({
                let c = theme.accent;
                move |el| el.text_color(c)
            })
            .on_click(|_, window, _| {
                let next = !crate::win::is_topmost(window);
                crate::win::set_topmost(window, next);
            })
            .child(crate::ui::icon(
                "icons/pin.svg",
                12.,
                if pinned {
                    theme.accent
                } else {
                    theme.text_tertiary
                },
            ));

        // ---------- 进度条（三形态） ----------
        let wave = div()
            .id("lite-progress")
            .absolute()
            .left(px(CONTENT_LEFT))
            .bottom(px(PROG_BOTTOM))
            .w(px(PROG_W))
            .h(px(PROG_H))
            .cursor_pointer()
            .child(
                // 上下各多 6px 的点击热区
                div()
                    .absolute()
                    .left(px(0.))
                    .right(px(0.))
                    .top(px(-6.))
                    .bottom(px(-2.))
                    .child({
                        let th = theme.clone();
                        canvas(
                            |_, _, _| {},
                            move |bounds: Bounds<Pixels>, _, window, _| {
                                paint_bars(window, bounds, ratio, phase, &th, bars);
                            },
                        )
                        .size_full()
                    }),
            )
            .when(bars == Bars::Nyan, |el| {
                // 猫头：右缘随进度走，0% 时停在 30px，100% 时贴右端
                let x = ((ratio * PROG_W).max(30.) - 26.).max(0.);
                el.child(
                    img("lite/nyan-cat.png")
                        .absolute()
                        .left(px(x))
                        .top(px(-5.5))
                        .w(px(26.))
                        .h(px(20.)),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|v: &mut LiteView, ev: &MouseDownEvent, _, cx| {
                    v.seeking = true;
                    v.seek_to(ev.position.x, cx);
                }),
            )
            .on_mouse_move(
                cx.listener(|v: &mut LiteView, ev: &MouseMoveEvent, _, cx| {
                    if v.seeking && ev.pressed_button == Some(MouseButton::Left) {
                        v.seek_to(ev.position.x, cx);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|v: &mut LiteView, _, _, cx| {
                    if v.seeking {
                        v.seeking = false;
                        v.store.update(cx, |s, _| s.engine.seek(s.seek_target));
                        cx.notify();
                    }
                }),
            );

        // ---------- 组装 ----------
        let bar = div()
            .id("lite-bar")
            .w_full()
            .h(px(LITE_H))
            .flex_shrink_0()
            .relative()
            .flex()
            .items_center()
            .pl(px(PAD_L))
            .pr(px(PAD_R))
            .when(panel_open, |el| el.rounded_b(px(0.)))
            .child(rail)
            .child(cover)
            .child(meta)
            .child(actions)
            .child(pin)
            .child(wave);

        let body = self.panel_body(cx);

        div()
            .id("lite-root")
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .rounded(px(RADIUS))
            .bg(theme.bg_panel)
            .overflow_hidden()
            // 底层拖动层：前景没被 occlude 的区域（文字、空白）按住即拖窗口
            .child(
                div()
                    .id("lite-drag")
                    .absolute()
                    .size_full()
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        crate::win::start_drag(window);
                    }),
            )
            .child(bar)
            .when_some(body, |el, b| el.child(b))
            .into_any_element()
    }
}

// ==================== 进度条绘制 ====================

/// 三种形态都在同一块 canvas 里画（波浪 / 普通胶囊 / 彩虹 + 猫）。
fn paint_bars(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    ratio: f32,
    phase: f32,
    theme: &crate::theme::Theme,
    bars: Bars,
) {
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    if w <= 0.0 {
        return;
    }
    let x0 = bounds.origin.x;
    let y0 = bounds.origin.y;
    match bars {
        Bars::Plain => {
            let bar_h = 4.0;
            let top = y0 + px((h - bar_h) / 2.0);
            window.paint_quad(
                fill(
                    Bounds::new(point(x0, top), size(px(w), px(bar_h))),
                    theme.progress_track,
                )
                .corner_radii(Corners::all(px(2.))),
            );
            let fw = w * ratio;
            if fw > 0.5 {
                window.paint_quad(
                    fill(
                        Bounds::new(point(x0, top), size(px(fw), px(bar_h))),
                        theme.accent,
                    )
                    .corner_radii(Corners::all(px(2.))),
                );
            }
        }
        Bars::Wave => {
            // 正弦波：周期 26、振幅 3、线宽 1.7。
            // gpui 没有描边路径，这里按 0.75px 步长画一串小方块拼出曲线
            // （相邻两块纵向最多差 0.7px，视觉上连续）。
            let mid = h / 2.0;
            let amp = 3.0;
            let period = 26.0;
            let fill_w = w * ratio;
            let mut x = 0.0;
            while x < w {
                let y = mid - amp * (((x + phase) / period) * std::f32::consts::TAU).sin();
                let played = x < fill_w;
                let mut color = if played {
                    theme.accent
                } else {
                    theme.progress_track
                };
                if played {
                    // 末端 12px 柔化（原型用 mask 渐变收口）
                    let fade = fill_w - 12.0;
                    if x > fade {
                        let t = ((x - fade) / 12.0).clamp(0.0, 1.0);
                        color = color.opacity(1.0 - t * 0.85);
                    }
                }
                window.paint_quad(fill(
                    Bounds::new(
                        point(x0 + px(x), y0 + px(y - 0.85)),
                        size(px(1.05), px(1.7)),
                    ),
                    color,
                ));
                x += 0.75;
            }
        }
        Bars::Nyan => {
            // 6 条 1px 横带：红 / 橙 / 黄 / 绿 / 蓝 / 紫
            const RAINBOW: [u32; 6] =
                [0xFF0000, 0xFFCB00, 0xFEFF00, 0x00AD00, 0x008BFF, 0x8D24FF];
            let top = y0 + px((h - 6.0) / 2.0);
            let fw = w * ratio;
            if fw <= 0.5 {
                return;
            }
            for (i, c) in RAINBOW.iter().enumerate() {
                window.paint_quad(fill(
                    Bounds::new(point(x0, top + px(i as f32)), size(px(fw), px(1.2))),
                    hsla(*c),
                ));
            }
        }
    }
}

// ==================== 面板 ====================

impl LiteView {
    fn panel_body(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.panel {
            Panel::None => None,
            Panel::Queue => Some(self.queue_panel(cx)),
            Panel::Fav => Some(self.fav_panel(cx)),
            Panel::Lyric => Some(self.lyric_panel(cx)),
            Panel::Account => Some(self.account_panel(cx)),
            Panel::Login => Some(self.login_panel(cx)),
        }
    }

    /// 队列 / 我最喜爱 共用的一行（四列：序号 / 歌名 / 歌手 / 时长）。
    fn list_row(
        store: &Entity<AppStore>,
        ix: usize,
        song: &echo_core::Song,
        active: bool,
        theme: &crate::theme::Theme,
    ) -> AnyElement {
        let st = store.clone();
        div()
            .id(SharedString::from(format!("lite-row-{ix}")))
            .h(px(ROW_H))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .rounded(px(6.))
            .cursor_pointer()
            .when(active, |el| el.bg(theme.accent.opacity(0.10)))
            .when(!active, |el| {
                let bg = theme.bg_hover;
                el.hover(move |s| s.bg(bg))
            })
            // 面板容器上挂着 on_mouse_up（滚动条收尾），子元素的 on_click 拿不到 up
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                crate::ui::play_index(&st, ix, cx)
            })
            .child(
                div()
                    .w(px(16.))
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(if active {
                        theme.accent
                    } else {
                        theme.text_tertiary
                    })
                    .child(format!("{}", ix + 1)),
            )
            .child(
                div()
                    .w(px(124.))
                    .flex_shrink_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(if active { theme.accent } else { theme.text })
                    .child(song.title.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.))
                    .text_color(theme.text_tertiary)
                    .child(song.artist.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(theme.text_tertiary)
                    .child(fmt_time(song.duration)),
            )
            .into_any_element()
    }

    /// 面板头部：`标题 · 副标题` + 右侧计数。
    fn head(title: &str, sub: &str, count: &str, theme: &crate::theme::Theme) -> Div {
        let mut el = div()
            .h(px(34.))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(5.))
            .px(px(8.))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(theme.text_secondary)
                    .child(title.to_string()),
            );
        if !sub.is_empty() {
            el = el
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(theme.text_tertiary)
                        .child("·"),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(theme.accent)
                        .child(sub.to_string()),
                );
        }
        if !count.is_empty() {
            el = el.child(
                div()
                    .ml_auto()
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(theme.text_tertiary)
                    .child(count.to_string()),
            );
        }
        el
    }

    fn queue_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.store.read(cx).theme();
        let (rows, total) = {
            let s = self.store.read(cx);
            if self.fav_mode {
                let v = s.library.get().favorites.to_vec();
                (v.clone(), v.len())
            } else {
                (s.player.queue.clone(), s.player.queue.len())
            }
        };
        let handle = self.queue_scroll.clone();
        let drag_view = cx.entity();
        let store = self.store.clone();
        let count = rows.len();
        let fav_mode = self.fav_mode;

        let list = if count == 0 {
            let st = store.clone();
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text_tertiary)
                        .child(if fav_mode {
                            "收藏里还没有歌曲，点 ♥ 收藏"
                        } else {
                            "播放队列是空的"
                        }),
                )
                .when(!fav_mode, |el| {
                    el.child(
                        div()
                            .id("lite-play-fm")
                            .h(px(26.))
                            .px(px(12.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .bg(theme.accent)
                            .hover(|el| el.opacity(0.9))
                            .on_click(move |_, _, cx| {
                                crate::actions::load_fm(&st, true, cx);
                            })
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(hsla(0xffffff))
                                    .child("播放私人 FM"),
                            ),
                    )
                })
                .into_any_element()
        } else {
            let rows_store = store.clone();
            uniform_list("lite-queue-list", count, move |range, _w, cx: &mut App| {
                let theme = rows_store.read(cx).theme();
                let active_ix = rows_store.read(cx).player.index;
                range
                    .map(|ix| match rows.get(ix) {
                        Some(song) => {
                            Self::list_row(&rows_store, ix, song, active_ix == Some(ix), &theme)
                        }
                        None => div().h(px(ROW_H)).into_any_element(),
                    })
                    .collect::<Vec<_>>()
            })
            .track_scroll(handle.clone())
            .size_full()
            .into_any_element()
        };

        let sb_handle = handle.clone();
        let overlay = div()
            .absolute()
            .top(px(0.))
            .bottom(px(0.))
            .right(px(0.))
            .w(px(10.))
            .child(crate::ui::uniform_scrollbar_overlay(
                "lite-queue-sb",
                sb_handle,
                move |cx, start| {
                    let _ = drag_view.update(cx, |v: &mut LiteView, _| v.queue_drag = Some(start));
                },
            ));

        // FM 模式下头部右侧多一个「换一批」（只刷新推荐，不打断当前播放）
        let refresh_fm = if fav_mode {
            None
        } else {
            let st = store.clone();
            Some(
                div()
                    .id("lite-fm-refresh")
                    .ml_auto()
                    .h(px(20.))
                    .px(px(7.))
                    .rounded(px(5.))
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .cursor_pointer()
                    .hover({
                        let bg = theme.bg_hover;
                        move |el| el.bg(bg)
                    })
                    .on_click(move |_, _, cx| crate::actions::load_fm(&st, true, cx))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(theme.text_secondary)
                            .child("换一批"),
                    ),
            )
        };

        let head = {
            let mut h = Self::head(
                "播放队列",
                if fav_mode { "收藏" } else { "FM" },
                "",
                &theme,
            );
            match refresh_fm {
                Some(btn) => h = h.child(btn),
                None => {
                    h = h.child(
                        div()
                            .ml_auto()
                            .text_size(px(11.))
                            .text_color(theme.text_tertiary)
                            .child(format!("{total} 首")),
                    )
                }
            }
            h
        };

        div()
            .id("lite-panel-queue")
            .w_full()
            .h(px(PANEL_QUEUE_H))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .px(px(4.))
            .pb(px(6.))
            .child(head)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .pt(px(4.))
                    .on_mouse_move(
                        cx.listener(|v: &mut LiteView, ev: &MouseMoveEvent, _, cx| {
                            if let Some(start) = v.queue_drag {
                                let h = v.queue_scroll.clone();
                                crate::ui::apply_scrollbar_drag(&h, start, f32::from(ev.position.y));
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|v: &mut LiteView, _, _, cx| {
                            v.queue_drag = None;
                            cx.notify();
                        }),
                    )
                    .child(list)
                    .child(overlay),
            )
            .into_any_element()
    }

    fn fav_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.store.read(cx).theme();
        let (rows, total) = {
            let s = self.store.read(cx);
            let lib = s.library.get();
            (lib.favorites.to_vec(), lib.favorites.len())
        };
        let store = self.store.clone();
        let count = rows.len();

        let head = div()
            .h(px(34.))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("lite-fav-back")
                    .size(px(18.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover({
                        let bg = theme.bg_hover;
                        move |el| el.bg(bg)
                    })
                    .on_click(cx.listener(|v: &mut LiteView, _, window, cx| {
                        v.set_panel(Panel::Account, window, cx);
                    }))
                    .child(crate::ui::icon(
                        "icons/arrow-left.svg",
                        13.,
                        theme.text_secondary,
                    )),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(theme.text_secondary)
                    .child("我最喜爱"),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(theme.text_tertiary)
                    .child(format!("{total} 首")),
            )
            .when(total > 0, |el| {
                let st = store.clone();
                el.child(
                    div()
                        .id("lite-play-all")
                        .ml_auto()
                        .h(px(22.))
                        .px(px(8.))
                        .rounded(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .bg(theme.accent)
                        .hover(|el| el.opacity(0.9))
                        .on_click(move |_, _, cx| crate::ui::play_index(&st, 0, cx))
                        .child(crate::ui::icon("icons/mini-play.svg", 10., hsla(0xffffff)))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(hsla(0xffffff))
                                .child("播放全部"),
                        ),
                )
            });

        let list = if count == 0 {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(theme.text_tertiary)
                .child("还没有收藏的歌曲，点 ♥ 即可加入")
                .into_any_element()
        } else {
            let rows_store = store.clone();
            uniform_list("lite-fav-list", count, move |range, _w, cx: &mut App| {
                let theme = rows_store.read(cx).theme();
                let active_ix = rows_store.read(cx).player.index;
                range
                    .map(|ix| match rows.get(ix) {
                        Some(song) => {
                            Self::list_row(&rows_store, ix, song, active_ix == Some(ix), &theme)
                        }
                        None => div().h(px(ROW_H)).into_any_element(),
                    })
                    .collect::<Vec<_>>()
            })
            .size_full()
            .into_any_element()
        };

        div()
            .id("lite-panel-fav")
            .w_full()
            .h(px(PANEL_FAV_H))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .px(px(4.))
            .pb(px(6.))
            .child(head)
            .child(div().flex_1().min_h_0().pt(px(4.)).child(list))
            .into_any_element()
    }

    fn lyric_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.store.read(cx).theme();
        let (lyric, position, loading) = {
            let s = self.store.read(cx);
            (s.lyric.clone(), s.player.position, s.lyric_loading)
        };
        let cur = lyric
            .as_ref()
            .and_then(|l| l.index_at(position * 1000.0))
            .unwrap_or(0);
        let lines: Vec<String> = lyric
            .as_ref()
            .map(|l| l.lines.iter().map(|line| line.text.clone()).collect())
            .unwrap_or_default();

        let body = if lines.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(theme.text_tertiary)
                .child(if loading { "歌词加载中…" } else { "暂无歌词" })
                .into_any_element()
        } else {
            let start = cur.saturating_sub(2);
            let end = (cur + 3).min(lines.len());
            let mut col = div()
                .flex_1()
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(8.))
                .px(px(14.));
            for i in start..end {
                let active = i == cur;
                col = col.child(
                    div()
                        .w_full()
                        .truncate()
                        .text_size(px(if active { 14. } else { 12. }))
                        .text_color(if active {
                            theme.accent
                        } else {
                            theme.text_tertiary
                        })
                        .child(lines[i].clone()),
                );
            }
            col.into_any_element()
        };

        div()
            .id("lite-panel-lyric")
            .w_full()
            .h(px(PANEL_LYRIC_H))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .px(px(4.))
            .pb(px(6.))
            .child(Self::head("歌词", "", "", &theme))
            .child(body)
            .into_any_element()
    }
}

impl LiteView {
    fn account_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.store.read(cx).theme();
        let (logged, nickname, vip, vip_end, fav_count) = {
            let s = self.store.read(cx);
            let ses = s.session.get();
            (
                ses.userid != 0,
                ses.nickname.clone(),
                ses.vip_type,
                ses.vip_end_time,
                s.library.get().favorites.len(),
            )
        };
        let store = self.store.clone();
        let bars = Bars::from_settings(&self.store.read(cx).settings.get().lite_bar);
        let accent_hex = self.store.read(cx).settings.get().theme_color.clone();
        let h = if logged {
            PANEL_ACC_H
        } else {
            PANEL_ACC_GUEST_H
        };

        // ---------- 用户行 ----------
        let avatar = {
            let ch = nickname.chars().next().unwrap_or('我').to_string();
            div()
                .size(px(34.))
                .flex_shrink_0()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.accent.opacity(0.12))
                .text_size(px(15.))
                .text_color(theme.accent)
                .child(if logged { ch } else { String::new() })
                .when(!logged, |el| {
                    el.child(crate::ui::icon("icons/user.svg", 17., theme.text_secondary))
                })
        };

        let user_row = {
            let mut row = div()
                .w_full()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(avatar)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.5))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(theme.text)
                                        .child(if logged {
                                            if nickname.is_empty() {
                                                "已登录".to_string()
                                            } else {
                                                nickname.clone()
                                            }
                                        } else {
                                            "未登录".to_string()
                                        }),
                                )
                                .when(logged && vip > 0, |el| {
                                    el.child(
                                        div()
                                            .flex_shrink_0()
                                            .px(px(4.))
                                            .rounded(px(4.))
                                            .bg(alpha(0xf59e0b, 0.16))
                                            .text_size(px(9.))
                                            .text_color(hsla(0xb45309))
                                            .child("VIP"),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(theme.text_tertiary)
                                .child(if logged {
                                    if vip_end > 0 {
                                        format!("会员有效期至 {}", fmt_date(vip_end))
                                    } else {
                                        "已同步收藏与播放记录".to_string()
                                    }
                                } else {
                                    "扫码登录后同步收藏与播放记录".to_string()
                                }),
                        ),
                );

            if logged {
                let st = store.clone();
                row = row.child(
                    div()
                        .id("lite-logout")
                        .h(px(22.))
                        .px(px(9.))
                        .rounded(px(6.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .border_1()
                        .border_color(theme.border)
                        .hover({
                            let bg = theme.bg_hover;
                            move |el| el.bg(bg)
                        })
                        .on_click(move |_, _, cx| crate::actions::logout(&st, cx))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme.text_secondary)
                                .child("退出登录"),
                        ),
                );
            } else {
                // 未登录：直接进扫码面板（整个 app 只有这一个登录入口）
                row = row.child(
                    div()
                        .id("lite-goto-login")
                        .h(px(22.))
                        .px(px(10.))
                        .rounded(px(6.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .bg(theme.accent)
                        .hover(|el| el.opacity(0.9))
                        .on_click(cx.listener(|v: &mut LiteView, _, window, cx| {
                            v.set_panel(Panel::Login, window, cx);
                            let st = v.store.clone();
                            crate::actions::start_qr_login(&st, cx);
                        }))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(hsla(0xffffff))
                                .child("扫码登录"),
                        ),
                );
            }
            row
        };

        // ---------- 我最喜爱入口 ----------
        let fav_entry = div()
            .id("lite-fav-entry")
            .mt(px(10.))
            .h(px(32.))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(7.))
            .px(px(9.))
            .rounded(px(6.))
            .cursor_pointer()
            .border_1()
            .border_color(theme.border)
            .hover({
                let bg = theme.bg_hover;
                move |el| el.bg(bg)
            })
            .on_click(cx.listener(|v: &mut LiteView, _, window, cx| {
                v.set_panel(Panel::Fav, window, cx);
            }))
            .child(crate::ui::icon("icons/heart.svg", 14., theme.accent))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text)
                    .child("我最喜爱"),
            )
            .child(
                div()
                    .ml_auto()
                    .text_size(px(11.))
                    .text_color(theme.text_tertiary)
                    .child(format!("{fav_count} 首")),
            )
            .child(crate::ui::icon(
                "icons/arrow-right.svg",
                12.,
                theme.text_tertiary,
            ));

        // ---------- 主题色 ----------
        const SWATCHES: [u32; 7] =
            [0xF43F5E, 0xF97316, 0xF59E0B, 0x10B981, 0x3B82F6, 0x8B5CF6, 0xEC4899];
        let mut sw = div().flex().items_center().gap(px(7.));
        for c in SWATCHES {
            let hexs = format!("#{c:06x}");
            let selected = hexs.eq_ignore_ascii_case(&accent_hex);
            let st = store.clone();
            let hx = hexs.clone();
            sw = sw.child(
                div()
                    .id(SharedString::from(format!("lite-sw-{hx}")))
                    .size(px(18.))
                    .rounded_full()
                    .flex_shrink_0()
                    .cursor_pointer()
                    .bg(hsla(c))
                    .when(selected, |el| {
                        el.border_2().border_color(theme.text_secondary)
                    })
                    .on_click(move |_, _, cx| {
                        st.update(cx, |s, cx| {
                            s.settings.update(|set| set.theme_color = hx.clone());
                            cx.notify();
                        });
                    }),
            );
        }

        let theme_block = div()
            .mt(px(10.))
            .pt(px(9.))
            .border_t_1()
            .border_color(theme.border)
            .w_full()
            .flex_shrink_0()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.text_tertiary)
                    .child("主题色"),
            )
            .child(div().mt(px(7.)).child(sw));

        // ---------- 进度条形态 ----------
        let mut opts = div().flex().items_center().gap(px(6.)).mt(px(7.));
        for b in [Bars::Plain, Bars::Wave, Bars::Nyan] {
            let st = store.clone();
            let on = b == bars;
            opts = opts.child(
                div()
                    .id(SharedString::from(format!("lite-bar-{}", b.key())))
                    .h(px(22.))
                    .px(px(9.))
                    .rounded(px(6.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .when(on, |el| {
                        el.bg(theme.accent.opacity(0.12))
                            .border_1()
                            .border_color(theme.accent.opacity(0.35))
                    })
                    .when(!on, |el| {
                        el.border_1().border_color(theme.border).hover({
                            let bg = theme.bg_hover;
                            move |el| el.bg(bg)
                        })
                    })
                    .on_click(move |_, _, cx| {
                        st.update(cx, |s, cx| {
                            s.settings.update(|set| set.lite_bar = b.key().to_string());
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(if on {
                                theme.accent
                            } else {
                                theme.text_secondary
                            })
                            .child(b.label()),
                    ),
            );
        }
        let bar_block = div()
            .mt(px(10.))
            .pt(px(9.))
            .border_t_1()
            .border_color(theme.border)
            .w_full()
            .flex_shrink_0()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.text_tertiary)
                    .child("进度条"),
            )
            .child(opts);

        div()
            .id("lite-panel-acc")
            .w_full()
            .h(px(h))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .px(px(12.))
            .pt(px(10.))
            .pb(px(12.))
            .child(user_row)
            .when(logged, |el| {
                el.child(fav_entry).child(theme_block).child(bar_block)
            })
            .into_any_element()
    }

    /// 扫码登录面板：二维码 + 状态文案 + 刷新/取消。
    ///
    /// 二维码矩阵与轮询都复用主窗口登录页那套（`actions::start_qr_login` 把
    /// key/url/status 写在 `store.login` 里），这里只换成 330 宽的紧凑版。
    fn login_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.store.read(cx).theme();
        let (url, status, loading, err) = {
            let s = self.store.read(cx);
            (
                s.login.qr_url.clone(),
                s.login.qr_status,
                s.login.qr_loading,
                s.login.error.clone(),
            )
        };
        let logged = self.store.read(cx).session.get().userid != 0;

        let (tip, color) = if logged {
            ("登录成功", theme.accent)
        } else {
            match status {
                4 => ("登录成功", theme.accent),
                2 => ("已扫描，请在手机上确认", theme.accent),
                0 => ("二维码已过期", theme.text_tertiary),
                _ => ("打开酷狗 App 扫一扫", theme.text_secondary),
            }
        };

        let qr: AnyElement = if url.is_empty() {
            div()
                .size(px(104.))
                .flex_shrink_0()
                .rounded(px(6.))
                .bg(theme.bg_hover)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.))
                .text_color(theme.text_tertiary)
                .child(if loading { "生成中…" } else { "点刷新重试" })
                .into_any_element()
        } else {
            crate::actions::qr_widget(url.clone(), 104., theme.text).into_any_element()
        };

        let side = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child("扫码登录"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(color)
                    .child(tip.to_string()),
            )
            .when(!err.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(10.))
                        .text_color(hsla(0xef4444))
                        .child(err.clone()),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .mt(px(2.))
                    .child({
                        let st = self.store.clone();
                        div()
                            .id("lite-qr-refresh")
                            .h(px(20.))
                            .px(px(8.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .border_1()
                            .border_color(theme.border)
                            .hover({
                                let bg = theme.bg_hover;
                                move |el| el.bg(bg)
                            })
                            .on_click(move |_, _, cx| crate::actions::start_qr_login(&st, cx))
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(theme.text_secondary)
                                    .child("刷新"),
                            )
                    })
                    .child(
                        div()
                            .id("lite-qr-cancel")
                            .h(px(20.))
                            .px(px(8.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .border_1()
                            .border_color(theme.border)
                            .hover({
                                let bg = theme.bg_hover;
                                move |el| el.bg(bg)
                            })
                            .on_click(cx.listener(|v: &mut LiteView, _, window, cx| {
                                v.set_panel(Panel::Account, window, cx);
                            }))
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(theme.text_secondary)
                                    .child(if logged { "完成" } else { "取消" }),
                            ),
                    ),
            );

        div()
            .id("lite-panel-login")
            .w_full()
            .h(px(PANEL_LOGIN_H))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .px(px(4.))
            .pb(px(6.))
            .child(Self::head("账号", if logged { "已登录" } else { "未登录" }, "", &theme))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(10.))
                    .child(qr)
                    .child(side),
            )
            .into_any_element()
    }
}

/// unix 秒 → `yyyy-mm-dd`（会员到期日只用来展示，简单的民用历算法即可）。
fn fmt_date(unix: i64) -> String {
    if unix <= 0 {
        return "-".into();
    }
    let (y, m, d) = civil_from_days(unix / 86400);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Howard Hinnant 的 `civil_from_days`。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
