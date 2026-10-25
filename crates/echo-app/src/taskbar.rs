//! 任务栏缩略图工具栏：鼠标悬浮任务栏按钮（或 Alt+Tab 预览卡片）时，
//! 缩略图下面出现「上一曲 / 播放暂停 / 下一曲 / 收藏」四个按钮。
//!
//! 三个关键点：
//! 1. 系统要的是 **HICON**，SVG 用不了。这里直接在内存里画 32×32 白色像素
//!    （超采样抗锯齿），再走 `win::rgba_to_icon` 转成 HICON。
//! 2. 按钮点击不会回调 Rust，而是给窗口发 `WM_COMMAND`（HIWORD = `THBN_CLICKED`）。
//!    gpui 的 WndProc 不处理它，所以用 `SetWindowSubclass` 挂一层子类化自己拦，
//!    拦到后交给 `DefSubclassProc` 继续走 gpui 的处理。
//! 3. 拦到之后经 mpsc 回主线程，和托盘菜单 / 媒体键走同一套动作
//!    （`ui::prev_track` / `toggle_play` / `next_track` / `liked::toggle_like`）。

use crate::win;
use std::sync::atomic::{AtomicIsize, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, SetWindowSubclass, THB_FLAGS, THB_ICON, THB_TOOLTIP, THBN_CLICKED,
    THUMBBUTTON, THBF_ENABLED, TaskbarList,
};
use windows::Win32::UI::WindowsAndMessaging::HICON;

/// 任务栏按钮点击（回主线程执行）。
#[derive(Clone, Copy, Debug)]
pub enum TaskbarEvent {
    Prev,
    TogglePlay,
    Next,
    ToggleFav,
}

const WM_COMMAND: u32 = 0x0111;
const BTN_PREV: u32 = 1;
const BTN_PLAY: u32 = 2;
const BTN_NEXT: u32 = 3;
const BTN_FAV: u32 = 4;
/// 图标像素边长（系统会按 DPI 缩放到 16 逻辑像素）
const ICON: i32 = 32;

static CLICK_TX: OnceLock<Mutex<Sender<TaskbarEvent>>> = OnceLock::new();
static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
/// 上一次同步的 (playing | liked<<1)：只在变化时才去改按钮，避免每帧碰 COM
static LAST_STATE: AtomicU8 = AtomicU8::new(0xFF);

/// `ITaskbarList3` 不是 Send/Sync，但只在 UI 线程用。
struct Taskbar(ITaskbarList3);
unsafe impl Send for Taskbar {}
unsafe impl Sync for Taskbar {}

static TASKBAR: OnceLock<Mutex<Option<Taskbar>>> = OnceLock::new();

// ==================== 初始化 ====================

/// 给主窗口挂上缩略图工具栏。`hwnd_raw` 是主窗口 HWND（0 = 还没有窗口，直接跳过）。
pub fn init(hwnd_raw: isize, tx: Sender<TaskbarEvent>) {
    if hwnd_raw == 0 {
        return;
    }
    let _ = CLICK_TX.set(Mutex::new(tx));
    HWND_RAW.store(hwnd_raw, Ordering::Relaxed);

    unsafe {
        // gpui / WinRT 可能已经初始化过 COM（MTA），这里失败也无所谓
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let list: ITaskbarList3 = match CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) {
            Ok(l) => l,
            Err(e) => {
                log::warn!("任务栏工具栏：CoCreateInstance 失败 {e:?}");
                return;
            }
        };
        if let Err(e) = list.HrInit() {
            log::warn!("任务栏工具栏：HrInit 失败 {e:?}");
            return;
        }

        let hwnd = HWND(hwnd_raw as *mut _);
        let buttons = [
            button(BTN_PREV, icon_prev(), "上一曲"),
            button(BTN_PLAY, icon_play(), "播放 / 暂停"),
            button(BTN_NEXT, icon_next(), "下一曲"),
            button(BTN_FAV, icon_heart(false), "收藏"),
        ];
        if let Err(e) = list.ThumbBarAddButtons(hwnd, &buttons) {
            log::warn!("任务栏工具栏：ThumbBarAddButtons 失败 {e:?}");
            return;
        }
        // 图标画失败会得到空 HICON，工具栏就成了几个空白方块，这里留个记号
        if buttons.iter().any(|b| b.hIcon.is_invalid()) {
            log::warn!("任务栏工具栏：有按钮图标生成失败（HICON 为空）");
        }
        let slot = TASKBAR.get_or_init(|| Mutex::new(None));
        if let Ok(mut g) = slot.lock() {
            *g = Some(Taskbar(list));
        }
        // 拦 WM_COMMAND（THBN_CLICKED）
        if SetWindowSubclass(hwnd, Some(subclass_proc), 0xEC40, 0).0 == 0 {
            log::warn!("任务栏工具栏：SetWindowSubclass 失败");
        }
        log::info!("任务栏缩略图工具栏已就绪（上一曲/播放暂停/下一曲/收藏）");
    }
}

/// 播放状态 / 收藏状态变化时刷新按钮图标（内部做了去重，可以每帧调）。
pub fn sync(playing: bool, liked: bool) {
    let state = u8::from(playing) | (u8::from(liked) << 1);
    // 工具栏还没挂上时不要记账，否则首次状态会被吞掉（后面同样的状态就不会再刷）
    let raw = HWND_RAW.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    if LAST_STATE.load(Ordering::Relaxed) == state {
        return;
    }
    LAST_STATE.store(state, Ordering::Relaxed);
    let list = TASKBAR
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|g| g.as_ref().map(|t| t.0.clone()));
    let Some(list) = list else { return };

    let buttons = [
        button(
            BTN_PLAY,
            if playing { icon_pause() } else { icon_play() },
            if playing { "暂停" } else { "播放" },
        ),
        button(
            BTN_FAV,
            icon_heart(liked),
            if liked { "取消收藏" } else { "收藏" },
        ),
    ];
    unsafe {
        if let Err(e) = list.ThumbBarUpdateButtons(HWND(raw as *mut _), &buttons) {
            log::warn!("任务栏工具栏：ThumbBarUpdateButtons 失败 {e:?}");
        }
    }
}

// ==================== 子类化拦点击 ====================

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_COMMAND {
        // WM_COMMAND：LOWORD(wParam) = 控件 id，HIWORD(wParam) = 通知码（缩略图按钮是 THBN_CLICKED）
        let w = wparam.0 as u32;
        let (id, code) = (w & 0xFFFF, (w >> 16) & 0xFFFF);
        if code == THBN_CLICKED {
            let ev = match id {
                BTN_PREV => Some(TaskbarEvent::Prev),
                BTN_PLAY => Some(TaskbarEvent::TogglePlay),
                BTN_NEXT => Some(TaskbarEvent::Next),
                BTN_FAV => Some(TaskbarEvent::ToggleFav),
                _ => None,
            };
            if let Some(ev) = ev {
                log::info!("任务栏按钮：{ev:?}");
                if let Some(tx) = CLICK_TX.get().and_then(|m| m.lock().ok()) {
                    let _ = tx.send(ev);
                }
            }
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

// ==================== 按钮 / 图标 ====================

fn button(id: u32, icon: HICON, tip: &str) -> THUMBBUTTON {
    let mut b = THUMBBUTTON {
        dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
        iId: id,
        iBitmap: 0,
        hIcon: icon,
        szTip: [0; 260],
        dwFlags: THBF_ENABLED,
    };
    for (i, u) in tip.encode_utf16().take(259).enumerate() {
        b.szTip[i] = u;
    }
    b
}

/// HICON 缓存槽（图标只有 4 种，创建一次即可；系统不会替我们释放）。
fn cached(slot: &'static AtomicIsize, make: impl FnOnce() -> HICON) -> HICON {
    let v = slot.load(Ordering::Relaxed);
    if v != 0 {
        return HICON(v as *mut _);
    }
    let icon = make();
    slot.store(icon.0 as isize, Ordering::Relaxed);
    icon
}

static I_PLAY: AtomicIsize = AtomicIsize::new(0);
static I_PAUSE: AtomicIsize = AtomicIsize::new(0);
static I_PREV: AtomicIsize = AtomicIsize::new(0);
static I_NEXT: AtomicIsize = AtomicIsize::new(0);
static I_HEART: AtomicIsize = AtomicIsize::new(0);
static I_HEART_ON: AtomicIsize = AtomicIsize::new(0);

fn icon_play() -> HICON {
    cached(&I_PLAY, || paint_icon(|x, y| tri((12., 7.), (12., 25.), (25., 16.), x, y)))
}
fn icon_pause() -> HICON {
    cached(&I_PAUSE, || {
        paint_icon(|x, y| rect(10., 7., 15., 25., x, y) || rect(18., 7., 23., 25., x, y))
    })
}
fn icon_prev() -> HICON {
    cached(&I_PREV, || {
        paint_icon(|x, y| {
            rect(7., 7., 11., 25., x, y) || tri((24., 7.), (24., 25.), (11., 16.), x, y)
        })
    })
}
fn icon_next() -> HICON {
    cached(&I_NEXT, || {
        paint_icon(|x, y| {
            rect(21., 7., 25., 25., x, y) || tri((8., 7.), (8., 25.), (21., 16.), x, y)
        })
    })
}
/// `filled = false` 画空心心形（未收藏），`true` 画实心（已收藏）。
fn icon_heart(filled: bool) -> HICON {
    let slot = if filled { &I_HEART_ON } else { &I_HEART };
    cached(slot, move || {
        paint_icon(move |x, y| {
            let inside = heart(x, y);
            if filled {
                inside
            } else {
                // 空心：外心减去向内缩一圈的心
                inside && !heart(16. + (x - 16.) * 0.62, 15.5 + (y - 15.5) * 0.62)
            }
        })
    })
}

fn heart(x: f32, y: f32) -> bool {
    circle(12.4, 12.8, 5.6, x, y)
        || circle(19.6, 12.8, 5.6, x, y)
        || tri((6.9, 13.6), (25.1, 13.6), (16., 26.4), x, y)
}

// ---- 极简光栅化：只用「点在不在形状里」的判定函数，4×4 超采样出 alpha ----

fn rect(x0: f32, y0: f32, x1: f32, y1: f32, x: f32, y: f32) -> bool {
    x >= x0 && x <= x1 && y >= y0 && y <= y1
}

fn circle(cx: f32, cy: f32, r: f32, x: f32, y: f32) -> bool {
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= r * r
}

fn tri(a: (f32, f32), b: (f32, f32), c: (f32, f32), x: f32, y: f32) -> bool {
    let cross = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    let p = (x, y);
    let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

fn paint_icon(inside: impl Fn(f32, f32) -> bool) -> HICON {
    const SS: i32 = 4;
    let mut rgba = vec![0u8; (ICON * ICON * 4) as usize];
    for y in 0..ICON {
        for x in 0..ICON {
            let mut hit = 0u32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let fy = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    if inside(fx, fy) {
                        hit += 1;
                    }
                }
            }
            if hit == 0 {
                continue;
            }
            let a = (hit * 255 / (SS * SS) as u32) as u8;
            let o = ((y * ICON + x) * 4) as usize;
            // 任务栏工具栏图标是单色的，统一白色 + alpha
            rgba[o] = 255;
            rgba[o + 1] = 255;
            rgba[o + 2] = 255;
            rgba[o + 3] = a;
        }
    }
    win::rgba_to_icon(ICON, ICON, &rgba)
        .map(|h| HICON(h as *mut _))
        .unwrap_or(HICON(std::ptr::null_mut()))
}
