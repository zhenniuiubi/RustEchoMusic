//! 用户歌单与红心收藏接口（需要登录态）。
//! 端点参考 KuGouMusicApi：`user_playlist` / `playlist_track_all_new`
//! / `playlist_tracks_add` / `playlist_tracks_del`。

use base64::Engine as _;

use crate::client::{EncryptType, KuGouClient, RequestSpec};
use echo_core::{Playlist, Song};
use serde_json::{json, Value};

impl KuGouClient {
    /// 用户歌单列表（`/v7/get_all_list`，x-router cloudlist.service.kugou.com）。
    ///
    /// 第一个通常就是「我喜欢的音乐」。
    pub async fn user_playlists(&self, page: i64, pagesize: i64) -> anyhow::Result<Vec<Playlist>> {
        let auth = self.auth();
        let body = json!({
            "userid": auth.userid,
            "token": auth.token,
            "total_ver": 979,
            "type": 2,
            "page": page,
            "pagesize": pagesize,
        });
        let mut spec = RequestSpec {
            url: "/v7/get_all_list".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("plat".into(), "1".into());
        spec.params.insert("userid".into(), auth.userid.to_string());
        spec.params.insert("token".into(), auth.token.clone());
        spec.x_router = Some("cloudlist.service.kugou.com".into());

        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/list")
            .or_else(|| res.pointer("/data/info"))
            .or_else(|| res.pointer("/data/lists"))
            .or_else(|| res.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        let mut out = crate::normalize::parse_playlists(&list);
        // 用户歌单接口的封面字段名与普通歌单不同，补一次兜底解析
        if let Some(arr) = list.as_array() {
            for (i, pl) in out.iter_mut().enumerate() {
                let raw = &arr[i];
                if pl.pic.is_empty() {
                    if let Some(p) = raw
                        .get("imgurl")
                        .or_else(|| raw.get("pic"))
                        .or_else(|| raw.get("cover"))
                        .and_then(|v| v.as_str())
                    {
                        pl.pic = crate::normalize::normalize_cover(p, 480);
                    }
                }
                if pl.count == 0 {
                    pl.count = raw.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
                }
            }
        }
        Ok(out)
    }

    /// 用户歌单歌曲（`/v4/get_list_all_file_v3`，按 listid 取）。
    pub async fn user_playlist_tracks(
        &self,
        listid: &str,
        page: i64,
        pagesize: i64,
    ) -> anyhow::Result<Vec<Song>> {
        let auth = self.auth();
        let body = json!({
            "listid": listid,
            "userid": auth.userid,
            "token": auth.token,
            "type": 0,
            "page": page,
            "pagesize": pagesize,
            "area_code": 1,
            "allplatform": 1,
            "show_cover": 1,
        });
        let mut spec = RequestSpec {
            url: "/v4/get_list_all_file_v3".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("cloudlist.service.kugou.com".into());

        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/file_list")
            .or_else(|| res.pointer("/data/info"))
            .or_else(|| res.pointer("/data/list"))
            .or_else(|| res.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(crate::normalize::parse_songs(&list))
    }

    /// 「我喜欢的音乐」歌单的 listid（找不到返回 None）。
    pub async fn liked_playlist(&self) -> anyhow::Result<Option<Playlist>> {
        let list = self.user_playlists(1, 50).await?;
        // 优先按名字匹配；否则退回第一个（酷狗固定把红心歌单置顶）
        Ok(list
            .iter()
            .find(|p| p.is_liked() && !p.listid.is_empty())
            .or_else(|| list.iter().find(|p| p.is_liked()))
            .or_else(|| list.first())
            .cloned())
    }

    /// 「我喜欢的音乐」全部歌曲（未登录或出错时回落到本地收藏）。
    pub async fn liked_songs(&self) -> anyhow::Result<Vec<Song>> {
        let pl = match self.liked_playlist().await? {
            Some(p) if !p.listid.is_empty() => p,
            _ => anyhow::bail!("未找到「我喜欢的音乐」歌单"),
        };
        let mut out = Vec::new();
        let pagesize = 300i64;
        for page in 1..=5 {
            let batch = self.user_playlist_tracks(&pl.listid, page, pagesize).await?;
            if batch.is_empty() {
                break;
            }
            let n = batch.len();
            out.extend(batch);
            if (n as i64) < pagesize {
                break;
            }
        }
        Ok(out)
    }

    /// 判断歌曲是否已在红心歌单里（用 mixsongid 匹配）。
    pub async fn is_liked_song(&self, song: &Song) -> anyhow::Result<bool> {
        let songs = self.user_playlist_tracks_of_first().await.unwrap_or_default();
        let key = song.mix_song_id.clone();
        Ok(songs
            .iter()
            .any(|s| !key.is_empty() && s.mix_song_id == key))
    }

    async fn user_playlist_tracks_of_first(&self) -> anyhow::Result<Vec<Song>> {
        let pl = match self.liked_playlist().await? {
            Some(p) if !p.listid.is_empty() => p,
            _ => anyhow::bail!("未找到红心歌单"),
        };
        self.user_playlist_tracks(&pl.listid, 1, 300).await
    }

    /// 加入红心（`/cloudlist.service/v6/add_song`）。
    pub async fn like_song(&self, song: &Song, listid: &str) -> anyhow::Result<()> {
        self.like_song_with_fileid(song, listid, None).await
    }

    /// 加入红心（可指定云歌单条目 id）。
    pub async fn like_song_with_fileid(
        &self,
        song: &Song,
        listid: &str,
        fileid: Option<i64>,
    ) -> anyhow::Result<()> {
        let auth = self.auth();
        if auth.userid == 0 {
            anyhow::bail!("未登录");
        }
        let clienttime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let resource = json!([{
            "number": 1,
            "name": song.title,
            "hash": song.hash,
            "size": 0,
            "sort": 0,
            "timelen": song.duration,
            "bitrate": 0,
            "album_id": song.album_id.parse::<i64>().unwrap_or(0),
            "mixsongid": song.mix_song_id.parse::<i64>().unwrap_or(0),
            "fileid": fileid.unwrap_or(0),
        }]);
        let body = json!({
            "userid": auth.userid,
            "token": auth.token,
            "listid": listid,
            "list_ver": 0,
            "type": 0,
            "slow_upload": 1,
            "scene": "false;null",
            "data": resource,
        });
        let mut spec = RequestSpec {
            url: "/cloudlist.service/v6/add_song".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("last_time".into(), clienttime.to_string());
        spec.params.insert("last_area".into(), "gztx".into());
        spec.params.insert("userid".into(), auth.userid.to_string());
        spec.params.insert("token".into(), auth.token.clone());

        let res = self.request(spec).await?;
        check_ok(&res, "收藏失败")
    }

    /// 取消红心（`/v4/delete_songs`）。
    pub async fn unlike_song(&self, song: &Song, listid: &str) -> anyhow::Result<()> {
        self.unlike_song_with_fileid(song, listid, None).await
    }

    /// 取消红心（可指定云歌单条目 id）。
    pub async fn unlike_song_with_fileid(
        &self,
        song: &Song,
        listid: &str,
        fileid: Option<i64>,
    ) -> anyhow::Result<()> {
        let auth = self.auth();
        if auth.userid == 0 {
            anyhow::bail!("未登录");
        }
        // fileid：云歌单里是 `audio_id`（条目 id），兼容 mixsongid / album_audio_id
        let fileid = fileid
            .or_else(|| song.cloud_audio_id.parse::<i64>().ok().filter(|v| *v > 0))
            .or_else(|| song.mix_song_id.parse::<i64>().ok().filter(|v| *v > 0))
            .or_else(|| song.album_audio_id.parse::<i64>().ok().filter(|v| *v > 0))
            .unwrap_or(0);
        let body = json!({
            "listid": listid,
            "userid": auth.userid,
            "token": auth.token,
            "data": [{ "fileid": fileid }],
            "type": 0,
            "list_ver": 0,
        });
        let mut spec = RequestSpec {
            url: "/v4/delete_songs".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("plat".into(), "1".into());
        spec.x_router = Some("cloudlist.service.kugou.com".into());

        let res = self.request(spec).await?;
        check_ok(&res, "取消收藏失败")
    }
}

/// 检查业务返回码，非成功时抛出可读错误。
fn check_ok(res: &Value, fallback: &str) -> anyhow::Result<()> {
    if let Some(err) = response_error_opt(res) {
        anyhow::bail!("{err}");
    }
    // `status` 字段：1 / 0 均视为成功时直接放行
    let status = res.get("status").and_then(|v| v.as_i64());
    let err_code = res.get("err_code").and_then(|v| v.as_i64()).unwrap_or(0);
    if err_code != 0 || status == Some(-1) {
        anyhow::bail!("{fallback}（err_code={err_code}）");
    }
    Ok(())
}

fn response_error_opt(res: &Value) -> Option<String> {
    res.get("error")
        .or_else(|| res.get("errmsg"))
        .or_else(|| res.get("msg"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("ok"))
        .map(|s| s.to_string())
}
impl KuGouClient {
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

    /// 从 VIP 权益响应里推断 vip_type（0 表示非 VIP）。
    ///
    /// 顶层 `vip_type` / `is_vip` 指的是传统会员；`busi_vip[]` 数组里才是
    /// 各业务线（concept 等）的会员，`is_vip=1` 且 `product_type=svip` 即超级 VIP。
    pub fn vip_type_from_detail(detail: &Value) -> i64 {
        let mut best = 0i64;
        // 传统会员
        if let Some(n) = detail.get("vip_type").and_then(|v| v.as_i64()) {
            best = best.max(n);
        }
        if detail.get("is_vip").and_then(|v| v.as_i64()).unwrap_or(0) == 1 {
            best = best.max(1);
        }
        // 业务线会员
        if let Some(arr) = detail.get("busi_vip").and_then(|v| v.as_array()) {
            for item in arr {
                if item.get("is_vip").and_then(|v| v.as_i64()).unwrap_or(0) != 1 {
                    continue;
                }
                let pt = item
                    .get("product_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let level = match pt {
                    "svip" | "super_vip" => 4,
                    "qvip" | "vip" => 2,
                    _ => 1,
                };
                best = best.max(level);
            }
        }
        best
    }
}

// ==================== 已购音乐 / 音乐云盘 ====================

impl KuGouClient {
    /// 已购单曲（`/openapi/copyright/v1/audio/get_goods`）。
    ///
    /// 注意 `pagesize` 不能超过 50（实测 100 会返回 `20010`）。
    pub async fn purchased_songs(&self, page: i64, pagesize: i64) -> anyhow::Result<Vec<Song>> {
        let pagesize = pagesize.clamp(1, 50);
        let auth = self.auth();
        let body = json!({
            "appid": self.appid_public(),
            "userid": auth.userid,
            "token": auth.token,
            "page": page,
            "pagesize": pagesize,
            "clientver": self.clientver_public().to_string(),
            "deleted": 0,
            "need_audio_info": 1,
            "area_code": "1",
        });
        log::debug!(
            "purchased_songs req: userid={} token_len={} play_auth_len={} appid={} clientver={}",
            auth.userid,
            auth.token.len(),
            auth.auth.len(),
            self.appid_public(),
            self.clientver_public()
        );
        let spec = RequestSpec {
            url: "/openapi/copyright/v1/audio/get_goods".into(),
            encrypt_type: EncryptType::Android,
            body: Some(body),
            ..Default::default()
        };
        let res = self.request(spec).await?;
        // 响应里 songs / lists / data 都可能是数组，交给通用解析
        Ok(crate::normalize::parse_songs(&res))
    }

    /// 已购专辑（`/openapi/v1/copyright/get_album_goods`）。
    pub async fn purchased_albums(
        &self,
        page: i64,
        pagesize: i64,
    ) -> anyhow::Result<Vec<echo_core::Album>> {
        let pagesize = pagesize.clamp(1, 15);
        let auth = self.auth();
        let body = json!({
            "appid": self.appid_public(),
            "userid": auth.userid,
            "token": auth.token,
            "page": page,
            "pagesize": pagesize,
            "clientver": self.clientver_public().to_string(),
            "deleted": 0,
        });
        let spec = RequestSpec {
            url: "/openapi/v1/copyright/get_album_goods".into(),
            encrypt_type: EncryptType::Android,
            body: Some(body),
            ..Default::default()
        };
        let res = self.request(spec).await?;
        Ok(crate::normalize::parse_albums(&res))
    }

    /// 音乐云盘（`mcloudservice.kugou.com/v1/get_list`）。
    ///
    /// 这条链路和别的接口都不一样：
    /// 1. `p` = RSA(PKCS1v1.5) 加密 `{aes: 6位种子, uid, token}`，大写 hex；
    /// 2. body = 明文 `{page,pagesize,getkmr:1}` 用该种子 AES-CBC 加密后的**原始字节**；
    /// 3. 响应是密文，用同一个种子解密成 JSON。
    /// 云歌单（cloudlist.service）的加密 POST：
    /// 请求体 AES-CBC(seed) 加密，`p` = RSA({aes,uid,token})，签名 key 走标准版盐。
    ///
    /// 与「音乐云盘」(`mcloudservice`) 是同一套协议，只是路由与 body 不同。
    async fn cloudlist_post(&self, path: &str, plain: serde_json::Value) -> anyhow::Result<Value> {
        let auth = self.auth();
        if auth.userid == 0 || auth.token.is_empty() {
            anyhow::bail!("需要登录");
        }
        let plain = plain.to_string();
        let (seed, cipher_b64) = crate::crypto::playlist_aes_encrypt(&plain);
        let p_plain = serde_json::json!({
            "aes": seed,
            "uid": auth.userid,
            "token": auth.token,
        })
        .to_string();
        let p = crate::crypto::rsa_encrypt_hex(p_plain.as_bytes(), self.lite)?.to_uppercase();

        let clienttime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
            .to_string();
        let key = crate::crypto::sign_params_key(
            &clienttime,
            self.appid_public(),
            self.clientver_public(),
            self.lite,
        );
        let body = base64::engine::general_purpose::STANDARD
            .decode(cipher_b64.as_bytes())
            .map_err(|e| anyhow::anyhow!("请求体 base64 解码失败：{e}"))?;

        let spec = RequestSpec {
            url: path.into(),
            body: Some(serde_json::Value::String(String::from_utf8_lossy(&body).to_string())),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        let mut spec = spec;
        spec.x_router = Some("cloudlist.service.kugou.com".into());
        spec.params.insert("clienttime".into(), clienttime.clone());
        spec.params.insert("key".into(), key);
        spec.params.insert("p".into(), p);
        spec.params.insert("last_time".into(), clienttime);
        spec.params.insert("last_area".into(), "gztx".into());

        // 响应同样是 AES 密文
        let res = self.request_bytes(spec).await?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(res);
        let bytes = crate::crypto::playlist_aes_decrypt(&seed, &b64)?;
        let text = String::from_utf8_lossy(&bytes).to_string();
        serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("cloudlist 响应解密后不是 JSON：{e}; {:.200}", text))
    }

    /// 新建歌单（`type=0`）或收藏别人的歌单（`type=1`）。
    pub async fn playlist_add(
        &self,
        name: &str,
        collect: bool,
        src_listid: &str,
        src_userid: &str,
        src_gid: &str,
    ) -> anyhow::Result<()> {
        let auth = self.auth();
        let spec = RequestSpec {
            url: "/cloudlist.service/v5/add_list".into(),
            body: Some(serde_json::json!({
                "userid": auth.userid,
                "token": auth.token,
                "total_ver": 0,
                "name": name,
                "type": if collect { 1 } else { 0 },
                "source": 1,
                "is_pri": 0,
                "list_create_userid": src_userid,
                "list_create_listid": src_listid,
                "list_create_gid": src_gid,
                "from_shupinmv": 0,
            })),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        self.request(spec).await?;
        Ok(())
    }

    /// 删除（取消收藏）歌单：`/v2/delete_list`。
    pub async fn playlist_delete(&self, listid: &str) -> anyhow::Result<()> {
        let res = self
            .cloudlist_post(
                "/v2/delete_list",
                serde_json::json!({ "listid": listid, "total_ver": 0, "type": 1 }),
            )
            .await?;
        let st = res.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
        if st != 1 {
            anyhow::bail!(
                "删除歌单失败：{}",
                res.get("error_msg")
                    .or_else(|| res.get("msg"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("未知错误")
            );
        }
        Ok(())
    }

    /// 歌单排序：`data` 形如 `listid|type|sort,listid|type|sort`。
    pub async fn playlist_sort(&self, total_ver: i64, data: &str) -> anyhow::Result<()> {
        let resource: Vec<serde_json::Value> = data
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .filter_map(|s| {
                let parts: Vec<&str> = s.split('|').collect();
                if parts.is_empty() {
                    return None;
                }
                let listid: i64 = parts[0].trim().parse().ok()?;
                let ty: i64 = parts.get(1).and_then(|x| x.trim().parse().ok()).unwrap_or(0);
                let sort: i64 = parts.get(2).and_then(|x| x.trim().parse().ok()).unwrap_or(0);
                Some(serde_json::json!({ "listid": listid, "type": ty, "sort": sort }))
            })
            .collect();
        let res = self
            .cloudlist_post(
                "/v1/modify_list_sort",
                serde_json::json!({ "total_ver": total_ver, "data": resource }),
            )
            .await?;
        let st = res.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
        if st != 1 {
            anyhow::bail!("歌单排序失败");
        }
        Ok(())
    }

    pub async fn cloud_songs(&self, page: i64, pagesize: i64) -> anyhow::Result<Vec<Song>> {
        let pagesize = pagesize.clamp(1, 30);
        let auth = self.auth();
        if auth.userid == 0 || auth.token.is_empty() {
            anyhow::bail!("音乐云盘需要登录");
        }
        let plain = json!({
            "page": page,
            "pagesize": pagesize,
            "getkmr": 1,
        })
        .to_string();
        let (seed, cipher_b64) = crate::crypto::playlist_aes_encrypt(&plain);
        let p_plain = json!({
            "aes": seed,
            "uid": auth.userid,
            "token": auth.token,
        })
        .to_string();
        let p = crate::crypto::rsa_encrypt_hex(p_plain.as_bytes(), self.lite)?.to_uppercase();

        let clienttime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
            .to_string();
        let key = crate::crypto::sign_params_key(
            &clienttime,
            self.appid_public(),
            self.clientver_public(),
            self.lite,
        );
        let body = base64::engine::general_purpose::STANDARD
            .decode(cipher_b64.as_bytes())
            .map_err(|e| anyhow::anyhow!("云盘请求体 base64 解码失败：{e}"))?;

        let params: Vec<(String, String)> = vec![
            ("clienttime".into(), clienttime),
            ("mid".into(), self.device().mid),
            ("key".into(), key),
            ("clientver".into(), self.clientver_public().to_string()),
            ("appid".into(), self.appid_public().to_string()),
            ("p".into(), p),
        ];
        let resp = self
            .post_cloud_raw("https://mcloudservice.kugou.com/v1/get_list", &params, body)
            .await?;

        let b64 = base64::engine::general_purpose::STANDARD.encode(resp);
        let bytes = crate::crypto::playlist_aes_decrypt(&seed, &b64)?;
        let text = String::from_utf8_lossy(&bytes).to_string();
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("云盘响应解密后不是 JSON：{e}; body={:.200}", text))?;
        Ok(crate::normalize::parse_cloud_songs(&v))
    }
}

impl KuGouClient {
    /// 云盘歌曲取播放地址（`/bsstrackercdngz/v2/query_musicclound_url`）。
    ///
    /// 云盘里的歌是用户自己上传的，普通 `/tracker/v5/url` 未必给地址；
    /// 这个接口直接按 bucket=musicclound 查原始文件（返回的通常是原始 mp3）。
    pub async fn cloud_song_url(&self, song: &echo_core::Song) -> anyhow::Result<String> {
        let hash = song.hash.to_lowercase();
        if hash.is_empty() {
            anyhow::bail!("云盘歌曲缺少 hash");
        }
        let mut params = std::collections::BTreeMap::new();
        params.insert("hash".to_string(), hash.clone());
        params.insert("ssa_flag".to_string(), "is_fromtrack".to_string());
        params.insert("version".to_string(), "20102".to_string());
        params.insert("ssl".to_string(), "0".to_string());
        params.insert(
            "album_audio_id".to_string(),
            song.play_album_audio_id().to_string(),
        );
        params.insert("pid".to_string(), CLOUD_PID.to_string());
        params.insert(
            "audio_id".to_string(),
            song.audio_id.parse::<i64>().unwrap_or(0).to_string(),
        );
        params.insert("kv_id".to_string(), "1".to_string());
        params.insert(
            "key".to_string(),
            crate::crypto::sign_cloud_key(&hash, CLOUD_PID),
        );
        params.insert("bucket".to_string(), "musicclound".to_string());
        params.insert(
            "name".to_string(),
            if song.title.is_empty() {
                hash.clone()
            } else {
                song.title.clone()
            },
        );
        params.insert("with_res_tag".to_string(), "0".to_string());

        let spec = RequestSpec {
            url: "/bsstrackercdngz/v2/query_musicclound_url".into(),
            params,
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        let res = self.request(spec).await?;
        let url = res
            .get("data")
            .and_then(|d| d.get("url"))
            .or_else(|| res.get("url"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if url.is_empty() {
            anyhow::bail!("云盘取地址失败：接口没返回 url");
        }
        Ok(url.to_string())
    }
}

/// 云盘接口固定用的 pid（对齐 KuGouMusicApi `user_cloud_url.js`）。
const CLOUD_PID: &str = "20026";
