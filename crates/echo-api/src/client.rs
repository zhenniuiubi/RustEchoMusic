//! 酷狗网关 HTTP 客户端（对齐 KuGouMusicApi `util/request.js`）。
//!
//! - 统一发往 `https://gateway.kugou.com`，目标服务由 `x-router` 头指定
//! - 自动注入默认参数（dfid/mid/uuid/appid/clientver/clienttime[/token/userid]）
//! - 自动计算 Android signature（参数按 key 排序 `k=v` 串 + body + 盐）
//! - `status == 0` 或 `error_code != 0` 视为业务失败

use crate::crypto;
use echo_core::DeviceIdentity;
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub const GATEWAY: &str = "https://gateway.kugou.com";
pub const USER_AGENT: &str = "Android15-1070-11083-46-0-DiscoveryDRADProtocol-wifi";
pub const KG_RF: &str = "B9EDA08A64250DEFFBCADDEE00F8F25F";

/// 设备 Cookie（对齐 main/server.ts buildDefaultCookies）。
///
/// 登录态接口（user_verify / get_all_list 等）依赖 Cookie 里的
/// `userid` / `token` / `auth`，缺了会返回 20017 / 20006。
fn device_cookie(dev: &DeviceIdentity, dfid: Option<&str>, auth: Option<&AuthState>) -> String {
    let mut c = format!(
        "KUGOU_API_PLATFORM=lite; KUGOU_API_MID={}; KUGOU_API_GUID={}; KUGOU_API_DEV=EchoMusic; KUGOU_API_MAC=02:00:00:00:00:00; KUGOU_API_WEBGL={}",
        dev.mid, dev.guid, dev.webgl
    );
    if let Some(dfid) = dfid {
        c.push_str(&format!("; dfid={dfid}"));
    }
    if let Some(a) = auth {
        if !a.token.is_empty() {
            c.push_str(&format!("; token={}", a.token));
        }
        if a.userid != 0 {
            c.push_str(&format!("; userid={}", a.userid));
        }
        if !a.auth.is_empty() {
            c.push_str(&format!("; auth={}", a.auth));
        }
    }
    c
}

/// 签名类型。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EncryptType {
    /// Android 客户端签名（默认）
    #[default]
    Android,
    /// 设备注册签名
    Register,
    /// Web (H5) 版签名
    Web,
    /// 不签名
    None,
}

/// 请求描述。
#[derive(Debug, Clone, Default)]
pub struct RequestSpec {
    /// 路由路径，如 `/v2/special_recommend`
    pub url: String,
    /// x-router 头（网关目标服务）
    pub x_router: Option<String>,
    /// 非 gateway 域名时使用（如 userservice.kugou.com）
    pub base_url: Option<String>,
    /// 额外 query 参数
    pub params: BTreeMap<String, String>,
    /// POST JSON body（保留插入顺序）
    pub body: Option<Value>,
    pub encrypt_type: EncryptType,
    /// 是否生成 key 参数（signKey，歌曲地址接口用）
    pub encrypt_key: bool,
    /// 覆盖 dfid（song_url 接口用随机 24 位）
    pub dfid_override: Option<String>,
    /// 登录后 token/userid 是否注入（默认注入）
    pub skip_user_auth: bool,
    /// 完全不带默认参数（dfid/mid/uuid/appid/clientver/clienttime），
    /// 对齐 KuGouMusicApi 的 `clearDefaultParams`（歌词接口要求）。
    pub clear_default_params: bool,
    /// 额外请求头（如登录接口的 support-calm）
    pub extra_headers: Vec<(String, String)>,
    /// 覆盖 User-Agent
    pub user_agent: Option<String>,
    /// 覆盖 Cookie 里的 dfid（播放地址接口要求随机 dfid）
    pub cookie_dfid: Option<String>,
}

/// API 错误。
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("network: {0}")]
    Network(String),
    #[error("api status={status} error_code={error_code} msg={msg}")]
    Business {
        status: i64,
        error_code: i64,
        msg: String,
    },
    #[error("parse: {0}")]
    Parse(String),
}

/// 登录态失效的错误码：`20018` 会话过期、`20017` token 无效、`20006` 缺少 token。
///
/// 出现这些码时，唯一出路是重新登录（刷新 token 也需要原始 token 有效）。
pub const AUTH_EXPIRED_CODES: [i64; 3] = [20018, 20017, 20006];

/// 判断错误是否为「登录态失效」。
pub fn is_auth_expired(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<ApiError>(),
        Some(ApiError::Business { error_code, .. }) if AUTH_EXPIRED_CODES.contains(error_code)
    )
}

/// 取业务错误码（网络/解析错误返回 `None`）。
///
/// UI 侧据此把 `api status=0 error_code=20021 msg=` 这类原始串换成中文提示。
pub fn business_code(err: &anyhow::Error) -> Option<i64> {
    match err.downcast_ref::<ApiError>() {
        Some(ApiError::Business { error_code, .. }) => Some(*error_code),
        _ => None,
    }
}

/// 登录态。
#[derive(Debug, Clone, Default)]
pub struct AuthState {
    pub token: String,
    pub userid: u64,
    /// `user_verify` 返回的播放授权串，放进 Cookie 供后续接口复用
    pub auth: String,
}

/// 酷狗客户端。
pub struct KuGouClient {
    http: reqwest::Client,
    device: Mutex<DeviceIdentity>,
    auth: Mutex<AuthState>,
    pub(crate) lite: bool,
    pub(crate) appid: u32,
    pub(crate) clientver: u32,
}

impl KuGouClient {
    pub fn new(device: DeviceIdentity, lite: bool) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(20))
            .build()?;
        Ok(Self {
            http,
            device: Mutex::new(device),
            auth: Mutex::new(AuthState::default()),
            lite,
            appid: if lite { crypto::LITE_APPID } else { crypto::APPID },
            clientver: if lite { crypto::LITE_CLIENTVER } else { crypto::CLIENTVER },
        })
    }

    pub fn shared(device: DeviceIdentity, lite: bool) -> anyhow::Result<Arc<Self>> {
        Ok(Arc::new(Self::new(device, lite)?))
    }

    pub fn device(&self) -> DeviceIdentity {
        self.device.lock().clone()
    }

    pub fn set_dfid(&self, dfid: &str) {
        self.device.lock().dfid = dfid.to_string();
    }

    pub fn set_auth(&self, token: &str, userid: u64) {
        let mut a = self.auth.lock();
        a.token = token.to_string();
        a.userid = userid;
        a.auth.clear();
    }

    /// 缓存 `user_verify` 返回的播放授权串（会随 Cookie 发给后续接口）。
    pub fn set_play_auth(&self, auth: &str) {
        self.auth.lock().auth = auth.to_string();
    }

    /// 当前是否为lite 模式（探测用）
    pub fn lite_public(&self) -> bool {
        self.lite
    }

    /// 当前 appid / clientver（云盘等接口要把它们放进请求体）。
    pub fn appid_public(&self) -> u32 {
        self.appid
    }

    pub fn clientver_public(&self) -> u32 {
        self.clientver
    }

    /// 云盘接口专用的「裸」POST：只带给定 query + Cookie，**不签名**、不注入默认参数。
    ///
    /// 对齐 KuGouMusicApi 云盘模块的 `clearDefaultParams + notSignature`：
    /// 请求体是 AES 密文的原始字节，响应也是密文，所以只能拿 bytes。
    pub async fn post_cloud_raw(
        &self,
        url: &str,
        params: &[(String, String)],
        body: Vec<u8>,
    ) -> Result<Vec<u8>, ApiError> {
        let dev = self.device.lock().clone();
        let auth = self.auth.lock().clone();
        let mut req = self.http.post(url);
        for (k, v) in params {
            req = req.query(&[(k.as_str(), v.as_str())]);
        }
        let resp = req
            .header("Cookie", &device_cookie(&dev, None, Some(&auth)))
            .header("User-Agent", USER_AGENT)
            .body(body)
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        if !status.is_success() && bytes.is_empty() {
            return Err(ApiError::Network(format!("HTTP {status}")));
        }
        Ok(bytes.to_vec())
    }

    pub fn auth(&self) -> AuthState {
        self.auth.lock().clone()
    }

    fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// 发送请求并返回 JSON body。
    /// 发一次请求并返回响应原始字节（签名/加密/Cookie 都在这里处理）。
    async fn send_request(&self, spec: RequestSpec) -> Result<Vec<u8>, ApiError> {
        let clienttime = Self::now_secs().to_string();
        let dev = self.device.lock().clone();
        let auth = self.auth.lock().clone();

        let dfid = spec.dfid_override.clone().unwrap_or_else(|| dev.dfid.clone());

        // ===== 默认参数 =====
        let mut params: BTreeMap<String, String> = BTreeMap::new();
        if !spec.clear_default_params {
            params.insert("dfid".into(), dfid.clone());
            params.insert("mid".into(), dev.mid.clone());
            params.insert("uuid".into(), "-".into());
            params.insert("appid".into(), self.appid.to_string());
            params.insert("clientver".into(), self.clientver.to_string());
            params.insert("clienttime".into(), clienttime.clone());
        }
        if !spec.skip_user_auth {
            if !auth.token.is_empty() {
                params.insert("token".into(), auth.token.clone());
            }
            if auth.userid != 0 {
                params.insert("userid".into(), auth.userid.to_string());
            }
        }
        for (k, v) in &spec.params {
            params.insert(k.clone(), v.clone());
        }

        // ===== encryptKey：key = signKey(hash, mid, userid, appid) =====
        if spec.encrypt_key {
            let hash = params.get("hash").cloned().unwrap_or_default();
            let userid: u64 = params.get("userid").and_then(|v| v.parse().ok()).unwrap_or(0);
            let key = crypto::sign_key(&hash, &dev.mid, userid, self.appid, self.lite);
            params.insert("key".into(), key);
        }

        // ===== body 序列化（对象→紧凑 JSON，字符串原样） =====
        let body_str: String = match &spec.body {
            Some(Value::String(s)) => s.clone(),
            Some(v) => serde_json::to_string(v).unwrap_or_default(),
            None => String::new(),
        };

        // ===== signature =====
        match spec.encrypt_type {
            EncryptType::None => {}
            EncryptType::Register => {
                let mut values: Vec<String> = params.values().cloned().collect();
                let sig = crypto::signature_register(&mut values);
                params.insert("signature".into(), sig);
            }
            EncryptType::Android => {
                let sorted: Vec<(String, String)> = params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let sig = crypto::signature_android(&sorted, &body_str, self.lite);
                params.insert("signature".into(), sig);
            }
            EncryptType::Web => {
                let sorted: Vec<(String, String)> = params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let sig = crypto::signature_web(&sorted, &body_str);
                params.insert("signature".into(), sig);
            }
        }

        // ===== 构造 HTTP 请求 =====
        let base = spec.base_url.as_deref().unwrap_or(GATEWAY);
        let mut req = match &spec.body {
            Some(_) => self.http.post(format!("{base}{}", spec.url)),
            None => self.http.get(format!("{base}{}", spec.url)),
        };
        for (k, v) in &params {
            req = req.query(&[(k.as_str(), v.as_str())]);
        }
        match &spec.body {
            // 字符串 body 原样发送（如 register_dev 的 AES 密文串）
            Some(Value::String(s)) => {
                req = req
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(s.clone());
            }
            Some(v) => {
                req = req.header("Content-Type", "application/json;charset=utf-8");
                req = req.json(v);
            }
            None => {}
        }
        req = req
            .header("dfid", &dfid)
            .header("clienttime", &clienttime)
            .header("mid", &dev.mid)
            .header("kg-rc", "1")
            .header("kg-thash", "5d816a0")
            .header("kg-rec", "1")
            .header("kg-rf", KG_RF)
            .header(
                "Cookie",
                &device_cookie(
                    &dev,
                    spec.cookie_dfid.as_deref(),
                    if spec.skip_user_auth {
                        None
                    } else {
                        Some(&auth)
                    },
                ),
            );
        for (k, v) in &spec.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(ua) = &spec.user_agent {
            req = req.header("User-Agent", ua.as_str());
        }
        if let Some(router) = &spec.x_router {
            req = req.header("x-router", router);
        }

        // ECHO_HTTP_TRACE=1：把请求参数与 body 打进日志（排查签名类问题用）
        if std::env::var("ECHO_HTTP_TRACE").is_ok() {
            log::info!(
                "HTTP {} params={:?} body={}",
                spec.url,
                params,
                if body_str.len() > 300 {
                    &body_str[..300]
                } else {
                    &body_str
                }
            );
        }
        let resp = req.send().await.map_err(|e| ApiError::Network(e.to_string()))?;
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?
            .to_vec();
        if !status.is_success() && bytes.is_empty() {
            return Err(ApiError::Network(format!("HTTP {status}")));
        }
        Ok(bytes)
    }

    /// 与 `request` 相同，但返回**未经解析的响应原文**（歌词接口不是统一 JSON 包装）。
    pub async fn request_text(&self, spec: RequestSpec) -> Result<String, ApiError> {
        let bytes = self.send_request(spec).await?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    /// 返回原始字节：cloudlist 之类的接口响应是 AES 密文，不能当 UTF-8 处理。
    pub async fn request_bytes(&self, spec: RequestSpec) -> Result<Vec<u8>, ApiError> {
        self.send_request(spec).await
    }

    /// 网关请求：签名 → 发送 → 解析 JSON → 业务状态判断。
    pub async fn request(&self, spec: RequestSpec) -> Result<Value, ApiError> {
        let bytes = self.send_request(spec).await?;
        let text = String::from_utf8_lossy(&bytes).to_string();

        // 部分接口（搜索等）会用 HTML 注释包裹 JSON：<!--KG_TAG_RES_START-->{...}<!--KG_TAG_RES_END-->
        let cleaned = sanitize_json(&text);
        let mut body: Value = serde_json::from_str(cleaned)
            .map_err(|e| ApiError::Parse(format!("{e}; body={:.200}", text)))?;

        // ===== 业务状态判断（对齐 request.js） =====
        // 允许外层包数组或纯 data（如 register_dev 已在外层解密）
        if !body.is_object() {
            return Ok(body);
        }
        let st = body.get("status").and_then(|v| v.as_i64()).unwrap_or(1);
        let ec = body.get("error_code").and_then(|v| v.as_i64()).unwrap_or(0);
        if st == 0 || ec != 0 {
            let msg = body
                .get("msg")
                .or_else(|| body.get("error"))
                .and_then(|v| {
                    if v.is_string() {
                        v.as_str().map(|s| s.to_string())
                    } else {
                        Some(v.to_string())
                    }
                })
                .unwrap_or_default();
            return Err(ApiError::Business {
                status: st,
                error_code: ec,
                msg,
            });
        }
        if !body.get("data").is_some() {
            // 部分接口直接返回数据对象，包装为 { data }
            let inner = body.clone();
            body = serde_json::json!({ "data": inner });
        }
        Ok(body)
    }

    /// 设备注册：POST userservice.kugou.com/risk/v2/r_register_dev，返回 dfid。
    pub async fn register_device(&self) -> Result<String, ApiError> {
        use serde_json::json;
        let auth = self.auth.lock().clone();
        let dev = self.device.lock().clone();

        let data_map = json!({
            "availableRamSize": 4983533568i64,
            "availableRomSize": 48114719i64,
            "availableSDSize": 48114717i64,
            "basebandVer": "",
            "batteryLevel": 100,
            "batteryStatus": 3,
            "brand": "Redmi",
            "buildSerial": "unknown",
            "device": "marble",
            "imei": dev.guid,
            "imsi": "",
            "manufacturer": "Xiaomi",
            "uuid": dev.uuid,
            "accelerometer": false,
            "accelerometerValue": "",
            "gravity": false,
            "gravityValue": "",
            "gyroscope": false,
            "gyroscopeValue": "",
            "light": false,
            "lightValue": "",
            "magnetic": false,
            "magneticValue": "",
            "orientation": false,
            "orientationValue": "",
            "pressure": false,
            "pressureValue": "",
            "step_counter": false,
            "step_counterValue": "",
            "temperature": false,
            "temperatureValue": "",
        });

        // AES 加密请求体
        let (seed, aes_str) = crypto::playlist_aes_encrypt(&serde_json::to_string(&data_map).unwrap());
        // p = RSA(JSON{aes, uid, token}) hex 大写
        let portrait_plain =
            serde_json::to_string(&json!({ "aes": seed, "uid": auth.userid, "token": auth.token })).unwrap();
        let p = crypto::rsa_encrypt_hex(portrait_plain.as_bytes(), self.lite)
            .map_err(|e| ApiError::Network(format!("rsa: {e}")))?;

        let mut spec = RequestSpec {
            url: "/risk/v2/r_register_dev".into(),
            base_url: Some("https://userservice.kugou.com".into()),
            body: Some(Value::String(aes_str.clone())),
            encrypt_type: EncryptType::Android,
            skip_user_auth: true,
            ..Default::default()
        };
        spec.params.insert("part".into(), "1".into());
        spec.params.insert("platid".into(), "1".into());
        spec.params.insert("p".into(), p);

        // register_dev 响应是 base64 密文，不能走通用 JSON 解析 —— 手动发送
        let body = self.raw_request(spec).await?;
        // 响应为 AES 密文二进制，按 base64 编码后解密（对齐 JS res.body.toString('base64')）
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&body);
        let decrypted = crypto::playlist_aes_decrypt(&seed, &b64)
            .map_err(|e| ApiError::Parse(format!("register decrypt: {e}")))?;
        let value: Value = serde_json::from_slice(&decrypted).map_err(|e| ApiError::Parse(e.to_string()))?;
        let dfid = value
            .pointer("/data/dfid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if dfid.is_empty() {
            return Err(ApiError::Parse(format!("register_dev no dfid: {value}")));
        }
        self.set_dfid(&dfid);
        Ok(dfid)
    }

    /// 直接 GET 一个普通 URL 并返回文本（CDN 旧接口用，无需签名）。
    pub(crate) async fn get_text(&self, url: &str) -> Result<String, ApiError> {
        let resp = self
            .http
            .get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        resp.text().await.map_err(|e| ApiError::Network(e.to_string()))
    }

    /// 发送原始请求，返回原始响应字节（不解 JSON）。
    pub async fn send_raw(&self, spec: RequestSpec) -> Result<Vec<u8>, ApiError> {
        self.raw_request(spec).await
    }

    /// 发送原始请求，返回原始响应字节（不解 JSON）。
    async fn raw_request(&self, spec: RequestSpec) -> Result<Vec<u8>, ApiError> {
        // 复用 request 的签名逻辑：临时走一遍参数构建
        // （与 request() 一致的参数/签名流程，但返回原始 bytes）
        let clienttime = Self::now_secs().to_string();
        let dev = self.device.lock().clone();
        let auth = self.auth.lock().clone();
        let dfid = spec.dfid_override.clone().unwrap_or_else(|| dev.dfid.clone());

        let mut params: BTreeMap<String, String> = BTreeMap::new();
        params.insert("dfid".into(), dfid.clone());
        params.insert("mid".into(), dev.mid.clone());
        params.insert("uuid".into(), "-".into());
        params.insert("appid".into(), self.appid.to_string());
        params.insert("clientver".into(), self.clientver.to_string());
        params.insert("clienttime".into(), clienttime.clone());
        if !auth.token.is_empty() {
            params.insert("token".into(), auth.token.clone());
        }
        if auth.userid != 0 {
            params.insert("userid".into(), auth.userid.to_string());
        }
        for (k, v) in &spec.params {
            params.insert(k.clone(), v.clone());
        }
        let body_str = match &spec.body {
            Some(Value::String(s)) => s.clone(),
            Some(v) => serde_json::to_string(v).unwrap_or_default(),
            None => String::new(),
        };
        match spec.encrypt_type {
            EncryptType::None => {}
            EncryptType::Register => {
                let mut values: Vec<String> = params.values().cloned().collect();
                let sig = crypto::signature_register(&mut values);
                params.insert("signature".into(), sig);
            }
            EncryptType::Android => {
                let sorted: Vec<(String, String)> =
                    params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let sig = crypto::signature_android(&sorted, &body_str, self.lite);
                params.insert("signature".into(), sig);
            }
            EncryptType::Web => {
                let sorted: Vec<(String, String)> =
                    params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let sig = crypto::signature_web(&sorted, &body_str);
                params.insert("signature".into(), sig);
            }
        }

        let base = spec.base_url.as_deref().unwrap_or(GATEWAY);
        // 有 body 走 POST，否则走 GET（与前台 request 一致）
        let mut req = if spec.body.is_some() {
            self.http.post(format!("{base}{}", spec.url))
        } else {
            self.http.get(format!("{base}{}", spec.url))
        };
        for (k, v) in &params {
            req = req.query(&[(k.as_str(), v.as_str())]);
        }
        match &spec.body {
            Some(Value::String(s)) => {
                req = req
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(s.clone());
            }
            Some(v) => {
                req = req.header("Content-Type", "application/json;charset=utf-8").json(v);
            }
            None => {}
        }
        req = req
            .header("dfid", &dfid)
            .header("clienttime", &clienttime)
            .header("mid", &dev.mid)
            .header("kg-rc", "1")
            .header("kg-thash", "5d816a0")
            .header("kg-rec", "1")
            .header("kg-rf", KG_RF)
            .header(
                "Cookie",
                &device_cookie(
                    &dev,
                    spec.cookie_dfid.as_deref(),
                    if spec.skip_user_auth {
                        None
                    } else {
                        Some(&auth)
                    },
                ),
            );
        for (k, v) in &spec.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(ua) = &spec.user_agent {
            req = req.header("User-Agent", ua.as_str());
        }
        if let Some(router) = &spec.x_router {
            req = req.header("x-router", router);
        }
        let resp = req.send().await.map_err(|e| ApiError::Network(e.to_string()))?;
        let bytes = resp.bytes().await.map_err(|e| ApiError::Network(e.to_string()))?;
        Ok(bytes.to_vec())
    }
}

/// 剥离响应中的 HTML 注释包装，截取最外层 JSON。
fn sanitize_json(text: &str) -> &str {
    let t = text.trim();
    if t.starts_with('{') || t.starts_with('[') {
        return t;
    }
    // 跳到第一个 { 或 [
    let start = t
        .char_indices()
        .find(|(_, c)| *c == '{' || *c == '[')
        .map(|(i, _)| i);
    let Some(start) = start else { return t };
    // 从末尾回退到最后一个 } 或 ]
    let end = t
        .char_indices()
        .rev()
        .find(|(_, c)| *c == '}' || *c == ']')
        .map(|(i, c)| i + c.len_utf8());
    match end {
        Some(end) if end > start => &t[start..end],
        _ => &t[start..],
    }
}
