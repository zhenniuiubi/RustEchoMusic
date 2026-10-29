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
    /// 默认音质：128 / 320 / flac / high
    pub audio_quality: String,
    /// 最近一次「每日 VIP」领取尝试的日期（yyyy-mm-dd），同一天不重复打接口
    pub vip_claim_date: String,
    /// 上面那次尝试是否成功
    pub vip_claim_ok: bool,
    /// 最近一次领取「尝试」的时间（unix 秒），用于失败后退避、避免刷接口
    pub vip_claim_try_ts: i64,
    /// 播放倍速（0.5–2.0，1.0 为原速）
    pub speed: f64,
    /// 音频输出设备名（空 = 系统默认）
    pub output_device: String,
    /// Lite 播放条的进度条形态：`wave` 波浪（默认）/ `plain` 普通 / `nyan` 彩虹猫
    pub lite_bar: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::System,
            theme_color: "#f55e55".into(),
            volume: 0.8,
            play_mode: PlayMode::List,
            audio_quality: "128".into(),
            vip_claim_date: String::new(),
            vip_claim_ok: false,
            vip_claim_try_ts: 0,
            speed: 1.0,
            output_device: String::new(),
            lite_bar: "wave".into(),
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
