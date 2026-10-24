//! 设备身份：mid / uuid / guid / dfid 生成与持久化。
//!
//! 对齐原项目 `main/server.ts` 中的设备身份逻辑：
//! - `mid`：32 位十六进制设备指纹
//! - `uuid` / `guid`：标准 GUID
//! - `dfid`：由 `register_dev` 接口返回，注册前为 `-`
//! 这些值参与每个请求的默认参数与签名。

use rand::Rng;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 设备身份。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceIdentity {
    /// 设备 MID（32 hex）
    #[serde(default)]
    pub mid: String,
    /// 设备 UUID
    #[serde(default)]
    pub uuid: String,
    /// 设备 GUID
    #[serde(default)]
    pub guid: String,
    /// 设备指纹 ID（register_dev 返回；未注册时为 "-"）
    #[serde(default = "default_dfid")]
    pub dfid: String,
    /// WebGL 指纹哈希（SSA 模拟用，可为空）
    #[serde(default)]
    pub webgl: String,
}

fn default_dfid() -> String {
    "-".into()
}

impl DeviceIdentity {
    /// 生成新的设备身份（不包含 dfid，dfid 需注册获得）。
    ///
    /// 对齐原项目 `main/server.ts`：
    /// - `guid` = md5(随机 GUID)（32 位 hex）
    /// - `mid`  = calculateMid(guid)：md5(guid) 视为 128 位大端整数的十进制表示
    /// - `uuid` 复用 guid
    pub fn generate() -> Self {
        let mut rng = rand::rng();
        let raw: String = (0..32).map(|_| format!("{:x}", rng.random_range(0..16))).collect();
        let guid = md5_hex(&raw);
        let mid = calculate_mid(&guid);
        let webgl: String = (0..16).map(|_| format!("{:x}", rng.random_range(0..16))).collect();
        Self {
            mid,
            uuid: guid.clone(),
            guid,
            dfid: default_dfid(),
            webgl,
        }
    }

    /// 从磁盘加载，不存在或字段缺失时生成并保存。
    pub fn load_or_create(data_dir: &Path) -> anyhow::Result<Self> {
        let path = device_file(data_dir);
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(mut id) = serde_json::from_str::<DeviceIdentity>(&text) {
                let mut changed = false;
                if id.mid.is_empty() {
                    id.mid = Self::generate().mid;
                    changed = true;
                }
                if id.uuid.is_empty() {
                    id.uuid = Self::generate().uuid;
                    changed = true;
                }
                if id.guid.is_empty() {
                    id.guid = Self::generate().guid;
                    changed = true;
                }
                if changed {
                    id.save(data_dir)?;
                }
                return Ok(id);
            }
        }
        let id = Self::generate();
        id.save(data_dir)?;
        Ok(id)
    }

    /// 保存到磁盘。
    pub fn save(&self, data_dir: &Path) -> anyhow::Result<()> {
        let path = device_file(data_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    /// 更新 dfid 并持久化。
    pub fn set_dfid(&mut self, dfid: &str, data_dir: &Path) -> anyhow::Result<()> {
        self.dfid = dfid.to_string();
        self.save(data_dir)
    }
}

fn device_file(data_dir: &Path) -> PathBuf {
    data_dir.join("device.json")
}

/// calculateMid：`md5(str)` 十六进制摘要解析为 128 位大端整数的十进制字符串。
pub fn calculate_mid(s: &str) -> String {
    let digest = md5_hex(s);
    u128::from_str_radix(&digest, 16)
        .map(|n| n.to_string())
        .unwrap_or_else(|_| digest)
}

fn md5_hex(input: &str) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}
