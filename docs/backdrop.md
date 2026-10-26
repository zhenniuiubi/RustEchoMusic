# 背景特效（Mica / Acrylic）设计与测试说明

EchoMusic 主窗口支持 Windows 系统背景材质。实现集中在 `crates/echo-app/src/windows_backdrop.rs`，
设置项在 `echo-core` 的 `Settings::backdrop`（`system` / `mica` / `acrylic` / `off`，
默认 `system`），设置页「外观 → 背景特效」可切换，落盘于 `settings.json`。

## 技术方案

- **HWND 获取**：gpui 0.2.2 的 `Window` 实现了 `HasWindowHandle`（raw-window-handle 0.6），
  `win::hwnd(window)` 从中取 HWND，无需窗口标题/枚举等不可靠手段，也未修改 gpui 源码。
- **渲染前提**：gpui Windows 后端启用 DirectComposition 时以 `WS_EX_NOREDIRECTIONBITMAP`
  + 预乘 alpha 交换链渲染，每帧清屏为全透明黑。窗口打开时 `window_background` 保持
  `Opaque`（= ACCENT_DISABLED，不与系统材质冲突），材质激活时由 UI 把基础背景画成
  半透明（`ui.rs` 中 `backdrop_active()` 分支），透明像素即透出 DWM 合成的材质。
- **模式 → 系统调用**（按 build 号分层，RtlGetVersion 读真实版本）：

| 模式 | Win11 22H2+（22621+） | Win11 21H2（22000+） | Win10 1809+（17763+） | Win10 1809 之前 |
|------|------|------|------|------|
| 跟随系统 | 系统透明效果开 → `DWMSBT_MAINWINDOW`(2)；关 → 不设属性（同关闭） | gpui Blurred 磨砂 | gpui Blurred 磨砂 | 普通不透明 |
| 云母 Mica | **先 `DwmExtendFrameIntoClientArea`(margins 全 -1)**，再 `DWMSBT_MAINWINDOW`(2) | 未公开属性 `DWMWA_MICA_EFFECT`(1029) + DwmExtendFrameIntoClientArea | gpui Blurred 磨砂 | 普通不透明 |
| 亚克力 Acrylic | **先扩展框架**，再 `DWMSBT_TRANSIENTWINDOW`(3)（Desktop Acrylic） | gpui Blurred 磨砂 | gpui Blurred 磨砂 | 普通不透明 |
| 关闭 | 不设任何 backdrop 属性 + ACCENT_DISABLED | ACCENT_DISABLED | ACCENT_DISABLED | 普通不透明 |

**两条关键经验（25H2 / build 26200 实测）**：
1. 无边框窗口（WM_NCCALCSIZE 去掉整个框架）开系统材质前必须
   `DwmExtendFrameIntoClientArea(margins={-1,-1,-1,-1})`，否则 DWM 只往
   「框架区域」画材质——等于没画，窗口只剩 UI 半透明色叠在灰底上
   （用户看到的「云母无效、侧栏发灰」就是这个）。
2. 关闭特效**不要**写 `DWMSBT_AUTO`(0) / `DWMSBT_NONE`(1)。锁屏期间取得的
   屏幕截图不可靠（拍到的是锁屏壁纸），这两条路径在健康会话下的表现未获
   可信验证；而「从不设置该属性」的窗口（原始应用）始终正常，所以关闭
   特效保持属性不动 + UI 切回不透明配色是最安全的实现。

- **为什么跟随系统不用 `DWMSBT_AUTO`**：系统关闭「透明效果」时 AUTO 不会绘制任何材质，
  透明像素直接透出桌面（无模糊的尖锐透明，观感破碎）。因此读
  `HKCU\...\Themes\Personalize\EnableTransparency` 自行二选一（进程内缓存，改系统设置需重启生效）。
- **错误处理**：所有 DWM 调用检查 HRESULT，失败记 warn 不 panic；RtlGetVersion /
  SetWindowCompositionAttribute 解析失败按不支持降级。
- **窗口重建 / 主题变化**：`DWMWA_SYSTEMBACKDROP_TYPE` 与 accent 策略随 HWND 生命周期持续
  生效（最大化/还原/全屏不重建 HWND），无需重设；深浅色由 gpui 在系统主题变化时重设
  `DWMWA_USE_IMMERSIVE_DARK_MODE`，Mica/Acrylic 明暗自动跟随。
- **降级可见性**：`material_visible(mode)` 决定 UI 是否切换为半透明配色——材质不可用时
  自动回到普通不透明外观，不会出现「透明但没材质」的黑屏/透桌面。

## 测试方法与结果（Windows 11 25H2，build 26200）

1. **四模式启动读回**：`ECHO_BACKDROP=system|mica|acrylic|off` 环境变量启动
   （只影响本次运行，不落盘），用 `DwmGetWindowAttribute(hwnd, 38)` 读回：

   | 模式 | 期望 DWMSBT | 结果 |
   |------|------|------|
   | system | 2（透明效果开 → Mica） | PASS |
   | mica | 2 | PASS |
   | acrylic | 3 | PASS |
   | off | 0（属性不动，读回默认值） | PASS |

2. **运行时切换**：`ECHO_SMOKE=backdrop-mica|acrylic|off|system` 走与设置页按钮相同的
   `ui::set_backdrop_mode` 生产路径，四种迁移（off→mica、system→acrylic、mica→off、
   off→system）读回全部 PASS（4/4）。设置页按钮点击由人工验证（自动化合成点击对
   gpui 0.2.2 自绘按钮的 hover 匹配不可靠——真实鼠标不受影响，主题/音质按钮的人工
   与半自动点击均已验证生效）。
3. **窗口行为**：最大化/还原（`IsZoomed`）在背景特效各状态下验证通过；拖动/缩放走
   系统 `HTCAPTION`/`WS_THICKFRAME` 路径，本次改动未触碰输入与布局代码。
4. **截图**：`test-output/` 下有各模式窗口截图（PrintWindow 抓 DComp 表面不受
   锁屏影响；CopyFromScreen 全屏截图需在解锁状态下进行）。Mica/Acrylic 的实际
   观感取决于桌面壁纸与系统明暗主题。

## Win10 / 不支持材质环境的预期表现

- Win10 1809+：所有非关闭模式降级为 gpui Blurred（未公开的
  `SetWindowCompositionAttribute`/`ACCENT_ENABLE_ACRYLICBLURBEHIND`），窗口拖动时
  模糊更新略滞后是系统已知行为；UI 半透明配色不变。
- **Win10 也必须先 `DwmExtendFrameIntoClientArea(margins={-1,-1,-1,-1})`**：gpui 的
  无边框窗口其实还留着一圈非客户区（`WM_NCCALCSIZE` 只把客户区内缩了
  `frame_thickness`，外圈仍归 DWM）。Win11 那条路因为先扩展了框架，材质会铺到外圈；
  Win10 以前没扩展，外圈仍是 DWM 画的窗口边框色 —— 客户区半透明、外圈实心，看起来就是
  「开了特效后 app 多了一个边框」。现在 Win10 降级路径统一先扩展框架（`fallback_blurred_win10`），
  关闭特效时 `reset_frame` 收回。
- Win10 1809 之前 / 版本检测失败：普通不透明背景（自动回到不透明配色）。
- 验证方法：在目标系统上运行 `ECHO_BACKDROP=mica echo-music.exe`，观察日志
  `背景特效：... 降级为 gpui Blurred 磨砂`，并用截图对比窗口边缘（无 DWM 材质时
  半透明配色会自动关闭，界面恢复纯色）。

## 已知边界

- 「跟随系统」模式的透明效果开关在启动时读取一次，运行中改系统设置需重启应用。
- 桌面歌词 / mini 播放器等浮窗不参与背景特效（保持原外观）。
