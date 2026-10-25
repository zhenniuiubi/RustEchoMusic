//! Windows SMTC（System Media Transport Controls）系统媒体控制。
//!
//! 任务栏悬浮卡片、音量浮层媒体控件、蓝牙耳机/键盘的播放按键都走这套协议。
//! gpui 拿不到 WndProc，但 SMTC 提供 `ISystemMediaTransportControlsInterop::
//! GetForWindow(hwnd)`，只需要 HWND（`win::hwnd` 已有），不依赖 WndProc。
//!
//! 线程模型：专用 "smtc" 线程持有全部 WinRT 对象（先 `CoInitializeEx`）；
//! UI 侧通过 `sync()` 往通道里塞快照（4Hz 进度事件也直接塞，线程内去重）；
//! 按键事件从 WinRT 线程经托盘事件通道（`win::send`）回到主线程，
//! 与托盘菜单共用一套播放控制处理。
//!
//! 封面：按 URL 拉一次字节（阻塞 HTTP，仅在本线程），写入内存流交给
//! DisplayUpdater；失败就没有缩略图，不影响其他元数据。

#![cfg(windows)]
#![allow(unsafe_op_in_unsafe_fn)]

use std::sync::mpsc::{Receiver, Sender};
use std::sync::OnceLock;
use windows::core::{factory, HSTRING, Result};
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
    SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
    SystemMediaTransportControlsDisplayUpdater, SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;

/// UI → SMTC 线程的更新（线程内做去重/节流）。
pub enum SmtcUpdate {
    Track {
        title: String,
        artist: String,
        pic: String,
    },
    Status {
        playing: bool,
    },
    Position {
        pos: f64,
        duration: f64,
    },
}

static SMTC_TX: OnceLock<Sender<SmtcUpdate>> = OnceLock::new();

/// 播放器状态同步（任何 PlayerEvent 后调用；未初始化时是 no-op）。
pub fn sync(title: &str, artist: &str, pic: &str, playing: bool, pos: f64, duration: f64) {
    let Some(tx) = SMTC_TX.get() else { return };
    let _ = tx.send(SmtcUpdate::Track {
        title: title.to_string(),
        artist: artist.to_string(),
        pic: pic.to_string(),
    });
    let _ = tx.send(SmtcUpdate::Status { playing });
    let _ = tx.send(SmtcUpdate::Position {
        pos,
        duration,
    });
}

/// 启动 SMTC 线程。`hwnd` 是主窗口句柄（进程内共享槽位的值）。
pub fn start(hwnd: isize) {
    let (tx, rx) = std::sync::mpsc::channel::<SmtcUpdate>();
    if SMTC_TX.set(tx).is_err() {
        return; // 已启动
    }
    let spawned = std::thread::Builder::new()
        .name("smtc".into())
        .spawn(move || {
            // WinRT 工厂激活需要先初始化 COM apartment
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_MULTITHREADED,
                );
            }
            match SmtcSession::new(hwnd) {
                Ok(s) => s.run(rx),
                Err(e) => log::warn!("SMTC 初始化失败（系统媒体控件不可用）: {e}"),
            }
        });
    if let Err(e) = spawned {
        log::warn!("SMTC 线程启动失败: {e}");
    }
}

struct SmtcSession {
    ctl: SystemMediaTransportControls,
    updater: SystemMediaTransportControlsDisplayUpdater,
    /// 封面字节缓存（按 URL）
    cover_url: String,
    cover_bytes: Option<Vec<u8>>,
    http: reqwest::blocking::Client,
}

impl SmtcSession {
    fn new(hwnd: isize) -> Result<Self> {
        let interop: ISystemMediaTransportControlsInterop =
            factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
        let ctl: SystemMediaTransportControls =
            unsafe { interop.GetForWindow::<SystemMediaTransportControls>(HWND(hwnd as *mut _))? };
        ctl.SetIsEnabled(true)?;
        ctl.SetIsPlayEnabled(true)?;
        ctl.SetIsPauseEnabled(true)?;
        ctl.SetIsNextEnabled(true)?;
        ctl.SetIsPreviousEnabled(true)?;

        let updater = ctl.DisplayUpdater()?;
        updater.SetType(MediaPlaybackType::Music)?;

        // 按键事件：蓝牙耳机 / 键盘媒体键 → 托盘事件通道（与托盘菜单共用处理）
        ctl.ButtonPressed(&TypedEventHandler::new(
            move |_sender: windows::core::Ref<'_, SystemMediaTransportControls>,
                  args: windows::core::Ref<'_, SystemMediaTransportControlsButtonPressedEventArgs>|
                  -> Result<()> {
                let args = args.ok()?;
                let event = match args.Button()? {
                    SystemMediaTransportControlsButton::Play
                    | SystemMediaTransportControlsButton::Pause => {
                        crate::win::TrayEvent::TogglePlay
                    }
                    SystemMediaTransportControlsButton::Next => crate::win::TrayEvent::Next,
                    SystemMediaTransportControlsButton::Previous => crate::win::TrayEvent::Prev,
                    _ => return Ok(()),
                };
                crate::win::send(event);
                Ok(())
            },
        ))?;

        log::info!("SMTC 已就绪（任务栏/蓝牙媒体控件）");
        Ok(Self {
            ctl,
            updater,
            cover_url: String::new(),
            cover_bytes: None,
            http: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        })
    }

    fn run(mut self, rx: Receiver<SmtcUpdate>) {
        let mut last_track = String::new();
        let mut last_playing: Option<bool> = None;
        while let Ok(up) = rx.recv() {
            match up {
                SmtcUpdate::Track { title, artist, pic } => {
                    let key = format!("{title}\u{1}{artist}\u{1}{pic}");
                    if key == last_track {
                        continue;
                    }
                    last_track = key;
                    let _ = self.updater.SetType(MediaPlaybackType::Music);
                    // 标题/歌手在 MusicDisplayProperties 上
                    if let Ok(props) = self.updater.MusicProperties() {
                        let _ = props.SetTitle(&HSTRING::from(title.as_str()));
                        let _ = props.SetArtist(&HSTRING::from(artist.as_str()));
                    }
                    if self.cover_url != pic {
                        self.cover_url = pic.clone();
                        self.cover_bytes = self.fetch_cover(&pic);
                    }
                    if let Some(bytes) = &self.cover_bytes {
                        match self.thumbnail(bytes) {
                            Ok(thumb) => {
                                let _ = self.updater.SetThumbnail(&thumb);
                            }
                            Err(e) => log::debug!("SMTC 封面缩略图失败: {e}"),
                        }
                    }
                    let _ = self.updater.Update();
                }
                SmtcUpdate::Status { playing } => {
                    if last_playing == Some(playing) {
                        continue;
                    }
                    last_playing = Some(playing);
                    let _ = self.ctl.SetPlaybackStatus(if playing {
                        MediaPlaybackStatus::Playing
                    } else {
                        MediaPlaybackStatus::Paused
                    });
                }
                SmtcUpdate::Position { pos, duration } => {
                    // TimelineProperties 是运行时类：new() + Set 逐项赋值
                    let n = |secs: f64| TimeSpan {
                        Duration: (secs.max(0.0) * 10_000_000.0) as i64,
                    };
                    match SystemMediaTransportControlsTimelineProperties::new() {
                        Ok(props) => {
                            let _ = props.SetStartTime(n(0.0));
                            let _ = props.SetEndTime(n(duration));
                            let _ = props.SetMinSeekTime(n(0.0));
                            let _ = props.SetMaxSeekTime(n(duration));
                            let _ = props.SetPosition(n(pos));
                            let _ = self.ctl.UpdateTimelineProperties(&props);
                        }
                        Err(e) => log::debug!("SMTC 时间轴失败: {e}"),
                    }
                }
            }
        }
    }

    /// 拉取封面字节（失败返回 None，不重试同 URL）。
    fn fetch_cover(&mut self, url: &str) -> Option<Vec<u8>> {
        if url.is_empty() {
            return None;
        }
        match self.http.get(url).send() {
            Ok(resp) if resp.status().is_success() => match resp.bytes() {
                Ok(b) if !b.is_empty() && b.len() < 8 * 1024 * 1024 => Some(b.to_vec()),
                _ => None,
            },
            _ => None,
        }
    }

    /// 字节 → SMTC 缩略图（写临时文件 → file:// Uri → RandomAccessStreamReference）。
    fn thumbnail(&self, bytes: &[u8]) -> Result<RandomAccessStreamReference> {
        let path = std::env::temp_dir().join("EchoMusic-smtc-cover.img");
        std::fs::write(&path, bytes)
            .map_err(|_| windows::core::Error::from_hresult(windows::core::HRESULT(1)))?;
        // file:/// URI（反斜杠换正斜杠）
        let uri_str = format!(
            "file:///{}",
            path.to_string_lossy().replace('\\', "/")
        );
        let uri = windows::Foundation::Uri::CreateUri(&HSTRING::from(uri_str))?;
        RandomAccessStreamReference::CreateFromUri(&uri)
    }
}
