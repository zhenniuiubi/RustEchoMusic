//! 登录会话：token / userid 持久化（`%APPDATA%/EchoMusic/session.json`）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 登录态。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub userid: u64,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub pic: String,
    #[serde(default)]
    pub vip_type: i64,
    /// VIP 到期时间（unix 秒），0 = 未知
    #[serde(default)]
    pub vip_end_time: i64,
    #[serde(default)]
    pub logged_in: bool,
}

impl Session {
    pub fn is_valid(&self) -> bool {
        self.logged_in && !self.token.is_empty() && self.userid != 0
    }
}

/// 会话存储：内存副本 + 变更落盘。
pub struct SessionStore {
    path: PathBuf,
    session: Mutex<Session>,
}

impl SessionStore {
    pub fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join("session.json");
        let session = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Session::default(),
        };
        Ok(Self {
            path,
            session: Mutex::new(session),
        })
    }

    pub fn get(&self) -> Session {
        self.session.lock().unwrap().clone()
    }

    pub fn set(&self, session: Session) {
        *self.session.lock().unwrap() = session.clone();
        self.persist(&session);
    }

    /// 局部更新会话字段（自动落盘）。
    pub fn update(&self, f: impl FnOnce(&mut Session)) {
        let mut s = self.session.lock().unwrap();
        f(&mut s);
        let snapshot = s.clone();
        drop(s);
        self.persist(&snapshot);
    }

    pub fn update_nickname(&self, nickname: &str) {
        self.update(|s| s.nickname = nickname.to_string());
    }

    pub fn update_pic(&self, pic: &str) {
        self.update(|s| s.pic = pic.to_string());
    }

    pub fn update_vip_type(&self, vip_type: i64) {
        self.update(|s| s.vip_type = vip_type);
    }

    /// 同时更新 VIP 等级与到期时间（拉过会员详情后调用）。
    pub fn update_vip(&self, vip_type: i64, vip_end_time: i64) {
        self.update(|s| {
            s.vip_type = vip_type;
            s.vip_end_time = vip_end_time;
        });
    }

    pub fn clear(&self) {
        let empty = Session::default();
        *self.session.lock().unwrap() = empty.clone();
        self.persist(&empty);
    }

    fn persist(&self, s: &Session) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(s) {
            let _ = std::fs::write(&self.path, json);
        }
    }
}
