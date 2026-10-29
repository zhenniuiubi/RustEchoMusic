//! 高层 API 方法（第一期集合：首页 / 搜索 / 歌单 / 播放地址 / 榜单）。
//!
//! 每个方法对应原项目 `src/renderer/api/*` + `server/module/*` 中的一个接口。

use crate::client::{EncryptType, KuGouClient, RequestSpec};
use crate::normalize::parse_songs;
use echo_core::Song;
use serde_json::{json, Value};

/// 私人 FM 一批推荐（`cur_mark` 要带回下一次请求）。
#[derive(Debug, Clone, Default)]
pub struct FmBatch {
    pub songs: Vec<Song>,
    pub cur_mark: i64,
}

impl KuGouClient {
    // ==================== 首页 ====================

    // ==================== 搜索 ====================

    // ==================== 歌单 ====================


    // ==================== 播放地址 ====================

    /// 二次授权（song_auth → `/v1/authorization`），返回 (auth, open_time)。
    async fn song_authorization(
        &self,
        hash: &str,
        album_audio_id: i64,
        authorization: &str,
    ) -> anyhow::Result<(String, i64)> {
        let mut spec = RequestSpec {
            url: "/v1/authorization".into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("authorization".into(), authorization.into());
        spec.params.insert("module_id".into(), "51".into());
        spec.params
            .insert("album_audio_id".into(), album_audio_id.to_string());
        spec.params.insert("clientver".into(), "11561".into());
        spec.params.insert("hash".into(), hash.into());
        spec.x_router = Some("trackercdngz.kugou.com".into());

        let res = self.request(spec).await?;
        let auth = res.pointer("/data/auth").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let open_time = res.pointer("/data/open_time").and_then(|v| v.as_i64()).unwrap_or(0);
        Ok((auth, open_time))
    }

    /// 获取播放地址（原 getSongUrl → song_url_auth_merge 三段式）。
    ///
    /// - 已登录：`register_device` → `/v1/user_verify` → `/v1/authorization`
    ///   → `/tracker/v5/url`，VIP 歌曲也能拿到地址。
    /// - 未登录：走免签名 CDN `/i/?cmd=4`，非 VIP 歌曲可得 128k。
    ///
    /// 返回候选 URL 列表（首个为主地址，其余为 CDN 备份）。
    pub async fn song_url(
        &self,
        hash: &str,
        quality: &str,
        album_audio_id: i64,
    ) -> anyhow::Result<Vec<String>> {
        let hash = hash.trim().to_lowercase();
        if hash.is_empty() {
            anyhow::bail!("歌曲缺少 hash，无法播放");
        }
        let mut last_err: Option<String> = None;
        let logged_in = self.auth().userid != 0;
        log::info!("song_url: hash={hash} quality={quality} aa={album_audio_id} logged_in={logged_in}");

        // 已登录：授权链（含 VIP）；未登录：免签名 CDN（免费歌曲可用）
        if logged_in {
            match self.url_via_auth(&hash, quality, album_audio_id).await {
                Ok(urls) if !urls.is_empty() => {
                    log::info!("song_url: url_via_auth OK n={}", urls.len());
                    return Ok(urls);
                }
                Ok(_) => log::warn!("song_url: url_via_auth empty"),
                Err(e) => {
                    log::warn!("song_url: url_via_auth ERR {e}");
                    last_err = Some(format!("{e}"));
                }
            }

            // 高音质授权失败时降级 128k 重试（部分歌曲/数据源的 album_audio_id 与授权所需 audio_id 不一致，
            // 320/flac 会失败，但 128k 通常可拿地址）
            if quality != "128" {
                match self.url_via_auth(&hash, "128", album_audio_id).await {
                    Ok(urls) if !urls.is_empty() => {
                        log::warn!("song_url: quality downgrade {quality} -> 128 OK n={}", urls.len());
                        return Ok(urls);
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("song_url: downgrade 128 ERR {e}"),
                }
            }
        }

        // 免签名 CDN 接口：非 VIP 歌曲未登录即可拿到 128k mp3
        match self.url_cdn(&hash).await {
            Ok(urls) if !urls.is_empty() => return Ok(urls),
            Ok(_) => {}
            Err(e) => last_err = Some(format!("{e}")),
        }

        // 再兜底：授权链 / 老 /v5/url
        if !logged_in {
            if let Ok(urls) = self.url_via_auth(&hash, quality, album_audio_id).await {
                if !urls.is_empty() {
                    return Ok(urls);
                }
            }
        }
        if let Ok(urls) = self.url_legacy(&hash, quality, album_audio_id).await {
            if !urls.is_empty() {
                return Ok(urls);
            }
        }

        anyhow::bail!("{}", last_err.unwrap_or_else(|| "未能获取播放地址".into()))
    }

    pub async fn user_verify(&self) -> anyhow::Result<String> {
        let mut spec = RequestSpec {
            url: "/v1/user_verify".into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("module_id".into(), "51".into());
        spec.x_router = Some("trackercdngz.kugou.com".into());

        let res = self.request(spec).await?;
        let auth = res
            .pointer("/data/auth")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if auth.is_empty() {
            anyhow::bail!("{}", response_error(&res));
        }
        self.set_play_auth(&auth);
        Ok(auth)
    }

    /// 三段式授权取地址。
    async fn url_via_auth(
        &self,
        hash: &str,
        quality: &str,
        album_audio_id: i64,
    ) -> anyhow::Result<Vec<String>> {
        let dfid = crate::crypto::random_string_24();
        // `user_verify` 必须建立在服务端已登记的设备上，否则返回 35005
        let _ = self.register_device().await;
        let verify_auth = match self.user_verify().await {
            Ok(a) => {
                log::info!("url_via_auth: user_verify OK");
                a
            }
            Err(e) => {
                log::warn!("url_via_auth: user_verify ERR {e}");
                return Err(e);
            }
        };
        let (auth, open_time) = self
            .song_authorization(hash, album_audio_id, &verify_auth)
            .await?;
        if auth.is_empty() && open_time == 0 {
            anyhow::bail!("授权失败（auth 为空）");
        }
        log::info!("url_via_auth: authorization auth_len={} open_time={open_time}", auth.len());

        // `page_id` / `pid` / `ppage_id` 随平台变化（见 song_url_auth.js）
        let (page_id, pid, ppage_id) = if self.lite {
            ("967177915", "411", "356753938,823673182,967485191")
        } else {
            ("151369488", "2", "463467626,350369493,788954147")
        };
        let mut spec = RequestSpec {
            url: "/tracker/v5/url".into(),
            // 对齐 song_url_auth.js：encryptType 为 'android'（参考实现的 notSign:true 是笔误，
            // 其 request.js 检查的是 notSignature，实际仍计算并携带 android signature）。
            // 实测：不发签名（EncryptType::None）服务端返回 status=0 空响应；
            // 用 Android 签名则正常返回 VIP/付费歌曲的 mp3 地址。
            encrypt_type: EncryptType::Android,
            encrypt_key: true,
            dfid_override: Some(dfid.clone()),
            cookie_dfid: Some(dfid),
            ..Default::default()
        };
        let params = [
            ("album_id", "0".to_string()),
            ("area_code", "1".to_string()),
            ("module", String::new()),
            ("hash", hash.to_string()),
            ("need_m", "0".to_string()),
            ("ssa_flag", "is_fromtrack".to_string()),
            ("version", "11430".to_string()),
            ("open_time", open_time.to_string()),
            ("ptype", "0".to_string()),
            ("need_ogg", "1".to_string()),
            ("page_id", page_id.to_string()),
            ("auth", auth),
            ("mtype", "0".to_string()),
            ("quality", quality.to_string()),
            ("album_audio_id", album_audio_id.to_string()),
            ("behavior", "play".to_string()),
            ("pid", pid.to_string()),
            ("module_id", "51".to_string()),
            ("cmd", "26".to_string()),
            ("ppage_id", ppage_id.to_string()),
            ("clientver", "11561".to_string()),
            ("pidversion", "3001".to_string()),
            ("cdnBackup", "1".to_string()),
        ];
        for (k, v) in params {
            spec.params.insert(k.into(), v);
        }

        let res = self.request(spec).await?;
        let urls = resolve_urls_from_response(&res);
        if urls.is_empty() {
            anyhow::bail!("{}", response_error(&res));
        }
        Ok(urls)
    }

    /// 免签名 CDN 接口（`/i/?cmd=4`）：免费歌曲无需登录即可获取 128k mp3 地址。
    async fn url_cdn(&self, hash: &str) -> anyhow::Result<Vec<String>> {
        let key = crate::crypto::md5_hex(&format!("{hash}kgcloud"));
        let url = format!(
            "https://trackercdn.kugou.com/i/?cmd=4&hash={hash}&key={key}&pid=1&acceptMp3=1&forceDown=0&vip=1"
        );
        let text = self.get_text(&url).await?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("CDN 响应解析失败：{e}"))?;
        let status = v.get("status").and_then(|x| x.as_i64()).unwrap_or(0);
        if status != 1 {
            let msg = v
                .get("error")
                .and_then(|x| x.as_str())
                .unwrap_or("该歌曲暂不可播放");
            let msg = if msg.contains("Paid") {
                "该歌曲需要购买或 VIP 才能播放"
            } else {
                msg
            };
            anyhow::bail!("{msg}");
        }
        let mut urls = Vec::new();
        for k in ["url", "backup_url", "url_128"] {
            if let Some(u) = v.get(k).and_then(|x| x.as_str()) {
                if !u.is_empty() && !urls.iter().any(|x: &String| x == u) {
                    urls.push(u.to_string());
                }
            }
        }
        if urls.is_empty() {
            anyhow::bail!("CDN 未返回播放地址");
        }
        Ok(urls)
    }

    /// 老接口 `/v5/url`（trackercdn），不签名。
    async fn url_legacy(
        &self,
        hash: &str,
        quality: &str,
        album_audio_id: i64,
    ) -> anyhow::Result<Vec<String>> {
        let dfid = crate::crypto::random_string_24();
        let mut spec = RequestSpec {
            url: "/v5/url".into(),
            base_url: Some("https://trackercdn.kugou.com".into()),
            encrypt_type: EncryptType::Android,
            encrypt_key: true,
            dfid_override: Some(dfid.clone()),
            cookie_dfid: Some(dfid),
            ..Default::default()
        };
        let params = [
            ("album_id", "0".to_string()),
            ("area_code", "1".to_string()),
            ("hash", hash.to_string()),
            ("ssa_flag", "is_fromtrack".to_string()),
            ("version", "11430".to_string()),
            ("module", String::new()),
            ("quality", quality.to_string()),
            ("album_audio_id", album_audio_id.to_string()),
            ("behavior", "play".to_string()),
            ("pid", "411".to_string()),
            ("cmd", "26".to_string()),
            ("pidversion", "3001".to_string()),
            ("IsFreePart", "0".to_string()),
            ("page_id", "967177915".to_string()),
            ("ppage_id", "356753938,823673182,967485191".to_string()),
            ("cdnBackup", "1".to_string()),
            ("clientver", "11430".to_string()),
        ];
        for (k, v) in params {
            spec.params.insert(k.into(), v);
        }
        let res = self.request(spec).await?;
        let urls = resolve_urls_from_response(&res);
        if urls.is_empty() {
            anyhow::bail!("{}", response_error(&res));
        }
        Ok(urls)
    }

    // ==================== 榜单 ====================

}

/// 从播放地址响应中递归提取 URL 列表（对齐 player/utils.ts `resolveUrlsFromResponse`）。
pub fn resolve_urls_from_response(payload: &Value) -> Vec<String> {
    let mut urls = Vec::new();
    fn append(urls: &mut Vec<String>, v: &Value) {
        match v {
            Value::String(s) => {
                let u = s.trim();
                if !u.is_empty() && !urls.contains(&u.to_string()) {
                    urls.push(u.to_string());
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    append(urls, item);
                }
            }
            _ => {}
        }
    }
    match payload {
        Value::String(_) | Value::Array(_) => {
            append(&mut urls, payload);
        }
        Value::Object(obj) => {
            for key in ["url", "play_url", "playUrl", "backupUrl", "backup_url", "backupUrls"] {
                if let Some(v) = obj.get(key) {
                    append(&mut urls, v);
                }
            }
            if urls.is_empty() {
                for key in ["data", "info"] {
                    if let Some(v) = obj.get(key) {
                        let inner = resolve_urls_from_response(v);
                        if !inner.is_empty() {
                            return inner;
                        }
                    }
                }
            }
        }
        _ => {}
    }
    urls
}

// ==================== 第二期接口 ====================

impl KuGouClient {
    /// 用本地保存的 token 刷新登录态（`/v5/login_by_token`）。
    ///
    /// 酷狗 token 会过期，过期后**所有**登录态接口都返回 `20018`（请重新登录），
    /// 表现就是「界面还显示已登录，但歌单/VIP 歌曲全都拉不到」。
    /// 客户端启动时先换一个新 token，能显著延长免登录时间。
    pub async fn refresh_token(&self) -> anyhow::Result<LoginResult> {
        let auth = self.auth();
        if auth.token.is_empty() || auth.userid == 0 {
            anyhow::bail!("未登录");
        }
        let dev = self.device();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;

        // p3：固定 key/iv 的 AES-256，明文是 { clienttime(秒), token }
        let p3_plain = json!({
            "clienttime": now_ms / 1000,
            "token": auth.token,
        })
        .to_string();
        let p3 = crate::crypto::aes256_cbc_encrypt_hex(
            REFRESH_AES_KEY.as_bytes(),
            REFRESH_AES_IV.as_bytes(),
            p3_plain.as_bytes(),
        );

        // params/pk：随机会话密钥；响应里的 secu_params 要用 temp_key 解
        let (params_hex, temp_key) = crate::crypto::login_aes_encrypt("{}");
        let pk = crate::crypto::rsa_encrypt_hex(
            json!({ "clienttime_ms": now_ms, "key": temp_key })
                .to_string()
                .as_bytes(),
            self.lite,
        )?;

        let mut body = json!({
            "dfid": dev.dfid,
            "p3": p3,
            "plat": 1,
            "t1": 0,
            "t2": 0,
            "t3": "MCwwLDAsMCwwLDAsMCwwLDA=",
            "pk": pk,
            "params": params_hex,
            "userid": auth.userid,
            "clienttime_ms": now_ms,
        });
        if self.lite {
            body["dev"] = json!("EchoMusic");
        }

        let spec = RequestSpec {
            url: "/v5/login_by_token".into(),
            base_url: Some("http://login.user.kugou.com".into()),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        let res = self.request(spec).await?;
        let data = res.get("data").cloned().unwrap_or(Value::Null);

        let mut login = parse_login_payload(&data, Some(&temp_key));
        if login.token.is_empty() {
            anyhow::bail!("{}", response_error(&res));
        }
        if login.userid == 0 {
            login.userid = auth.userid;
        }

        self.set_auth(&login.token, login.userid);
        self.set_t1(&login.t1);
        Ok(login)
    }

    // ==================== 扫码登录 / 短信验证码登录 ====================

    /// 扫码登录第一步：申请二维码 key。
    /// 返回 `(key, 二维码内容 URL)`；URL 由前端渲染成二维码图片。
    /// 对应 `server/module/login_qr_key.js`（Web 签名，appid 固定 1001）。
    pub async fn qr_key(&self) -> anyhow::Result<(String, String)> {
        let mut spec = RequestSpec {
            url: "/v2/qrcode".into(),
            base_url: Some("https://login-user.kugou.com".into()),
            encrypt_type: EncryptType::Web,
            skip_user_auth: true,
            ..Default::default()
        };
        spec.params.insert("appid".into(), "1001".into());
        spec.params.insert("type".into(), "1".into());
        spec.params.insert("plat".into(), "4".into());
        spec.params.insert(
            "qrcode_txt".into(),
            format!(
                "https://h5.kugou.com/apps/loginQRCode/html/index.html?appid={}&",
                crate::crypto::APPID
            ),
        );
        spec.params.insert("srcappid".into(), "2919".into());

        let res = self.request(spec).await?;
        let data = res.get("data").cloned().unwrap_or(Value::Null);
        let key = crate::normalize::pick_str(&data, &["qrcode", "key"]);
        if key.is_empty() {
            anyhow::bail!("二维码获取失败：{}", res);
        }
        let url = format!("https://h5.kugou.com/apps/loginQRCode/html/index.html?qrcode={key}");
        Ok((key, url))
    }

    /// 扫码登录第二步：轮询扫码状态。
    /// `status`：0 过期 / 1 等待扫码 / 2 已扫码待确认 / 4 授权成功（带 token）。
    pub async fn qr_check(&self, key: &str) -> anyhow::Result<QrState> {
        let mut spec = RequestSpec {
            url: "/v2/get_userinfo_qrcode".into(),
            base_url: Some("https://login-user.kugou.com".into()),
            encrypt_type: EncryptType::Web,
            skip_user_auth: true,
            ..Default::default()
        };
        spec.params.insert("plat".into(), "4".into());
        spec.params.insert("appid".into(), crate::crypto::APPID.to_string());
        spec.params.insert("srcappid".into(), "2919".into());
        spec.params.insert("qrcode".into(), key.to_string());
        spec.params.insert("dev".into(), self.device().mid);

        let res = self.request(spec).await?;
        let data = res.get("data").cloned().unwrap_or(Value::Null);
        let status = data.get("status").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
        let mut state = QrState {
            status,
            ..Default::default()
        };
        if status == 4 {
            state.token = crate::normalize::pick_str(&data, &["token"]);
            state.t1 = crate::normalize::pick_str(&data, &["t1"]);
            state.userid = data.get("userid").and_then(|v| v.as_u64()).unwrap_or_else(|| {
                data.get("userid")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0)
            });
            state.nickname = crate::normalize::pick_str(&data, &["nickname", "username"]);
            state.pic = crate::normalize::pick_str(&data, &["pic", "user_pic", "avatar"]);
            state.vip_type = data.get("vip_type").and_then(|v| v.as_i64()).unwrap_or(0);
            if !state.token.is_empty() {
                self.set_auth(&state.token, state.userid);
                self.set_t1(&state.t1);
            }
        }
        Ok(state)
    }

    /// 用户详情（`/v1/get_my_userinfo`，relation.user.kugou.com）。
    ///
    /// 旧的 `/v3/get_my_info` 已失效（20006）。
    /// VIP 权益（`/v1/get_union_vip`，kugouvip.kugou.com）。
    ///
    /// 返回该账号的会员产品状态（svip / qvip / dvip），用于判断是否 VIP。
    pub async fn vip_detail(&self) -> anyhow::Result<Value> {
        let mut spec = RequestSpec {
            url: "/v1/get_union_vip".into(),
            base_url: Some("https://kugouvip.kugou.com".into()),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("busi_type".into(), "concept".into());
        spec.params
            .insert("opt_product_types".into(), "dvip,qvip".into());
        spec.params.insert("product_type".into(), "svip".into());

        let res = self.request(spec).await?;
        Ok(res.pointer("/data").cloned().unwrap_or(res))
    }

    pub async fn user_detail(&self) -> anyhow::Result<echo_core::User> {
        let auth = self.auth();
        let dev = self.device();
        let clienttime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        let pk_plain = serde_json::json!({
            "clienttime": clienttime,
            "token": auth.token,
        });
        let pk = crate::crypto::rsa_encrypt_hex(pk_plain.to_string().as_bytes(), self.lite)?
            .to_uppercase();
        let key = crate::crypto::sign_params_key(
            &clienttime.to_string(),
            self.appid,
            self.clientver,
            self.lite,
        );

        let body = serde_json::json!({
            "p": pk,
            "appid": self.appid,
            "mid": dev.mid,
            "clientver": self.clientver,
            "source": 0,
            "clienttime": clienttime,
            "uuid": "-",
            "userid": auth.userid,
            "key": key,
        });
        let spec = RequestSpec {
            url: "/v1/get_my_userinfo".into(),
            // 该域名只提供 http（https 直接连接失败）
            base_url: Some("http://relation.user.kugou.com".into()),
            body: Some(body),
            encrypt_type: EncryptType::None,
            ..Default::default()
        };

        let res = self.request(spec).await?;
        let data = res
            .pointer("/data")
            .or_else(|| res.pointer("/userinfo"))
            .cloned()
            .unwrap_or(Value::Null);
        let mut user = crate::normalize::parse_user(&data);
        if user.userid == 0 {
            user.userid = auth.userid;
        }
        user.token = auth.token.clone();
        // `get_my_userinfo` 不返回会员等级，用 VIP 权益接口补齐
        if user.vip_type == 0 {
            if let Ok(detail) = self.vip_detail().await {
                user.vip_type = Self::vip_type_from_detail(&detail);
            }
        }
        Ok(user)
    }

    /// 私人 FM（`/v2/personal_recommend` → persnfm.service.kugou.com）。
    ///
    /// 和 `fm_songs`（私人漫游，按种子歌推荐）不同，这个是**按听歌口味**推的电台，
    /// 会把当前这一批放完再要下一批（`remain_songcnt` 带上剩余数量）。
    /// 响应在 `data.song_list`。
    pub async fn personal_fm(
        &self,
        remain_songcnt: i64,
        cur_mark: i64,
        action: &str,
        mode: &str,
        song_pool_id: i64,
    ) -> anyhow::Result<FmBatch> {
        let auth = self.auth();
        let dev = self.device();
        let clienttime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        let key = crate::crypto::sign_params_key(
            &clienttime.to_string(),
            self.appid,
            self.clientver,
            self.lite,
        );
        let mut body = serde_json::json!({
            "appid": self.appid,
            "clienttime": clienttime,
            "mid": dev.mid,
            "action": action,
            "recommend_source_locked": 0,
            "song_pool_id": song_pool_id,
            "callerid": 0,
            "m_type": 1,
            "platform": "ios",
            "area_code": 1,
            "remain_songcnt": remain_songcnt,
            "clientver": self.clientver,
            "is_overplay": 0,
            "mode": mode,
            "fakem": "ca981cfc583a4c37f28d2d49000013c16a0a",
            "key": key,
        });
        // 上一批返回的游标：不带回去服务端会认为「刚推过」，第二次直接给空列表
        if cur_mark > 0 {
            body["cur_mark"] = serde_json::json!(cur_mark);
        }
        if auth.userid != 0 {
            body["userid"] = serde_json::json!(auth.userid);
            body["kguid"] = serde_json::json!(auth.userid);
            if !auth.token.is_empty() {
                body["token"] = serde_json::json!(auth.token);
            }
        }

        let mut spec = RequestSpec {
            url: "/v2/personal_recommend".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("persnfm.service.kugou.com".into());

        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/song_list")
            .or_else(|| res.pointer("/data/songs"))
            .or_else(|| res.get("song_list"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(FmBatch {
            songs: parse_songs(&list),
            cur_mark: res
                .pointer("/data/cur_mark")
                .and_then(|v| v.as_i64())
                .unwrap_or(cur_mark),
        })
    }

}

/// 登录结果。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LoginResult {
    pub token: String,
    pub userid: u64,
    /// 续期凭证：下次 `/v5/login_by_token` 必须带上（否则 token 到期续不上）
    pub t1: String,
    /// token 到期时间（unix 秒，0 = 服务端没给）
    pub expires: i64,
    pub nickname: String,
    pub pic: String,
    pub vip_type: i64,
}

impl LoginResult {
    fn empty() -> Self {
        Self {
            token: String::new(),
            userid: 0,
            t1: String::new(),
            expires: 0,
            nickname: String::new(),
            pic: String::new(),
            vip_type: 0,
        }
    }

    /// 用一份响应数据补齐缺失字段（`data` → `secu_params` 解出来的那份）。
    fn fill_from(&mut self, v: &Value) {
        if self.token.is_empty() {
            self.token = crate::normalize::pick_str(v, &["token"]);
        }
        if self.userid == 0 {
            self.userid = crate::normalize::pick_i64(v, &["userid"]).max(0) as u64;
        }
        if self.t1.is_empty() {
            self.t1 = crate::normalize::pick_str(v, &["t1"]);
        }
        if self.expires == 0 {
            self.expires = crate::normalize::pick_i64(v, &["expires", "expire", "token_expires"]);
        }
        if self.nickname.is_empty() {
            self.nickname = crate::normalize::pick_str(v, &["nickname", "username"]);
        }
        if self.vip_type == 0 {
            self.vip_type = crate::normalize::pick_i64(v, &["vip_type"]);
        }
        if self.pic.is_empty() {
            self.pic = crate::normalize::pick_str(v, &["pic", "user_pic", "avatar"]);
        }
    }
}

/// 解析登录 / 刷新响应里的登录态。
///
/// 密码登录、验证码登录、token 刷新三条路径结构完全一样：明文在 `data`，
/// 隐私字段塞在 `data.secu_params`（用本次请求的 `temp_key` 解）。
/// 以前三处各抄了一份，且都只取了 token/userid/nickname/pic/vip_type，
/// 把 **`t1`（续期凭证）和 `expires`（到期时间）丢掉了** —— 于是每次刷新都
/// 拿不到新的 `t1`，下一次续期必失败，token 到期就成了「昨天登录今天过期」。
fn parse_login_payload(data: &Value, temp_key: Option<&str>) -> LoginResult {
    let mut out = LoginResult::empty();
    out.fill_from(data);
    if let (Some(key), Some(secu)) = (
        temp_key,
        data.get("secu_params").and_then(|v| v.as_str()),
    ) {
        if !secu.is_empty() {
            if let Ok(text) = crate::crypto::login_aes_decrypt(secu, key) {
                let trimmed = text.trim().to_string();
                match serde_json::from_str::<Value>(&trimmed) {
                    Ok(v) => out.fill_from(&v),
                    // secu_params 也可能直接就是一串 token
                    Err(_) => {
                        if out.token.is_empty() && !trimmed.is_empty() {
                            out.token = trimmed;
                        }
                    }
                }
            }
        }
    }
    out
}

/// 从失败响应里取可读原因。
fn response_error(res: &Value) -> String {
    res.get("error")
        .or_else(|| res.get("errmsg"))
        .or_else(|| res.get("msg"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("未返回播放地址")
        .to_string()
}

/// 扫码登录状态。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct QrState {
    /// 0 过期 / 1 等待扫码 / 2 已扫码待确认 / 4 授权成功
    pub status: i32,
    pub token: String,
    /// 续期凭证（扫码登录的响应里一般也有，没有就留空 → 靠后续刷新补）
    pub t1: String,
    pub userid: u64,
    pub nickname: String,
    pub pic: String,
    pub vip_type: i64,
}

/// token 刷新（`/v5/login_by_token`）里 p3 的固定 AES key/iv。
const REFRESH_AES_KEY: &str = "90b8382a1bb4ccdcf063102053fd75b8";
const REFRESH_AES_IV: &str = "f063102053fd75b8";
