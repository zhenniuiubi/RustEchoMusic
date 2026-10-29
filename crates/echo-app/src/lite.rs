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
/// 封面圆角（原型 rounded-md = 6px）
const COVER_R: f32 = 6.;
/// 封面左边距（相对左栏）。
const COVER_ML: f32 = 4.;
const META_ML: f32 = 6.;
/// 文字与进度条的左端（相对窗口左边）。
const CONTENT_LEFT: f32 = PAD_L + RAIL_W + COVER_ML + COVER + META_ML;
/// 进度条距窗口底边、自身高度。
const PROG_BOTTOM: f32 = 10.;
const PROG_H: f32 = 9.;
/// 进度槽上下多出来的点击热区（canvas 因此比槽高 8px，绘制时中线要减掉 `PROG_TAIL`）。
const PROG_HEAD: f32 = 6.;
const PROG_TAIL: f32 = 2.;
/// 进度条宽度。
const PROG_W: f32 = LITE_W - CONTENT_LEFT - PAD_R;
/// 右侧图标：按钮边长（22 的可点区 + 两侧各 2 的留白 = 视觉间距 4）/ 整组宽度 / 整组左端。
///
/// 间距不用 `gap` / `margin`（在 gpui 0.2.2 的这个盒子里实测都不生效，三块会贴成一条），
/// 直接把留白算进按钮自身宽度 —— 几何就完全可预测了。
const ACT_W: f32 = 26.;
const ACTIONS_W: f32 = ACT_W * 3.;
const ACTIONS_LEFT: f32 = LITE_W - PAD_R - ACTIONS_W;
/// 封面左上角（hover 判定 + 鼠标命中）。
const COVER_X: f32 = PAD_L + RAIL_W + COVER_ML;
const COVER_Y: f32 = (LITE_H - COVER) / 2.;

/// 波浪周期 / 流速：原型是 26px 的波浪图块，1.6s 平移一格 → 16.25 px/s，
/// 方向朝右（`background-position-x: 0 → 26px`）。
const WAVE_PERIOD: f32 = 26.;
const WAVE_SPEED: f32 = WAVE_PERIOD / 1.6;
/// 波浪振幅：原型路径 `M0 4.5 Q6.5 1.5 13 4.5` 的峰谷是 ±1.5px。
const WAVE_AMP: f32 = 1.5;
/// 波浪线宽（原型 1.5px 描边）。
const WAVE_THICK: f32 = 1.5;

/// 彩虹猫：条纹带的阶梯块宽（px）与向左流动的速度（px/s）。
/// 参考 GIF 是 8 帧 × 50ms 循环，一格阶梯 ≈ 2px/帧 ≈ 40px/s；这里放慢一点。
const NYAN_BLOCK: f32 = 6.;
const NYAN_SPEED: f32 = 26.;

/// 列表滚动条的可拖拽热区宽度（只占右侧一条，不盖住列表行）。
const SCROLL_STRIP: f32 = 11.;

/// 播放控制那一组的宽度：prev 24 + gap 4 + play 30 + gap 4 + next 24 + gap 4
/// + 分隔条 3 + gap 4 + mode 22 = 119。
///
/// hover 判定用它：鼠标停在歌名/歌手文字上才浮出控制键，浮出之后要保证
/// 指针移到控制键上时不会「判断为离开」而闪回去。
const CTL_W: f32 = 120.;

/// 面板展开 / 收起动画时长（原型 height .42s / 收起 .26s，这里取中间值）。
const PANEL_ANIM_SECS: f32 = 0.26;

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

/// 面板展开 / 收起动画：只动窗口高度，内容被窗口裁掉自然「长出来」。
#[derive(Debug, Clone, Copy)]
struct PanelAnim {
    from: f32,
    to: f32,
    start: Instant,
    /// 动画结束后要显示的 panel（收起时先留着旧内容，等高度归零再撤）
    target: Panel,
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
    /// 我最喜爱列表滚动句柄
    fav_scroll: gpui::UniformListScrollHandle,
    /// 滚动条拖拽中：(被拖的那条列表句柄, 按下时的 `(鼠标 y, 滚动偏移)`)
    list_drag: Option<(gpui::UniformListScrollHandle, (f32, f32))>,
    /// 歌名 / 歌手文字块的实际绘制矩形 `(x, y, w, h)`（窗口逻辑坐标）。
    /// hover 只认这一块 —— 鼠标落在同一行右侧的空白上不算「移到歌名上」。
    meta_rect: Arc<Mutex<Option<(f32, f32, f32, f32)>>>,
    /// 进度条拖拽中
    seeking: bool,
    /// 波浪相位（px，0..WAVE_PERIOD）：播放时才前进，暂停即冻结
    phase: f32,
    /// 上一次积分相位 / 面板动画的时刻
    tick_at: Instant,
    /// 窗口当前高度（面板动画的起点，也是 resize 的唯一来源）
    cur_h: f32,
    /// 面板开合动画（None = 已到位）
    anim: Option<PanelAnim>,
}

static LITE_WIN: OnceLock<Mutex<Option<WeakEntity<LiteView>>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<WeakEntity<LiteView>>> {
    LITE_WIN.get_or_init(|| Mutex::new(None))
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
            cx.new(|cx| {
                // 设置 / 会话 / 播放状态全在 store 里：订阅它，任何变更都立刻重绘。
                // 少了这一步，「进度条形态」这类只写 store 的操作要等下一次别的刷新才生效。
                cx.observe(&store_c, |_, _, cx| cx.notify()).detach();
                LiteView {
                    store: store_c.clone(),
                    closed: Arc::new(AtomicBool::new(false)),
                    focus: Arc::new(AtomicBool::new(false)),
                    hwnd_raw: 0,
                    hovered: false,
                    hover_cover: false,
                    panel: Panel::None,
                    fav_mode: false,
                    press_at: None,
                    queue_scroll: gpui::UniformListScrollHandle::new(),
                    fav_scroll: gpui::UniformListScrollHandle::new(),
                    list_drag: None,
                    meta_rect: Arc::new(Mutex::new(None)),
                    seeking: false,
                    phase: 0.,
                    tick_at: Instant::now(),
                    cur_h: LITE_H,
                    anim: None,
                }
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
                spawn_anim_pump(handle, cx);
                spawn_smoke_panels(handle, cx);
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

/// 自检钩子：`ECHO_SMOKE=panel` 时依次展开各下方面板并打印窗口尺寸。
///
/// 远程会话 / CI 里鼠标事件不可用（`GetCursorPos` 会被拒），靠它回归面板高度、
/// 列表渲染与窗口 resize 这条链路；正常启动时这个任务直接返回。
fn spawn_smoke_panels(handle: WindowHandle<LiteView>, cx: &mut App) {
    if std::env::var("ECHO_SMOKE").ok().as_deref() != Some("panel") {
        return;
    }
    cx.spawn(async move |cx: &mut AsyncApp| {
        for (name, panel) in [
            ("queue", Panel::Queue),
            ("lyric", Panel::Lyric),
            ("account", Panel::Account),
            ("none", Panel::None),
        ] {
            smol::Timer::after(std::time::Duration::from_millis(1500)).await;
            let _ = handle.update(cx, |v: &mut LiteView, window, cx| {
                v.set_panel(panel, window, cx);
                let b = window.bounds();
                log::warn!(
                    "[smoke-panel] {name} -> {}x{}",
                    f32::from(b.size.width),
                    f32::from(b.size.height)
                );
            });
        }
    })
    .detach();
}

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
                    // 拖滚动条时在窗口外松手：鼠标抬起事件收不到，这里收尾
                    if v.list_drag.is_some() && !crate::win::left_button_down() {
                        v.list_drag = None;
                        cx.notify();
                    }
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

/// 波浪 / 彩虹猫要连续动，面板开合也要逐帧推高度，所以需要自己的帧泵
/// （主循环的刷新跟着播放事件走，暂停时就没有了）。
///
/// 只有「面板在动」时才跑 60fps，波浪/彩虹猫 30fps，其余降到 5fps 省 CPU。
fn spawn_anim_pump(handle: WindowHandle<LiteView>, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        let pace = handle
            .update(cx, |v, window, cx| {
                let animating = v.tick(window, cx);
                if !animating {
                    let s = v.store.read(cx);
                    let bars = Bars::from_settings(&s.settings.get().lite_bar);
                    if bars.animated() || s.player.playing {
                        cx.notify();
                        33
                    } else {
                        200
                    }
                } else {
                    12
                }
            })
            .unwrap_or(0);
        if pace == 0 {
            // 窗口已经没了
            break;
        }
        cx.background_executor()
            .timer(std::time::Duration::from_millis(pace))
            .await;
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
    /// 光标是否落在「歌名 / 歌手」文字上。
    ///
    /// 矩形是文字块真正画出来的范围（`meta_rect`，绘制时记录），不是整条中间区域 ——
    /// 鼠标停在同一行右侧的空白上不该把歌名换成控制键。
    ///
    /// 控制键浮出后按 `CTL_W` 向右放宽，否则指针刚移到「下一首」上就被判成离开，
    /// 会一阵阵闪回歌名。
    fn cursor_on_meta(&self) -> bool {
        let saved = self.meta_rect.lock().ok().and_then(|v| *v);
        let (x, y, w, h) = saved.unwrap_or((
            CONTENT_LEFT,
            (LITE_H - 36.) / 2.,
            ACTIONS_LEFT - CONTENT_LEFT - 8.,
            36.,
        ));
        let w = if self.hovered { w.max(CTL_W) } else { w };
        crate::win::cursor_in_client_rect(self.hwnd_raw, LITE_W, x - 2., y - 2., w + 4., h + 4.)
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

    /// 推进波浪相位：只在播放时前进，暂停即冻结（对齐原型 `animation-play-state:paused`）。
    fn advance_phase(&mut self, playing: bool) {
        let now = Instant::now();
        let dt = now.duration_since(self.tick_at).as_secs_f32().min(0.20);
        self.tick_at = now;
        if playing {
            self.phase = (self.phase + dt * WAVE_SPEED) % WAVE_PERIOD;
        }
    }

    /// 逻辑面板 = 动画目标（动画途中按钮高亮 / 再次点击都按目标算）。
    fn shown_panel(&self) -> Panel {
        self.anim.map_or(self.panel, |a| a.target)
    }

    /// 面板高度。
    fn panel_height(&self, panel: Panel, cx: &App) -> f32 {
        match panel {
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

    /// 换面板：窗口高度做 0.26s 的缓出动画，内容由窗口裁切自然展开 / 收起。
    fn set_panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        let target_h = LITE_H + self.panel_height(panel, cx);
        // 展开：立刻换成新内容，让它跟着窗口一起长出来；
        // 收起：内容留到高度归零再撤，否则会先看到面板内容一空、再慢慢变矮。
        if panel != Panel::None {
            self.panel = panel;
        }
        if panel == Panel::Queue {
            if let Some(ix) = self.store.read(cx).player.index {
                self.queue_scroll
                    .scroll_to_item(ix, gpui::ScrollStrategy::Center);
            }
        }
        if (self.cur_h - target_h).abs() < 0.5 {
            // 高度没变（队列 ↔ 收藏同高）：不用动画，直接到位
            self.cur_h = target_h;
            if panel == Panel::None {
                self.panel = Panel::None;
            }
            self.anim = None;
            window.resize(size(px(LITE_W), px(target_h)));
        } else {
            self.anim = Some(PanelAnim {
                from: self.cur_h,
                to: target_h,
                start: Instant::now(),
                target: panel,
            });
            // 动画一拍可能比帧间隔短（同高切换已排除），先推第一帧避免看到旧高度
            self.tick(window, cx);
        }
        cx.notify();
    }

    /// 推进面板高度动画，返回「是否还在动」。
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(a) = self.anim else {
            return false;
        };
        let t = (a.start.elapsed().as_secs_f32() / PANEL_ANIM_SECS).clamp(0., 1.);
        // 原型 cubic-bezier(.2,.9,.24,1)：起步快、收尾缓
        let e = 1. - (1. - t).powi(3);
        let h = if t >= 1. {
            a.to
        } else {
            a.from + (a.to - a.from) * e
        };
        self.cur_h = h;
        window.resize(size(px(LITE_W), px(h)));
        if t >= 1. {
            self.panel = a.target;
            self.anim = None;
            cx.notify();
            return false;
        }
        true
    }

    /// 点同一个按钮就收起。
    fn toggle_panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.shown_panel() == panel {
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
        // 任务栏缩略图工具栏的按钮状态（播放/暂停、红心）跟着走；内部去重，每帧调也安全
        crate::taskbar::sync(playing, fav);
        let bars = Bars::from_settings(&self.store.read(cx).settings.get().lite_bar);
        // 自检钩子：`ECHO_SMOKE=half` 把进度钉在 50%，用来截图比对三种进度条形态
        let ratio = if std::env::var("ECHO_SMOKE").ok().as_deref() == Some("half") {
            0.5
        } else if duration > 0.01 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        // 波浪相位只在播放时前进，暂停即冻结
        self.advance_phase(playing);
        let phase = self.phase;
        let phase_step = (phase / 6.0).floor() as i32;
        let hovered = self.hovered;
        let hover_cover = self.hover_cover;
        // panel = 正在显示的内容（收起动画期间仍是旧面板）
        // shown = 逻辑面板（按钮高亮与互斥判断按它算）
        let panel = self.panel;
        let shown = self.shown_panel();
        let panel_open = panel != Panel::None || self.anim.is_some();
        let store = self.store.clone();

        // ---------- 左栏：关闭（退出应用）+ 头像 ----------
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
            .on_click(cx.listener(|v: &mut LiteView, _, _, cx| {
                // 没有托盘了：关闭即退出（面板高度不必收，进程马上就没了）
                v.closed.store(true, Ordering::Relaxed);
                cx.quit();
            }))
            .child(crate::ui::icon("icons/close.svg", 13., theme.text_secondary));

        let avatar_btn = {
            let active = shown == Panel::Account || shown == Panel::Login;
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
                            .right(px(0.5))
                            // 贴住用户图标的笔画底边（按钮 24 高，人像笔画到 ~22）
                            .bottom(px(1.5))
                            .size(px(7.5))
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
            // 关闭键顶边贴封面顶边、头像底边贴封面底边：18 + 8 + 24 = 50 = 封面高。
            // 间隔用固定高的小方片而不是 `gap`（这个盒子里 gap 不生效）
            .pb(px((LITE_H - COVER) / 2.))
            .occlude()
            .child(close_btn)
            .child(div().w(px(1.)).h(px(8.)).flex_shrink_0())
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
                // 灰色投影（原型 box-shadow: 0 1px 3px rgba(62,72,88,.28)，
                // 这里加重一点，白底上要看得出来）
                .shadow(vec![BoxShadow {
                    color: alpha(0x3e4858, 0.42),
                    offset: point(px(0.), px(1.5)),
                    blur_radius: px(5.),
                    spread_radius: px(0.),
                }])
                // 封面自成一块可点区域：不 occlude 的话按下会落到下面的拖动层，
                // 系统移动循环会吃掉 mouse-up，点击就永远不触发。
                .occlude()
                // 点封面 = 播放/暂停；按住拖动 = 移动窗口（位移超过 4px 就不再当点击）
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|v: &mut LiteView, ev: &MouseDownEvent, _, cx| {
                        v.press_at = Some(ev.position);
                        cx.notify();
                    }),
                )
                .on_mouse_move(cx.listener(
                    |v: &mut LiteView, ev: &MouseMoveEvent, window, _cx| {
                        let Some(p) = v.press_at else { return };
                        let moved = (f32::from(ev.position.x - p.x).powi(2)
                            + f32::from(ev.position.y - p.y).powi(2))
                        .sqrt();
                        if moved > 4. {
                            // 真的在拖：交给系统移动循环，别在松手时误判成点击
                            v.press_at = None;
                            crate::win::start_drag(window);
                        }
                    },
                ))
                .on_mouse_up(MouseButton::Left, {
                    let st = store.clone();
                    cx.listener(move |v: &mut LiteView, _ev, _, cx: &mut Context<LiteView>| {
                        // 没被「拖动」吃掉按下 → 就是一次点击：播放/暂停
                        if v.press_at.take().is_some() {
                            crate::ui::toggle_play(&st, cx);
                        }
                        cx.notify();
                    })
                })
                .when_some(cur, |el, c| {
                    el.when(!c.cover_url.is_empty(), |el| {
                        el.child(
                            // 圆角必须加在 Img 上：`overflow_hidden` 只按矩形裁剪
                            // （ContentMask 没有圆角），而 Img 会把 corner_radii 传给
                            // 圆角 sprite 着色器。
                            img(SharedUri::from(c.cover_url.clone()))
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                                .rounded(px(COVER_R)),
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
                if v.shown_panel() == Panel::Queue {
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
                // 宽度跟着文字走（最多铺满 meta）：hover 判定要的正是「歌名 / 歌手」这一块
                let rect = self.meta_rect.clone();
                el.child(
                    div()
                        .relative()
                        .min_w_0()
                        .max_w(px(PROG_W - 2.))
                        .flex()
                        .flex_col()
                        .justify_center()
                        .gap(px(1.))
                        // 记录这一块的真实矩形（逻辑坐标），hover 轮询按它判定
                        .child(
                            canvas(
                                move |b: Bounds<Pixels>, _, _| {
                                    if let Ok(mut g) = rect.lock() {
                                        *g = Some((
                                            f32::from(b.origin.x),
                                            f32::from(b.origin.y),
                                            f32::from(b.size.width),
                                            f32::from(b.size.height),
                                        ));
                                    }
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .left(px(0.))
                            .top(px(0.))
                            .right(px(0.))
                            .bottom(px(0.)),
                        )
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
            let open = shown == Panel::Queue;
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
            let open = shown == Panel::Lyric;
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
                // 上下各多出来的点击热区
                div()
                    .absolute()
                    .left(px(0.))
                    .right(px(0.))
                    .top(px(-PROG_HEAD))
                    .bottom(px(-PROG_TAIL))
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
                // GIF 里猫是两帧交替、上下颠 1px 的；相位走一格跳一次
                let bob = (phase_step.rem_euclid(2)) as f32 * 1.2;
                el.child(
                    img("lite/nyan-cat.png")
                        .absolute()
                        .left(px(x))
                        .top(px(-5.5 + bob))
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
            // 卡片外圈浅灰描边（gpui 的 border 只画不占位，几何常量不受影响）
            .border_1()
            .border_color(theme.card_border)
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
            // 面板整块 occlude：否则面板里的按钮按下会落到下面的拖动层（见封面注释）
            .when_some(body, |el, b| {
                el.child(
                    div()
                        .id("lite-panel-host")
                        .w_full()
                        .flex_shrink_0()
                        .occlude()
                        .child(b),
                )
            })
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
    // canvas 比进度槽上下各多出几像素做点击热区（下沿 -2），
    // 真正的中线要按「槽」算，不然会比封面底边的对齐线偏高 2px。
    let center = h - PROG_TAIL - PROG_H / 2.0;
    match bars {
        Bars::Plain => {
            let bar_h = 4.0;
            let top = y0 + px(center - bar_h / 2.0);
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
            // 正弦波：周期 26、振幅 ±1.5、线宽 1.5（= 原型 SVG 路径 + 1.5px 描边）。
            //
            // gpui 没有描边路径，只能拿小方块拼曲线。这里按**列**画：每一列取左右
            // 边界处的正弦值，把这一列的竖直包络（|Δy| + 线宽）一次填满 —— 相邻列
            // 首尾相接，既没有缝也没有 0.8px 小方块错位造成的锯齿。
            let fill_w = w * ratio;
            let y_at = |x: f32| {
                center - WAVE_AMP * (((x - phase) / WAVE_PERIOD) * std::f32::consts::TAU).sin()
            };
            // 步长 0.5：纵向台阶从 1px 降到 0.5px，边缘交给渲染器的 AA 混合，
            // 在 330×70 这种小尺寸下才不至于看着像「方波」。
            let mut x = 0.0;
            while x < w {
                let xr = (x + 0.5).min(w);
                let ya = y_at(x);
                let yb = y_at(xr);
                let top = ya.min(yb) - WAVE_THICK / 2.0;
                let bot = ya.max(yb) + WAVE_THICK / 2.0;
                let mid = (x + xr) / 2.0;
                let mut color = if mid < fill_w {
                    theme.accent
                } else {
                    theme.progress_track
                };
                if mid < fill_w {
                    // 末端 12px 柔化（原型用 mask 渐变收口）
                    let fade = fill_w - 12.0;
                    if mid > fade {
                        let t = ((mid - fade) / 12.0).clamp(0.0, 1.0);
                        color = color.opacity(1.0 - t * 0.85);
                    }
                }
                window.paint_quad(
                    fill(
                        Bounds::new(
                            point(x0 + px(x), y0 + px(top)),
                            size(px(xr - x + 0.30), px(bot - top)),
                        ),
                        color,
                    )
                    // 带一点圆角才会走 SDF 抗锯齿分支：纯直角 quad 是硬边，
                    // 0.5px 的纵向差会原样变成一级级台阶（这就是「锯齿」的来源）。
                    .corner_radii(px(0.6)),
                );
                x += 0.5;
            }
        }
        Bars::Nyan => {
            // 参考 GIF：6 条横带（红橙黄绿蓝紫）拼成的彩虹尾巴跟在像素猫后面。
            // 每一帧整条带子的「阶梯」向左挪一点 —— 也就是条纹边界按列错位半像素，
            // 错位方向每 `NYAN_BLOCK` 换一次，看着就是彩虹在流动。
            const RAINBOW: [u32; 6] =
                [0xFF0000, 0xFFCB00, 0xFEFF00, 0x00AD00, 0x008BFF, 0x8D24FF];
            const STRIPE: f32 = 1.5;
            let band_top = center - STRIPE * 3.0;
            let fill_w = (w * ratio).clamp(0.0, w);
            // 未播放部分：一条淡灰细线（跟波浪形态一个体量）。
            // 别用整条 9px 的灰带 —— 在小播放条里太重，像加载进度条。
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(x0, y0 + px(center - 1.5)),
                        size(px(w), px(3.)),
                    ),
                    theme.progress_track,
                )
                .corner_radii(Corners::all(px(1.5))),
            );
            if fill_w < 0.5 {
                return;
            }
            // phase 是按波浪速度积分的（px），换成彩虹的速度
            let np = phase * (NYAN_SPEED / WAVE_SPEED);
            // 6 条带子 = 7 条分界。首尾固定（带子上/下沿是直线），
            // 中间 5 条按块错位 ±0.6 → 分界形成一级级台阶，正是 GIF 里那种像素彩虹。
            let mut bounds = [0.0f32; 7];
            let mut x = 0.0;
            while x < fill_w {
                let xr = (x + 1.0).min(fill_w);
                let blk = ((x + np) / NYAN_BLOCK).floor() as i32;
                bounds[0] = band_top;
                bounds[6] = band_top + STRIPE * 6.;
                // 一整块内所有分界同向错半像素；块与块之间上下交替 —— 看着就是
                // 一条整体在「一级级往下/往上」走的像素彩虹。逐条纹各错各的会乱成一团。
                let d = if blk.rem_euclid(2) == 0 { -0.5 } else { 0.5 };
                for i in 1..6 {
                    bounds[i] = band_top + STRIPE * i as f32 + d;
                }
                for (i, c) in RAINBOW.iter().enumerate() {
                    let top = bounds[i];
                    let bot = bounds[i + 1].max(top + 0.9);
                    window.paint_quad(fill(
                        Bounds::new(
                            point(x0 + px(x), y0 + px(top)),
                            size(px(xr - x + 0.35), px(bot - top + 0.1)),
                        ),
                        hsla(*c),
                    ));
                }
                x += 1.0;
            }
        }
    }
}

// ==================== 面板 ====================

/// 给面板里的列表套一层「虚拟化列表 + 自绘滚动条」。
///
/// 滚动条热区只有右侧 11px 一条 —— 之前铺满整块面板，鼠标按下的都被它吃掉，
/// 行的点击根本到不了（`stop_propagation` 之后 list 收不到事件）。
/// 拖拽起点记进 `list_drag`，后续位移由本容器的 `on_mouse_move` 1:1 应用
/// （uniform_list 的内容高度是精确的，拇指和鼠标完全跟手）。
fn scroll_wrap(
    list: AnyElement,
    handle: gpui::UniformListScrollHandle,
    id: &'static str,
    cx: &mut Context<LiteView>,
) -> AnyElement {
    let entity = cx.entity();
    let down_handle = handle.clone();
    let overlay = div()
        .id(SharedString::from(format!("{id}-strip")))
        .absolute()
        .top(px(0.))
        .bottom(px(0.))
        .right(px(0.))
        .w(px(SCROLL_STRIP))
        .child(crate::ui::uniform_scrollbar_overlay(
            id,
            handle,
            move |cx, start| {
                let h = down_handle.clone();
                let _ = entity.update(cx, |v: &mut LiteView, _| {
                    v.list_drag = Some((h.clone(), start));
                });
            },
        ));
    div()
        .flex_1()
        .min_h_0()
        .relative()
        .flex()
        .flex_col()
        .pt(px(4.))
        .on_mouse_move(cx.listener(|v: &mut LiteView, ev: &MouseMoveEvent, _, cx| {
            if let Some((h, start)) = v.list_drag.clone() {
                crate::ui::apply_scrollbar_drag(&h, start, f32::from(ev.position.y));
                cx.notify();
            }
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(|v: &mut LiteView, _, _, cx| {
            if v.list_drag.take().is_some() {
                cx.notify();
            }
        }))
        .child(list)
        .child(overlay)
        .into_any_element()
}

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

    /// 「我最喜爱」的刷新：登录用户重拉云端红心歌单，未登录重读本地 library.json。
    fn refresh_favorites(store: &Entity<AppStore>, cx: &mut App) {
        let logged = store.read(cx).session.get().userid != 0;
        if logged {
            crate::liked::load_remote_liked(store, true, cx);
            return;
        }
        let fresh = echo_core::LibraryStore::load(&echo_core::data_dir());
        store.update(cx, |s, cx| {
            if let Ok(lib) = fresh {
                s.library = lib;
            }
            s.toast = "已刷新本地收藏".into();
            cx.notify();
        });
    }

    /// 队列 / 我最喜爱 共用的一行（四列：序号 / 歌名 / 歌手 / 时长）。
    ///
    /// `from_favorites` 决定点击行为：队列里点行 = 在当前队列内跳转（`play_index`），
    /// 收藏里点行 = 先把队列换成收藏列表再播（否则会播到队列里同下标的另一首）。
    fn list_row(
        store: &Entity<AppStore>,
        ix: usize,
        song: &echo_core::Song,
        active: bool,
        from_favorites: bool,
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
                if from_favorites {
                    crate::ui::play_from(&st, crate::ui::PlaySource::Favorites, ix, cx);
                } else {
                    crate::ui::play_index(&st, ix, cx);
                }
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
    ///
    /// 间距一律用 `ml` 而不是 `gap`：这一行里有 `ml_auto`（计数/按钮靠右），
    /// gpui 0.2.2 里同组 flex 一旦有 auto margin，`gap` 就整排失效。
    fn head(title: &str, sub: &str, count: &str, theme: &crate::theme::Theme) -> Div {
        let mut el = div()
            .h(px(34.))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
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
                        .ml(px(5.))
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(theme.text_tertiary)
                        .child("·"),
                )
                .child(
                    div()
                        .ml(px(5.))
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
        let total = {
            let s = self.store.read(cx);
            if self.fav_mode {
                s.favorites_source().len()
            } else {
                s.player.queue.len()
            }
        };
        let handle = self.queue_scroll.clone();
        let store = self.store.clone();
        let fav_mode = self.fav_mode;

        let list = if total == 0 {
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
            // `uniform_list` 是虚拟化的：只构建可见那几行，滚动条按「行数 × 行高」
            // 精确算内容高度。歌按索引现取，不整表克隆。
            uniform_list("lite-queue-list", total, move |range, _w, cx: &mut App| {
                let theme = rows_store.read(cx).theme();
                let (active_ix, cur_key, fav_rows) = {
                    let st = rows_store.read(cx);
                    (
                        st.player.index,
                        st.player.current().map(|c| c.key()),
                        if fav_mode {
                            Some(st.favorites_source())
                        } else {
                            None
                        },
                    )
                };
                let st = rows_store.read(cx);
                range
                    .map(|ix| {
                        let song = match &fav_rows {
                            Some(rows) => rows.get(ix).cloned(),
                            None => st.player.queue.get(ix).cloned(),
                        };
                        match song {
                            Some(song) => {
                                let active = if fav_mode {
                                    cur_key.as_deref() == Some(song.key().as_str())
                                } else {
                                    active_ix == Some(ix)
                                };
                                Self::list_row(&rows_store, ix, &song, active, fav_mode, &theme)
                            }
                            None => div().h(px(ROW_H)).into_any_element(),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .track_scroll(handle.clone())
            .size_full()
            .into_any_element()
        };

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
            let mut h = Self::head("播放队列", if fav_mode { "收藏" } else { "FM" }, "", &theme);
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
            .child(scroll_wrap(list, handle, "lite-queue-sb", cx))
            .into_any_element()
    }

    fn fav_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.store.read(cx).theme();
        let (total, loading) = {
            let s = self.store.read(cx);
            (s.favorites_source().len(), s.remote_liked_loading)
        };
        let store = self.store.clone();
        let count = total;

        let head = div()
            .h(px(34.))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
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
                    .ml(px(6.))
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(theme.text_secondary)
                    .child("我最喜爱"),
            )
            .child(
                div()
                    .ml(px(6.))
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(theme.text_tertiary)
                    .child(format!("{total} 首")),
            )
            // 数量右边的刷新（灰色小按钮）：登录用户重拉云端红心，未登录重读本地
            .child({
                let st = store.clone();
                let tint = if loading {
                    theme.text_tertiary.opacity(0.45)
                } else {
                    theme.text_secondary
                };
                div()
                    .id("lite-fav-refresh")
                    .ml(px(3.))
                    .size(px(18.))
                    .rounded(px(4.))
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover({
                        let bg = theme.bg_hover;
                        move |el| el.bg(bg)
                    })
                    .on_click(move |_, _, cx| Self::refresh_favorites(&st, cx))
                    .child(crate::ui::icon("icons/refresh.svg", 12., tint))
            })
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
                        .on_click(move |_, _, cx| {
                            crate::ui::play_from(&st, crate::ui::PlaySource::Favorites, 0, cx)
                        })
                        .child(crate::ui::icon("icons/mini-play.svg", 10., hsla(0xffffff)))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(hsla(0xffffff))
                                .child("播放全部"),
                        ),
                )
            });

        let handle = self.fav_scroll.clone();
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
                let (cur_key, rows) = {
                    let st = rows_store.read(cx);
                    (st.player.current().map(|c| c.key()), st.favorites_source())
                };
                range
                    .map(|ix| match rows.get(ix) {
                        Some(song) => {
                            let active = cur_key.as_deref() == Some(song.key().as_str());
                            Self::list_row(&rows_store, ix, song, active, true, &theme)
                        }
                        None => div().h(px(ROW_H)).into_any_element(),
                    })
                    .collect::<Vec<_>>()
            })
            .track_scroll(handle.clone())
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
            .child(scroll_wrap(list, handle, "lite-fav-sb", cx))
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
                s.favorites_source().len(),
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
                    .ml(px(8.))
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
