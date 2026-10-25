//! 应用设置：内存模型 + JSON 持久化。
//!
//! 对齐原项目 `stores/setting.ts`（第一期保留核心字段，后续按期扩展）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 播放模式（对齐 playMode）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayMode {
    /// 顺序播放
    Sequential,
    /// 列表循环（默认）
    #[default]
    List,
    /// 随机播放
    Random,
    /// 单曲循环
    Single,
}

impl PlayMode {
    pub fn label(&self) -> &'static str {
        match self {
            PlayMode::Sequential => "顺序播放",
            PlayMode::List => "列表循环",
            PlayMode::Random => "随机播放",
            PlayMode::Single => "单曲循环",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            PlayMode::Sequential => PlayMode::List,
            PlayMode::List => PlayMode::Random,
            PlayMode::Random => PlayMode::Single,
            PlayMode::Single => PlayMode::Sequential,
        }
    }
}

/// 主题模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    System,
}

/// 主窗口背景特效（Windows Mica 云母 / Desktop Acrylic 亚克力）。
///
/// 具体落在哪种系统材质由 `echo-app` 的 `windows_backdrop` 模块按系统版本
/// 决定（Win11 22H2+ 走 DWM 系统背景 API，更老的系统降级磨砂或关闭）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackdropMode {
    /// 跟随系统（Win11 22H2+ 由 DWM 按系统设置决定；其余系统降级为磨砂）
    #[default]
    System,
    /// 云母（Mica）
    Mica,
    /// 亚克力（Desktop Acrylic）
    Acrylic,
    /// 关闭特效（普通不透明背景）
    Off,
}

impl BackdropMode {
    pub fn label(&self) -> &'static str {
        match self {
            BackdropMode::System => "跟随系统",
            BackdropMode::Mica => "云母 (Mica)",
            BackdropMode::Acrylic => "亚克力 (Acrylic)",
            BackdropMode::Off => "关闭特效",
        }
    }

    /// 设置页展示用的简短标签。
    pub fn short_label(&self) -> &'static str {
        match self {
            BackdropMode::System => "跟随系统",
            BackdropMode::Mica => "云母",
            BackdropMode::Acrylic => "亚克力",
            BackdropMode::Off => "关闭",
        }
    }
}

/// 应用设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme_mode: ThemeMode,
    /// 主题色（hex，如 "#f55e55"）
    pub theme_color: String,
    /// 音量 0.0 - 1.0
    pub volume: f64,
    pub play_mode: PlayMode,
    pub sidebar_collapsed: bool,
    pub search_history: Vec<String>,
    /// 上次播放曲目 key
    pub last_track_key: Option<String>,
    /// 默认音质：128 / 320 / flac / high
    pub audio_quality: String,
    /// 语言（第一期固定 zh-CN）
    pub language: String,
    /// 最近一次「每日 VIP」领取尝试的日期（yyyy-mm-dd），同一天不重复打接口
    pub vip_claim_date: String,
    /// 上面那次尝试是否成功
    pub vip_claim_ok: bool,
    /// 最近一次领取「尝试」的时间（unix 秒），用于失败后退避、避免刷接口
    pub vip_claim_try_ts: i64,
    /// 播放倍速（0.5–2.0，1.0 为原速）
    pub speed: f64,
    /// 睡眠定时器（分钟，0 = 关闭）
    pub sleep_minutes: i64,
    /// 音频输出设备名（空 = 系统默认）
    pub output_device: String,
    /// 是否显示桌面歌词窗口
    pub desktop_lyric: bool,
    /// 桌面歌词窗口位置（x, y；None 时自动居中贴底）
    pub desktop_lyric_pos: Option<(f64, f64)>,
    /// 主窗口背景特效模式（Windows Mica / Acrylic）
    pub backdrop: BackdropMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::System,
            theme_color: "#f55e55".into(),
            volume: 0.8,
            play_mode: PlayMode::List,
            sidebar_collapsed: false,
            search_history: Vec::new(),
            last_track_key: None,
            audio_quality: "128".into(),
            language: "zh-CN".into(),
            vip_claim_date: String::new(),
            vip_claim_ok: false,
            vip_claim_try_ts: 0,
            speed: 1.0,
            sleep_minutes: 0,
            output_device: String::new(),
            desktop_lyric: false,
            desktop_lyric_pos: None,
            backdrop: BackdropMode::default(),
        }
    }
}

/// 设置存储：线程安全的内存副本 + 变更落盘。
pub struct SettingsStore {
    path: PathBuf,
    settings: Mutex<Settings>,
}

impl SettingsStore {
    /// 从磁盘加载，不存在时使用默认值。
    pub fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join("settings.json");
        let settings = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Settings::default(),
        };
        Ok(Self {
            path,
            settings: Mutex::new(settings),
        })
    }

    pub fn get(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// 原子修改并落盘。
    pub fn update<F: FnOnce(&mut Settings)>(&self, f: F) -> Settings {
        let mut guard = self.settings.lock().unwrap();
        f(&mut guard);
        let snapshot = guard.clone();
        drop(guard);
        self.persist(&snapshot);
        snapshot
    }

    fn persist(&self, s: &Settings) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(s) {
            let _ = std::fs::write(&self.path, json);
        }
    }
}
