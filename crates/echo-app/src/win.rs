//! Windows 平台能力：窗口图标、显示/隐藏、最大化切换、系统托盘。
//!
//! gpui 0.2.2 没提供「设置窗口图标」「隐藏窗口」「还原最大化」这些 API
//! （它的 `platform_window.zoom()` 只会 `SW_MAXIMIZE`，从不 `SW_RESTORE`，
//! 所以点两次最大化按钮回不去），这里直接用 Win32 补上。

#![cfg(windows)]
// edition 2024 里 unsafe fn 的函数体默认是「安全」的，逐个包 unsafe {} 太啰嗦
#![allow(unsafe_op_in_unsafe_fn)]

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

/// 托盘菜单命令回调消息。
const WM_APP_TRAY: u32 = WM_APP + 1;

/// 托盘菜单项 id。
const ID_SHOW: usize = 1;
const ID_TOGGLE: usize = 2;
const ID_PREV: usize = 3;
const ID_NEXT: usize = 4;
const ID_QUIT: usize = 5;

/// 托盘 → UI 的命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// 显示并激活主窗口
    Show,
    /// 播放 / 暂停
    TogglePlay,
    Prev,
    Next,
    /// 退出应用
    Quit,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

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

/// 隐藏窗口（进程继续跑，托盘里还在）。
pub fn hide_window(window: &Window) {
    if let Some(h) = hwnd(window) {
        unsafe {
            ShowWindow(h, SW_HIDE);
        }
        log::info!("窗口已隐藏（进程继续驻留托盘）");
    }
}

/// 显示并置前窗口（托盘里点「显示主窗口」）。
pub fn show_window(hwnd_raw: isize) {
    if hwnd_raw == 0 {
        return;
    }
    let h = hwnd_raw as HWND;
    unsafe {
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        }
        ShowWindow(h, SW_SHOW);
        SetForegroundWindow(h);
    }
}

/// 按原始 HWND 请求「还原 / 显示」（不进入 gpui 的 window 上下文）。
///
/// 用 `SC_RESTORE` 而不是直接 `ShowWindow`：ShowWindow 会**同步**派发 `WM_SIZE`，
/// 而 gpui 的 resize 回调又是 `window.update(...)`，在自己的事件/布局周期里调它会重入
/// （日志里是 “RefCell already borrowed”）。投递系统命令让窗口线程自己处理就绕开了。
/// `SC_RESTORE` 对 `SW_HIDE` 掉的窗口同样生效——还原会把窗口显示出来。
pub fn post_restore(hwnd_raw: isize) {
    if hwnd_raw == 0 {
        return;
    }
    unsafe {
        PostMessageW(hwnd_raw as HWND, WM_SYSCOMMAND, SC_RESTORE as WPARAM, 0);
    }
    log::info!("post_restore -> post SC_RESTORE");
}

/// 按原始 HWND 请求最大化（不进入 gpui 的 window 上下文，可从任意回调里调）。
pub fn post_maximize(hwnd_raw: isize) {
    if hwnd_raw == 0 {
        return;
    }
    unsafe {
        PostMessageW(hwnd_raw as HWND, WM_SYSCOMMAND, SC_MAXIMIZE as WPARAM, 0);
    }
    log::info!("post_maximize -> post SC_MAXIMIZE");
}

/// 当前是否处于最大化状态。
///
/// 直接问系统（而不是自己记标志位）：Win+Up、双击标题栏、边缘吸附导致的
/// 最大化/还原也能被正确反映到「最大化/还原」按钮的图标上。
pub fn is_maximized(window: &Window) -> bool {
    match hwnd(window) {
        Some(h) => unsafe { IsZoomed(h) != 0 },
        None => false,
    }
}

/// 开始拖动窗口（等价于按住系统标题栏）。
///
/// 为什么不直接用 `window_control_area(Drag)`：gpui 的命中测试要拿「上一次鼠标移动
/// 算出来的 hitbox 列表」去匹配，点快一点/刚移进来时匹配不到，就成了普通点击，
/// 于是左上角怎么按都拖不动。这里直接给系统发一条「非客户区左键按下 + HTCAPTION」，
/// 走 Windows 自己的窗口移动循环（贴边吸附、跨屏拖动都是系统的原生行为）。
pub fn start_drag(window: &Window) {
    /// gpui 会把这条 WM_NCLBUTTONDOWN 再派发回 view（本函数会被再调一次），
    /// 用标志位保证只发一条，第二次放行给 DefWindowProc 去执行默认拖动。
    static PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    if PENDING.swap(false, Ordering::SeqCst) {
        return;
    }
    if let Some(h) = hwnd(window) {
        PENDING.store(true, Ordering::SeqCst);
        unsafe {
            PostMessageW(h, WM_NCLBUTTONDOWN, HTCAPTION as WPARAM, 0);
        }
        log::debug!("start_drag: post WM_NCLBUTTONDOWN/HTCAPTION");
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

/// 鼠标此刻是否落在窗口矩形内。
///
/// gpui 0.2.2 的 Windows 端**没有处理 `WM_MOUSELEAVE`**，鼠标移出窗口后
/// `on_hover` 不会回调 `false`——浮窗里「hover 才显示的控制条」就会一直挂着。
/// 所以自己按光标位置判断（只需要原始 HWND，不必进 gpui 的 window 上下文）。
pub fn cursor_in_rect(hwnd_raw: isize) -> bool {
    if hwnd_raw == 0 {
        return false;
    }
    let h = hwnd_raw as HWND;
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut pt) == 0 {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(h, &mut r) == 0 {
            return false;
        }
        pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
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

/// 请求「最大化 / 还原」。///
/// **绝对不能在本进程内同步改窗口大小**：`ShowWindow`/`SetWindowPos` 会同步派发
/// `WM_SIZE`，而 gpui 的 resize 回调是 `window.update(...)`，重入时会失败
/// （被 `log_err` 吞掉），于是窗口铺开了、布局却还是旧的 ——
/// 表现就是「只是移动到了左上角」。
/// 所以这里只把 `WM_SYSCOMMAND` 塞进消息队列，等本轮事件处理完、
/// gpui 空闲时由主循环派发，就和用户按 Win+Up 完全一样。
pub fn set_maximized(window: &Window, on: bool) {
    if let Some(h) = hwnd(window) {
        let cmd = if on { SC_MAXIMIZE } else { SC_RESTORE };
        unsafe {
            PostMessageW(h, WM_SYSCOMMAND, cmd as WPARAM, 0);
        }
        log::info!(
            "set_maximized({on}) -> post {}",
            if on { "SC_MAXIMIZE" } else { "SC_RESTORE" }
        );
    }
}

// ==================== 系统托盘 ====================

struct TrayShared {
    /// 主窗口 HWND（UI 线程创建后写入）
    main: Arc<AtomicIsize>,
    tx: Mutex<Sender<TrayEvent>>,
}

static TRAY: OnceLock<TrayShared> = OnceLock::new();

/// 托盘窗口句柄（UI 线程更新提示文字时要用）。
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);

/// 更新托盘图标的悬浮提示（显示当前播放的歌曲）。
pub fn set_tray_tip(tip: &str) {
    let raw = TRAY_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = raw as HWND;
        nid.uID = 1;
        nid.uFlags = NIF_TIP;
        let w = wide(tip);
        let n = w.len().min(nid.szTip.len());
        nid.szTip[..n].copy_from_slice(&w[..n]);
        Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// 投递托盘事件到主线程（SMTC 按键等内部模块也复用）。
pub(crate) fn send(ev: TrayEvent) {
    if let Some(st) = TRAY.get() {
        if let Ok(tx) = st.tx.lock() {
            let _ = tx.send(ev);
        }
    }
}

/// 起一个专用线程建托盘图标 + 跑消息循环。
///
/// `main` 是主窗口 HWND 的共享槽位（UI 线程拿到后写入），
/// 托盘线程只负责转发命令，真正的窗口操作仍在 UI 线程做。
pub fn start_tray(main: Arc<AtomicIsize>, tx: Sender<TrayEvent>) {
    let _ = TRAY.set(TrayShared {
        main: main.clone(),
        tx: Mutex::new(tx),
    });

    let spawned = std::thread::Builder::new()
        .name("echo-tray".into())
        .spawn(move || unsafe { tray_thread() });
    if let Err(e) = spawned {
        log::warn!("托盘线程启动失败：{e}");
    }
}

unsafe fn tray_thread() {
    let hinst = GetModuleHandleW(std::ptr::null());
    let class_name = wide("EchoMusicTrayWnd");

    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(tray_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: app_icon().unwrap_or(std::ptr::null_mut()),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    if RegisterClassW(&wc) == 0 {
        log::warn!("托盘窗口类注册失败");
        return;
    }

    // 隐藏的普通窗口（不用 HWND_MESSAGE：消息专用窗口 FindWindowW 找不到，不利于自测）：
    // 不显示、只收托盘回调
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        wide("EchoMusicTray").as_ptr(),
        0,
        0,
        0,
        0,
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinst,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        log::warn!("托盘窗口创建失败");
        return;
    }

    if add_icon(hwnd) {
        TRAY_HWND.store(hwnd as isize, Ordering::Relaxed);
        log::info!("托盘图标已就绪");
    } else {
        log::warn!("托盘图标添加失败");
    }

    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    remove_icon(hwnd);
}

unsafe fn add_icon(hwnd: HWND) -> bool {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_APP_TRAY;
    nid.hIcon = app_icon().unwrap_or(std::ptr::null_mut());
    let tip = wide("EchoMusic");
    let n = tip.len().min(nid.szTip.len());
    nid.szTip[..n].copy_from_slice(&tip[..n]);
    Shell_NotifyIconW(NIM_ADD, &nid) != 0
}

unsafe fn remove_icon(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

unsafe fn show_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    let add = |id: usize, text: &str, flags: MENU_ITEM_FLAGS| {
        AppendMenuW(menu, flags, id, wide(text).as_ptr());
    };
    add(ID_SHOW, "显示主窗口", MF_STRING);
    add(0, "", MF_SEPARATOR);
    add(ID_TOGGLE, "播放 / 暂停", MF_STRING);
    add(ID_PREV, "上一首", MF_STRING);
    add(ID_NEXT, "下一首", MF_STRING);
    add(0, "", MF_SEPARATOR);
    add(ID_QUIT, "退出 EchoMusic", MF_STRING);

    let mut pt = POINT_DEFAULT;
    GetCursorPos(&mut pt);
    // 必须先把托盘窗口置前，否则点菜单外面菜单不会消失
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN,
        pt.x,
        pt.y,
        0,
        hwnd,
        std::ptr::null(),
    );
    DestroyMenu(menu);
    PostMessageW(hwnd, WM_NULL, 0, 0);

    if cmd != 0 {
        dispatch_cmd(cmd as usize, hwnd);
    }
}

/// 执行托盘菜单命令（菜单点击与 WM_COMMAND 共用）。
unsafe fn dispatch_cmd(cmd: usize, hwnd: HWND) {
    match cmd {
        ID_SHOW => {
            let raw = TRAY
                .get()
                .map(|s| s.main.load(Ordering::Relaxed))
                .unwrap_or(0);
            show_window(raw);
            send(TrayEvent::Show);
        }
        ID_TOGGLE => send(TrayEvent::TogglePlay),
        ID_PREV => send(TrayEvent::Prev),
        ID_NEXT => send(TrayEvent::Next),
        ID_QUIT => {
            send(TrayEvent::Quit);
            remove_icon(hwnd);
            DestroyWindow(hwnd);
        }
        _ => {}
    }
}

#[allow(non_upper_case_globals)]
const POINT_DEFAULT: windows_sys::Win32::Foundation::POINT =
    windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_APP_TRAY => {
            match (lparam as u32) & 0xFFFF {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    let raw = TRAY
                        .get()
                        .map(|s| s.main.load(Ordering::Relaxed))
                        .unwrap_or(0);
                    show_window(raw);
                    send(TrayEvent::Show);
                }
                WM_RBUTTONUP => show_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            // 低 16 位是菜单 id（自动化测试与键盘触发菜单都走这里）
            dispatch_cmd(wparam & 0xFFFF, hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => {
            // Explorer 重启后会 Broadcast 这个自定义消息，需要重新加图标
            let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
            if taskbar_created != 0 && msg == taskbar_created {
                add_icon(hwnd);
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}


// ==================== 开机自启 ====================

/// 开机自启：写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`。
///
/// 不用 `reg.exe`：本项目的构建沙箱禁它，而注册表 API 本身不受限。
pub fn set_auto_start(enable: bool) -> anyhow::Result<()> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_SZ,
    };

    let sub_key = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let value = wide("EchoMusic");

    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            sub_key.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        );
        if status != 0 {
            anyhow::bail!("打开注册表失败：{status}");
        }
        let result = if enable {
            let exe = std::env::current_exe().unwrap_or_default();
            let exe_w = wide(&exe.to_string_lossy());
            let bytes: Vec<u8> = exe_w
                .iter()
                .flat_map(|c| c.to_le_bytes())
                .collect();
            RegSetValueExW(
                key,
                value.as_ptr(),
                0,
                REG_SZ,
                bytes.as_ptr(),
                bytes.len() as u32,
            )
        } else {
            RegDeleteValueW(key, value.as_ptr())
        };
        RegCloseKey(key);
        if result != 0 {
            anyhow::bail!("写注册表失败：{result}");
        }
    }
    Ok(())
}

/// 是否已开启开机自启。
pub fn auto_start_enabled() -> bool {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };

    let sub_key = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let value = wide("EchoMusic");

    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, sub_key.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return false;
        }
        let mut ty: u32 = 0;
        let mut len: u32 = 0;
        // 先取长度，成功（含 ERROR_MORE_DATA=234）都算存在
        let status = RegQueryValueExW(
            key,
            value.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            std::ptr::null_mut(),
            &mut len,
        );
        RegCloseKey(key);
        status == 0 || status == 234
    }
}

// ==================== 全局媒体键 ====================

/// 虚拟键码（windows-sys 的常量在 Win32_UI_Input_KeyboardAndMouse 里，
/// 这里直接写值避免再开一个 feature）。
const VK_MEDIA_NEXT: i32 = 0xB0;
const VK_MEDIA_PREV: i32 = 0xB1;
const VK_MEDIA_STOP: i32 = 0xB2;
const VK_MEDIA_PLAY_PAUSE: i32 = 0xB3;

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

/// 从任务栏（和 Alt-Tab）隐藏窗口（mini 模式：任务栏只留主窗口/托盘）。
///
/// WS_EX_TOOLWINDOW 的副作用是同时从 Alt-Tab 消失——对 mini 小窗正合适。
pub fn hide_from_taskbar(window: &Window) {
    if let Some(h) = hwnd(window) {
        unsafe {
            let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            let new = (ex & !WS_EX_APPWINDOW) | WS_EX_TOOLWINDOW;
            SetWindowLongPtrW(h, GWL_EXSTYLE, new as isize);
            SetWindowPos(
                h,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        }
        log::debug!("mini 窗口已从任务栏隐藏");
    }
}
