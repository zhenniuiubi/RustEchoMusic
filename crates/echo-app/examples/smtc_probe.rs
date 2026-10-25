//! 系统侧 SMTC 会话探针：读 GlobalSystemMediaTransportControlsSessionManager，
//! 验证 EchoMusic 注册的媒体控件元数据（标题/歌手/状态）。
//! 用法：target/release/examples/smtc_probe.exe
#![cfg(windows)]

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager,
};

fn main() -> anyhow::Result<()> {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }
    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?
        .get()?;
    let sessions = mgr.GetSessions()?;
    println!("sessions={}", sessions.Size()?);
    for s in &sessions {
        let app = s.SourceAppUserModelId()?;
        let props = s.TryGetMediaPropertiesAsync()?.get()?;
        let status = s.GetPlaybackInfo()?.PlaybackStatus()?;
        println!(
            "app={app} status={status:?} title={} artist={}",
            props.Title()?,
            props.Artist()?
        );
    }
    Ok(())
}

// windows future 的 get() 在 windows crate 里是 windows_future
mod _reexport {}
