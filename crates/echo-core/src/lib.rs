//! EchoMusic 核心库：数据模型、设备身份、应用设置。

pub mod device;
pub mod library;
pub mod models;
pub mod queue;
pub mod session;
pub mod settings;

pub use device::DeviceIdentity;
pub use library::{Library, LibraryStore, PlayLog};
pub use queue::{QueueStore, SavedQueue};
pub use models::{
    Lyric, LyricLine, LyricWord, Playlist, Song,
    SongArtist, User,
};
pub use session::{Session, SessionStore};
pub use settings::{PlayMode, Settings, SettingsStore, ThemeMode};

use std::path::PathBuf;

/// 应用数据目录：`%APPDATA%/EchoMusic`。
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("EchoMusic")
}
