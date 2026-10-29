//! 核心数据模型（对齐原项目 `src/renderer/models/*`）。

use serde::{Deserialize, Serialize};

/// 歌曲演唱者信息。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SongArtist {
    pub id: String,
    pub name: String,
    pub pic: String,
}

/// 歌曲模型（对齐 `Song`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Song {
    /// 主键（mix_song_id 优先，前端生成兜底）
    pub id: String,
    /// 酷狗文件 hash（取播放地址的核心标识）
    pub hash: String,
    /// 歌曲名
    pub title: String,
    /// 歌手展示串（多个以 "/" 连接）
    pub artist: String,
    /// 歌手明细
    #[serde(default)]
    pub artists: Vec<SongArtist>,
    /// 专辑名
    #[serde(default)]
    pub album: String,
    /// 专辑 id
    #[serde(default)]
    pub album_id: String,
    /// 时长（秒）
    #[serde(default)]
    pub duration: u64,
    /// 封面地址（120px，列表 / 播放条用）
    #[serde(default)]
    pub cover_url: String,
    /// 高清封面地址（480px）。
    ///
    /// 播放页封面要显示到 264px，用 120px 的图拉上去是糊的；但把 `cover_url`
    /// 统一提到 480 又会让几十上百行的列表白下载一大堆大图，所以只在需要大图的
    /// 地方（播放页封面、全屏背景）用这一份。
    #[serde(default)]
    pub cover_url_hd: String,
    /// 解析后的播放地址
    #[serde(default)]
    pub audio_url: String,
    /// 混音歌曲 id（酷狗全局歌曲标识）
    #[serde(default)]
    pub mix_song_id: String,
    /// 云歌单条目 id（`audio_id`），加歌 / 删歌时作为 fileid 使用
    #[serde(default)]
    pub cloud_audio_id: String,
    /// 专辑音频 id（酷狗 MixSongID；`/tracker/v5/url`、`/v1/authorization` 的 `album_audio_id`）
    #[serde(default)]
    pub album_audio_id: String,
    /// 歌曲音频 id（酷狗 `audio_id`/`Audioid`）。
    ///
    /// 注意：**不是** `album_audio_id`，两者是不同的 id 空间。
    /// 实测把 audio_id 当 album_audio_id 传给取地址接口，会返回成另外一首歌
    /// （例：「别叹」的 audio_id=472778418 会解析成「Beiman」）。
    #[serde(default)]
    pub audio_id: String,
    /// 播放权限标记（原 privilege / pay_type）
    #[serde(default)]
    pub privilege: i64,
    #[serde(default)]
    pub pay_type: i64,
    /// 音质标记（quality_map）
    #[serde(default)]
    pub quality_map: i64,
    /// MV hash
    #[serde(default)]
    pub mv_hash: String,
    /// 歌词摘要
    #[serde(default)]
    pub lyric_snippet: String,
    /// 推荐理由（私人FM 的 `rec_song_info.rec_desc`，如「根据你的听歌口味推荐」）
    #[serde(default)]
    pub rec_reason: String,
    /// 相似度（私人FM 的 `rec_song_info.similar_desc`，如「高」「低」）
    #[serde(default)]
    pub similar: String,
    /// 是否来自「音乐云盘」。
    ///
    /// 云盘歌曲不在普通 CDN 的曲库里，取地址要走
    /// `/bsstrackercdngz/v2/query_musicclound_url`（见 `KuGouClient::cloud_song_url`）。
    #[serde(default)]
    pub from_cloud: bool,
}

impl Song {
    /// 需要 VIP 才能播放。
    pub fn is_vip(&self) -> bool {
        self.privilege == 10 && self.pay_type != 2
    }

    /// 取播放地址时要传的 `album_audio_id`（= 酷狗 MixSongID）。
    ///
    /// 也兼容历史数据：早期解析把 `audio_id` 写进了 `album_audio_id`，
    /// 而 `mix_song_id` 是同一份数据里单独存的，所以只要它有效就优先用它，
    /// 免得按错的 id 解析成另一首歌。
    pub fn play_album_audio_id(&self) -> i64 {
        for raw in [&self.mix_song_id, &self.album_audio_id, &self.audio_id] {
            if let Ok(v) = raw.parse::<i64>() {
                if v > 0 {
                    return v;
                }
            }
        }
        0
    }

    /// 展示用唯一键：优先 album_audio_id，其次 hash。
    /// 唯一键：优先 hash（各接口都会返回且稳定），其次 album_audio_id。
    ///
    /// 不能用 album_audio_id 优先：云歌单（`/v4/get_list_all_file_v3`）返回的
    /// `album_audio_id` 常缺失或与推荐接口不一致，会导致同一首歌匹配不上。
    pub fn key(&self) -> String {
        if !self.hash.is_empty() {
            format!("hash:{}", self.hash)
        } else if !self.album_audio_id.is_empty() {
            format!("aa:{}", self.album_audio_id)
        } else {
            format!("id:{}", self.id)
        }
    }
}

/// 歌单模型（对齐 `Playlist` / `PlaylistInfo`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub pic: String,
    #[serde(default)]
    pub intro: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub user_pic: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub play_count: u64,
    /// 歌曲数量
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub source: i64,
    /// 用户歌单 listid（加歌 / 删歌需要；`id` 是对外展示 id）
    #[serde(default)]
    pub listid: String,
    /// 创建者 userid
    #[serde(default)]
    pub userid: String,
}

impl Playlist {
    /// 是否是「我喜欢的音乐」歌单（酷狗默认置顶的红心歌单）。
    pub fn is_liked(&self) -> bool {
        let n = self.name.trim();
        n == "我喜欢的音乐" || n == "我喜欢" || n.contains("我喜欢的")
    }
}




/// 用户信息（对齐 `User`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct User {
    pub userid: u64,
    pub token: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub pic: String,
    #[serde(default)]
    pub vip_type: i64,
}


// ==================== 歌词 ====================

/// 歌词里的一个字（KRC 的逐字时间轴）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LyricWord {
    /// 相对行首的开始时间（毫秒）
    pub start_ms: u64,
    /// 持续时长（毫秒）
    pub dur_ms: u64,
    pub text: String,
}

/// 歌词一行。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LyricLine {
    /// 行开始时间（毫秒）
    pub time_ms: u64,
    /// 整行文本（去掉逐字标签后的纯文本）
    pub text: String,
    /// 逐字时间轴（没有则为空，按行高亮）
    pub words: Vec<LyricWord>,
}

/// 歌词。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lyric {
    /// 全部歌词行（按时间升序）
    pub lines: Vec<LyricLine>,
    /// 整体偏移（毫秒，来自 KRC 的 `[offset:]`）
    pub offset_ms: i64,
    /// 是否带逐字时间轴（KRC 才有）
    pub word_time: bool,
    /// 是否有译文（翻译歌词）
    pub has_translation: bool,
}

impl Lyric {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// 当前时间对应的行号（没有就返回 None）。
    pub fn index_at(&self, ms: f64) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        let t = (ms * 1000.0) as i64 + self.offset_ms;
        let mut found = None;
        for (i, line) in self.lines.iter().enumerate() {
            if (line.time_ms as i64) <= t {
                found = Some(i);
            } else {
                break;
            }
        }
        found
    }
}


