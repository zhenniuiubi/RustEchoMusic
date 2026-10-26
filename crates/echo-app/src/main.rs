//! EchoMusic 桌面应用入口（gpui）。

// release 构建不弹出控制台窗口；debug 构建保留，方便看日志
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod amll;
mod assets;
mod icons;
mod imgcache;
mod liked;
mod player;
mod state;
mod text_input;
mod theme;
mod desktop;
mod smtc;
mod taskbar;
mod ui;
mod views;
mod vip;
mod win;
mod windows_backdrop;

use assets::{Assets, ReqwestHttpClient};
use echo_audio::{AudioEngine, PlayerEvent};
use echo_core::{DeviceIdentity, SettingsStore};
use gpui::prelude::*;
use gpui::*;
use parking_lot::Mutex;
use state::{AppGlobal, AppStore, Page};
use std::sync::Arc;

/// 窗口根视图。
pub struct Root {
    pub store: Entity<AppStore>,
    /// 根焦点：没有输入框占用时由它接管键盘（空格播放/暂停）
    pub root_focus: FocusHandle,
    pub search_input: Entity<text_input::TextInput>,
    pub search_focus: FocusHandle,
    /// 「我最喜爱」页的前端模糊搜索框
    pub fav_search: Entity<text_input::TextInput>,
    pub fav_search_focus: FocusHandle,
    /// 封面图 LRU 缓存（有上限，避免浏览列表时位图无限累积）
    pub cover_cache: Entity<imgcache::CoverCache>,
    /// 登录页：账号输入
    pub login_user: Entity<text_input::TextInput>,
    /// 登录页：密码输入
    pub login_pass: Entity<text_input::TextInput>,
    /// 登录页：手机号输入
    pub login_phone: Entity<text_input::TextInput>,
    /// 登录页：短信验证码输入
    pub login_code: Entity<text_input::TextInput>,
    /// 进度条 bounds（canvas paint 阶段记录，点击 seek 用）
    pub progress_bounds: Arc<Mutex<Option<Bounds<Pixels>>>>,
    /// 音量滑条 bounds（canvas 记录）
    pub volume_bounds: Arc<Mutex<Option<Bounds<Pixels>>>>,
    /// 主内容区滚动条句柄（自绘滚动条用）
    pub content_scroll: ScrollHandle,
    /// 侧栏歌单列表滚动条句柄
    pub sidebar_scroll: ScrollHandle,
    /// 播放页歌词列表：当前行变化时把它滚到视口中间
    pub lyric_scroll: UniformListScrollHandle,
    /// 上一次已跟随的行号（只在换行时滚，否则会抢掉用户手动滚动）
    pub lyric_follow: Option<usize>,
    /// 歌词滚动动画：(起点偏移, 目标偏移, 开始时刻)。换行时启动，
    /// 用 ease-out-back 缓动滚到「当前句居中」，观感是向上弹一下再稳住
    pub lyric_scroll_anim: Option<(f32, f32, std::time::Instant)>,
    /// 用户最后一次手动滚歌词的时间：这段时间内不自动居中，免得抢鼠标
    pub lyric_user_scroll_at: Option<std::time::Instant>,
    /// 用户滚过歌词、等着 5 秒后回弹到当前句
    pub lyric_restore_pending: bool,
    /// 上一次「程序自己」设的滚动偏移。渲染时和真实偏移一比，
    /// 差得多说明是用户滚的（uniform_list 会把 wheel 事件吃掉，
    /// 挂在父节点上的 on_scroll_wheel 根本收不到，只能这样感知）
    pub lyric_auto_off: f32,
    /// 上一次写进窗口标题的文本（避免每帧都调 SetWindowText）
    pub window_title: String,
}

impl Root {
    /// 任务栏缩略图按钮跟随播放 / 收藏状态（`taskbar::sync` 内部去重，每帧调也安全）。
    fn sync_taskbar_buttons(&mut self, cx: &mut Context<Self>) {
        let (playing, fav) = {
            let s = self.store.read(cx);
            let playing = s.player.playing;
            let fav = s
                .player
                .current()
                .map(|c| {
                    if s.session.get().userid != 0 {
                        crate::liked::is_remote_liked(&s, c)
                    } else {
                        s.library.is_favorite(c)
                    }
                })
                .unwrap_or(false);
            (playing, fav)
        };
        taskbar::sync(playing, fav);
    }

    /// 把「正在播放」写进窗口标题。
    ///
    /// Windows 的任务栏悬浮提示、任务管理器、Alt-Tab 都读窗口标题，
    /// 以前一直是固定的 "EchoMusic"，所以外面看不到在放什么歌。
    fn sync_window_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = {
            let s = self.store.read(cx);
            match s.player.current() {
                Some(song) if !song.title.is_empty() => {
                    if song.artist.is_empty() {
                        format!("{} - EchoMusic", song.title)
                    } else {
                        format!("{} - {} - EchoMusic", song.title, song.artist)
                    }
                }
                _ => "EchoMusic".to_string(),
            }
        };
        if self.window_title != title {
            window.set_window_title(&title);
            // 托盘悬浮提示同步（128 字符上限，长了系统会截断）
            crate::win::set_tray_tip(&title);
            log::debug!("窗口标题 -> {title}");
            self.window_title = title;
        }
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_window_title(window, cx);
        self.sync_taskbar_buttons(cx);
        // 整棵树套一层自定义图片缓存：所有 img 都走 LRU，不再无限常驻
        // 播放页（歌词页）全屏接管：铺满整个窗口，盖住侧栏/标题栏/播放条
        let content: gpui::AnyElement =
            if matches!(self.store.read(cx).routes.current(), Some(crate::state::Page::Lyric)) {
                self.render_lyric(window, cx)
            } else {
                self.render_layout(window, cx).into_any_element()
            };
        image_cache(self.cover_cache.clone())
            .size_full()
            .child(content)
    }
}

/// 日志过滤器：只压掉「已知、且上游已自行降级」的噪音，其它照旧透传。
struct QuietDxgiProbe<L>(L);

impl<L: log::Log> log::Log for QuietDxgiProbe<L> {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.0.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        if is_dxgi_probe_noise(record) {
            return;
        }
        self.0.log(record);
    }

    fn flush(&self) {
        self.0.flush();
    }
}

/// DXGI 调试层探测失败的噪音。
///
/// `DXGI_ERROR_SDK_COMPONENT_MISSING`（0x887A002D）：gpui 在 **debug 构建**里会调
/// `DXGIGetDebugInterface1` 探一下有没有 DXGI 调试接口，系统没装「图形工具」
/// （Graphics Tools）这个可选功能就必然失败。gpui 自己已经降级处理
/// （打一条 WARN 后关掉 DirectX 调试特性继续跑），渲染不受影响。
/// 但它在降级前先用 `log_err()` 把原始 Error 以 **ERROR** 级别打出来，
/// 于是每次启动日志第一行都是这条假错误，`grep ERROR` 的自动化会被它带偏。
///
/// 这里只精确放行这一个 HRESULT（错误码与系统语言无关），其它 ERROR 一律保留。
/// 不想过滤也可以装真正的调试层：
/// `DISM /Online /Add-Capability /CapabilityName:Tools.Graphics.DirectX~~~~0.0.1.0`
fn is_dxgi_probe_noise(record: &log::Record<'_>) -> bool {
    record.level() == log::Level::Error && record.args().to_string().contains("0x887A002D")
}

fn main() {
    ui::init_tokio();
    // 初始化日志：debug 构建默认输出 info+ 到控制台；RUST_LOG 可覆盖；
    // ECHO_LOG_FILE 可把日志重定向到文件（自动化测试用）。
    let mut log_builder = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    );
    if let Ok(path) = std::env::var("ECHO_LOG_FILE") {
        if let Ok(f) = std::fs::File::create(&path) {
            log_builder.target(env_logger::Target::Pipe(Box::new(f)));
        }
    }
    // 包一层只用来压掉已知噪音，其它行为与 env_logger 完全一致
    let logger = log_builder.build();
    let max_level = logger.filter();
    if log::set_boxed_logger(Box::new(QuietDxgiProbe(logger))).is_ok() {
        log::set_max_level(max_level);
    }
    // panic 落日志：release 无控制台，否则崩溃时看不到任何原因。
    // 先写消息再抓 backtrace（force_capture 不受 RUST_BACKTRACE 影响）。
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("PANIC: {info}");
        let bt = std::backtrace::Backtrace::force_capture();
        log::error!("BACKTRACE:\n{bt}");
        prev_hook(info);
    }));

    Application::new()
        .with_assets(Assets)
        .with_http_client(ReqwestHttpClient::new().expect("init http client"))
        .run(|cx: &mut App| {
            text_input::bind_input_keys(cx);

            // ===== 数据初始化 =====
            let data_dir = echo_core::data_dir();
            let settings = SettingsStore::load(&data_dir).expect("load settings");
            let device = DeviceIdentity::load_or_create(&data_dir).expect("load device identity");
            // 标准版客户端（appid 1005 / clientver 20489）：
            // 登录态接口（get_all_list / get_my_info）与歌曲地址 key 签名都只认标准版。
            let client = echo_api::KuGouClient::shared(device, false).expect("init kugou client");

            // 设备注册（后台）
            let reg_client = client.clone();
            cx.background_executor()
                .spawn(async move {
                    match ui::run_tokio(reg_client.register_device()) {
                        Ok(dfid) => log::info!("device registered: {dfid}"),
                        Err(e) => log::warn!("register_dev failed: {e}"),
                    }
                })
                .detach();

            // ===== 播放引擎 =====
            let (ev_tx, ev_rx) = std::sync::mpsc::channel::<PlayerEvent>();
            let engine = Arc::new(AudioEngine::new(ev_tx).expect("init audio engine"));
            // 引擎初始音量必须与设置一致，否则 UI 显示 0 但实际出声
            engine.set_volume(settings.get().volume);
            // 倍速（第三期）：设置里存的值直接灌给引擎
            engine.set_speed(settings.get().speed);
            // 输出设备（第三期）：设置里存了设备名就切过去（运行中切换已支持）
            let saved_output = settings.get().output_device;
            if !saved_output.is_empty() {
                log::info!("输出设备（设置）：{saved_output}");
                engine.switch_device(Some(&saved_output));
            }

            // ===== 会话与本地曲库 =====
            let session_store = echo_core::SessionStore::load(&data_dir).expect("load session");
            let library_store = echo_core::LibraryStore::load(&data_dir).expect("load library");
            // 播放队列快照：关掉 App 再打开要能还原（不自动播放，点播放时再解析地址）
            let queue_store = echo_core::QueueStore::load(&data_dir).expect("load queue");
            let saved_queue = queue_store.get();
            if !saved_queue.is_empty() {
                log::info!(
                    "restored queue: {} 首 (index={:?}, title={})",
                    saved_queue.songs.len(),
                    saved_queue.index,
                    saved_queue.title
                );
            }
            let saved = session_store.get();
            if saved.is_valid() {
                client.set_auth(&saved.token, saved.userid);
            }
            let saved_user = if saved.is_valid() {
                Some(echo_core::User {
                    userid: saved.userid,
                    token: saved.token.clone(),
                    nickname: saved.nickname.clone(),
                    username: saved.nickname.clone(),
                    pic: saved.pic.clone(),
                    vip_type: saved.vip_type,
                })
            } else {
                None
            };

            // ===== 状态 =====
            // 进度条 bounds 的共享槽：canvas 写入、鼠标事件读取（必须是同一个 Arc）
            let progress_bounds = Arc::new(Mutex::new(None));
            let volume_bounds = Arc::new(Mutex::new(None));
            let volume = settings.get().volume;
            let play_mode = settings.get().play_mode;
            // 背景特效模式：ECHO_BACKDROP=system|mica|acrylic|off 可临时覆盖
            // （只影响本次运行，不写回设置文件，供自动化测试用）。
            let backdrop_mode = match std::env::var("ECHO_BACKDROP") {
                Ok(v) => windows_backdrop::parse_mode(&v).unwrap_or_else(|| settings.get().backdrop),
                Err(_) => settings.get().backdrop,
            };
            let store = cx.new(|_| AppStore {
                settings,
                backdrop_effective: backdrop_mode,
                client: client.clone(),
                engine: engine.clone(),
                player: player::PlayerState {
                    volume,
                    play_mode,
                    duration: saved_queue
                        .index
                        .and_then(|i| saved_queue.songs.get(i))
                        .map(|s| s.duration as f64)
                        .unwrap_or(0.0),
                    queue: saved_queue.songs,
                    queue_title: saved_queue.title,
                    index: saved_queue.index,
                    // 引擎是空的，点播放时先解析地址
                    loaded: false,
                    ..Default::default()
                },
                routes: Default::default(),
                home: Default::default(),
                search: Default::default(),
                playlist_detail: None,
                playlists_loaded: Default::default(),
                user_playlists: Vec::new(),
                user_playlists_loaded: false,
                search_hot: Default::default(),
                session: session_store,
                user: saved_user,
                login: Default::default(),
                explore: Default::default(),
                daily: Arc::new(Vec::new()),
                daily_loaded: false,
                daily_loading: false,
                artist_view: None,
                album_view: None,
                rank_view: Default::default(),
                artist_loading: false,
                album_loading: false,
                rank_loading: false,
                library: library_store,
                queue_store,
                audio_diag: None,
                lyric: None,
                lyric_key: String::new(),
                lyric_loading: false,
                player_menu: false,
                sleep_until: None,
                mv: None,
                mv_loading: false,
                fm: Arc::new(Vec::new()),
                fm_mode: "peak".into(),
                fm_pool: 0,
                fm_loaded: false,
                fm_loading: false,
                cloud: Arc::new(Vec::new()),
                cloud_loaded: false,
                cloud_loading: false,
                purchased_songs: Arc::new(Vec::new()),
                purchased_albums: Arc::new(Vec::new()),
                purchased_loaded: false,
                purchased_loading: false,
                purchased_tab: 0,
                remote_liked: Arc::new(Vec::new()),
                remote_liked_loaded: false,
                remote_liked_loading: false,
                remote_liked_listid: String::new(),
                remote_liked_keys: Default::default(),
                remote_liked_fileids: Default::default(),
                queue_open: false,
                volume_open: false,
                seeking: false,
                seek_target: 0.0,
                search_input: String::new(),
                toast: String::new(),
                position_at: std::time::Instant::now(),
                sidebar_collapsed: false,
                sidebar_found_open: true,
                sidebar_lib_open: true,
                sidebar_tab: 0,
                theme_anim: None,
                theme_reveal: None,
                progress_bounds_slot: progress_bounds.clone(),
                volume_bounds_slot: volume_bounds.clone(),
                queue_scroll: gpui::UniformListScrollHandle::new(),
                queue_drag: None,
                song_scroll: gpui::UniformListScrollHandle::new(),
                list_drag: None,
                queue_bounds: Arc::new(parking_lot::Mutex::new(None)),
                search_bounds: Arc::new(parking_lot::Mutex::new(None)),
                fav_search_bounds: Arc::new(parking_lot::Mutex::new(None)),
                fav_filter: String::new(),
                fav_filtered: Arc::new(Vec::new()),
                vip_claiming: false,
                vip_note: String::new(),
                session_expired: false,
                login_redirect: None,
            });

            // 默认窗口 1100x650；显示器比这小时再按 88% 收缩
            let (win_w, win_h) = match cx.primary_display() {
                Some(d) => {
                    let b = d.bounds();
                    let (sw, sh) = (f32::from(b.size.width), f32::from(b.size.height));
                    if sw >= 1250. && sh >= 740. {
                        (1100., 650.)
                    } else {
                        ((sw * 0.88).max(900.), (sh * 0.88).max(560.))
                    }
                }
                None => (1100., 650.),
            };

            // 启动引导：刷新 token → 拉用户详情（VIP 等级/昵称/头像）→ 需要时自动领取每日 VIP。
            // 领取逻辑见 `vip.rs`（VIP 过期后不领就取不到 VIP 歌曲的播放地址）。
            vip::bootstrap(&store, cx);
            // 播放中会员到期：每 60s 看一次真实到期时间，过期就自动领
            vip::start_watchdog(&store, cx);

            // 启动后加载红心歌单（listid 供收藏切换使用）
            if saved.is_valid() {
                let liked_store = store.clone();
                let liked_client = client.clone();
                let liked_task = cx
                    .background_executor()
                    .spawn(async move { ui::run_tokio(liked_client.liked_playlist()) });
                cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                    if let Ok(Some(pl)) = liked_task.await {
                        let _ = liked_store.update(cx, |s, cx| {
                            s.remote_liked_loaded = pl.listid.is_empty();
                            s.remote_liked_listid = pl.listid;
                            cx.notify();
                        });
                    }
                })
                .detach();
            }

            // 打开窗口
            let search_store = store.clone();
            let search_input = cx.new(|cx| {
                text_input::TextInput::new(
                    cx.focus_handle(),
                    "搜索音乐、歌手、专辑".into(),
                    Some(Box::new(move |text, _window, cx| {
                        let kw = text.trim().to_string();
                        if kw.is_empty() {
                            return;
                        }
                        ui::navigate(&search_store, Page::Search(kw), cx);
                    })),
                )
            });
            let search_focus = search_input.read(cx).focus();

            // 「我最喜爱」页的前端模糊搜索：输入即过滤（纯本地，不打接口）
            let fav_store = store.clone();
            let fav_search = cx.new(|cx| {
                text_input::TextInput::new(cx.focus_handle(), "在喜欢的音乐中搜索".into(), None)
                    .with_on_change(Box::new(move |text, _window, cx| {
                        let next = text.to_string();
                        fav_store.update(cx, |s, cx| {
                            if s.fav_filter == next {
                                return;
                            }
                            s.fav_filter = next;
                            s.refresh_fav_filter();
                            cx.notify();
                        });
                    }))
            });
            let fav_search_focus = fav_search.read(cx).focus();

            // 登录页输入框
            let login_user = cx.new(|cx| {
                text_input::TextInput::new(cx.focus_handle(), "手机号 / 邮箱".into(), None)
            });
            let login_pass = cx.new(|cx| {
                text_input::TextInput::new(cx.focus_handle(), "密码".into(), None)
            });
            let login_phone = cx.new(|cx| {
                text_input::TextInput::new(cx.focus_handle(), "手机号".into(), None)
            });
            let login_code = cx.new(|cx| {
                text_input::TextInput::new(cx.focus_handle(), "短信验证码".into(), None)
            });

            // 封面图 LRU 缓存（全局共用一份，跨页面共享命中）
            let cover_cache = cx.new(|_| imgcache::CoverCache::new(imgcache::DEFAULT_CAP));
            let root_cache = cover_cache.clone();

            let window = cx
                .open_window(
                    WindowOptions {
                        titlebar: None,
                        window_background: gpui::WindowBackgroundAppearance::Opaque,
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(win_w), px(win_h)),
                            cx,
                        ))),
                        window_min_size: Some(size(px(900.), px(560.))),
                        ..Default::default()
                    },
                    |_window, cx| {
                        cx.new(|cx| Root {
                            store: store.clone(),
                            root_focus: cx.focus_handle(),
                            cover_cache: root_cache.clone(),
                            search_input: search_input.clone(),
                            search_focus: search_focus.clone(),
                            fav_search: fav_search.clone(),
                            fav_search_focus: fav_search_focus.clone(),
                            login_user: login_user.clone(),
                            login_pass: login_pass.clone(),
                            login_phone: login_phone.clone(),
                            login_code: login_code.clone(),
                            progress_bounds: progress_bounds.clone(),
                            volume_bounds: volume_bounds.clone(),
                            content_scroll: ScrollHandle::new(),
                            sidebar_scroll: ScrollHandle::new(),
                            lyric_scroll: UniformListScrollHandle::new(),
                            lyric_follow: None,
                            lyric_scroll_anim: None,
                            lyric_user_scroll_at: None,
                            lyric_restore_pending: false,
                            lyric_auto_off: 0.0,
                            window_title: String::new(),
                        })
                    },
                )
                .expect("open window");

            // ===== 窗口图标 / 背景特效 / 关闭即隐藏 / 系统托盘 =====
            // 主窗口 HWND 共享给托盘线程（托盘线程只转发命令，窗口操作仍在 UI 线程）
            let main_hwnd = Arc::new(std::sync::atomic::AtomicIsize::new(0));
            {
                let slot = main_hwnd.clone();
                let _ = window.update(cx, |root, window, cx| {
                    win::apply_window_icon(window);
                    if let Some(h) = win::hwnd(window) {
                        slot.store(h as isize, std::sync::atomic::Ordering::Relaxed);
                    }
                    // 背景特效（Mica / Acrylic / 磨砂降级），设置项在「设置 → 外观」
                    windows_backdrop::apply(window, backdrop_mode);
                    // 键盘默认落点：没有输入框时由根节点接住（空格播放/暂停）
                    window.focus(&root.root_focus);
                    // 关闭按钮 / Alt+F4 只隐藏窗口，真正退出走托盘菜单
                    window.on_window_should_close(cx, |window, _| {
                        win::hide_window(window);
                        false
                    });
                });
            }
            // ===== SMTC 系统媒体控制（任务栏悬浮卡片 / 蓝牙耳机播放控件）=====
            // 按键事件经托盘事件通道回主线程，与托盘菜单共用处理逻辑
            crate::smtc::start(main_hwnd.load(std::sync::atomic::Ordering::Relaxed));

            {
                let (tray_tx, tray_rx) = std::sync::mpsc::channel::<win::TrayEvent>();
                win::start_tray(main_hwnd.clone(), tray_tx);
                let tray_store = store.clone();
                let tray_win = window;
                cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                    loop {
                        smol::Timer::after(std::time::Duration::from_millis(250)).await;
                        loop {
                            match tray_rx.try_recv() {
                                Ok(win::TrayEvent::Show) => {
                                    let _ = tray_win.update(cx, |_, window, cx| {
                                        window.activate_window();
                                        cx.notify();
                                    });
                                }
                                Ok(win::TrayEvent::TogglePlay) => {
                                    let _ = tray_win.update(cx, |_, _, cx| {
                                        ui::toggle_play(&tray_store, cx);
                                    });
                                }
                                Ok(win::TrayEvent::Prev) => {
                                    let _ = tray_win.update(cx, |_, _, cx| {
                                        ui::prev_track(&tray_store, cx);
                                    });
                                }
                                Ok(win::TrayEvent::Next) => {
                                    let _ = tray_win.update(cx, |_, _, cx| {
                                        ui::next_track(&tray_store, false, cx);
                                    });
                                }
                                Ok(win::TrayEvent::Quit) => {
                                    let _ = cx.update(|app| app.quit());
                                    return;
                                }
                                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                            }
                        }
                    }
                })
                .detach();
            }

            // ===== 任务栏缩略图工具栏（悬停任务栏按钮 → 上一曲/播放暂停/下一曲/收藏）=====
            // 点击经子类化拦 WM_COMMAND 后走 mpsc 回来，与托盘菜单同一套动作
            {
                let (tb_tx, tb_rx) = std::sync::mpsc::channel::<taskbar::TaskbarEvent>();
                taskbar::init(
                    main_hwnd.load(std::sync::atomic::Ordering::Relaxed),
                    tb_tx,
                );
                let tb_store = store.clone();
                let tb_win = window;
                cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                    loop {
                        smol::Timer::after(std::time::Duration::from_millis(120)).await;
                        loop {
                            match tb_rx.try_recv() {
                                Ok(taskbar::TaskbarEvent::Prev) => {
                                    let _ = tb_win.update(cx, |_, _, cx| {
                                        ui::prev_track(&tb_store, cx);
                                    });
                                }
                                Ok(taskbar::TaskbarEvent::TogglePlay) => {
                                    let _ = tb_win.update(cx, |_, _, cx| {
                                        ui::toggle_play(&tb_store, cx);
                                    });
                                }
                                Ok(taskbar::TaskbarEvent::Next) => {
                                    let _ = tb_win.update(cx, |_, _, cx| {
                                        ui::next_track(&tb_store, false, cx);
                                    });
                                }
                                Ok(taskbar::TaskbarEvent::ToggleFav) => {
                                    let _ = tb_win.update(cx, |_, _, cx| {
                                        let song =
                                            tb_store.read(cx).player.current().cloned();
                                        if let Some(song) = song {
                                            crate::liked::toggle_like(&tb_store, &song, cx);
                                        }
                                    });
                                }
                                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                            }
                        }
                    }
                })
                .detach();
            }

            // 全局句柄
            cx.set_global(AppGlobal {                store: store.downgrade(),
                main_window: Some(window.into()),
            });

            // 按设置恢复桌面歌词（浮窗之前只能从设置页手动开，重启即丢）
            if store.read(cx).settings.get().desktop_lyric {
                crate::desktop::toggle_desktop_lyric(&store, cx);
            }

            // ===== 播放器事件桥接（std mpsc → smol channel → 主线程） =====
            let (ui_tx, ui_rx) = smol::channel::unbounded::<PlayerEvent>();
            std::thread::Builder::new()
                .name("player-event-bridge".into())
                .spawn(move || {
                    for ev in ev_rx {
                        if smol::block_on(ui_tx.send(ev)).is_err() {
                            break;
                        }
                    }
                })
                .ok();
            let event_store = store.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                while let Ok(ev) = ui_rx.recv().await {
                    let to_load = event_store
                        .update(cx, |s, cx| ui::handle_player_event_state(s, ev, cx))
                        .ok()
                        .flatten();
                    // SMTC 同步：任务栏/蓝牙媒体控件跟随播放状态
                    let _ = event_store.read_with(cx, |s, _| {
                        if let Some(song) = s.player.current() {
                            crate::smtc::sync(
                                &song.title,
                                &song.artist,
                                &song.cover_url,
                                s.player.playing,
                                s.player.position,
                                s.player.duration,
                            );
                        }
                    });
                    if let Some((rid, song)) = to_load {
                        let _ = cx.update(|app| ui::spawn_resolve(&event_store, rid, song, app));
                    }
                    // 桌面歌词 / mini 播放器是独立窗口，进度变了要跟着重绘
                    let _ = cx.update(|app| crate::desktop::refresh_float_windows(app));
                }
            })
            .detach();

            // ===== 全局媒体键（播放/暂停、上一首、下一首、停止） =====
            {
                let (key_tx, key_rx) = std::sync::mpsc::channel::<win::MediaKey>();
                let key_rx = std::sync::Arc::new(std::sync::Mutex::new(key_rx));
                win::start_media_key_listener(key_tx);
                let key_store = store.clone();
                cx.spawn(async move |cx: &mut AsyncApp| {
                    loop {
                        let krx = key_rx.clone();
                        let key = smol::unblock(move || krx.lock().ok().and_then(|r| r.recv().ok())).await;
                        let Some(key) = key else { break };
                        let _ = cx.update(|app| {
                            key_store.update(app, |s, cx| {
                                s.toast = match key {
                                    win::MediaKey::Toggle => {
                                        ui::toggle_play(&key_store, cx);
                                        "媒体键：播放/暂停".to_string()
                                    }
                                    win::MediaKey::Next => {
                                        ui::next_track(&key_store, false, cx);
                                        "媒体键：下一首".to_string()
                                    }
                                    win::MediaKey::Prev => {
                                        ui::prev_track(&key_store, cx);
                                        "媒体键：上一首".to_string()
                                    }
                                    win::MediaKey::Stop => {
                                        s.engine.pause();
                                        s.player.playing = false;
                                        "媒体键：停止".to_string()
                                    }
                                };
                                cx.notify();
                            });
                        });
                    }
                })
                .detach();
            }

            // 第三期：登录后拉取自建歌单（侧栏显示 + 新建/删除）
            crate::views::load_user_playlists(&store, cx);

            // 默认进入首页
            ui::navigate(&store, Page::Home, cx);

            // 动画泵：加载态（旋转）/ 停在歌词页或开着桌面歌词且在播放时（AMLL 扫光 ~30fps）重绘
            {
                let pump_store = store.clone();
                cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                    loop {
                        let (loading, lyric_anim) = pump_store
                            .read_with(cx, |s, _| {
                                let on_lyric =
                                    matches!(s.routes.current(), Some(crate::state::Page::Lyric));
                                (
                                    s.any_loading(),
                                    s.player.playing
                                        && (on_lyric || crate::desktop::lyric_window_open()),
                                )
                            })
                            .unwrap_or((false, false));
                        if loading || lyric_anim {
                            let _ = pump_store.update(cx, |_, cx| cx.notify());
                            if lyric_anim {
                                // 桌面歌词浮窗跟着刷（独立窗口不吃主窗口重绘）
                                let _ = cx.update(|app| crate::desktop::refresh_float_windows(app));
                            }
                        }
                        smol::Timer::after(std::time::Duration::from_millis(
                            if loading || lyric_anim { 33 } else { 300 },
                        ))
                        .await;
                    }
                })
                .detach();
            }

            // 自检钩子：ECHO_SMOKE=search|playlist 时自动跳转，便于自动化截图验证
            if let Ok(target) = std::env::var("ECHO_SMOKE") {
                let smoke_store = store.clone();
                cx.spawn(async move |cx: &mut AsyncApp| {
                    smol::Timer::after(std::time::Duration::from_secs(4)).await;
                    let page = match target.as_str() {
                        "explore" => Page::Explore,
                        "daily" => Page::DailyRecommend,
                        "ranking" => Page::Ranking,
                        "rank-top" => Page::Rank("2".into(), "TOP500".into()),
                        "album" => Page::Album("212206631".into(), "秋风一吹又老一岁".into()),
                        "search-artist" | "search-album" => Page::Search("周杰伦".into()),
                        // 搜索补全冒烟：进搜索页后自动切 歌词(4)/MV(5) tab
                        "search-lyric" | "search-mv" => Page::Search("晴天".into()),
                        "settings" => Page::Settings,
                        // ECHO_SMOKE=backdrop-mica|acrylic|system|off：
                        // 进设置页后直接调用与点击按钮相同的生产路径切换背景特效，
                        // 供自动化读回验证（绕过合成点击，真实用户仍走按钮点击）
                        t if t.starts_with("backdrop-") => Page::Settings,
                        "cloud" => Page::Cloud,
                        "fm" => Page::Fm,
                        "lyric" => Page::Lyric,
                        "purchased" => Page::Purchased,
                        "profile" => Page::Profile,
                        "login" => Page::Login,
                        "login-sms" => {
                            let _ = smoke_store.update(cx, |s, _| {
                                s.login.method = state::LoginMethod::Sms;
                            });
                            Page::Login
                        }
                        "login-password" => {
                            let _ = smoke_store.update(cx, |s, _| {
                                s.login.method = state::LoginMethod::Password;
                            });
                            Page::Login
                        }
                        "favorites" => Page::Favorites,
                        "collapsed" => {
                            let _ = smoke_store.update(cx, |s, _| {
                                s.sidebar_collapsed = true;
                            });
                            Page::Home
                        }
                        "history" => Page::History,
                        "search" => Page::Search("周杰伦".into()),
                        "playlist" => {
                            let pid = smoke_store
                                .read_with(cx, |s, _| {
                                    s.home.recommend_playlists.first().map(|p| (p.id.clone(), p.name.clone()))
                                })
                                .ok()
                                .flatten();
                            match pid {
                                Some((id, name)) => Page::Playlist(id, name),
                                None => Page::Explore,
                            }
                        }
                        _ => Page::Home,
                    };
                    println!("[smoke] target={target} page={page:?}");
                    let _ = cx.update(|app| ui::navigate(&smoke_store, page, app));

                    // ECHO_SMOKE=search-artist|search-album：搜索后切到对应 Tab
                    if target == "search-artist" || target == "search-album" {
                        let tab = if target == "search-artist" { 2 } else { 3 };
                        smol::Timer::after(std::time::Duration::from_secs(3)).await;
                        let _ = cx.update(|app| {
                            ui::load_search_tab(&smoke_store, "周杰伦", tab, app);
                        });
                    }

                    // ECHO_SMOKE=search-lyric|search-mv：搜索补全（歌词/MV tab）
                    if target == "search-lyric" || target == "search-mv" {
                        let tab = if target == "search-lyric" { 4 } else { 5 };
                        smol::Timer::after(std::time::Duration::from_secs(3)).await;
                        let _ = cx.update(|app| {
                            ui::load_search_tab(&smoke_store, "晴天", tab, app);
                        });
                    }

                    // ECHO_SMOKE=queue：跳搜索页 → 自动播放第一首 → 打开队列面板（验证队列渲染）
                    if target == "queue" {
                        let _ = cx.update(|app| ui::navigate(&smoke_store, Page::Search("晴天".into()), app));
                        smol::Timer::after(std::time::Duration::from_secs(4)).await;
                        let _ = cx.update(|app| {
                            let n = smoke_store.read_with(app, |s, _| s.search.songs.len());
                            log::warn!("[smoke-queue] search songs n={n}");
                            if n > 0 {
                                ui::play_from(&smoke_store, ui::PlaySource::Search, 0, app);
                            }
                        });
                        smol::Timer::after(std::time::Duration::from_secs(3)).await;
                        let _ = smoke_store.update(cx, |s, cx| {
                            log::warn!("[smoke-queue] queue len={} index={:?} open=true", s.player.queue.len(), s.player.index);
                            s.queue_open = true;
                            cx.notify();
                        });
                    }

                    // ECHO_SMOKE=mini：打开 mini 播放器（自动化验证浮窗）
                    if target == "mini" {
                        smol::Timer::after(std::time::Duration::from_secs(2)).await;
                        let _ = cx.update(|app| {
                            crate::desktop::open_mini_player(&smoke_store, app);
                        });
                    }

                    // ECHO_SMOKE=devswitch：播放一首歌 → 运行中切换输出设备，
                    // 验证不断流、从原位置继续（日志看「输出设备已切换」）
                    if target == "devswitch" {
                        let _ = cx
                            .update(|app| ui::navigate(&smoke_store, Page::Search("晴天".into()), app));
                        smol::Timer::after(std::time::Duration::from_secs(4)).await;
                        let _ = cx.update(|app| {
                            ui::play_from(&smoke_store, ui::PlaySource::Search, 0, app)
                        });
                        smol::Timer::after(std::time::Duration::from_secs(8)).await;
                        let _ = smoke_store.update(cx, |s, _| {
                            s.engine.switch_device(None); // 回系统默认（重建输出流）
                        });
                    }

                    // ECHO_SMOKE=theme：走与侧边栏按钮相同的 set_theme_mode 触发一次
                    // 深浅切换（合成鼠标点不到侧边栏底部的小图标，这里直接调生产路径）
                    if target == "theme" {
                        smol::Timer::after(std::time::Duration::from_secs(2)).await;
                        let _ = cx.update(|app| {
                            // 切到「跟当前相反」的模式：设置会落盘，写死 Dark 的话
                            // 第二次跑就命中「模式相同直接返回」，动画不会再触发
                            let cur = smoke_store.read(app).settings.get().theme_mode;
                            let next = if cur == echo_core::ThemeMode::Dark {
                                echo_core::ThemeMode::Light
                            } else {
                                echo_core::ThemeMode::Dark
                            };
                            ui::set_theme_mode(
                                &smoke_store,
                                next,
                                Some(gpui::point(px(166.), px(523.))),
                                app,
                            );
                        });
                    }

                    // ECHO_SMOKE=backdrop-*：走与设置页按钮相同的 set_backdrop_mode
                    if let Some(mode_str) = target.strip_prefix("backdrop-") {
                        if let Some(mode) = windows_backdrop::parse_mode(mode_str) {
                            smol::Timer::after(std::time::Duration::from_secs(2)).await;
                            let _ = cx.update(|app| {
                                ui::set_backdrop_mode(&smoke_store, mode, app);
                            });
                        }
                    }
                })
                .detach();
            }

            window
                .update(cx, |_root, _window, cx| {
                    cx.activate(true);
                })
                .ok();
        });
}
