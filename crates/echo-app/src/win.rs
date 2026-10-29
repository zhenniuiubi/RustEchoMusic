//! Windows 平台能力：窗口图标、拖动、置顶、媒体命令通道。
//!
//! gpui 0.2.2 没提供「设置窗口图标」「隐藏窗口」「还原最大化」这些 API
//! （它的 `platform_window.zoom()` 只会 `SW_MAXIMIZE`，从不 `SW_RESTORE`，
//! 所以点两次最大化按钮回不去），这里直接用 Win32 补上。

#![cfg(windows)]
// edition 2024 里 unsafe fn 的函数体默认是「安全」的，逐个包 unsafe {} 太啰嗦
#![allow(unsafe_op_in_unsafe_fn)]

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER,
    DIB_RGB_COLORS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture};

/// 取 gpui 窗口底层的 HWND。
pub fn hwnd(window: &Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get() as HWND),
        _ => None,
    }
}

// ==================== 图标 ====================

static ICON: OnceLock<isize> = OnceLock::new();

/// 应用图标（HICON），首次调用时由内嵌 PNG 生成，失败返回 None。
pub fn app_icon() -> Option<HICON> {
    let raw = *ICON.get_or_init(|| {
        create_icon(include_bytes!("../assets/icon.png"))
            .map(|h| h as isize)
            .unwrap_or(0)
    });
    if raw == 0 {
        None
    } else {
        Some(raw as HICON)
    }
}

/// PNG → HICON：32bpp 顶朝下 DIB + 全 0 的 AND 掩码（靠 alpha 做透明）。
fn create_icon(png: &[u8]) -> Option<HICON> {
    let img = image::load_from_memory(png).ok()?.to_rgba8();
    rgba_to_icon(img.width() as i32, img.height() as i32, img.as_raw())
}

/// RGBA（非预乘，行序自上而下）→ HICON。
///
/// 任务栏缩略图按钮那种 32×32 小图标直接在内存里画像素，再走这条路变成 HICON。
pub fn rgba_to_icon(w: i32, h: i32, rgba: &[u8]) -> Option<HICON> {
    if w <= 0 || h <= 0 || rgba.len() < (w * h * 4) as usize {
        return None;
    }

    unsafe {
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // 负数 = 顶朝下，和 RGBA 缓冲区行序一致
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: DIB_RGB_COLORS as u32,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(
            std::ptr::null_mut(),
            &mut bmi,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if color.is_null() || bits.is_null() {
            return None;
        }

        // BGRA + 预乘 alpha（避免图标边缘出现白边）
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
        for i in 0..(w * h) as usize {
            let o = i * 4;
            let (r, g, b, a) = (
                rgba[o] as u16,
                rgba[o + 1] as u16,
                rgba[o + 2] as u16,
                rgba[o + 3] as u16,
            );
            dst[o] = ((b * a) / 255) as u8;
            dst[o + 1] = ((g * a) / 255) as u8;
            dst[o + 2] = ((r * a) / 255) as u8;
            dst[o + 3] = a as u8;
        }

        // 1bpp 掩码，每行按 16 位对齐
        let row_bytes = ((w as usize + 15) / 16) * 2;
        let mask_bits = vec![0u8; row_bytes * h as usize];
        let mask = CreateBitmap(w, h, 1, 1, mask_bits.as_ptr() as *const _);

        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);

        DeleteObject(color as _);
        DeleteObject(mask as _);
        if icon.is_null() {
            None
        } else {
            Some(icon)
        }
    }
}

/// 给窗口设置任务栏 / Alt-Tab 图标。
pub fn apply_window_icon(window: &Window) {
    let (Some(h), Some(icon)) = (hwnd(window), app_icon()) else {
        return;
    };
    unsafe {
        SendMessageW(h, WM_SETICON, ICON_BIG as WPARAM, icon as LPARAM);
        SendMessageW(h, WM_SETICON, ICON_SMALL as WPARAM, icon as LPARAM);
    }
    log::info!("窗口图标已设置（taskbar / alt-tab）");
}

// ==================== 窗口操作 ====================

/// 开始拖动窗口（等价于按住系统标题栏）。
///
/// 为什么不直接用 `window_control_area(Drag)`：
/// 1. gpui 的命中测试要拿「上一次鼠标移动算出来的 hitbox 列表」去匹配，点快一点 /
///    刚把鼠标从别的窗口移进来时匹配不到，就成了普通点击，于是顶部怎么按都拖不动；
/// 2. 命中成功时 gpui 会在 `WM_NCHITTEST` 里回 `HTCAPTION`，系统立刻起一个移动循环，
///    而我们在 `on_mouse_down` 里再补发一条 `WM_NCLBUTTONDOWN` 就会**嵌出第二个
///    移动循环**——两个循环同时吃鼠标消息，窗口一帧移动两次，表现为拖动严重掉帧。
/// 所以拖动区统一只用这一条路：给系统发一条「非客户区左键按下 + HTCAPTION」，
/// 走 Windows 自己的窗口移动循环（贴边吸附、跨屏拖动都是系统的原生行为）。
pub fn start_drag(window: &Window) {
    if let Some(h) = hwnd(window) {
        unsafe {
            // gpui 在鼠标按下时 `SetCapture` 了，不先释放的话系统的移动循环
            // 收不到后续鼠标事件（拖起来发飘 / 一顿一顿）。
            ReleaseCapture();
            // `SC_MOVE | HTCAPTION`：等价按住系统标题栏，让 DefWindowProc 进
            // Windows 自己的窗口移动循环（贴边吸附、跨屏拖动都是原生行为）。
            //
            // **不能用 `WM_NCLBUTTONDOWN | HTCAPTION`**：gpui 的窗口过程收到它
            // 会当成一次真实鼠标按下重新派发给 view，`ClickState::update` 把
            // `click_count` 累加成 2，于是「单击顶部」被判成「双击」→ 窗口最大化，
            // 系统移动循环也被 `ShowWindow` 打断（表现为顶部拖不动）。
            // `WM_SYSCOMMAND` 不是鼠标消息，不经过 input 回调，click_state 干净。
            // lParam 传 0：WPF `Window.DragMove()` 内部也是这么发的，系统用当前
            // 光标位置算偏移，不会跳窗。
            PostMessageW(h, WM_SYSCOMMAND, (SC_MOVE | HTCAPTION) as WPARAM, 0);
        }
        log::debug!("start_drag: post WM_SYSCOMMAND/SC_MOVE|HTCAPTION");
    }
}

/// 隐藏系统窗口边框（透明浮窗用：DWMWA_BORDER_COLOR = DWMWA_COLOR_NONE）。
pub fn hide_border(window: &Window) {
    if let Some(h) = hwnd(window) {
        // 属性 34 = DWMWA_BORDER_COLOR
        // **DWMWA_COLOR_NONE = 0xFFFFFFFE**（0xFFFFFFFF 是 DWMWA_COLOR_DEFAULT，
        // 等于「没设置」，这就是之前边框一直去不掉的原因）
        let none: u32 = 0xFFFF_FFFE;
        let hr = unsafe {
            DwmSetWindowAttribute(h, 34, &none as *const _ as *const _, 4)
        };
        if hr != 0 {
            log::debug!("隐藏窗口边框失败 (HRESULT=0x{hr:08x})");
        }
        // 属性 2 = DWMWA_NCRENDERING_POLICY，1 = DWMNCRP_DISABLED。
        // 关掉非客户区渲染：DWM 不再画那圈 1px 的窗口描边/阴影，
        // 桌面歌词在鼠标没移入时才是「纯文字浮在桌面上」。
        let policy: u32 = 1;
        let hr2 = unsafe {
            DwmSetWindowAttribute(h, 2, &policy as *const _ as *const _, 4)
        };
        if hr2 != 0 {
            log::debug!("关闭非客户区渲染失败 (HRESULT=0x{hr2:08x})");
        }
    }
}

/// 去掉窗口系统边框的**视觉**部分（桌面歌词这类透明浮窗用）。
///
/// 只改 DWM 的绘制属性，**不动 `WS_CAPTION/WS_THICKFRAME`**：
/// 摘窗口样式会让 gpui 拿不到客户区尺寸，窗口直接被压成 0×0 然后被
/// `remove_window` 掉（实测）。同理 **不能用 `DwmExtendFrameIntoClientArea`**，
/// 它会把窗口区/客户区搞成两套坐标，gpui 的 resize 一样会算错。
pub fn remove_window_frame(window: &Window) {
    if let Some(h) = hwnd(window) {
        // **不要摘 WS_CAPTION/WS_THICKFRAME**：gpui 靠客户区尺寸算布局，
        // 摘掉后窗口会被压成 0×0 然后 `remove_window` 掉（实测）。
        // DWM 那边的「无边框 + 无圆角」足够干净了。
        hide_border(window);
        // 33 = DWMWA_WINDOW_CORNER_PREFERENCE，1 = DWMWCP_DONOTROUND
        let round: u32 = 1;
        let hr = unsafe { DwmSetWindowAttribute(h, 33, &round as *const _ as *const _, 4) };
        if hr != 0 {
            log::debug!("关闭窗口圆角失败 (HRESULT=0x{hr:08x})");
        }
    }
}

/// 光标是否落在窗口**客户区**内的某个矩形里。
///
/// 参数用**逻辑像素**（相对客户区左上角），DPI 比例按 `logical_w` 反推：
/// 客户区实际宽度 / 逻辑宽度。mini 用它把「hover 区」收窄到中间那块
/// （鼠标停在收藏/队列/歌词上不该把歌名换掉）。
///
/// 注意必须用 `GetClientRect` + `ClientToScreen` 而不是 `GetWindowRect`：
/// 后者含窗口那圈 8px 的不可见边框，坐标会整体偏出去。
pub fn cursor_in_client_rect(
    hwnd_raw: isize,
    logical_w: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> bool {
    if hwnd_raw == 0 || logical_w <= 0.0 {
        return false;
    }
    let win = hwnd_raw as HWND;
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut pt) == 0 {
            return false;
        }
        let mut rc = RECT::default();
        if GetClientRect(win, &mut rc) == 0 {
            return false;
        }
        let mut origin = POINT { x: 0, y: 0 };
        if ClientToScreen(win, &mut origin) == 0 {
            return false;
        }
        let cw = (rc.right - rc.left) as f32;
        let ch = (rc.bottom - rc.top) as f32;
        if cw <= 1.0 || ch <= 1.0 {
            return false;
        }
        // 客户区宽度 / 逻辑宽度 = 缩放因子（横向纵向一致）
        let scale = cw / logical_w;
        let lx = (pt.x - origin.x) as f32;
        let ly = (pt.y - origin.y) as f32;
        lx >= x * scale
            && lx < (x + w) * scale
            && ly >= y * scale
            && ly < (y + h) * scale
    }
}

/// 当前是否处于置顶状态（浮窗的置顶按钮读它显示状态）。
pub fn is_topmost(window: &Window) -> bool {
    match hwnd(window) {
        Some(h) => unsafe { (GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST) != 0 },
        None => false,
    }
}

/// 窗口置顶 / 取消置顶（mini 播放器、桌面歌词浮窗用）。
pub fn set_topmost(window: &Window, on: bool) {
    if let Some(h) = hwnd(window) {
        unsafe {
            SetWindowPos(
                h,
                if on { HWND_TOPMOST } else { HWND_NOTOPMOST },
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        log::debug!("set_topmost({on})");
    }
}

// ==================== 媒体命令通道 ====================

/// 系统媒体控件（SMTC：任务栏悬浮卡片 / 蓝牙耳机按键）→ UI 线程的命令。
///
/// Lite 没有托盘了，这条通道只做「WinRT 线程 → UI 线程」的一次转发。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCmd {
    TogglePlay,
    Prev,
    Next,
}

static MEDIA_TX: OnceLock<Mutex<Sender<MediaCmd>>> = OnceLock::new();

/// 登记命令通道（UI 线程启动时调一次）。
pub fn init_media_cmds(tx: Sender<MediaCmd>) {
    let _ = MEDIA_TX.set(Mutex::new(tx));
}

/// 投递命令（SMTC 按键线程调用）。
pub(crate) fn send_cmd(cmd: MediaCmd) {
    if let Some(tx) = MEDIA_TX.get() {
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(cmd);
        }
    }
}

// ==================== 开机自启 ====================

// ==================== 全局媒体键 ====================

/// 虚拟键码（windows-sys 的常量在 Win32_UI_Input_KeyboardAndMouse 里，
/// 这里直接写值避免再开一个 feature）。
const VK_MEDIA_NEXT: i32 = 0xB0;
const VK_MEDIA_PREV: i32 = 0xB1;
const VK_MEDIA_STOP: i32 = 0xB2;
const VK_MEDIA_PLAY_PAUSE: i32 = 0xB3;

/// 左键当前是否按着（拖拽在窗口外松手时用来收尾）。
pub fn left_button_down() -> bool {
    // VK_LBUTTON = 0x01（windows-sys 里挂在 KeyboardAndMouse 下，这里直接写字面量）
    unsafe { (GetAsyncKeyState(0x01) as u16 & 0x8000) != 0 }
}

/// 上一次按键状态（用于「按下沿」判定）。
static KEY_STATE: std::sync::Mutex<[bool; 4]> = std::sync::Mutex::new([false; 4]);

/// 起一个后台线程监听系统媒体键（播放/暂停、上一首、下一首、停止）。
///
/// gpui 0.2.2 拿不到 WndProc，注册 `RegisterHotKey` 收不到 `WM_HOTKEY`，
/// 所以用 `GetAsyncKeyState` 轮询（30ms，开销可忽略）。
pub fn start_media_key_listener(on_key: std::sync::mpsc::Sender<MediaKey>) {
    const KEYS: [(i32, MediaKey); 4] = [
        (VK_MEDIA_PLAY_PAUSE, MediaKey::Toggle),
        (VK_MEDIA_NEXT, MediaKey::Next),
        (VK_MEDIA_PREV, MediaKey::Prev),
        (VK_MEDIA_STOP, MediaKey::Stop),
    ];
    let _ = std::thread::Builder::new()
        .name("media-keys".into())
        .spawn(move || unsafe {
            loop {
                if let Ok(mut state) = KEY_STATE.lock() {
                    for (i, (vk, key)) in KEYS.iter().enumerate() {
                        let down = (GetAsyncKeyState(*vk) as u16 & 0x8000) != 0;
                        if down && !state[i] {
                            let _ = on_key.send(*key);
                        }
                        state[i] = down;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
        });
}

/// 媒体键事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKey {
    Toggle,
    Next,
    Prev,
    Stop,
}

