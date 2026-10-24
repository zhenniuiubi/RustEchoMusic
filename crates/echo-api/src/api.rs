//! 高层 API 方法（第一期集合：首页 / 搜索 / 歌单 / 播放地址 / 榜单）。
//!
//! 每个方法对应原项目 `src/renderer/api/*` + `server/module/*` 中的一个接口。

use crate::client::{EncryptType, KuGouClient, RequestSpec};
use crate::normalize::{parse_playlists, parse_rank, parse_songs};
use echo_core::{Playlist, RankMeta, Song};
use serde_json::{json, Value};

/// 风格推荐标签。
#[derive(Debug, Clone, serde::Serialize)]
pub struct StyleTag {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// 风格推荐分类（含标签）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct StyleCategory {
    pub name: String,
    pub tags: Vec<StyleTag>,
}

/// 风格推荐整体结果。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StyleRecommend {
    pub categories: Vec<StyleCategory>,
    pub songs: Vec<Song>,
}

/// 风格推荐分组。
#[derive(Debug, Clone, serde::Serialize)]
pub struct StyleGroup {
    pub id: String,
    pub name: String,
    pub songs: Vec<Song>,
}

/// 私人 FM 一批推荐（`cur_mark` 要带回下一次请求）。
#[derive(Debug, Clone, Default)]
pub struct FmBatch {
    pub songs: Vec<Song>,
    pub cur_mark: i64,
}

impl KuGouClient {
    // ==================== 首页 ====================

    /// 推荐歌单（原 getPlaylistByCategory → top_playlist → `/v2/special_recommend`）。
    /// `category_id`：0=推荐，11292=Hi-Res。
    pub async fn recommend_playlists(
        &self,
        category_id: i64,
        page: i64,
        pagesize: i64,
    ) -> anyhow::Result<Vec<Playlist>> {
        let clienttime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
            .to_string();
        let auth = self.auth();
        let key = crate::crypto::sign_params_key(&clienttime, crate::crypto::APPID, crate::crypto::CLIENTVER, false);

        let body = json!({
            "appid": crate::crypto::APPID,
            "mid": self.device().mid,
            "clientver": crate::crypto::CLIENTVER,
            "platform": "android",
            "clienttime": clienttime,
            "userid": auth.userid,
            "module_id": 1,
            "page": page,
            "pagesize": pagesize,
            "key": key,
            "special_recommend": {
                "withtag": 1,
                "withsong": 1,
                "sort": 1,
                "ugc": 1,
                "is_selected": 0,
                "withrecommend": 1,
                "area_code": 1,
                "categoryid": category_id,
            },
            "req_multi": 1,
            "retrun_min": 5,
            "return_special_falg": 1,
        });

        let mut spec = RequestSpec {
            url: "/v2/special_recommend".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("specialrec.service.kugou.com".into());

        let res = self.request(spec).await?;
        let lists = res
            .pointer("/data/special_list")
            .or_else(|| res.pointer("/data/lists"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(parse_playlists(&lists))
    }

    /// 编辑精选（原 getTopIP → `/v1/daily_recommend`）。
    pub async fn top_ip(&self) -> anyhow::Result<Vec<Playlist>> {
        let mut spec = RequestSpec {
            url: "/v1/daily_recommend".into(),
            body: Some(json!({ "tags": {} })),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.base_url = Some("http://musicadservice.kugou.com".into());
        spec.params.insert("clientver".into(), "12349".into());
        spec.params.insert("area_code".into(), "1".into());

        let res = self.request(spec).await?;
        let lists = res.pointer("/data/list").cloned().unwrap_or(Value::Null);
        // 编辑精选条目带 ip 信息，歌单 id 取 global_collection_id / ip_id
        let mut out = Vec::new();
        if let Some(arr) = lists.as_array() {
            for item in arr {
                let mut pl = crate::normalize::parse_playlist(item);
                if pl.id.is_empty() {
                    if let Some(ip) = item.pointer("/extra/ip_id").and_then(|v| v.as_i64()) {
                        pl.id = ip.to_string();
                    }
                }
                if !pl.id.is_empty() {
                    out.push(pl);
                }
            }
        }
        Ok(out)
    }

    /// 风格推荐（原 getEverydayStyleRecommend → `/everydayrec.service/everyday_style_recommend`）。
    ///
    /// 响应结构：
    /// - `data.tag_info[]` = `{ name, child: [{ id, name, default }] }`（分类 → 标签）
    /// - `data.song_list[]` = 歌曲列表
    pub async fn style_recommend(&self, tagids: &str) -> anyhow::Result<StyleRecommend> {
        let mut spec = RequestSpec {
            url: "/everydayrec.service/everyday_style_recommend".into(),
            body: Some(json!({})),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("tagids".into(), tagids.into());

        let res = self.request(spec).await?;
        let data = res.get("data").cloned().unwrap_or(Value::Null);

        // 分类 + 标签
        let mut categories = Vec::new();
        if let Some(groups) = data.get("tag_info").and_then(|x| x.as_array()) {
            for group in groups {
                let name = crate::normalize::pick_str(group, &["name", "title"]);
                let mut tags = Vec::new();
                if let Some(children) = group
                .get("son")
                .or_else(|| group.get("child"))
                .and_then(|x| x.as_array())
            {
                    for tag in children {
                        let id = crate::normalize::pick_str(tag, &["id", "tagid", "tag_id"]);
                        let tag_name = crate::normalize::pick_str(tag, &["name", "tagname"]);
                        let is_default = tag
                            .get("default")
                            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                            .map(|v| v == 1)
                            .unwrap_or(false);
                        if !id.is_empty() && !tag_name.is_empty() {
                            tags.push(StyleTag { id, name: tag_name, is_default });
                        }
                    }
                }
                if !name.is_empty() && !tags.is_empty() {
                    categories.push(StyleCategory { name, tags });
                }
            }
        }

        // 歌曲
        let songs = data
            .get("song_list")
            .map(parse_songs)
            .unwrap_or_default();

        Ok(StyleRecommend { categories, songs })
    }

    // ==================== 搜索 ====================

    /// 热搜（原 getSearchHot → `/api/v3/search/hot_tab`）。
    pub async fn search_hot(&self) -> anyhow::Result<Vec<String>> {
        let mut spec = RequestSpec {
            url: "/api/v3/search/hot_tab".into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("navid".into(), "1".into());
        spec.params.insert("plat".into(), "2".into());
        spec.x_router = Some("msearch.kugou.com".into());

        let res = self.request(spec).await?;
        let mut out = Vec::new();
        // 结构：data.list[].keywords[].keyword
        if let Some(groups) = res.pointer("/data/list").and_then(|x| x.as_array()) {
            for group in groups {
                if let Some(kws) = group.get("keywords").and_then(|x| x.as_array()) {
                    for item in kws {
                        let w = crate::normalize::pick_str(item, &["keyword", "kw", "word", "name"]);
                        if !w.is_empty() && !out.contains(&w) {
                            out.push(w);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// 搜索联想（原 getSearchSuggest → `/v2/getSearchTip`）。
    pub async fn search_suggest(&self, keyword: &str) -> anyhow::Result<Vec<String>> {
        let mut spec = RequestSpec {
            url: "/v2/getSearchTip".into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("keyword".into(), keyword.into());
        spec.params.insert("AlbumTipCount".into(), "5".into());
        spec.params.insert("CorrectTipCount".into(), "5".into());
        spec.params.insert("MVTipCount".into(), "0".into());
        spec.params.insert("MusicTipCount".into(), "10".into());
        spec.params.insert("radiotip".into(), "1".into());
        spec.x_router = Some("searchtip.kugou.com".into());

        let res = self.request(spec).await?;
        let mut out = Vec::new();
        // 常见形态：data.correctTip / data.musicTip[].keyword 等
        for key in ["correctTip", "musicTip", "albumTip", "tips"] {
            if let Some(arr) = res
                .pointer(&format!("/data/{key}"))
                .or_else(|| res.pointer(&format!("/data/{key}/lists")))
                .and_then(|x| x.as_array())
            {
                for item in arr {
                    let w = crate::normalize::pick_str(item, &["keyword", "kw", "name", "word"]);
                    if !w.is_empty() && !out.contains(&w) {
                        out.push(w);
                    }
                }
                if !out.is_empty() {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// 歌曲搜索（原 search(type=song) → `/v2/search/song`）。
    pub async fn search_songs(&self, keyword: &str, page: i64, pagesize: i64) -> anyhow::Result<Vec<Song>> {
        let mut spec = RequestSpec {
            url: "/v2/search/song".into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("keyword".into(), keyword.into());
        spec.params.insert("page".into(), page.to_string());
        spec.params.insert("pagesize".into(), pagesize.to_string());
        spec.params.insert("platform".into(), "AndroidFilter".into());
        spec.params.insert("iscorrection".into(), "1".into());
        spec.params.insert("privilegefilter".into(), "0".into());
        spec.params.insert("area_code".into(), "1".into());
        spec.params.insert("dopicfull".into(), "1".into());
        spec.x_router = Some("complexsearch.kugou.com".into());

        let res = self.request(spec).await?;
        let lists = res
            .pointer("/data/lists")
            .cloned()
            .unwrap_or(Value::Null);
        Ok(parse_songs(&lists))
    }

    // ==================== 歌单 ====================


    /// 搜索歌单（`/v1/search/special`）。
    pub async fn search_playlists(
        &self,
        keyword: &str,
        page: i64,
        pagesize: i64,
    ) -> anyhow::Result<Vec<Playlist>> {
        let mut spec = RequestSpec {
            url: "/v1/search/special".into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("albumhide".into(), "0".into());
        spec.params.insert("iscorrection".into(), "1".into());
        spec.params.insert("keyword".into(), keyword.into());
        spec.params.insert("nocollect".into(), "0".into());
        spec.params.insert("page".into(), page.to_string());
        spec.params.insert("pagesize".into(), pagesize.to_string());
        spec.params.insert("platform".into(), "AndroidFilter".into());
        spec.x_router = Some("complexsearch.kugou.com".into());

        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/lists")
            .or_else(|| res.pointer("/data/info"))
            .or_else(|| res.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(parse_playlists(&list))
    }

    /// 搜索专辑（`/v1/search/album`）。
    pub async fn search_albums(
        &self,
        keyword: &str,
        page: i64,
        pagesize: i64,
    ) -> anyhow::Result<Vec<echo_core::Album>> {
        let mut spec = RequestSpec {
            url: "/v1/search/album".into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("keyword".into(), keyword.into());
        spec.params.insert("page".into(), page.to_string());
        spec.params.insert("pagesize".into(), pagesize.to_string());
        spec.params.insert("platform".into(), "AndroidFilter".into());
        spec.params.insert("iscorrection".into(), "1".into());
        spec.params.insert("category".into(), "1".into());
        spec.params.insert("sorttype".into(), "0".into());
        spec.params.insert("searchsong".into(), "0".into());
        spec.x_router = Some("complexsearch.kugou.com".into());

        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/lists")
            .or_else(|| res.pointer("/data/info"))
            .or_else(|| res.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(crate::normalize::parse_albums(&list))
    }

    /// 搜索歌手（`/v1/search/author`）。
    pub async fn search_artists(
        &self,
        keyword: &str,
        page: i64,
        pagesize: i64,
    ) -> anyhow::Result<Vec<echo_core::Artist>> {
        let mut spec = RequestSpec {
            url: "/v1/search/author".into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("albumhide".into(), "0".into());
        spec.params.insert("iscorrection".into(), "1".into());
        spec.params.insert("keyword".into(), keyword.into());
        spec.params.insert("nocollect".into(), "0".into());
        spec.params.insert("page".into(), page.to_string());
        spec.params.insert("pagesize".into(), pagesize.to_string());
        spec.params.insert("platform".into(), "AndroidFilter".into());
        spec.x_router = Some("complexsearch.kugou.com".into());

        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/lists")
            .or_else(|| res.pointer("/data/info"))
            .or_else(|| res.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(crate::normalize::parse_artist_list(&list))
    }

    /// 歌单详情（原 getPlaylistDetail → `/v3/get_list_info`）。
    pub async fn playlist_detail(&self, global_collection_id: &str) -> anyhow::Result<Option<Playlist>> {
        let auth = self.auth();
        let body = json!({
            "data": [{ "global_collection_id": global_collection_id }],
            "userid": auth.userid,
            "token": auth.token,
        });
        let mut spec = RequestSpec {
            url: "/v3/get_list_info".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("pubsongs.kugou.com".into());

        let res = self.request(spec).await?;
        let first = res
            .pointer("/data/list/0")
            .or_else(|| res.pointer("/data/0"))
            .cloned()
            .unwrap_or(Value::Null);
        if first.is_null() {
            return Ok(None);
        }
        Ok(Some(crate::normalize::parse_playlist(&first)))
    }

    /// 歌单歌曲（原 getPlaylistTracks → `/v5/get_list_songs`? 走 pubsongs）。
    /// KuGouMusicApi playlist_track：POST `/services/list/v2/get_list_songs`
    pub async fn playlist_tracks(
        &self,
        global_collection_id: &str,
        page: i64,
        pagesize: i64,
    ) -> anyhow::Result<Vec<Song>> {
        // 对齐原项目 playlist_track_all → GET `/pubsongs/v2/get_other_list_file_nofilt`
        let mut spec = RequestSpec {
            url: "/pubsongs/v2/get_other_list_file_nofilt".into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("area_code".into(), "1".into());
        spec.params.insert("begin_idx".into(), ((page - 1).max(0) * pagesize).to_string());
        spec.params.insert("plat".into(), "1".into());
        spec.params.insert("type".into(), "1".into());
        spec.params.insert("mode".into(), "1".into());
        spec.params.insert("personal_switch".into(), "1".into());
        spec.params
            .insert("extend_fields".into(), "abtags,hot_cmt,popularization".into());
        spec.params.insert("pagesize".into(), pagesize.to_string());
        spec.params
            .insert("global_collection_id".into(), global_collection_id.to_string());

        let res = self.request(spec).await?;
        let songs = res
            .pointer("/data/info")
            .or_else(|| res.pointer("/data/songs"))
            .or_else(|| res.pointer("/data/list"))
            .or_else(|| res.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(parse_songs(&songs))
    }

    // ==================== 播放地址 ====================

    /// 播放授权（原 getUserVerify → `/v1/user_verify`），返回 auth。
    ///
    /// 结果会缓存进客户端 Cookie（`auth=`），后续 `/v1/authorization` 复用。
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

    /// 榜单列表（原 getRanks → `/ocean/v6/rank/list`）。
    pub async fn rank_list(&self) -> anyhow::Result<Vec<RankMeta>> {
        let mut spec = RequestSpec {
            url: "/ocean/v6/rank/list".into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("plat".into(), "2".into());
        spec.params.insert("withsong".into(), "1".into());
        spec.params.insert("parentid".into(), "0".into());

        let res = self.request(spec).await?;
        let lists = res
            .pointer("/data/list")
            .or_else(|| res.pointer("/data/info"))
            .cloned()
            .unwrap_or(Value::Null);
        let mut out = Vec::new();
        if let Some(arr) = lists.as_array() {
            for item in arr {
                out.push(parse_rank(item));
            }
        }
        Ok(out)
    }

    /// 榜单歌曲（`/openapi/kmr/v2/rank/audio`，POST）。
    ///
    /// 旧的 `/ocean/v5/rank/audio` 已下线（404）。
    pub async fn rank_songs(&self, rankid: &str, page: i64, pagesize: i64) -> anyhow::Result<Vec<Song>> {
        self.rank_songs_with_cid(rankid, 0, page, pagesize).await
    }

    /// 榜单歌曲（带 rank_cid；cid 为 0 时服务端只回 1 首）。
    pub async fn rank_songs_with_cid(
        &self,
        rankid: &str,
        rank_cid: i64,
        page: i64,
        pagesize: i64,
    ) -> anyhow::Result<Vec<Song>> {
        let body = serde_json::json!({
            "show_portrait_mv": 1,
            "show_type_total": 1,
            "filter_original_remarks": 1,
            "area_code": 1,
            "pagesize": pagesize,
            "rank_cid": rank_cid,
            "type": 1,
            "page": page,
            "rank_id": rankid,
        });
        let mut spec = RequestSpec {
            url: "/openapi/kmr/v2/rank/audio".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.extra_headers.push(("kg-tid".into(), "369".into()));

        let res = self.request(spec).await?;
        let lists = res
            .pointer("/data/songlist")
            .or_else(|| res.pointer("/data/audios"))
            .or_else(|| res.pointer("/data/songs"))
            .or_else(|| res.pointer("/data/list"))
            .or_else(|| res.pointer("/data/info"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(parse_songs(&lists))
    }
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
    /// 密码登录（原 login → `/v9/login_by_pwd`）。
    /// 成功后写入内存登录态，并返回用户信息。
    pub async fn login_by_pwd(
        &self,
        username: &str,
        password: &str,
    ) -> anyhow::Result<crate::api::LoginResult> {
        let clienttime_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;

        let payload = serde_json::json!({
            "pwd": password,
            "code": "",
            "clienttime_ms": clienttime_ms,
        });
        let (params_hex, temp_key) = crate::crypto::login_aes_encrypt(&payload.to_string());

        let pk_plain = serde_json::json!({
            "clienttime_ms": clienttime_ms,
            "key": temp_key,
        });
        let pk = crate::crypto::rsa_encrypt_hex(pk_plain.to_string().as_bytes(), self.lite)?;

        let body = serde_json::json!({
            "plat": 1,
            "support_multi": 1,
            "clienttime_ms": clienttime_ms,
            "t1": LOGIN_T1,
            "t2": LOGIN_T2,
            "t3": "MCwwLDAsMCwwLDAsMCwwLDA=",
            "dev": "EchoMusic",
            "username": username,
            "params": params_hex,
            "pk": pk,
        });

        let mut spec = RequestSpec {
            url: "/v9/login_by_pwd".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("login.user.kugou.com".into());

        let res = self.request(spec).await?;
        let data = res.get("data").cloned().unwrap_or(Value::Null);

        let mut token = crate::normalize::pick_str(&data, &["token"]);
        let mut userid = data.get("userid").and_then(|v| v.as_i64()).unwrap_or(0) as u64;
        let mut nickname = crate::normalize::pick_str(&data, &["nickname", "username"]);
        let mut vip_type = data.get("vip_type").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut pic = crate::normalize::pick_str(&data, &["pic", "user_pic", "avatar"]);

        // secu_params 解密后可能是 JSON 对象，也可能是纯 token 字符串
        if let Some(secu) = data.get("secu_params").and_then(|v| v.as_str()) {
            if !secu.is_empty() {
                if let Ok(text) = crate::crypto::login_aes_decrypt(secu, &temp_key) {
                    let trimmed = text.trim().to_string();
                    if let Ok(v) = serde_json::from_str::<Value>(&trimmed) {
                        if token.is_empty() {
                            token = crate::normalize::pick_str(&v, &["token"]);
                        }
                        if userid == 0 {
                            userid = v.get("userid").and_then(|x| x.as_i64()).unwrap_or(0) as u64;
                        }
                        if nickname.is_empty() {
                            nickname = crate::normalize::pick_str(&v, &["nickname", "username"]);
                        }
                        if vip_type == 0 {
                            vip_type = v.get("vip_type").and_then(|x| x.as_i64()).unwrap_or(0);
                        }
                        if pic.is_empty() {
                            pic = crate::normalize::pick_str(&v, &["pic", "user_pic", "avatar"]);
                        }
                    } else if token.is_empty() && !trimmed.is_empty() {
                        token = trimmed;
                    }
                }
            }
        }

        if token.is_empty() {
            let msg = res
                .get("error")
                .or_else(|| res.get("errmsg"))
                .or_else(|| res.get("msg"))
                .and_then(|v| v.as_str())
                .unwrap_or("登录失败（账号或密码错误）");
            anyhow::bail!("{msg}");
        }

        self.set_auth(&token, userid);
        Ok(LoginResult { token, userid, nickname, pic, vip_type })
    }

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

        let mut token = crate::normalize::pick_str(&data, &["token"]);
        let mut userid = data.get("userid").and_then(|v| v.as_i64()).unwrap_or(0) as u64;
        let mut nickname = crate::normalize::pick_str(&data, &["nickname", "username"]);
        let mut vip_type = data.get("vip_type").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut pic = crate::normalize::pick_str(&data, &["pic", "user_pic", "avatar"]);

        if let Some(secu) = data.get("secu_params").and_then(|v| v.as_str()) {
            if !secu.is_empty() {
                if let Ok(text) = crate::crypto::login_aes_decrypt(secu, &temp_key) {
                    let trimmed = text.trim().to_string();
                    if let Ok(v) = serde_json::from_str::<Value>(&trimmed) {
                        if token.is_empty() {
                            token = crate::normalize::pick_str(&v, &["token"]);
                        }
                        if userid == 0 {
                            userid = v.get("userid").and_then(|x| x.as_i64()).unwrap_or(0) as u64;
                        }
                        if nickname.is_empty() {
                            nickname = crate::normalize::pick_str(&v, &["nickname", "username"]);
                        }
                        if vip_type == 0 {
                            vip_type = v.get("vip_type").and_then(|x| x.as_i64()).unwrap_or(0);
                        }
                        if pic.is_empty() {
                            pic = crate::normalize::pick_str(&v, &["pic", "user_pic", "avatar"]);
                        }
                    } else if token.is_empty() && !trimmed.is_empty() {
                        token = trimmed;
                    }
                }
            }
        }

        if token.is_empty() {
            anyhow::bail!("{}", response_error(&res));
        }
        if userid == 0 {
            userid = auth.userid;
        }

        self.set_auth(&token, userid);
        Ok(LoginResult { token, userid, nickname, pic, vip_type })
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
            }
        }
        Ok(state)
    }

    /// 发送手机验证码（`/v7/send_mobile_code`）。
    /// 注意：该域名仅支持 http（无 TLS），与原实现一致。
    pub async fn send_sms_code(&self, mobile: &str) -> anyhow::Result<()> {
        let body = json!({
            "businessid": 5,
            "mobile": mobile,
            "plat": 3,
        });
        let spec = RequestSpec {
            url: "/v7/send_mobile_code".into(),
            base_url: Some("http://login.user.kugou.com".into()),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            skip_user_auth: true,
            ..Default::default()
        };
        self.request(spec).await?;
        Ok(())
    }

    /// 验证码登录（`/v7/login_by_verifycode`）。
    pub async fn login_by_sms(&self, mobile: &str, code: &str) -> anyhow::Result<LoginResult> {
        let clienttime_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        let dev = self.device();

        let payload = json!({ "mobile": mobile, "code": code });
        let (params_hex, temp_key) = crate::crypto::login_aes_encrypt(&payload.to_string());

        // 概念版 t1/t2 指纹（对齐 login_cellphone.js）
        const LITE_T1_KEY: &str = "5e4ef500e9597fe004bd09a46d8add98";
        const LITE_T1_IV: &str = "04bd09a46d8add98";
        const LITE_T2_KEY: &str = "fd14b35e3f81af3817a20ae7adae7020";
        const LITE_T2_IV: &str = "17a20ae7adae7020";

        let t1 = crate::crypto::aes_cbc_encrypt_hex(
            LITE_T1_KEY.as_bytes(),
            LITE_T1_IV.as_bytes(),
            format!("|{clienttime_ms}").as_bytes(),
        );
        let t2_plain = format!(
            "{}|0f607264fc6318a92b9e13c65db7cd3c|02:00:00:00:00:00|EchoMusic|{clienttime_ms}",
            dev.guid
        );
        let t2 = crate::crypto::aes_cbc_encrypt_hex(
            LITE_T2_KEY.as_bytes(),
            LITE_T2_IV.as_bytes(),
            t2_plain.as_bytes(),
        );

        let pk_plain = json!({ "clienttime_ms": clienttime_ms, "key": temp_key });
        let pk = crate::crypto::rsa_encrypt_hex(pk_plain.to_string().as_bytes(), self.lite)?;

        // 手机号脱敏（对齐原实现：前 2 位 + ***** + 第 11 位）
        let digits: Vec<char> = mobile.chars().collect();
        let masked = if digits.len() >= 11 {
            format!(
                "{}*****{}",
                digits[0..2].iter().collect::<String>(),
                digits[10]
            )
        } else {
            mobile.to_string()
        };

        let body = json!({
            "plat": 1,
            "support_multi": 1,
            "t1": t1,
            "t2": t2,
            "clienttime_ms": clienttime_ms,
            "mobile": masked,
            "key": crate::crypto::sign_params_key(
                &clienttime_ms.to_string(),
                self.appid,
                self.clientver,
                self.lite
            ),
            "pk": pk,
            "params": params_hex,
            "dfid": dev.dfid,
            "dev": "EchoMusic",
            "gitversion": "5f0b7c4",
        });

        let mut spec = RequestSpec {
            url: "/v7/login_by_verifycode".into(),
            base_url: Some("https://loginserviceretry.kugou.com".into()),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            skip_user_auth: true,
            ..Default::default()
        };
        spec.extra_headers
            .push(("support-calm".into(), "1".into()));
        spec.user_agent =
            Some("Android16-1070-11440-130-0-LOGIN-wifi".into());

        let res = self.request(spec).await?;
        let data = res.get("data").cloned().unwrap_or(Value::Null);
        self.finish_login(&data, &temp_key, "验证码登录失败")
    }

    /// 从登录响应中提取 token/userid，并写入客户端登录态。
    fn finish_login(
        &self,
        data: &Value,
        temp_key: &str,
        fallback_msg: &str,
    ) -> anyhow::Result<LoginResult> {
        let mut token = crate::normalize::pick_str(data, &["token"]);
        let mut userid = data.get("userid").and_then(|v| v.as_i64()).unwrap_or(0) as u64;
        let mut nickname = crate::normalize::pick_str(data, &["nickname", "username"]);
        let mut vip_type = data.get("vip_type").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut pic = crate::normalize::pick_str(data, &["pic", "user_pic", "avatar"]);

        if let Some(secu) = data.get("secu_params").and_then(|v| v.as_str()) {
            if !secu.is_empty() {
                if let Ok(text) = crate::crypto::login_aes_decrypt(secu, temp_key) {
                    let trimmed = text.trim().to_string();
                    if let Ok(v) = serde_json::from_str::<Value>(&trimmed) {
                        if token.is_empty() {
                            token = crate::normalize::pick_str(&v, &["token"]);
                        }
                        if userid == 0 {
                            userid = v.get("userid").and_then(|x| x.as_i64()).unwrap_or(0) as u64;
                        }
                        if nickname.is_empty() {
                            nickname = crate::normalize::pick_str(&v, &["nickname", "username"]);
                        }
                        if vip_type == 0 {
                            vip_type = v.get("vip_type").and_then(|x| x.as_i64()).unwrap_or(0);
                        }
                        if pic.is_empty() {
                            pic = crate::normalize::pick_str(&v, &["pic", "user_pic", "avatar"]);
                        }
                    } else if token.is_empty() && !trimmed.is_empty() {
                        token = trimmed;
                    }
                }
            }
        }

        if token.is_empty() {
            let msg = data
                .get("error")
                .or_else(|| data.get("errmsg"))
                .or_else(|| data.get("msg"))
                .and_then(|v| v.as_str())
                .unwrap_or(fallback_msg);
            anyhow::bail!("{msg}");
        }

        self.set_auth(&token, userid);
        Ok(LoginResult { token, userid, nickname, pic, vip_type })
    }

    /// 用户详情（`/v1/get_my_userinfo`，relation.user.kugou.com）。
    ///
    /// 旧的 `/v3/get_my_info` 已失效（20006）。
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

    /// 每日推荐（`/everyday_song_recommend`）。
    pub async fn recommend_songs(&self) -> anyhow::Result<Vec<Song>> {
        let auth = self.auth();
        let body = serde_json::json!({
            "platform": "android",
            "userid": auth.userid,
        });
        let mut spec = RequestSpec {
            url: "/everyday_song_recommend".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("everydayrec.service.kugou.com".into());

        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/song_list")
            .or_else(|| res.pointer("/data/info"))
            .or_else(|| res.pointer("/data/lists"))
            .or_else(|| res.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(parse_songs(&list))
    }

    /// 私人漫游（`/v1/rcmd_list` → fm.service.kugou.com）。
    ///
    /// 返回 `data.song_list`；`hash`/`songid` 用于「不感兴趣」反馈。
    pub async fn fm_songs(&self, size: i64) -> anyhow::Result<Vec<Song>> {
        let clienttime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        let dev = self.device();
        let key = crate::crypto::sign_params_key(
            &clienttime.to_string(),
            self.appid,
            self.clientver,
            self.lite,
        );
        let body = serde_json::json!({
            "appid": self.appid,
            "clientver": self.clientver,
            "clienttime": clienttime,
            "mid": dev.mid,
            "key": key,
            "rcmdsongcount": size,
            "level": 0,
            "area_code": 1,
            "get_tracker": 1,
            "uid": 0,
        });

        let mut spec = RequestSpec {
            url: "/v1/rcmd_list".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("fm.service.kugou.com".into());

        let res = self.request(spec).await?;
        // 响应结构：data[] 为分组，每组内 rcmdlist[] 才是歌曲
        let mut songs = Vec::new();
        if let Some(groups) = res.get("data").and_then(|v| v.as_array()) {
            for g in groups {
                if let Some(list) = g.get("rcmdlist").and_then(|v| v.as_array()) {
                    songs.extend(parse_songs(&Value::Array(list.clone())));
                }
            }
        }
        if songs.is_empty() {
            let list = res
                .pointer("/data/song_list")
                .or_else(|| res.pointer("/data/info"))
                .or_else(|| res.pointer("/data/lists"))
                .cloned()
                .unwrap_or(Value::Null);
            songs = parse_songs(&list);
        }
        Ok(songs)
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

    /// 新歌速递（`/musicadservice/container/v1/newsong_publish`）。
    pub async fn top_songs(&self, page: i64, pagesize: i64) -> anyhow::Result<Vec<Song>> {
        let auth = self.auth();
        let body = serde_json::json!({
            "rank_id": 21608,
            "userid": auth.userid,
            "page": page,
            "pagesize": pagesize,
            "tags": [],
        });
        let spec = RequestSpec {
            url: "/musicadservice/container/v1/newsong_publish".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/info")
            .or_else(|| res.pointer("/data/song_list"))
            .or_else(|| res.pointer("/data/lists"))
            .or_else(|| res.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(parse_songs(&list))
    }

    /// 新碟上架（`/musicadservice/v1/mobile_newalbum_sp`）。
    pub async fn top_albums(&self, page: i64, pagesize: i64) -> anyhow::Result<Vec<echo_core::Album>> {
        let auth = self.auth();
        let body = serde_json::json!({
            "apiver": 5,
            "token": auth.token,
            "page": page,
            "pagesize": pagesize,
            "withpriv": 1,
        });
        let spec = RequestSpec {
            url: "/musicadservice/v1/mobile_newalbum_sp".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        let res = self.request(spec).await?;
        // 响应按地区分组：data.{chn,eur,jpn,kor}[]
        let mut albums = Vec::new();
        for region in ["chn", "eur", "jpn", "kor"] {
            if let Some(arr) = res.pointer(&format!("/data/{region}")) {
                albums.extend(crate::normalize::parse_albums(arr));
            }
        }
        if albums.is_empty() {
            if let Some(arr) = res.pointer("/data/info") {
                albums = crate::normalize::parse_albums(arr);
            }
        }
        Ok(albums)
    }

    /// 歌单分类标签（`/pubsongs/v1/get_tags_by_type`）。
    pub async fn playlist_tags(&self) -> anyhow::Result<Vec<(String, Vec<(String, String)>)>> {
        let body = serde_json::json!({
            "tag_type": "collection",
            "tag_id": 0,
            "source": 3,
        });
        let spec = RequestSpec {
            url: "/pubsongs/v1/get_tags_by_type".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        let res = self.request(spec).await?;
        let data = res.get("data").cloned().unwrap_or(Value::Null);
        let mut out = Vec::new();
        let groups = data
            .get("data")
            .and_then(|x| x.as_array())
            .or_else(|| data.as_array())
            .cloned()
            .unwrap_or_default();
        for group in groups.iter() {
            let name = crate::normalize::pick_str(group, &["tag_name", "name", "title"]);
            let mut tags = Vec::new();
            if let Some(children) = group
                .get("son")
                .or_else(|| group.get("child"))
                .and_then(|x| x.as_array())
            {
                for tag in children {
                    let id = crate::normalize::pick_str(tag, &["tag_id", "id"]);
                    let tag_name = crate::normalize::pick_str(tag, &["tag_name", "name"]);
                    if !id.is_empty() && !tag_name.is_empty() {
                        tags.push((id, tag_name));
                    }
                }
            }
            if !name.is_empty() && !tags.is_empty() {
                out.push((name, tags));
            }
        }
        Ok(out)
    }

    /// 歌手详情（`/kmr/v3/author`）。
    pub async fn artist_detail(&self, author_id: &str) -> anyhow::Result<echo_core::Artist> {
        let body = serde_json::json!({ "author_id": author_id });
        let mut spec = RequestSpec {
            url: "/kmr/v3/author".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("openapi.kugou.com".into());
        spec.extra_headers.push(("kg-tid".into(), "36".into()));
        let res = self.request(spec).await?;
        let data = res
            .pointer("/data/0")
            .or_else(|| res.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(crate::normalize::parse_artist(&data))
    }

    /// 歌手单曲（`/kmr/v1/audio_group/author`）。
    pub async fn artist_songs(
        &self,
        author_id: &str,
        page: i64,
        pagesize: i64,
    ) -> anyhow::Result<Vec<Song>> {
        let clienttime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let key = crate::crypto::sign_params_key(
            &clienttime.to_string(),
            crate::crypto::APPID,
            crate::crypto::CLIENTVER,
            self.lite,
        );
        let body = serde_json::json!({
            "appid": crate::crypto::APPID,
            "clientver": crate::crypto::CLIENTVER,
            "mid": self.device().mid,
            "clienttime": clienttime,
            "key": key,
            "author_id": author_id,
            "pagesize": pagesize,
            "page": page,
            "sort": 1,
            "area_code": "all",
        });
        let mut spec = RequestSpec {
            url: "/kmr/v1/audio_group/author".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            base_url: Some("https://openapi.kugou.com".into()),
            ..Default::default()
        };
        spec.x_router = Some("openapi.kugou.com".into());
        spec.extra_headers.push(("kg-tid".into(), "220".into()));
        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/0/audio")
            .or_else(|| res.pointer("/data/audio"))
            .or_else(|| res.pointer("/data/info"))
            .or_else(|| res.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(parse_songs(&list))
    }

    /// 歌手专辑（`/kmr/v1/author/albums`）。
    pub async fn artist_albums(&self, author_id: &str) -> anyhow::Result<Vec<echo_core::Album>> {
        let body = serde_json::json!({
            "author_id": author_id,
            "pagesize": 30,
            "page": 1,
            "sort": 1,
            "category": 1,
            "area_code": "all",
        });
        let mut spec = RequestSpec {
            url: "/kmr/v1/author/albums".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("openapi.kugou.com".into());
        spec.extra_headers.push(("kg-tid".into(), "36".into()));
        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/0")
            .or_else(|| res.pointer("/data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(crate::normalize::parse_albums(&list))
    }

    /// 专辑详情（`/kmr/v2/albums`）。
    pub async fn album_detail(&self, album_id: &str) -> anyhow::Result<Option<echo_core::Album>> {
        let body = serde_json::json!({
            "data": [{ "album_id": album_id }],
            "is_buy": 0,
            "fields": "album_id,album_name,publish_date,sizable_cover,intro,language,is_publish,heat,type,quality,authors,exclusive,author_name,trans_param",
        });
        let mut spec = RequestSpec {
            url: "/kmr/v2/albums".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("openapi.kugou.com".into());
        spec.extra_headers.push(("kg-tid".into(), "255".into()));
        let res = self.request(spec).await?;
        let first = res
            .pointer("/data/0")
            .or_else(|| res.get("data"))
            .cloned()
            .unwrap_or(Value::Null);
        if first.is_null() {
            return Ok(None);
        }
        Ok(Some(crate::normalize::parse_album(&first)))
    }

    /// 专辑歌曲（`/v1/album_audio/lite`）。
    pub async fn album_songs(&self, album_id: &str, page: i64, pagesize: i64) -> anyhow::Result<Vec<Song>> {
        // 服务端 pagesize 上限 30（>30 返回 error_code=20010）
        let pagesize = pagesize.clamp(1, 30);
        let body = serde_json::json!({
            "album_id": album_id,
            "is_buy": "",
            "page": page,
            "pagesize": pagesize,
        });
        let mut spec = RequestSpec {
            url: "/v1/album_audio/lite".into(),
            body: Some(body),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("openapi.kugou.com".into());
        spec.extra_headers.push(("kg-tid".into(), "255".into()));
        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/info")
            .or_else(|| res.pointer("/data/songs"))
            .or_else(|| res.pointer("/data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(parse_songs(&list))
    }

    /// 专辑全部歌曲（服务端单页上限 30，这里自动翻页，最多 120 首）。
    pub async fn album_songs_all(&self, album_id: &str) -> anyhow::Result<Vec<Song>> {
        let mut out = Vec::new();
        for page in 1..=4 {
            let batch = self.album_songs(album_id, page, 30).await?;
            let n = batch.len();
            out.extend(batch);
            if n < 30 {
                break;
            }
        }
        Ok(out)
    }

    /// 综合搜索（`/v6/search/complex`）：返回 (歌曲, 歌单, 专辑, 歌手)。
    pub async fn search_complex(
        &self,
        keyword: &str,
        page: i64,
        pagesize: i64,
    ) -> anyhow::Result<ComplexSearch> {
        let mut spec = RequestSpec {
            url: "/v6/search/complex".into(),
            encrypt_type: EncryptType::Android,
            base_url: Some("https://complexsearch.kugou.com".into()),
            ..Default::default()
        };
        spec.params.insert("platform".into(), "AndroidFilter".into());
        spec.params.insert("keyword".into(), keyword.into());
        spec.params.insert("page".into(), page.to_string());
        spec.params.insert("pagesize".into(), pagesize.to_string());
        spec.params.insert("cursor".into(), "0".into());

        let res = self.request(spec).await?;
        let songs = res
            .pointer("/data/lists")
            .map(parse_songs)
            .unwrap_or_default();
        let playlists = res
            .pointer("/data/special/lists")
            .map(crate::normalize::parse_playlists)
            .unwrap_or_default();
        let albums = res
            .pointer("/data/album/lists")
            .map(crate::normalize::parse_albums)
            .unwrap_or_default();
        let artists = res
            .pointer("/data/author/lists")
            .map(crate::normalize::parse_artist_list)
            .unwrap_or_default();
        Ok(ComplexSearch { songs, playlists, albums, artists })
    }

    /// 免费听（概念版）：`/concepts/v1/ai/recommend_song`。
    pub async fn free_listen_songs(&self, page: i64, pagesize: i64) -> anyhow::Result<Vec<Song>> {
        let spec = RequestSpec {
            url: "/concepts/v1/ai/recommend_song".into(),
            body: Some(serde_json::json!({
                "apiver": 2,
                "source": 2,
                "page": page,
                "pagesize": pagesize,
            })),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        let res = self.request(spec).await?;
        let list = res
            .pointer("/data/list")
            .or_else(|| res.pointer("/data/song_list"))
            .or_else(|| res.pointer("/data"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(parse_songs(&list))
    }

    /// 歌曲对应的 MV（`/kmr/v1/audio/mv`，x-router openapi.kugou.com）。
    pub async fn audio_mv(&self, album_audio_id: i64) -> anyhow::Result<Option<echo_core::MvInfo>> {
        if album_audio_id <= 0 {
            return Ok(None);
        }
        let mut spec = RequestSpec {
            url: "/kmr/v1/audio/mv".into(),
            body: Some(serde_json::json!({
                "data": [{"album_audio_id": album_audio_id.to_string()}],
                "fields": ""
            })),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.x_router = Some("openapi.kugou.com".into());
        spec.extra_headers.push(("KG-TID".into(), "38".into()));

        let res = self.request(spec).await?;
        // 结构是 data[group][]，取第一组里的第一条
        let item = res
            .pointer("/data/0/0")
            .or_else(|| res.pointer("/data/0"))
            .cloned();
        let Some(item) = item else {
            return Ok(None);
        };
        let mvhash = crate::normalize::pick_str(&item, &["mvhash", "hash", "mv_hash"]);
        if mvhash.is_empty() {
            return Ok(None);
        }
        Ok(Some(echo_core::MvInfo {
            mvhash,
            songid: crate::normalize::pick_str(&item, &["songid", "audio_id"]),
            title: crate::normalize::pick_str(&item, &["songname", "name", "title"]),
            singer: crate::normalize::pick_str(&item, &["singer", "singer_name", "author_name"]),
            thumb: crate::normalize::normalize_cover(
                &crate::normalize::pick_str(&item, &["thumb", "pic", "cover"]),
                400,
            ),
            hd_pic: crate::normalize::normalize_cover(
                &crate::normalize::pick_str(&item, &["hdpic", "hd_pic"]),
                720,
            ),
            duration: crate::normalize::pick_i64(&item, &["duration", "timelen"]).max(0) as u64,
            hit: crate::normalize::pick_i64(&item, &["hit", "play_count"]).max(0) as u64,
            desc: crate::normalize::pick_str(&item, &["desc", "description"]),
        }))
    }
}

/// 登录结果。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LoginResult {
    pub token: String,
    pub userid: u64,
    pub nickname: String,
    pub pic: String,
    pub vip_type: i64,
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
    pub userid: u64,
    pub nickname: String,
    pub pic: String,
    pub vip_type: i64,
}

/// 综合搜索结果。
#[derive(Debug, Clone, Default)]
pub struct ComplexSearch {
    pub songs: Vec<Song>,
    pub playlists: Vec<echo_core::Playlist>,
    pub albums: Vec<echo_core::Album>,
    pub artists: Vec<echo_core::Artist>,
}

/// 登录固定参数 t1/t2（对齐 KuGouMusicApi login.js）。
const LOGIN_T1: &str = "562a6f12a6e803453647d16a08f5f0c2ff7eee692cba2ab74cc4c8ab47fc467561a7c6b586ce7dc46a63613b246737c03a1dc8f8d162d8ce1d2c71893d19f1d4b797685a4c6d3d81341cbde65e488c4829a9b4d42ef2df470eb102979fa5adcdd9b4eecfea8b909ff7599abeb49867640f10c3c70fc444effca9d15db44a9a6c907731e2bb0f22cd9b3536380169995693e5f0e2424e3378097d3813186e3fe96bbe7023808a0981b4e2b6135a76faac";
const LOGIN_T2: &str = "31c4daf4cf480169ccea1cb7d4a209295865a9d2b788510301694db229b87807469ea0d41b4d4b9173c2151da7294aeebfc9738df154bbdf11a4e117bb5dff6a3af8ce5ce333e681c1f29a44038f27567d58992eb81283e080778ac77db1400fdf49b7cf7e26be2e5af4da7830cc3be4";

/// token 刷新（`/v5/login_by_token`）里 p3 的固定 AES key/iv。
const REFRESH_AES_KEY: &str = "90b8382a1bb4ccdcf063102053fd75b8";
const REFRESH_AES_IV: &str = "f063102053fd75b8";
