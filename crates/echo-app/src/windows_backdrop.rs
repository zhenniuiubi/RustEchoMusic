//! Windows 系统背景材质（Mica 云母 / Desktop Acrylic 亚克力）管理。
//!
//! 平台检测、DWM 调用、模式切换、错误处理与降级逻辑全部集中在本模块，
//! 外部只需要 `apply(window, mode)` 和 `material_visible(mode)` 两个入口。
//!
//! 按 Windows 版本分层：
//! - Win11 22H2+（build 22621+）：DWM 系统背景 API `DWMWA_SYSTEMBACKDROP_TYPE`
//!   （属性号 38）——`DWMSBT_MAINWINDOW`(2)=Mica（主窗口云母）、
//!   `DWMSBT_TRANSIENTWINDOW`(3)=Desktop Acrylic（桌面亚克力），两者是不同的
//!   材质，不能混用。开启任意一种前必须 `DwmExtendFrameIntoClientArea`(margins
//!   全 -1)，否则无边框窗口上 DWM 只往「框架区域」画材质（等于没画，窗口呈
//!   灰色）。
//!   ⚠ 25H2 (26200) 实测：对这个 WS_EX_NOREDIRECTIONBITMAP 的 DComp 窗口，
//!   显式设 `DWMSBT_AUTO`(0) 或 `DWMSBT_NONE`(1) 都会让 DWM 停止合成窗口
//!   内容（整个窗口隐身，SWP_FRAMECHANGED 也无效）；而「从未设置过」则一直
//!   正常。所以关闭特效**不能**写 AUTO/NONE——保留原属性，靠 UI 切回不透明
//!   配色实现。
//! - Win11 21H2（build 22000-22522）：22H2 的属性号还不存在，用旧版未公开
//!   的 `DWMWA_MICA_EFFECT`（1029）+ `DwmExtendFrameIntoClientArea`，仅 Mica；
//!   Acrylic / 跟随系统降级为磨砂。
//! - Win10 1809+（build 17763+）：没有 DWM 系统背景，降级为 gpui 的
//!   `WindowBackgroundAppearance::Blurred`（内部走未公开的
//!   `SetWindowCompositionAttribute` / `ACCENT_ENABLE_ACRYLICBLURBEHIND`，
//!   拖动窗口时略卡是系统的已知行为）。
//! - 更老的系统：没有可用材质，回退普通不透明背景。
//!
//! 为什么透明像素能透出系统材质：gpui 的 Windows 后端启用 DirectComposition
//! 时用 `WS_EX_NOREDIRECTIONBITMAP` + 预乘 alpha 合成交换链渲染，每帧把渲染
//! 目标清成全透明黑（见 gpui `directx_renderer.rs`），所以只要根视图不画
//! 不透明背景，透明区域就是 DWM 合成在窗口后面的系统材质。
//!
//! 窗口重建 / 主题变化的处理：
//! - `DWMWA_SYSTEMBACKDROP_TYPE` 和亚克力 accent 策略都挂在 HWND 上且伴随
//!   窗口生命周期持续生效（最大化 / 还原 / 全屏切换不会重建 HWND），无需重设；
//! - 深浅色由 gpui 负责：系统主题变化时 gpui 会重设
//!   `DWMWA_USE_IMMERSIVE_DARK_MODE`（见 gpui `events.rs`），Mica/Acrylic
//!   随之自动切换明暗。

use echo_core::BackdropMode;
use gpui::{Window, WindowBackgroundAppearance};

/// 应用背景模式到窗口（切换设置项和启动时都会走这里）。
///
/// DWM 调用失败只记日志不 panic——窗口留在当前状态，UI 配色不受影响
/// （`material_visible` 决定配色，与这里是否成功解耦）。
pub fn apply(window: &Window, mode: BackdropMode) {
    imp::apply(window, mode)
}

/// 该模式下系统材质是否真的垫在窗口后面。
///
/// UI 用它决定要不要把基础背景画成半透明：返回 false 时保持不透明配色
/// （材质不可用或模式为关闭），返回 true 时透明像素才透出材质。
pub fn material_visible(mode: BackdropMode) -> bool {
    imp::material_visible(mode)
}

/// 从 "system" / "mica" / "acrylic" / "off" 解析模式（自动化测试用）。
pub fn parse_mode(s: &str) -> Option<BackdropMode> {
    match s.trim().to_lowercase().as_str() {
        "system" | "auto" => Some(BackdropMode::System),
        "mica" => Some(BackdropMode::Mica),
        "acrylic" => Some(BackdropMode::Acrylic),
        "off" | "none" => Some(BackdropMode::Off),
        _ => None,
    }
}

#[cfg(windows)]
mod imp {
    use super::{BackdropMode, Window, WindowBackgroundAppearance};
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE,
        DWMSBT_MAINWINDOW, DWMSBT_TRANSIENTWINDOW,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

    /// Win11 22H2（build 22621）起才有 `DWMWA_SYSTEMBACKDROP_TYPE`。
    const WIN11_22H2_BUILD: u32 = 22621;
    /// Win11 21H2（build 22000）起可用旧版未公开属性 `DWMWA_MICA_EFFECT`。
    const WIN11_BUILD: u32 = 22000;
    /// Win10 1809（build 17763）起 `ACCENT_ENABLE_ACRYLICBLURBEHIND` 才可用
    /// （gpui 的 Blurred 内部也是按这个版本 guard 的）。
    const WIN10_1809_BUILD: u32 = 17763;
    /// 未公开属性号：Win11 21H2 的 Mica 开关。
    const DWMWA_MICA_EFFECT: i32 = 1029;
    /// 未公开属性号：`SetWindowCompositionAttribute` 的 ACCENT_POLICY。
    const WCA_ACCENT_POLICY: u32 = 0x13;
    /// ACCENT_POLICY.accent_state：关闭 accent（普通不透明）。
    /// （亚克力背板 = state 4，降级路径直接复用 gpui Blurred 的实现，不在这里重复。）
    const ACCENT_DISABLED: u32 = 0;

    /// UTF-16 编码（windows-sys 的 API 吃 `*const u16`，结尾补 0）。
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// `OSVERSIONINFOW`（只用到版本字段，szCSDVersion 按原布局占位）。
    #[repr(C)]
    struct OsVersionInfoW {
        dw_os_version_info_size: u32,
        dw_major_version: u32,
        dw_minor_version: u32,
        dw_build_number: u32,
        dw_platform_id: u32,
        sz_csd_version: [u16; 128],
    }

    /// `MARGINS`（与 dwmapi 的同名结构 ABI 一致）。
    #[repr(C)]
    struct Margins {
        cx_left_width: i32,
        cx_right_width: i32,
        cy_top_height: i32,
        cy_bottom_height: i32,
    }

    /// 未公开结构：`ACCENT_POLICY`（SetWindowCompositionAttribute 载荷）。
    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        /// ABGR：alpha 在最高字节，用近全透明（0x01）让系统磨砂透出来。
        gradient_color: u32,
        animation_id: u32,
    }

    /// 未公开结构：`WINDOWCOMPOSITIONATTRIBDATA`。
    #[repr(C)]
    struct WindowCompositionAttribData {
        attrib: u32,
        data: *mut AccentPolicy,
        size: usize,
    }

    /// 当前系统 build 号（进程内只查一次；拿不到时按 0 = 全不支持处理）。
    fn win_build() -> u32 {
        static BUILD: OnceLock<u32> = OnceLock::new();
        *BUILD.get_or_init(|| read_build_impl(false))
    }

    /// material_visible 用的静默版（渲染路径里每帧调用，不刷日志；
    /// 结果同 win_build，共享同一份缓存）。
    fn read_build_quiet() -> u32 {
        win_build()
    }

    /// 系统「透明效果」开关（个性化 → 颜色；DWMSBT_AUTO 会跟着它变成
    /// 无材质的尖锐透明，所以「跟随系统」模式要自己查它来二选一）。
    /// HKCU 注册表，进程内读一次缓存；运行中改系统开关需重启应用生效。
    fn system_transparency_enabled() -> bool {
        use windows_sys::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
        };
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            let path = wide(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize");
            let value = wide("EnableTransparency");
            unsafe {
                let mut key: HKEY = std::ptr::null_mut();
                if RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &mut key) != 0 {
                    return true;
                }
                let mut data: u32 = 0;
                let mut len = std::mem::size_of::<u32>() as u32;
                let status = RegQueryValueExW(
                    key,
                    value.as_ptr(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut data as *mut _ as *mut u8,
                    &mut len,
                );
                RegCloseKey(key);
                status != 0 || data != 0
            }
        })
    }

    /// DwmSetWindowAttribute 写 u32 值（失败记 warn，不 panic）。
    fn dwm_set_u32(hwnd: HWND, attr: i32, value: i32, what: &str) {
        let hr = unsafe {
            DwmSetWindowAttribute(hwnd, attr as u32, &value as *const _ as *const _, 4)
        };
        if hr != 0 {
            log::warn!("背景特效：{what} 失败 (HRESULT=0x{hr:08x})，保持当前状态");
        }
    }

    /// 未公开的 SetWindowCompositionAttribute（user32 动态解析，缺失返回 false）。
    fn set_accent_policy(hwnd: HWND, accent_state: u32) -> bool {
        type SetWindowCompositionAttribute =
            unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> i32;
        let user32_name = wide("user32.dll");
        let user32 = unsafe { GetModuleHandleW(user32_name.as_ptr()) };
        if user32.is_null() {
            return false;
        }
        let proc = unsafe {
            GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr() as *const u8)
        };
        let Some(proc) = proc else {
            return false;
        };
        let accent = AccentPolicy {
            accent_state,
            accent_flags: 0,
            gradient_color: 0x01000000,
            animation_id: 0,
        };
        let mut data = WindowCompositionAttribData {
            attrib: WCA_ACCENT_POLICY,
            data: &accent as *const AccentPolicy as *mut AccentPolicy,
            size: std::mem::size_of::<AccentPolicy>(),
        };
        unsafe {
            std::mem::transmute::<_, SetWindowCompositionAttribute>(proc)(hwnd, &mut data);
        }
        true
    }

    /// Win11 21H2 的 Mica：未公开属性 1029 + 扩展框架到整个客户区。
    fn enable_mica_21h2(hwnd: HWND) {
        extend_frame_glass(hwnd);
        dwm_set_u32(hwnd, DWMWA_MICA_EFFECT, 1, "DWMWA_MICA_EFFECT");
    }

    /// 把 DWM 框架扩展到整个客户区（sheet of glass，margins 全 -1）。
    ///
    /// 这一步是 22H2 系统背景能铺满客户区的关键：无边框窗口（WM_NCCALCSIZE
    /// 去掉了整个框架）不扩展框架时，DWM 只往「框架区域」画材质——也就是
    /// 什么都没画，窗口只剩 UI 自身那层半透明色，看起来就是灰色一块。
    fn extend_frame_glass(hwnd: HWND) {
        let margins = Margins {
            cx_left_width: -1,
            cx_right_width: -1,
            cy_top_height: -1,
            cy_bottom_height: -1,
        };
        let hr = unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins as *const Margins as *const _) };
        if hr != 0 {
            log::warn!("背景特效：DwmExtendFrameIntoClientArea 失败 (HRESULT=0x{hr:08x})");
        }
    }

    /// 恢复框架扩展为 0（关闭特效时用）。
    fn reset_frame(hwnd: HWND) {
        let margins = Margins {
            cx_left_width: 0,
            cx_right_width: 0,
            cy_top_height: 0,
            cy_bottom_height: 0,
        };
        unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins as *const Margins as *const _) };
    }

    /// 关掉 21H2 的旧版 Mica（仅 21H2 区间调用）。
    fn disable_mica_21h2(hwnd: HWND) {
        reset_frame(hwnd);
        dwm_set_u32(hwnd, DWMWA_MICA_EFFECT, 0, "清除 DWMWA_MICA_EFFECT");
    }

    /// 降级磨砂：gpui 的 Blurred（内部 = SetWindowCompositionAttribute 亚克力）。
    /// 低于 1809 的系统 gpui 会直接忽略该调用，窗口保持普通不透明。
    fn fallback_blurred(window: &Window, why: &str) {
        log::info!("背景特效：{why}，降级为 gpui Blurred 磨砂");
        window.set_background_appearance(WindowBackgroundAppearance::Blurred);
    }

    /// Win10 的磨砂降级（先把框架铺满整窗，再开亚克力）。
    ///
    /// gpui 的「无边框」窗口在 Win10 上其实还留着一圈非客户区（`WM_NCCALCSIZE`
    /// 只把**客户区**内缩了 frame_thickness，外圈仍归 DWM）。Win11 那条路因为
    /// 先调了 `DwmExtendFrameIntoClientArea`，材质会铺到外圈；Win10 这条以前没调，
    /// 于是外圈还是 DWM 画的窗口边框色 —— 客户区半透明、外圈实心，看起来就是
    /// 「开了特效后 app 多了一个边框」。这里补上扩展，材质铺满整窗，边框消失。
    fn fallback_blurred_win10(window: &Window, hwnd: HWND, why: &str) {
        extend_frame_glass(hwnd);
        fallback_blurred(window, why);
    }

    pub(super) fn apply(window: &Window, mode: BackdropMode) {
        let Some(hwnd) = crate::win::hwnd(window) else {
            log::warn!("背景特效：拿不到 HWND，跳过");
            return;
        };
        let build = win_build();

        match mode {
            BackdropMode::Off => {
                // 22H2+ 上**不要**碰 DWMWA_SYSTEMBACKDROP_TYPE：实测 25H2 (26200)
                // 显式设 AUTO(0) 或 NONE(1) 都会让 DWM 停止合成这个
                // NOREDIRECTIONBITMAP DComp 窗口的内容（窗口整个隐身，
                // SWP_FRAMECHANGED 也救不回来），而「从未设置过」的窗口一直正常。
                // 所以关闭特效 = 保留已有属性（材质被下面的不透明 UI 盖住，
                // 视觉上就是普通不透明窗口），UI 层由 material_visible=false
                // 切回不透明配色。
                // gpui 的 Opaque = ACCENT_DISABLED，把 accent 关回普通不透明
                window.set_background_appearance(WindowBackgroundAppearance::Opaque);
                set_accent_policy(hwnd, ACCENT_DISABLED);
                if build >= WIN11_BUILD && build < WIN11_22H2_BUILD {
                    // 21H2 专用清理（1029 + 框架扩展）
                    disable_mica_21h2(hwnd);
                } else if build >= WIN10_1809_BUILD && build < WIN11_BUILD {
                    // Win10：把材质期间扩展出去的框架收回，否则窗口四周会留下一圈
                    // 透明/磨砂的边（关掉特效后看起来就是「多了个边框」）
                    reset_frame(hwnd);
                }
                log::info!("背景特效：已关闭（普通不透明背景）");
            }
            BackdropMode::System => {
                if build >= WIN11_22H2_BUILD {
                    // 不用 DWMSBT_AUTO：系统关「透明效果」时 AUTO 会退化成
                    // 无材质的尖锐透明（透明像素直接透出桌面，观感破碎）。
                    // 这里按系统开关自己二选一：开 → Mica；关 → 无材质。
                    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
                    if system_transparency_enabled() {
                        extend_frame_glass(hwnd);
                        dwm_set_u32(
                            hwnd,
                            DWMWA_SYSTEMBACKDROP_TYPE,
                            DWMSBT_MAINWINDOW,
                            "DWMSBT_MAINWINDOW (跟随系统→Mica)",
                        );
                        log::info!("背景特效：跟随系统（系统透明效果开 → Mica）");
                    } else {
                        // 透明效果关 = 无材质：不碰 backdrop 属性（同 Off 分支的
                        // 原因），UI 配色由 material_visible=false 切回不透明
                        log::info!("背景特效：跟随系统（系统透明效果关 → 无材质）");
                    }
                } else if build >= WIN10_1809_BUILD {
                    if build >= WIN11_BUILD && build < WIN11_22H2_BUILD {
                        disable_mica_21h2(hwnd);
                    }
                    fallback_blurred_win10(window, hwnd, "系统不支持 DWM 系统背景");
                } else {
                    log::info!("背景特效：系统过老（build {build}），普通不透明背景");
                    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
                }
            }
            BackdropMode::Mica => {
                if build >= WIN11_22H2_BUILD {
                    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
                    extend_frame_glass(hwnd);
                    dwm_set_u32(
                        hwnd,
                        DWMWA_SYSTEMBACKDROP_TYPE,
                        DWMSBT_MAINWINDOW,
                        "DWMSBT_MAINWINDOW (Mica)",
                    );
                    log::info!("背景特效：Mica（DWMSBT_MAINWINDOW）");
                } else if build >= WIN11_BUILD {
                    enable_mica_21h2(hwnd);
                    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
                    log::info!("背景特效：Mica（Win11 21H2 未公开属性 1029）");
                } else if build >= WIN10_1809_BUILD {
                    fallback_blurred_win10(window, hwnd, "Win10 无 Mica");
                } else {
                    log::info!("背景特效：系统过老（build {build}），普通不透明背景");
                    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
                }
            }
            BackdropMode::Acrylic => {
                if build >= WIN11_22H2_BUILD {
                    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
                    extend_frame_glass(hwnd);
                    dwm_set_u32(
                        hwnd,
                        DWMWA_SYSTEMBACKDROP_TYPE,
                        DWMSBT_TRANSIENTWINDOW,
                        "DWMSBT_TRANSIENTWINDOW (Desktop Acrylic)",
                    );
                    log::info!("背景特效：Desktop Acrylic（DWMSBT_TRANSIENTWINDOW）");
                } else if build >= WIN10_1809_BUILD {
                    if build >= WIN11_BUILD && build < WIN11_22H2_BUILD {
                        disable_mica_21h2(hwnd);
                    }
                    fallback_blurred_win10(window, hwnd, "系统不支持 DWM 亚克力");
                } else {
                    log::info!("背景特效：系统过老（build {build}），普通不透明背景");
                    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
                }
            }
        }
    }

    pub(super) fn material_visible(mode: BackdropMode) -> bool {
        match mode {
            BackdropMode::Off => false,
            // 22H2+ 全部走系统背景；21H2 Mica 走 1029、其余走亚克力；
            // Win10 1809+ 走亚克力；再老（build 0 视为检测失败）就没有了。
            // 跟随系统模式还要叠加系统「透明效果」开关（关 = 无材质）。
            BackdropMode::System => {
                read_build_quiet() >= WIN10_1809_BUILD
                    && (read_build_quiet() < WIN11_22H2_BUILD || system_transparency_enabled())
            }
            _ => read_build_quiet() >= WIN10_1809_BUILD,
        }
    }

    /// ntdll RtlGetVersion 读真实系统版本（GetVersionExW 受兼容性 shim
    /// 影响会谎报）。`quiet` 用于探测路径，失败不刷日志。
    fn read_build_impl(quiet: bool) -> u32 {
        // GetVersionExW 受兼容性 shim 影响会谎报版本，用 ntdll 的
        // RtlGetVersion 拿真实版本（与 gpui 内部 util.rs 的做法一致）。
        let ntdll_name = wide("ntdll.dll");
        let ntdll = unsafe { GetModuleHandleW(ntdll_name.as_ptr()) };
        if ntdll.is_null() {
            if !quiet {
                log::warn!("背景特效：GetModuleHandleW(ntdll) 失败，按不支持处理");
            }
            return 0;
        }
        type RtlGetVersion = unsafe extern "system" fn(*mut OsVersionInfoW) -> i32;
        let proc = unsafe { GetProcAddress(ntdll, c"RtlGetVersion".as_ptr() as *const u8) };
        let Some(proc) = proc else {
            if !quiet {
                log::warn!("背景特效：找不到 RtlGetVersion，按不支持处理");
            }
            return 0;
        };
        let mut info: OsVersionInfoW = unsafe { std::mem::zeroed() };
        info.dw_os_version_info_size = std::mem::size_of::<OsVersionInfoW>() as u32;
        // RtlGetVersion 返回 NTSTATUS：0 = STATUS_SUCCESS
        if unsafe { std::mem::transmute::<_, RtlGetVersion>(proc)(&mut info) } == 0 {
            if !quiet {
                log::info!(
                    "背景特效：Windows {}.{}.{}",
                    info.dw_major_version,
                    info.dw_minor_version,
                    info.dw_build_number
                );
            }
            info.dw_build_number
        } else {
            if !quiet {
                log::warn!("背景特效：RtlGetVersion 失败，按不支持处理");
            }
            0
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{BackdropMode, Window};

    pub(super) fn apply(_window: &Window, _mode: BackdropMode) {}

    pub(super) fn material_visible(_mode: BackdropMode) -> bool {
        false
    }
}
