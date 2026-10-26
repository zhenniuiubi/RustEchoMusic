//! 酷狗响应 JSON → 核心模型归一化。
//!
//! 酷狗不同接口返回的字段名风格不一（大写驼峰 / 小写下划线），
//! 这里提供多候选字段拾取，对齐原项目 `utils/mappers/*` 的防御式解析思路。

use echo_core::{Album, Artist, Playlist, RankMeta, Song, SongArtist};
use serde_json::Value;

/// 从对象按候选键顺序拾取字符串。
pub fn pick_str(v: &serde_json::Value, keys: &[&str]) -> String {
    if let Some(obj) = v.as_object() {
        for k in keys {
            if let Some(val) = obj.get(*k) {
                match val {
                    serde_json::Value::String(s) => {
                        if !s.trim().is_empty() {
                            return s.clone();
                        }
                    }
                    serde_json::Value::Number(n) => return n.to_string(),
                    _ => {}
                }
            }
        }
    }
    String::new()
}

/// 从对象按候选键顺序拾取整数。
pub fn pick_i64(v: &serde_json::Value, keys: &[&str]) -> i64 {
    if let Some(obj) = v.as_object() {
        for k in keys {
            if let Some(val) = obj.get(*k) {
                if let Some(n) = val.as_i64() {
                    return n;
                }
                if let Some(s) = val.as_str() {
                    if let Ok(n) = s.parse::<i64>() {
                        return n;
                    }
                }
                // "03:30" 形式的时长
                if keys.contains(&"time_length") && k == &"time_length" {
                    if let Some(s) = val.as_str() {
                        return parse_time_length(s);
                    }
                }
            }
        }
    }
    0
}

/// "mm:ss" → 秒。
pub fn parse_time_length(s: &str) -> i64 {
    let parts: Vec<i64> = s.split(':').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.len() {
        2 => parts[0] * 60 + parts[1],
        3 => parts[0] * 3600 + parts[1] * 60 + parts[2],
        _ => 0,
    }
}

/// 归一化封面：把 URL 里的尺寸换成指定值。
///
/// 两种形态都要管：
/// 1. 模板形态 `.../{size}/...`（大多数接口直接给模板，替换即可）；
/// 2. **尺寸已经烧进路径**的形态，比如酷狗的
///    `http://imge.kugou.com/stdmusic/120/20210114/xxx.jpg`。
///    这种 URL 里没有 `{size}`，只做字符串替换等于什么都没干——于是
///    "高清封面" 拿到的还是 120×120 的缩略图（播放页 264pt 的封面糊成一片
///    就是这个原因）。所以要把路径里第一段纯数字（= 尺寸）改写掉。
pub fn normalize_cover(url: &str, size: u32) -> String {
    let u = url.trim();
    if u.is_empty() {
        return String::new();
    }
    let s = size.to_string();
    if u.contains("{size}") {
        return u.replace("{size}", &s);
    }
    // 只动图片 CDN 的路径，且只把「看起来就是尺寸」的那一段换掉：
    // 尺寸是几十到一千出头的小整数（120 / 240 / 480 / 1000），而日期、图片 id
    // 都是 8 位以上的大数（20210114…），用范围判断不会误伤。
    const MIN_SIZE: u32 = 50;
    const MAX_SIZE: u32 = 2000;
    if let Some((scheme, rest)) = u.split_once("://") {
        if let Some((host, path)) = rest.split_once('/') {
            if host.ends_with("kugou.com") {
                let mut segs: Vec<&str> = path.split('/').collect();
                for seg in segs.iter_mut() {
                    if seg.is_empty() || seg.len() > 4 || !seg.chars().all(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    match seg.parse::<u32>() {
                        Ok(n) if (MIN_SIZE..=MAX_SIZE).contains(&n) => {
                            *seg = &s;
                            return format!("{scheme}://{host}/{}", segs.join("/"));
                        }
                        _ => continue,
                    }
                }
            }
        }
    }
    u.to_string()
}

/// 歌手数组：`singers`/`authors` 明细优先，回退 `SingerName` 串。
fn parse_artists(v: &serde_json::Value) -> (Vec<SongArtist>, String) {
    let mut list = Vec::new();
    for container_key in ["Singers", "singers", "singerinfo", "authors", "singerlist"] {
        if let Some(arr) = v.get(container_key).and_then(|x| x.as_array()) {
            for item in arr {
                let name = pick_str(item, &["name", "singername", "author_name", "Name"]);
                if !name.is_empty() {
                    list.push(SongArtist {
                        id: pick_str(item, &["id", "singerid", "author_id", "Id"]),
                        name,
                        pic: normalize_cover(
                            &pick_str(item, &["pic", "imgurl", "sizable_cover", "avatar"]),
                            240,
                        ),
                    });
                }
            }
            if !list.is_empty() {
                break;
            }
        }
    }
    let joined = list
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join("/");
    if joined.is_empty() {
        let raw = pick_str(v, &["SingerName", "singername", "author_name", "singer_name"]);
        (Vec::new(), raw)
    } else {
        (list, joined)
    }
}

/// 解析单曲（兼容搜索 / 推荐流 / 歌单曲目等多种响应形态）。
/// 部分接口（如 `/v1/album_audio/lite`）把歌曲字段放在 `base` 子对象里，
/// 这里做一次「base 提升 + 顶层覆盖」的扁平化，让后续解析逻辑统一。
fn merge_base<'a>(v: &'a serde_json::Value) -> std::borrow::Cow<'a, serde_json::Value> {
    let Some(base) = v.get("base").and_then(|b| b.as_object()) else {
        return std::borrow::Cow::Borrowed(v);
    };
    let mut map = base.clone();
    if let Some(top) = v.as_object() {
        for (k, val) in top {
            if k != "base" {
                map.insert(k.clone(), val.clone());
            }
        }
    }
    std::borrow::Cow::Owned(serde_json::Value::Object(map))
}

pub fn parse_song(v: &serde_json::Value) -> Song {
    let merged = merge_base(v);
    let v = merged.as_ref();
    let file_name = pick_str(v, &["FileName"]);
    let mut title = pick_str(
        v,
        &["OriSongName", "SongName", "songname", "name", "title", "audio_name"],
    );
    let (artists, joined) = parse_artists(v);
    let mut artist_display = joined;

    // hash：顶层优先，缺失时从嵌套子对象（audio_info / deprecated / trans_param.hash_offset）提取。
    // 排行榜 `/openapi/kmr/v2/rank/audio` 的 hash 藏在 audio_info.hash_128 / deprecated.hash 里。
    let hash = {
        let top = pick_str(v, &["FileHash", "filehash", "hash", "hash_128", "Hash"]);
        if !top.is_empty() {
            top
        } else {
            let mut found = String::new();
            for sub_key in ["audio_info", "deprecated", "trans_param"] {
                if let Some(sub) = v.get(sub_key) {
                    found = pick_str(sub, &["hash_128", "hash_320", "hash_flac", "hash", "offset_hash", "ogg_128_hash", "clip_hash"]);
                    if !found.is_empty() {
                        break;
                    }
                }
            }
            found
        }
    }
    .to_lowercase();

    // 两个 id 是不同的 id 空间，绝不能混用：
    // - `album_audio_id`（= MixSongID）：取播放地址 `/tracker/v5/url` 与
    //   `/v1/authorization` 的 `album_audio_id` 参数要的就是它；
    // - `audio_id`（= `Audioid`/`audio_id`）：云歌单条目 id 之类场景用。
    //
    // 曾经把 audio_id 填进 album_audio_id，结果取地址接口按 id 解析成了**另一首歌**
    // （实测「别叹」audio_id=472778418 → 返回「Beiman」；mixsongid=770725889 → 正确）。
    let audio_id = pick_str(v, &["Audioid", "audioid", "audio_id", "AudioID"]);
    let album_audio_id = {
        let aa = pick_str(
            v,
            &[
                "AlbumAudioID",
                "album_audio_id",
                "MixSongID",
                "mixsongid",
                "mix_song_id",
            ],
        );
        if !aa.is_empty() {
            aa
        } else {
            // 实在没有：退到 audio_id（聊胜于无，但可能解析到别的歌）
            audio_id.clone()
        }
    };

    // 无歌名时退回 FileName（形如 "歌名 - 歌手1/歌手2"）
    if title.is_empty() {
        title = file_name;
    }
    // `歌手 - 歌名` 且歌手字段已有值：剥掉标题里的歌手前缀
    if !artist_display.is_empty() {
        let first_artist = artist_display
            .split(['/', '、', '&', ','])
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        let stripped = title
            .rsplit_once(" - ")
            .map(|(l, r)| (l.trim().to_string(), r.trim().to_string()))
            .filter(|(l, r)| {
                !r.is_empty()
                    && r.len() < 80
                    && (!first_artist.is_empty() && l.contains(&first_artist))
            });
        if let Some((_, r)) = stripped {
            title = r;
        }
    }
    // 歌单/推荐流里常见 `歌名 - 歌手` 形态且无独立歌手字段：拆分展示
    if artist_display.is_empty() {
        let split = title
            .rsplit_once(" - ")
            .map(|(t, a)| (t.trim().to_string(), a.trim().to_string()))
            .filter(|(t, a)| !t.is_empty() && !a.is_empty() && a.len() < 64);
        if let Some((t, a)) = split {
            title = t;
            artist_display = a;
        }
    }

    let mut cover_raw = pick_str(
        v,
        &[
            "album_sizable_cover",
            "sizable_cover",
            "sizableCover",
            "imgurl",
            "Image",
            "cover",
            "pic",
            "image",
        ],
    );
    if cover_raw.is_empty() {
        // 排行榜等新接口：封面在 album_info.sizable_cover 或 trans_param.union_cover
        cover_raw = v
            .get("album_info")
            .map(|sub| pick_str(sub, &["sizable_cover", "cover", "imgurl"]))
            .filter(|s| !s.is_empty())
            .or_else(|| {
                v.get("trans_param")
                    .map(|sub| pick_str(sub, &["union_cover", "sizable_cover"]))
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_default();
    }

    // 云歌单里歌名常带文件后缀（"回忆里的那个人.mp3"），去掉更干净
    let title = strip_file_ext(&title);

    // 私人FM 会给每首歌带推荐理由/相似度
    let rec_info = v.get("rec_song_info");

    Song {
        id: {
            let mix = pick_str(v, &["MixSongID", "mixsongid", "mix_song_id"]);
            if !mix.is_empty() {
                mix
            } else {
                pick_str(v, &["AlbumAudioID", "album_audio_id", "hash"])
            }
        },
        hash: hash.clone(),
        title,
        artist: artist_display,
        artists,
        album: {
            let top = pick_str(v, &["AlbumName", "albumname", "album_name", "album"]);
            if !top.is_empty() {
                top
            } else {
                // 排行榜用 album_info.album_name；歌单接口用 albuminfo.name（对象）
                v.get("album_info")
                    .map(|sub| pick_str(sub, &["album_name", "name"]))
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        v.get("albuminfo")
                            .map(|sub| pick_str(sub, &["albumname", "album_name", "name"]))
                    })
                    .unwrap_or_default()
            }
        },
        album_id: pick_str(v, &["AlbumID", "albumid", "album_id"]),
        duration: {
            // 各接口时长单位不一致：> 10000 视为毫秒
            let raw = pick_i64(
                v,
                &["Duration", "duration", "durationSecond", "time_length", "timelen", "timelength"],
            )
            .max(0);
            let raw = if raw == 0 {
                // 排行榜等新接口：时长藏在 audio_info.duration_128 / deprecated.duration（毫秒）
                v.get("audio_info")
                    .map(|sub| pick_i64(sub, &["duration_128", "duration_320", "duration_flac", "duration"]))
                    .filter(|x| *x > 0)
                    .or_else(|| {
                        v.get("deprecated")
                            .map(|sub| pick_i64(sub, &["duration"]))
                            .filter(|x| *x > 0)
                    })
                    .unwrap_or(0)
            } else {
                raw
            };
            if raw > 10000 { (raw / 1000) as u64 } else { raw as u64 }
        },
        cover_url: normalize_cover(&cover_raw, 120),
        cover_url_hd: normalize_cover(&cover_raw, 480),
        audio_url: String::new(),
        mix_song_id: pick_str(v, &["MixSongID", "mixsongid", "mix_song_id"]),
        // 云歌单条目 id：`fileid` 才是删除时用的 fileid（audio_id 是另一个编号）
        cloud_audio_id: {
            let f = pick_str(v, &["fileid", "file_id", "FileID"]);
            if f.is_empty() {
                pick_str(v, &["audio_id", "AudioID", "audioid"])
            } else {
                f
            }
        },
        album_audio_id: album_audio_id.clone(),
        audio_id,
        privilege: {
            let top = pick_i64(v, &["privilege", "Privilege"]);
            if top != 0 {
                top
            } else {
                // 排行榜等新接口：privilege 藏在 privilege_download.privilege
                v.get("privilege_download")
                    .map(|sub| pick_i64(sub, &["privilege", "Privilege"]))
                    .unwrap_or(0)
            }
        },
        pay_type: {
            let top = pick_i64(v, &["pay_type", "PayType"]);
            if top != 0 {
                top
            } else {
                v.get("deprecated")
                    .map(|sub| pick_i64(sub, &["pay_type", "PayType"]))
                    .unwrap_or(0)
            }
        },
        quality_map: pick_i64(v, &["quality", "qualityMap", "quality_map"]),
        mv_hash: pick_str(v, &["MvHash", "mvhash", "video_hash", "mv_hash"]).to_lowercase(),
        lyric_snippet: pick_str(v, &["Lyric", "lyric", "contents"]),
        rec_reason: rec_info
            .map(|r| pick_str(r, &["rec_desc", "rec_desc2"]))
            .unwrap_or_default(),
        similar: rec_info
            .map(|r| pick_str(r, &["similar_desc"]))
            .unwrap_or_default(),
        from_cloud: false,
    }
}

/// 去掉歌名末尾的文件扩展名（.mp3 / .flac / .m4a / .wav / .ogg / .ape）。
fn strip_file_ext(title: &str) -> String {
    const EXTS: [&str; 6] = [".mp3", ".flac", ".m4a", ".wav", ".ogg", ".ape"];
    let t = title.trim();
    for ext in EXTS {
        if t.to_ascii_lowercase().ends_with(ext) {
            return t[..t.len() - ext.len()].trim_end().to_string();
        }
    }
    title.to_string()
}

/// 解析歌曲数组。
pub fn parse_songs(v: &serde_json::Value) -> Vec<Song> {
    v.as_array().map(|arr| arr.iter().map(parse_song).collect()).unwrap_or_default()
}

/// 解析音乐云盘列表（`mcloudservice.kugou.com/v1/get_list`）。
///
/// 云盘条目的字段和普通歌曲不一样：
/// - 歌名是 `name`，且带歌手前缀和扩展名，例如 `陈一发儿 - 童话镇.mp3`；
/// - 歌手只有 `author_name`（不是 `Singers` 数组）；
/// - 时长是毫秒 `timelen`；
/// - 还有 `kv_id`（1/2 两套存储）、`hash_std`（标准 hash）。
pub fn parse_cloud_songs(v: &Value) -> Vec<Song> {
    let list = v
        .get("data")
        .and_then(|d| d.get("list"))
        .and_then(|l| l.as_array())
        .or_else(|| v.get("data").and_then(|d| d.as_array()))
        .or_else(|| v.get("list").and_then(|l| l.as_array()))
        .or_else(|| v.as_array());

    let Some(list) = list else {
        return Vec::new();
    };

    list.iter()
        .map(|item| {
            let mut song = parse_song(item);
            let author = pick_str(item, &["author_name", "singername", "author"]);
            // `name` = "歌手 - 歌名.mp3"：剥掉扩展名与歌手前缀
            let raw_name = pick_str(item, &["name", "songname", "filename"]);
            let cleaned = clean_cloud_name(&raw_name, &author);
            if !cleaned.is_empty() {
                song.title = cleaned;
            }
            if !author.is_empty() {
                song.artist = author.clone();
                song.artists = vec![SongArtist {
                    id: pick_str(item, &["author_id", "singerid"]),
                    name: author,
                    pic: String::new(),
                }];
            }
            song.from_cloud = true;
            // 云盘 hash 用 kv 里那一条（hash_std 是另一套存储的，不能用）
            let h = pick_str(item, &["hash"]);
            if !h.is_empty() {
                song.hash = h.to_lowercase();
            }
            song
        })
        .collect()
}

/// `陈一发儿 - 童话镇.mp3` + 歌手 `陈一发儿` → `童话镇`
fn clean_cloud_name(name: &str, author: &str) -> String {
    let mut s = name.trim();
    // 去扩展名
    for ext in [".mp3", ".flac", ".m4a", ".wav", ".ape", ".ogg", ".aac"] {
        if s.to_lowercase().ends_with(ext) {
            s = &s[..s.len() - ext.len()];
            break;
        }
    }
    let s = s.trim();
    // 去 "歌手 - " 前缀
    if !author.is_empty() {
        for sep in [" - ", "-", " – "] {
            let prefix = format!("{author}{sep}");
            if let Some(rest) = s.strip_prefix(&prefix) {
                return rest.trim().to_string();
            }
        }
    }
    s.to_string()
}

/// 解析歌单（special_recommend / get_list_info 等形态）。
pub fn parse_playlist(v: &serde_json::Value) -> Playlist {
    let cover_raw = pick_str(
        v,
        &["imgurl", "sizable_cover", "sizableCover", "pic", "Image", "cover", "img"],
    );
    // 歌单标识：global_collection_id 优先；否则由 specialid 拼出
    // `collection_3_{id}_0`（酷狗对外的全局歌单 id 格式）。
    let global_id = pick_str(
        v,
        &["global_collection_id", "global_special_id", "gid", "list_create_gid"],
    );
    let num_id = pick_str(v, &["special_id", "specialid", "listid", "id", "playlist_id"]);
    let id = if !global_id.is_empty() {
        global_id
    } else if !num_id.is_empty() && num_id.chars().all(|c| c.is_ascii_digit()) {
        format!("collection_3_{num_id}_0")
    } else {
        num_id
    };
    Playlist {
        id,
        name: pick_str(v, &["special_name", "specialname", "name", "listname", "title", "playlist_name"]),
        // 320 → 200：歌单/专辑卡展示约 180px，位图内存按 w*h*4 算，省 ~2.5 倍
        // 320 → 200：卡片展示约 180px，位图按 w*h*4 常驻，尺寸减半省 ~2.5 倍
        pic: normalize_cover(&cover_raw, 200),
        intro: pick_str(v, &["intro", "desc", "description"]),
        nickname: pick_str(v, &["nickname", "username", "creator", "author_name"]),
        user_pic: pick_str(v, &["user_avatar", "avatar", "user_pic"]),
        tags: v
            .get("tags")
            .and_then(|t| t.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        play_count: pick_i64(v, &["play_count", "playcount", "count_play"]).max(0) as u64,
        count: pick_i64(v, &["count", "songcount", "song_count", "audio_count"]).max(0) as u64,
        source: pick_i64(v, &["source"]),
        listid: pick_str(v, &["listid", "list_id"]),
        userid: pick_str(v, &["userid", "user_id", "list_create_userid"]),
    }
}

/// 解析歌单数组。
pub fn parse_playlists(v: &serde_json::Value) -> Vec<Playlist> {
    v.as_array()
        .map(|arr| arr.iter().map(parse_playlist).collect())
        .unwrap_or_default()
}

/// 解析专辑。
pub fn parse_album(v: &serde_json::Value) -> Album {
    let cover_raw = pick_str(
        v,
        &["sizable_cover", "img", "imgurl", "Image", "pic", "cover"],
    );
    Album {
        id: pick_str(v, &["albumid", "AlbumID", "album_id", "id"]),
        name: pick_str(v, &["albumname", "AlbumName", "album_name", "name"]),
        // 320 → 200：歌单/专辑卡展示约 180px，位图内存按 w*h*4 算，省 ~2.5 倍
        // 320 → 200：卡片展示约 180px，位图按 w*h*4 常驻，尺寸减半省 ~2.5 倍
        pic: normalize_cover(&cover_raw, 200),
        intro: pick_str(v, &["album_intro", "intro", "desc"]),
        singer_name: pick_str(v, &["author_name", "SingerName", "singername", "singer_name"]),
        singer_id: pick_str(v, &["singerid", "SingerID", "singer_id", "author_id"]),
        publish_time: pick_str(v, &["publishtime", "PublishTime", "publish_time"]),
        song_count: pick_i64(v, &["songcount", "SongCount", "song_count"]).max(0) as u64,
        play_count: pick_i64(v, &["playcount", "play_count"]).max(0) as u64,
    }
}

/// 解析歌手。
pub fn parse_artist(v: &serde_json::Value) -> Artist {
    // 搜索歌手接口（/v1/search/author）用 PascalCase：AuthorId / AuthorName / Avatar
    let cover_raw = pick_str(
        v,
        &["imgurl", "sizable_cover", "Avatar", "avatar", "Image", "pic"],
    );
    Artist {
        id: pick_str(v, &["AuthorId", "singerid", "SingerID", "author_id", "id"]),
        name: pick_str(v, &["AuthorName", "singername", "SingerName", "author_name", "name"]),
        pic: normalize_cover(&cover_raw, 160),
        intro: pick_str(v, &["intro", "desc"]),
        song_count: pick_i64(
            v,
            &["AudioCount", "songcount", "SongCount", "song_count", "worksnum"],
        )
        .max(0) as u64,
        album_count: pick_i64(v, &["AlbumCount", "albumcount", "AlbumCount", "album_count"]).max(0)
            as u64,
        mv_count: pick_i64(v, &["VideoCount", "mvcount", "mv_count"]).max(0) as u64,
        fans_count: pick_i64(v, &["FansNum", "fanscount", "fans_num"]).max(0) as u64,
    }
}

/// 解析榜单。
pub fn parse_rank(v: &serde_json::Value) -> RankMeta {
    RankMeta {
        id: pick_str(v, &["id", "rankid", "rank_id"]),
        name: pick_str(v, &["name", "rankname", "rank_name"]),
        pic: normalize_cover(&pick_str(v, &["imgurl", "pic", "Image", "cover"]), 240),
        update_frequency: pick_str(v, &["updatefrequency", "update_frequency", "retentate"]),
        description: pick_str(v, &["description", "desc", "intro"]),
        rank_cid: pick_str(v, &["rank_cid", "rankcid", "cid"]),
        // `/rank/audio` 的 rank_id 必须用 `rankid`（列表项 `id` 传进去只回 1 首）
        req_rank_id: {
            let r = pick_str(v, &["rankid", "rank_id"]);
            if r.is_empty() {
                pick_str(v, &["id"])
            } else {
                r
            }
        },
    }
}

/// 解析歌手列表（搜索 / 歌手 Tab）。
pub fn parse_artist_list(v: &serde_json::Value) -> Vec<Artist> {
    v.as_array()
        .map(|arr| arr.iter().map(parse_artist).collect())
        .unwrap_or_default()
}

/// 解析专辑列表。
pub fn parse_albums(v: &serde_json::Value) -> Vec<echo_core::Album> {
    v.as_array()
        .map(|arr| arr.iter().map(parse_album).collect())
        .unwrap_or_default()
}

/// 解析用户信息（`/v3/get_my_info`）。
pub fn parse_user(v: &serde_json::Value) -> echo_core::User {
    let cover = pick_str(v, &["pic", "user_pic", "avatar", "img"]);
    echo_core::User {
        userid: v.get("userid").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
        token: pick_str(v, &["token"]),
        nickname: pick_str(v, &["nickname", "username", "user_name"]),
        username: pick_str(v, &["username", "user_name"]),
        pic: normalize_cover(&cover, 200),
        vip_type: pick_i64(v, &["vip_type", "viptype"]),
    }
}


#[cfg(test)]
mod tests {
    use super::normalize_cover;

    #[test]
    fn cover_template_replaced() {
        assert_eq!(
            normalize_cover("http://imge.kugou.com/stdmusic/{size}/20210114/a.jpg", 480),
            "http://imge.kugou.com/stdmusic/480/20210114/a.jpg"
        );
    }

    #[test]
    fn cover_burnt_in_size_rewritten() {
        // 酷狗把尺寸烧在路径里，模板里没有 {size}
        assert_eq!(
            normalize_cover("http://imge.kugou.com/stdmusic/120/20210114/a.jpg", 480),
            "http://imge.kugou.com/stdmusic/480/20210114/a.jpg"
        );
        // 歌手头像这类更深一级的路径同样要换
        assert_eq!(
            normalize_cover("http://singerimg.kugou.com/uploadpic/softhead/400/20200101/a.jpg", 240),
            "http://singerimg.kugou.com/uploadpic/softhead/240/20200101/a.jpg"
        );
    }

    #[test]
    fn cover_big_numbers_untouched() {
        // 8 位日期/id 不是尺寸，不能动
        assert_eq!(
            normalize_cover("http://imge.kugou.com/stdmusic/120/20210114120846674495.jpg", 480),
            "http://imge.kugou.com/stdmusic/480/20210114120846674495.jpg"
        );
        assert_eq!(
            normalize_cover("http://other.cdn.com/20210114/a.jpg", 480),
            "http://other.cdn.com/20210114/a.jpg"
        );
        assert_eq!(normalize_cover("", 480), "");
    }
}
