//! 用户歌单与红心收藏接口（需要登录态）。
//! 端点参考 KuGouMusicApi：`user_playlist` / `playlist_track_all_new`
//! / `playlist_tracks_add` / `playlist_tracks_del`。


use crate::client::{EncryptType, KuGouClient, RequestSpec};
use echo_core::{Playlist, Song};
use serde_json::{json, Value};

impl KuGouClient {
    /// 用户歌单歌曲（`/v4/get_list_all_file_v3`，按 listid 取）。
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
