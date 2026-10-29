//! EchoMusic 桌面应用入口（gpui）。

// release 构建不弹出控制台窗口；debug 构建保留，方便看日志
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod icons;
mod liked;
mod lite;
mod player;
mod state;
mod theme;
mod smtc;
mod taskbar;
mod ui;
mod actions;
mod vip;
mod win;

use assets::{Assets, ReqwestHttpClient};
use echo_audio::{AudioEngine, PlayerEvent};
use echo_core::{DeviceIdentity, SettingsStore};
use gpui::prelude::*;
use gpui::*;
use state::AppStore;
use std::sync::Arc;

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
            // ===== 数据初始化 =====
            let data_dir = echo_core::data_dir();
            let settings = SettingsStore::load(&data_dir).expect("load settings");
            let device = DeviceIdentity::load_or_create(&data_dir).expect("load device identity");
            // 标准版客户端（appid 1005 / clientver 20489）：
            // 登录态接口（get_all_list / get_my_info）与歌曲地址 key 签名都只认标准版。
            let client = echo_api::KuGouClient::shared(device, false).expect("init kugou client");

            // 设备注册（后台）
            let reg_client = client.clone();
            let reg_dir = data_dir.clone();
            cx.background_executor()
                .spawn(async move {
                    match ui::run_tokio(reg_client.register_device()) {
                        Ok(dfid) => {
                            log::info!("device registered: {dfid}");
                            // 落盘：登录态是绑设备的，下次启动要拿同一个 dfid 出现，
                            // 不然每次换一个「新设备」，服务端那边看着像异地登录
                            if let Ok(mut dev) = echo_core::DeviceIdentity::load_or_create(&reg_dir) {
                                if dev.dfid != dfid {
                                    let _ = dev.set_dfid(&dfid, &reg_dir);
                                }
                            }
                        }
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
            let mut saved_queue = queue_store.get();
            // 快照里存的是歌的完整数据，包括封面 URL。老版 `normalize_cover` 只认
            // `{size}` 模板，酷狗那种「尺寸烧在路径里」的 URL（.../stdmusic/120/...）
            // 一直没被升级 —— 磁盘上的旧快照会让播放页永远拿 120 的缩略图（糊）。
            // 这里按当前规则重算一遍，老快照也能吃到修复。
            for s in saved_queue.songs.iter_mut() {
                s.cover_url = echo_api::normalize::normalize_cover(&s.cover_url, 120);
                s.cover_url_hd = echo_api::normalize::normalize_cover(&s.cover_url_hd, 480);
            }
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
                // 续期凭证也要还原：少了它 `/v5/login_by_token` 续不上，
                // token 一到期就只能重新登录
                client.set_t1(&saved.t1);
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
            let volume = settings.get().volume;
            let play_mode = settings.get().play_mode;
            let store = cx.new(|_| AppStore {
                settings,
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
                session: session_store,
                user: saved_user,
                login: Default::default(),
                library: library_store,
                queue_store,
                audio_diag: None,
                lyric: None,
                lyric_key: String::new(),
                lyric_loading: false,
                fm: Arc::new(Vec::new()),
                fm_mode: "peak".into(),
                fm_pool: 0,
                fm_loaded: false,
                fm_loading: false,
                fm_cur_mark: 0,
                fm_extending: false,
                fm_batch: Vec::new(),
                fm_heard: Default::default(),
                remote_liked: Arc::new(Vec::new()),
                remote_liked_loaded: false,
                remote_liked_loading: false,
                remote_liked_listid: String::new(),
                remote_liked_keys: Default::default(),
                remote_liked_fileids: Default::default(),
                seeking: false,
                seek_target: 0.0,
                toast: String::new(),
                position_at: std::time::Instant::now(),
                vip_claiming: false,
                vip_note: String::new(),
                session_expired: false,
                token_refresh_at: 0,
            });

            // 默认窗口 1100x650；显示器比这小时再按 88% 收缩
            let (_win_w, _win_h) = match cx.primary_display() {
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
            let _search_store = store.clone();
            // Lite 版：这个 app 只有一条 330×70 的迷你播放条（参考原型即主体）
            let window = crate::lite::open(&store, cx).expect("open window");

            // ===== 窗口图标 / 背景特效 / 关闭即隐藏 / 系统托盘 =====
            // 主窗口 HWND 共享给托盘线程（托盘线程只转发命令，窗口操作仍在 UI 线程）
            let main_hwnd = Arc::new(std::sync::atomic::AtomicIsize::new(0));
            {
                let slot = main_hwnd.clone();
                let _ = window.update(cx, |_, window, cx| {
                    win::apply_window_icon(window);
                    if let Some(h) = win::hwnd(window) {
                        slot.store(h as isize, std::sync::atomic::Ordering::Relaxed);
                    }
                    // 关闭按钮 / Alt+F4 只收起到托盘，真正退出走托盘菜单
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
                                    // 从托盘把播放条叫回来：先 show 再激活
                                    let _ = tray_win.update(cx, |_, window, _| {
                                        window.activate_window();
                                        window.refresh();
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
                    // Lite 播放条是独立窗口，进度变了要跟着重绘
                    let _ = cx.update(|app| crate::lite::refresh(app));
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

            // 动画泵：有加载态（旋转圈）时按 30fps 重绘，其余时候慢速轮询
            {
                let pump_store = store.clone();
                cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                    loop {
                        let loading = pump_store
                            .read_with(cx, |s, _| s.any_loading())
                            .unwrap_or(false);
                        if loading {
                            let _ = pump_store.update(cx, |_, cx| cx.notify());
                            let _ = cx.update(|app| crate::lite::refresh(app));
                        }
                        smol::Timer::after(std::time::Duration::from_millis(if loading {
                            33
                        } else {
                            300
                        }))
                        .await;
                    }
                })
                .detach();
            }

            // 自检钩子（Lite 只留两条：开箱即播 / 切主题）
            //   ECHO_SMOKE=play|fm  → 加载私人 FM 并播第一首（截图/自检用）
            //   ECHO_SMOKE=theme    → 走生产路径切一次深浅色
            if let Ok(target) = std::env::var("ECHO_SMOKE") {
                let smoke_store = store.clone();
                cx.spawn(async move |cx: &mut AsyncApp| {
                    if target == "play" || target == "fm" {
                        smol::Timer::after(std::time::Duration::from_secs(3)).await;
                        let _ = cx.update(|app| crate::actions::load_fm(&smoke_store, true, app));
                        smol::Timer::after(std::time::Duration::from_secs(4)).await;
                        let _ = cx.update(|app| {
                            let n = smoke_store.read_with(app, |s, _| s.fm.len());
                            log::warn!("[smoke-play] fm songs n={n}");
                            if n > 0 {
                                ui::play_from(&smoke_store, ui::PlaySource::Fm, 0, app);
                            }
                        });
                        smol::Timer::after(std::time::Duration::from_secs(3)).await;
                    }

                    if target == "theme" {
                        smol::Timer::after(std::time::Duration::from_secs(2)).await;
                        let _ = cx.update(|app| {
                            // 切到「跟当前相反」的模式：写死 Dark 的话第二次跑会命中
                            // 「模式相同直接返回」，动画不再触发
                            let cur = smoke_store.read(app).settings.get().theme_mode;
                            let next = if cur == echo_core::ThemeMode::Dark {
                                echo_core::ThemeMode::Light
                            } else {
                                echo_core::ThemeMode::Dark
                            };
                            ui::set_theme_mode(&smoke_store, next, app);
                        });
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
