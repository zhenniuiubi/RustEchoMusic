//! 背景材质（Mica 云母 / Desktop Acrylic 亚克力）最小验证程序。
//!
//! 目的：把「系统 / 接口 / 窗口样式」三层隔离开，判断 EchoMusic 的
//! 「云母 / 亚克力」到底卡在哪一层：
//!
//! - 这个窗口是**最原始的 Win32 GDI 窗口**（没有 DirectComposition、没有
//!   `WS_EX_NOREDIRECTIONBITMAP`、没有 DXGI swapchain），
//! - 只用 DWM 的公开/未公开属性去开材质。
//!
//! 用法：把窗口拖到能看到壁纸的位置，按 1~7 逐个试，看窗口内部是否透出**桌面
//! 壁纸并被模糊**。结果会同时写进 exe 旁边的 `backdrop-demo.log`。
//!
//! | 键 | 做法 |
//! |----|------|
//! | 1  | `DWMWA_SYSTEMBACKDROP_TYPE` = 2（Mica，Win11 22H2+） |
//! | 2  | `DWMWA_SYSTEMBACKDROP_TYPE` = 3（Desktop Acrylic） |
//! | 3  | `DWMWA_SYSTEMBACKDROP_TYPE` = 4（Tabbed） |
//! | 4  | `DWMWA_SYSTEMBACKDROP_TYPE` = 1（None） |
//! | 5  | `DWMWA_MICA_EFFECT`(1029)（Win11 21H2 老写法） |
//! | 6  | `SetWindowCompositionAttribute` + `ACCENT_ENABLE_ACRYLICBLURBEHIND` |
//! | 7  | `SetWindowCompositionAttribute` + `ACCENT_ENABLE_BLURBEHIND` |
//! | 8  | 全部关掉 |
//! | 0  | 重新 `DwmExtendFrameIntoClientArea(-1)` |
//! | B  | 切换「背景涂黑 / 完全不涂」，看哪种下材质才出来 |
//! | Esc| 退出 |

use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, InvalidateRect, SelectObject, SetBkMode,
    SetTextColor, DT_LEFT, DT_TOP, DT_WORDBREAK, HBRUSH, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW,
    RegisterClassW, SW_SHOW, ShowWindow, TranslateMessage, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND,
    WM_KEYDOWN, WM_PAINT, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

// ---- DWM 属性号（Windows SDK 里没公开的，自己定义）----
const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38; // 22H2+
const DWMWA_MICA_EFFECT: u32 = 1029; // 21H2 未公开
const DWMSBT_NONE: i32 = 1;
const DWMSBT_MAINWINDOW: i32 = 2;
const DWMSBT_TRANSIENTWINDOW: i32 = 3;
const DWMSBT_TABBEDWINDOW: i32 = 4;

// ---- SetWindowCompositionAttribute ----
const WCA_ACCENT_POLICY: u32 = 0x13;
const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_BLURBEHIND: u32 = 3;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

#[repr(C)]
struct AccentPolicy {
    accent_state: u32,
    accent_flags: u32,
    /// ABGR（alpha 在最高字节）
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct WindowCompositionAttribData {
    attrib: u32,
    data: *mut AccentPolicy,
    size: usize,
}

type SetWindowCompositionAttributeFn =
    unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> i32;

/// 当前选中的方案（0 = 没有）
static MODE: AtomicU32 = AtomicU32::new(0);
/// 是否把客户区涂成黑色（经典 Aero 玻璃里「黑色 = 透明」）
static PAINT_BLACK: AtomicU32 = AtomicU32::new(1);
/// 上一次调用的 HRESULT，显示在窗口里
static LAST_HR: AtomicU32 = AtomicU32::new(0);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn build_number() -> u32 {
    // RtlGetVersion 是内核导出的真实版本号（GetVersionEx 会被 manifest 骗）
    unsafe {
        let ntdll = GetModuleHandleW(wide("ntdll.dll").as_ptr());
        if ntdll.is_null() {
            return 0;
        }
        type RtlGetVersion = unsafe extern "system" fn(*mut OsVersionInfoW) -> i32;
        let Some(proc) = GetProcAddress(ntdll, c"RtlGetVersion".as_ptr() as *const u8) else {
            return 0;
        };
        let f: RtlGetVersion = std::mem::transmute(proc);
        let mut info = OsVersionInfoW::default();
        info.size = std::mem::size_of::<OsVersionInfoW>() as u32;
        if f(&mut info) < 0 {
            return 0;
        }
        info.build
    }
}

#[repr(C)]
struct OsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    csd: [u16; 128],
}

impl Default for OsVersionInfoW {
    fn default() -> Self {
        Self {
            size: 0,
            major: 0,
            minor: 0,
            build: 0,
            platform: 0,
            csd: [0; 128],
        }
    }
}

fn log_line(s: &str) {
    use std::io::Write;
    let path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("backdrop-demo.log")))
        .unwrap_or_else(|| "backdrop-demo.log".into());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{s}");
    }
    println!("{s}");
}

fn set_dwm(hwnd: HWND, attr: u32, value: i32) -> i32 {
    unsafe { DwmSetWindowAttribute(hwnd, attr, &value as *const _ as *const _, 4) }
}

fn extend_frame(hwnd: HWND) -> i32 {
    let m = MARGINS {
        cxLeftWidth: -1,
        cxRightWidth: -1,
        cyTopHeight: -1,
        cyBottomHeight: -1,
    };
    unsafe { DwmExtendFrameIntoClientArea(hwnd, &m) }
}

fn set_accent(hwnd: HWND, state: u32, color: u32) -> i32 {
    unsafe {
        let user32 = GetModuleHandleW(wide("user32.dll").as_ptr());
        if user32.is_null() {
            return -1;
        }
        let Some(proc) =
            GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr() as *const u8)
        else {
            return -2;
        };
        let f: SetWindowCompositionAttributeFn = std::mem::transmute(proc);
        let mut policy = AccentPolicy {
            accent_state: state,
            accent_flags: 0,
            gradient_color: color,
            animation_id: 0,
        };
        let mut data = WindowCompositionAttribData {
            attrib: WCA_ACCENT_POLICY,
            data: &mut policy,
            size: std::mem::size_of::<AccentPolicy>(),
        };
        f(hwnd, &mut data)
    }
}

/// 应用第 `mode` 号方案，返回 (描述, hr)
fn apply(hwnd: HWND, mode: u32) -> (String, i32) {
    match mode {
        1 => {
            let hr = extend_frame(hwnd).max(set_dwm(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW));
            ("Mica  DWMWA_SYSTEMBACKDROP_TYPE = 2".into(), hr)
        }
        2 => {
            let hr = extend_frame(hwnd)
                .max(set_dwm(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW));
            ("Acrylic  DWMWA_SYSTEMBACKDROP_TYPE = 3".into(), hr)
        }
        3 => {
            let hr =
                extend_frame(hwnd).max(set_dwm(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TABBEDWINDOW));
            ("Tabbed  DWMWA_SYSTEMBACKDROP_TYPE = 4".into(), hr)
        }
        4 => {
            let hr = set_dwm(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_NONE);
            ("None  DWMWA_SYSTEMBACKDROP_TYPE = 1".into(), hr)
        }
        5 => {
            let hr = extend_frame(hwnd).max(set_dwm(hwnd, DWMWA_MICA_EFFECT, 1));
            ("老 Mica  DWMWA_MICA_EFFECT(1029) = 1".into(), hr)
        }
        6 => {
            let hr = set_accent(hwnd, ACCENT_ENABLE_ACRYLICBLURBEHIND, 0x99202020);
            (
                "Accent  ACRYLICBLURBEHIND(4) color=0x99202020".into(),
                hr,
            )
        }
        7 => {
            let hr = set_accent(hwnd, ACCENT_ENABLE_BLURBEHIND, 0x99202020);
            ("Accent  BLURBEHIND(3)".into(), hr)
        }
        8 => {
            let hr = set_accent(hwnd, ACCENT_DISABLED, 0).max(set_dwm(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                DWMSBT_NONE,
            ));
            ("全部关闭".into(), hr)
        }
        _ => {
            let hr = extend_frame(hwnd);
            ("重新扩展边框 DwmExtendFrameIntoClientArea(-1)".into(), hr)
        }
    }
}

fn describe(mode: u32) -> &'static str {
    match mode {
        1 => "Mica",
        2 => "Desktop Acrylic",
        3 => "Tabbed",
        4 => "None",
        5 => "老 Mica(1029)",
        6 => "Accent 亚克力",
        7 => "Accent 模糊",
        8 => "关闭",
        _ => "未设置",
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = std::mem::zeroed();
            let hdc = unsafe { windows_sys::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps) };
            if PAINT_BLACK.load(Ordering::Relaxed) != 0 {
                // 经典 Aero 玻璃约定：扩展边框区域内画纯黑 = 透明
                let mut rc: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
                unsafe {
                    GetClientRect(hwnd, &mut rc);
                    let brush: HBRUSH = CreateSolidBrush(0x00000000);
                    windows_sys::Win32::Graphics::Gdi::FillRect(hdc, &rc, brush);
                    DeleteObject(brush as _);
                }
            }
            let build = build_number();
            let mode = MODE.load(Ordering::Relaxed);
            let hr = LAST_HR.load(Ordering::Relaxed) as i32;
            let paint = if PAINT_BLACK.load(Ordering::Relaxed) != 0 {
                "背景涂黑"
            } else {
                "背景不涂"
            };
            let text = format!(
                "EchoMusic 背景材质 Demo    Windows build {build}\r\n\r\n\
                 当前：{name}（{mode}）  HRESULT=0x{hr:08X}    {paint}\r\n\
                 窗口样式：普通 Win32 窗口（无 DirectComposition / 无 NOREDIRECTIONBITMAP）\r\n\r\n\
                 把本窗口拖到能看见壁纸的位置，按数字键逐个试，\r\n\
                 看窗口内部是否透出桌面壁纸并被模糊：\r\n\r\n\
                 1  Mica            DWMWA_SYSTEMBACKDROP_TYPE = 2\r\n\
                 2  Desktop Acrylic DWMWA_SYSTEMBACKDROP_TYPE = 3\r\n\
                 3  Tabbed          DWMWA_SYSTEMBACKDROP_TYPE = 4\r\n\
                 4  None            DWMWA_SYSTEMBACKDROP_TYPE = 1\r\n\
                 5  老 Mica         DWMWA_MICA_EFFECT(1029) + 扩展边框\r\n\
                 6  Accent 亚克力   ACCENT_ENABLE_ACRYLICBLURBEHIND\r\n\
                 7  Accent 模糊     ACCENT_ENABLE_BLURBEHIND\r\n\
                 8  全部关掉\r\n\
                 0  重新扩展边框\r\n\
                 B  切换「背景涂黑 / 不涂」（材质不出来时两种都试试）\r\n\
                 Esc 退出\r\n\r\n\
                 结果同时写入 exe 旁边的 backdrop-demo.log",
                name = describe(mode),
            );
            unsafe {
                let tw = text.encode_utf16().collect::<Vec<_>>();
                let font = CreateFontW(
                    -18,
                    0,
                    0,
                    0,
                    400,
                    0,
                    0,
                    0,
                    1, // DEFAULT_CHARSET
                    0,
                    0,
                    0,
                    0,
                    wide("Microsoft YaHei UI").as_ptr(),
                );
                let old_font = SelectObject(hdc, font as _);
                SetBkMode(hdc, TRANSPARENT as i32);
                SetTextColor(hdc, 0x00FF_FFFF); // 白字（COLORREF = 0x00BBGGRR）
                let mut rc: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                rc.left += 24;
                rc.top += 20;
                rc.right -= 24;
                DrawTextW(
                    hdc,
                    tw.as_ptr(),
                    tw.len() as i32,
                    &mut rc,
                    DT_LEFT | DT_TOP | DT_WORDBREAK,
                );
                SelectObject(hdc, old_font);
                DeleteObject(font as _);
                windows_sys::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            }
            0
        }
        WM_ERASEBKGND => 1, // 不擦背景，否则会把玻璃盖住
        WM_KEYDOWN => {
            let key = wparam as u32;
            match key {
                0x1B => {
                    // Esc
                    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) };
                }
                0x42 => {
                    // 'B'
                    let v = if PAINT_BLACK.load(Ordering::Relaxed) != 0 { 0 } else { 1 };
                    PAINT_BLACK.store(v, Ordering::Relaxed);
                    log_line(&format!(
                        "切换绘制方式 -> {}",
                        if v != 0 { "涂黑" } else { "不涂" }
                    ));
                    unsafe { InvalidateRect(hwnd, std::ptr::null(), 1) };
                }
                other => {
                    let mode = match other {
                        0x31 => 1,
                        0x32 => 2,
                        0x33 => 3,
                        0x34 => 4,
                        0x35 => 5,
                        0x36 => 6,
                        0x37 => 7,
                        0x38 => 8,
                        0x30 => 0,
                        _ => 255,
                    };
                    if mode != 255 {
                        let (what, hr) = apply(hwnd, mode);
                        MODE.store(mode, Ordering::Relaxed);
                        LAST_HR.store(hr as u32, Ordering::Relaxed);
                        log_line(&format!(
                            "应用 [{mode}] {what} -> HRESULT=0x{hr:08X} ({})",
                            if hr == 0 { "S_OK" } else { "失败" }
                        ));
                        unsafe { InvalidateRect(hwnd, std::ptr::null(), 1) };
                    }
                }
            }
            0
        }
        WM_CLOSE => {
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn main() {
    let build = build_number();
    log_line(&format!(
        "=== backdrop-demo 启动，Windows build {build}，透明效果开关={} ===",
        transparency_enabled()
    ));

    unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("EchoBackdropDemo");
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: std::ptr::null_mut(),
            hCursor: windows_sys::Win32::UI::WindowsAndMessaging::LoadCursorW(
                std::ptr::null_mut(),
                windows_sys::Win32::UI::WindowsAndMessaging::IDC_ARROW,
            ),
            hbrBackground: std::ptr::null_mut(), // 不设背景刷：让扩展边框区域保持透明
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            log_line("RegisterClassW 失败");
            return;
        }
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("EchoMusic 背景材质 Demo").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            480,
            240,
            760,
            560,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            log_line("CreateWindowExW 失败");
            return;
        }
        // 深色标题栏 + 扩展边框，和 EchoMusic 主窗口的处理保持一致
        let _ = set_dwm(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, 1);
        let hr = extend_frame(hwnd);
        log_line(&format!("初始 DwmExtendFrameIntoClientArea(-1) -> HRESULT=0x{hr:08X}"));

        ShowWindow(hwnd, SW_SHOW);
        // 键盘焦点：点一下窗口再按键
        windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);

        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn transparency_enabled() -> bool {
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        let path = wide(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        if RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return true;
        }
        let mut data: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        let value = wide("EnableTransparency");
        let rc = RegQueryValueExW(
            key,
            value.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut data as *mut _ as *mut u8,
            &mut len,
        );
        RegCloseKey(key);
        rc != 0 || data != 0
    }
}
