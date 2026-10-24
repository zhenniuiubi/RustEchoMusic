//! 播放队列持久化（`%APPDATA%/EchoMusic/queue.json`）。
//!
//! 关掉 App 再打开，队列（含当前曲目下标）要能还原。
//! 保存时剔掉 `audio_url`：那是带签名的临时地址，几个小时就失效，
//! 而且一长串 URL 会把文件撑大；还原后第一次点播放会重新解析。

use crate::models::Song;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 落盘的队列快照。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedQueue {
    /// 队列标题（播放来源，如「搜索结果」「我最喜爱」）
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub songs: Vec<Song>,
    /// 当前曲目下标（可能是 None：队列为空）
    #[serde(default)]
    pub index: Option<usize>,
}

impl SavedQueue {
    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }
}

/// 队列存储：内存副本 + 变更落盘。
pub struct QueueStore {
    path: PathBuf,
    data: Mutex<SavedQueue>,
}

impl QueueStore {
    pub fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join("queue.json");
        let data = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<SavedQueue>(&text).unwrap_or_default(),
            Err(_) => SavedQueue::default(),
        };
        // 下标越界的旧数据直接丢掉，避免还原出不一致状态
        let mut data = data;
        if let Some(i) = data.index {
            if i >= data.songs.len() {
                data.index = None;
            }
        }
        Ok(Self {
            path,
            data: Mutex::new(data),
        })
    }

    /// 启动时读取一次快照（深拷贝，只在初始化时用）。
    pub fn get(&self) -> SavedQueue {
        self.data.lock().unwrap().clone()
    }

    /// 写入快照。队列内容/下标没变时直接跳过，避免每次切歌都重写文件。
    pub fn save(&self, title: &str, songs: &[Song], index: Option<usize>) {
        {
            let guard = self.data.lock().unwrap();
            let same = guard.title == title
                && guard.index == index
                && guard.songs.len() == songs.len()
                && guard
                    .songs
                    .iter()
                    .zip(songs.iter())
                    .all(|(a, b)| a.hash == b.hash);
            if same {
                return;
            }
        }

        // 剔掉临时播放地址后再落盘
        let cleaned: Vec<Song> = songs
            .iter()
            .map(|s| {
                let mut c = s.clone();
                c.audio_url.clear();
                c
            })
            .collect();
        let snapshot = SavedQueue {
            title: title.to_string(),
            songs: cleaned,
            index,
        };
        *self.data.lock().unwrap() = snapshot.clone();

        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec(&snapshot) {
            let _ = std::fs::write(&self.path, json);
        }
    }

    /// 清空（同时删掉磁盘文件）。
    pub fn clear(&self) {
        *self.data.lock().unwrap() = SavedQueue::default();
        let _ = std::fs::remove_file(&self.path);
    }
}
