//! 本地曲库：收藏与播放历史（`%APPDATA%/EchoMusic/library.json`）。
//!
//! 列表用 `Arc<Vec<Song>>` 存放：`get()` 返回 `Arc<Library>` 后，
//! UI 每帧取列表只是引用计数递增，不再深拷贝上千首歌。

use crate::models::Song;
use chrono::{TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// `Arc<Vec<T>>` 的 serde 转发（不依赖 serde 的 `rc` feature）。
mod arc_vec {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::Arc;

    pub fn serialize<T, S>(v: &Arc<Vec<T>>, s: S) -> Result<S::Ok, S::Error>
    where
        T: Serialize,
        S: Serializer,
    {
        v.as_ref().serialize(s)
    }

    pub fn deserialize<'de, T, D>(d: D) -> Result<Arc<Vec<T>>, D::Error>
    where
        T: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Vec::<T>::deserialize(d).map(Arc::new)
    }
}

/// 曲库数据。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Library {
    #[serde(default, with = "arc_vec")]
    pub favorites: Arc<Vec<Song>>,
    #[serde(default, with = "arc_vec")]
    pub history: Arc<Vec<Song>>,
    /// 播放流水（带时间戳，供「听歌统计」用；只保留最近 3000 条）
    #[serde(default)]
    pub plays: Vec<PlayLog>,
}

/// 一次播放记录（听歌统计的最小单位）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayLog {
    /// 歌曲名（存快照，避免历史里换了元数据导致统计对不上）
    pub title: String,
    pub artist: String,
    /// 播放时间（unix 秒）
    pub ts: i64,
}

/// 听歌统计（七日趋势 / 时段偏好 / 常听歌手）。
#[derive(Debug, Clone, Default)]
pub struct ListeningStats {
    /// 最近 7 天，每天播放次数（下标 0 = 6 天前，6 = 今天）
    pub days: [u32; 7],
    /// 24 个小时段的播放次数
    pub hours: [u32; 24],
    /// 常听歌手（前 5 名）
    pub top_artists: Vec<(String, u32)>,
    pub total: u32,
}

impl Library {
    /// 由播放流水算出统计（纯本地计算，不联网）。
    pub fn listening_stats(&self) -> ListeningStats {
        let mut days = [0u32; 7];
        let mut hours = [0u32; 24];
        let mut artists: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let local_now = chrono::Local::now();
        let today = local_now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|d| {
                chrono::Local
                    .from_local_datetime(&d)
                    .single()
                    .map(|t| t.timestamp())
            })
            .unwrap_or_else(|| local_now.timestamp());

        for p in &self.plays {
            let diff_days = (today - p.ts) / 86_400;
            if (0..7).contains(&diff_days) {
                days[(6 - diff_days) as usize] += 1;
            }
            if let Some(t) = chrono::DateTime::from_timestamp(p.ts, 0) {
                let h = t.with_timezone(&chrono::Local).hour() as usize;
                hours[h.min(23)] += 1;
            }
            if !p.artist.is_empty() {
                *artists.entry(p.artist.clone()).or_default() += 1;
            }
        }
        let mut top: Vec<(String, u32)> = artists.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1));
        top.truncate(5);
        ListeningStats {
            days,
            hours,
            top_artists: top,
            total: self.plays.len() as u32,
        }
    }
}

/// 曲库存储。
pub struct LibraryStore {
    path: PathBuf,
    data: Mutex<Arc<Library>>,
}

impl LibraryStore {
    pub fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join("library.json");
        let data = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<Library>(&text).unwrap_or_default(),
            Err(_) => Library::default(),
        };
        Ok(Self {
            path,
            data: Mutex::new(Arc::new(data)),
        })
    }

    /// 只读快照：`Arc` 克隆，O(1)。
    pub fn get(&self) -> Arc<Library> {
        self.data.lock().unwrap().clone()
    }

    pub fn is_favorite(&self, song: &Song) -> bool {
        let key = song.key();
        self.data
            .lock()
            .unwrap()
            .favorites
            .iter()
            .any(|s| s.key() == key)
    }

    /// 切换收藏，返回切换后的状态（true=已收藏）。
    pub fn toggle_favorite(&self, song: &Song) -> bool {
        let key = song.key();
        let mut guard = self.data.lock().unwrap();
        let data = Arc::make_mut(&mut *guard);
        let existed = data.favorites.iter().position(|s| s.key() == key);
        let now_fav = match existed {
            Some(i) => {
                Arc::make_mut(&mut data.favorites).remove(i);
                false
            }
            None => {
                Arc::make_mut(&mut data.favorites).insert(0, song.clone());
                true
            }
        };
        let snapshot = data.clone();
        drop(guard);
        self.persist(&snapshot);
        now_fav
    }

    /// 记录播放历史（去重，最多 300 条）。
    pub fn push_history(&self, song: &Song) {
        let key = song.key();
        let mut guard = self.data.lock().unwrap();
        let data = Arc::make_mut(&mut *guard);
        let list = Arc::make_mut(&mut data.history);
        list.retain(|s| s.key() != key);
        list.insert(0, song.clone());
        list.truncate(300);
        // 播放流水：统计用，只留最近 3000 条
        data.plays.push(PlayLog {
            title: song.title.clone(),
            artist: song.artist.clone(),
            ts: chrono::Local::now().timestamp(),
        });
        if data.plays.len() > 3000 {
            let keep = data.plays.len() - 3000;
            data.plays.drain(..keep);
        }
        let snapshot = data.clone();
        drop(guard);
        self.persist(&snapshot);
    }

    /// 只清空播放流水（统计清零，保留历史列表）。
    pub fn clear_plays(&self) {
        let mut guard = self.data.lock().unwrap();
        let data = Arc::make_mut(&mut *guard);
        data.plays.clear();
        let snapshot = data.clone();
        drop(guard);
        self.persist(&snapshot);
    }

    pub fn clear_history(&self) {
        let mut guard = self.data.lock().unwrap();
        let data = Arc::make_mut(&mut *guard);
        data.history = Arc::new(Vec::new());
        let snapshot = data.clone();
        drop(guard);
        self.persist(&snapshot);
    }

    fn persist(&self, l: &Library) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(l) {
            let _ = std::fs::write(&self.path, json);
        }
    }
}
