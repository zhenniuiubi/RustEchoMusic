//! 歌词：`lyrics.kugou.com` 搜索 → 下载 → KRC 解密 → 解析。
//!
//! 流程（对齐 KuGouMusicApi 的 `search_lyric` + `lyric` + `util.decodeLyrics`）：
//! 1. `https://lyrics.kugou.com/search?ver=1&man=yes&client=pc&keyword=&duration=&hash=`
//!    → `candidates[]`，每条带 `id` + `accesskey`；
//! 2. `https://lyrics.kugou.com/download?ver=1&client=android&id=&accesskey=&fmt=krc&charset=utf8`
//!    → `content`（base64；`contenttype=0` 时是加密 KRC，否则是明文 LRC）；
//! 3. KRC：base64 → 跳过 4 字节头 → 与 16 字节 key 循环异或 → zlib inflate → 文本。
//!
//! 注意：`/v1/search`（KuGouMusicApi 里那个）实测返回空，旧版 `/search` 才有数据。

use base64::Engine as _;

use echo_core::{Lyric, LyricLine, LyricWord, Song};

use crate::client::{EncryptType, KuGouClient, RequestSpec};

/// 歌词搜索候选。
#[derive(Debug, Clone)]
pub struct LyricCandidate {
    pub id: String,
    pub accesskey: String,
    pub title: String,
    pub artist: String,
    /// 秒
    pub duration: u64,
}

/// KRC 的 XOR 密钥（16 字节，循环使用，与 KuGouMusicApi 一致）。
const KRC_KEY: [u8; 16] = [
    64, 71, 97, 119, 94, 50, 116, 71, 81, 54, 49, 45, 206, 210, 110, 105,
];

impl KuGouClient {
    /// 歌词搜索：按「歌手 - 歌名」+ hash + 时长找候选。
    pub async fn lyric_candidates(&self, song: &Song) -> anyhow::Result<Vec<LyricCandidate>> {
        let mut spec = RequestSpec {
            url: "/search".into(),
            base_url: Some("https://lyrics.kugou.com".into()),
            // 歌词域名不吃网关那套签名/默认参数
            encrypt_type: EncryptType::None,
            skip_user_auth: true,
            clear_default_params: true,
            ..Default::default()
        };
        let keyword = if song.artist.is_empty() {
            song.title.clone()
        } else {
            format!("{} - {}", song.artist, song.title)
        };
        spec.params.insert("ver".into(), "1".into());
        spec.params.insert("man".into(), "yes".into());
        spec.params.insert("client".into(), "pc".into());
        spec.params.insert("keyword".into(), keyword);
        if !song.hash.is_empty() {
            spec.params.insert("hash".into(), song.hash.clone());
        }
        if song.duration > 0 {
            spec.params.insert("duration".into(), song.duration.to_string());
        }

        let text = self.request_text(spec).await?;
        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("歌词搜索返回不是 JSON：{e}；{:.120}", text))?;
        let Some(arr) = v.pointer("/candidates").and_then(|x| x.as_array()) else {
            return Ok(Vec::new());
        };
        Ok(arr
            .iter()
            .map(|c| LyricCandidate {
                id: c
                    .get("id")
                    .and_then(|x| x.as_i64())
                    .map(|n| n.to_string())
                    .or_else(|| c.get("id").and_then(|x| x.as_str()).map(str::to_string))
                    .unwrap_or_default(),
                accesskey: c
                    .get("accesskey")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                title: c
                    .get("song")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                artist: c
                    .get("singer")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                duration: c.get("duration").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
            })
            .filter(|c| !c.id.is_empty())
            .collect())
    }

    /// 下载歌词原文（`fmt` = `krc` 或 `lrc`）。返回解密后的歌词文本。
    pub async fn lyric_download_raw(
        &self,
        id: &str,
        accesskey: &str,
        fmt: &str,
    ) -> anyhow::Result<String> {
        let mut spec = RequestSpec {
            url: "/download".into(),
            base_url: Some("https://lyrics.kugou.com".into()),
            encrypt_type: EncryptType::Android,
            skip_user_auth: true,
            ..Default::default()
        };
        spec.params.insert("ver".into(), "1".into());
        spec.params.insert("client".into(), "android".into());
        spec.params.insert("id".into(), id.to_string());
        spec.params.insert("accesskey".into(), accesskey.to_string());
        spec.params.insert("fmt".into(), fmt.to_string());
        spec.params.insert("charset".into(), "utf8".into());

        let text = self.request_text(spec).await?;
        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("歌词下载返回不是 JSON：{e}；{:.120}", text))?;
        let ec = v.get("error_code").and_then(|x| x.as_i64()).unwrap_or(0);
        if ec != 0 {
            anyhow::bail!(
                "歌词下载失败：{}",
                v.get("info").and_then(|x| x.as_str()).unwrap_or("未知错误")
            );
        }
        let content = v
            .get("content")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if content.is_empty() {
            anyhow::bail!("歌词内容为空");
        }
        let contenttype = v.get("contenttype").and_then(|x| x.as_i64()).unwrap_or(0);
        let plain = if fmt == "lrc" || contenttype != 0 {
            // 明文 LRC
            let bytes = base64::engine::general_purpose::STANDARD.decode(content.as_bytes())?;
            String::from_utf8_lossy(&bytes).to_string()
        } else {
            decode_krc(&content)?
        };
        Ok(plain)
    }

    /// 拿一首歌的歌词：搜索候选 → 挑时长最接近的 → 下载 KRC（失败退回 LRC）。
    pub async fn lyric(&self, song: &Song) -> anyhow::Result<Lyric> {
        let cands = self.lyric_candidates(song).await?;
        if cands.is_empty() {
            anyhow::bail!("没有找到歌词");
        }
        // 时长最接近的那个候选（单位不一致，按差值绝对值排序）
        let target = song.duration.max(1);
        let best = cands
            .iter()
            .min_by_key(|c| {
                let d = if c.duration == 0 { 999 } else { c.duration };
                (d as i64 - target as i64).abs()
            })
            .cloned()
            .unwrap_or_else(|| cands[0].clone());

        let mut last_err = None;
        for fmt in ["krc", "lrc"] {
            match self.lyric_download_raw(&best.id, &best.accesskey, fmt).await {
                Ok(text) => {
                    let lyric = if fmt == "krc" {
                        parse_krc(&text)
                    } else {
                        parse_lrc(&text)
                    };
                    if !lyric.is_empty() {
                        return Ok(lyric);
                    }
                    last_err = Some(anyhow::anyhow!("歌词解析后为空"));
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("歌词加载失败")))
    }
}

/// KRC 解密：base64 → 跳过 4 字节头 → XOR → zlib inflate。
pub fn decode_krc(b64: &str) -> anyhow::Result<String> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim().as_bytes())?;
    if bytes.len() <= 4 {
        anyhow::bail!("KRC 数据太短");
    }
    let mut data = bytes[4..].to_vec();
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= KRC_KEY[i % KRC_KEY.len()];
    }
    let mut out = String::new();
    ZlibDecoder::new(&data[..])
        .read_to_string(&mut out)
        .map_err(|e| anyhow::anyhow!("KRC zlib 解压失败：{e}"))?;
    Ok(out)
}

/// 解析 KRC 文本：`[1234,567]你<1234,100>好`
pub fn parse_krc(text: &str) -> Lyric {
    // 每行连同它的「行时长」一起攒着：逐字时长要用它兜底
    let mut rows: Vec<(LyricLine, u64)> = Vec::new();
    let mut offset_ms = 0i64;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // 头部元信息
        if let Some(v) = line.strip_prefix("[offset:") {
            offset_ms = v.trim_end_matches(']').trim().parse().unwrap_or(0);
            continue;
        }
        if line.starts_with("[ti:")
            || line.starts_with("[ar:")
            || line.starts_with("[al:")
            || line.starts_with("[by:")
            || line.starts_with("[hash:")
            || line.starts_with("[total:")
        {
            continue;
        }
        // 行头：[start,dur]
        let Some((head, body)) = split_time_head(line) else {
            continue;
        };
        let (time_ms, dur) = head;
        let (plain, words) = parse_krc_words(body);
        if plain.is_empty() {
            continue;
        }
        rows.push((
            LyricLine {
                time_ms,
                text: plain,
                words,
            },
            dur,
        ));
    }
    rows.sort_by_key(|(l, _)| l.time_ms);

    // ===== 逐字时长归一化 =====
    // **酷狗 KRC 的逐字标签普遍是 `<start,0>`，时长全是 0**（实测：
    // `小(0+0) 苹(121+0) 果(242+0)`）。时长是 0 的话，逐字填充只能算成
    // 「未唱 / 已唱」两态，看起来就是整字硬切、没有任何渐变。
    // 这里按「下一个字的开始时间」把它补出来；行内最后一个字用行时长
    // （或下一行开始时间）兜底，实在没有就给 400ms。
    let times: Vec<u64> = rows.iter().map(|(l, _)| l.time_ms).collect();
    for i in 0..rows.len() {
        let line_dur = rows[i].1;
        let line_t = times[i];
        let next_line_t = times.get(i + 1).copied();
        let starts: Vec<u64> = rows[i].0.words.iter().map(|w| w.start_ms).collect();
        for j in 0..starts.len() {
            if rows[i].0.words[j].dur_ms > 0 {
                continue;
            }
            let new_dur = match starts.get(j + 1).copied() {
                Some(s) if s > starts[j] => s - starts[j],
                _ => {
                    let total = if line_dur > 0 {
                        line_dur
                    } else if let Some(nt) = next_line_t {
                        nt.saturating_sub(line_t)
                    } else {
                        0
                    };
                    if total > starts[j] {
                        total - starts[j]
                    } else {
                        400
                    }
                }
            };
            rows[i].0.words[j].dur_ms = new_dur;
        }
    }

    let lines: Vec<LyricLine> = rows.into_iter().map(|(l, _)| l).collect();
    Lyric {
        word_time: lines.iter().any(|l| !l.words.is_empty()),
        lines,
        offset_ms,
        has_translation: false,
    }
}

/// `[1234,567]...` → ((1234, 567), 剩余部分)
fn split_time_head(line: &str) -> Option<((u64, u64), &str)> {
    let s = line.strip_prefix('[')?;
    let close = s.find(']')?;
    let inner = &s[..close];
    let (a, b) = inner.split_once(',')?;
    let start: u64 = a.trim().parse().ok()?;
    let dur: u64 = b.trim().parse().unwrap_or(0);
    let rest = &s[close + 1..];
    Some(((start, dur), rest))
}

/// KRC 逐字部分：`<start,dur>字<start,dur>字…` → ("字字…", [字时间轴])。
///
/// 时间标签在字的**前面**：`<a,b>` 后面到下一个 `<` 之间的文本才是这个标签对应的字。
/// （早期实现把标签前面的文本配给当前标签：一是每个字的起唱时间整体错后一个字，
/// 二是行尾最后一个字没有标签可配、只进了纯文本，于是「正在唱的那一行」
/// ——它按 words 渲染——永远少最后一个字。）
fn parse_krc_words(body: &str) -> (String, Vec<LyricWord>) {
    let mut plain = String::new();
    let mut words: Vec<LyricWord> = Vec::new();
    let mut rest = body;

    // 标签之前一般没有裸文本，有就并进整行文本（不进逐字时间轴）
    while let Some(open) = rest.find('<') {
        plain.push_str(&rest[..open]);
        rest = &rest[open + 1..];
        let Some(close) = rest.find('>') else {
            plain.push_str(rest);
            rest = "";
            break;
        };
        let tag = &rest[..close];
        rest = &rest[close + 1..];
        let (start_ms, dur_ms) = match tag.split_once(',') {
            Some((a, b)) => (
                a.trim().parse().unwrap_or(0u64),
                b.trim().parse().unwrap_or(0u64),
            ),
            None => (0, 0),
        };
        // 这个标签管到哪里：下一个 '<' 之前
        let end = rest.find('<').unwrap_or(rest.len());
        let text = &rest[..end];
        rest = &rest[end..];
        if !text.is_empty() {
            plain.push_str(text);
            words.push(LyricWord {
                start_ms,
                dur_ms,
                text: text.to_string(),
            });
        }
    }
    plain.push_str(rest);
    (plain, words)
}

/// 解析 LRC：`[mm:ss.xx]文本`，支持一行多个时间标签。
pub fn parse_lrc(text: &str) -> Lyric {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // 逐个取出 [时间]
        let mut rest = line;
        let mut times: Vec<u64> = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(close) = stripped.find(']') else { break };
            let inner = &stripped[..close];
            if let Some(t) = parse_lrc_time(inner) {
                times.push(t);
                rest = &stripped[close + 1..];
            } else {
                break;
            }
        }
        if times.is_empty() {
            continue;
        }
        let content = rest.trim().to_string();
        if content.is_empty() {
            continue;
        }
        for t in times {
            lines.push(LyricLine {
                time_ms: t,
                text: content.clone(),
                words: Vec::new(),
            });
        }
    }
    lines.sort_by_key(|l| l.time_ms);
    Lyric {
        lines,
        offset_ms: 0,
        word_time: false,
        has_translation: false,
    }
}

/// `mm:ss.xx` / `mm:ss` → 毫秒
fn parse_lrc_time(s: &str) -> Option<u64> {
    let (m, rest) = s.split_once(':')?;
    let m: u64 = m.trim().parse().ok()?;
    let (sec, frac) = match rest.split_once('.') {
        Some((a, b)) => (a.trim(), Some(b)),
        None => (rest.trim(), None),
    };
    let sec: u64 = sec.parse().ok()?;
    let ms: u64 = match frac {
        Some(f) => {
            let f: String = f.chars().filter(|c| c.is_ascii_digit()).take(3).collect();
            if f.is_empty() {
                0
            } else {
                f.parse::<u64>().unwrap_or(0)
            }
        }
        None => 0,
    };
    Some(m * 60_000 + sec * 1000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 逐字解析：标签在字**前面**，行尾最后一个字不能丢、时间不能错位。
    #[test]
    fn krc_words_align_with_tags() {
        let lyric = parse_krc("[1000,900]<0,300>作<300,300>词<600,300>方");
        assert_eq!(lyric.lines.len(), 1);
        let line = &lyric.lines[0];
        assert_eq!(line.text, "作词方");
        let words: Vec<(&str, u64, u64)> = line
            .words
            .iter()
            .map(|w| (w.text.as_str(), w.start_ms, w.dur_ms))
            .collect();
        assert_eq!(
            words,
            vec![("作", 0, 300), ("词", 300, 300), ("方", 600, 300)],
            "逐字时间轴必须与字一一对应，且包含行尾最后一个字"
        );
        // 纯文本一定是逐字文本拼起来的（正在唱的那一行按 words 渲染，少一个字就露馅）
        let joined: String = line.words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(joined, line.text);
    }

    /// 没有逐字标签时只出纯文本（退回整行高亮）。
    #[test]
    fn krc_line_without_words() {
        let lyric = parse_krc("[1000,900]纯文本歌词");
        assert_eq!(lyric.lines[0].text, "纯文本歌词");
        assert!(lyric.lines[0].words.is_empty());
    }
}

#[cfg(test)]
mod word_dur_tests {
    use super::*;

    /// 酷狗 KRC 的逐字标签时长常为 0（`<121,0>`）。必须按下一个字的开始时间
    /// 把时长补出来，否则逐字填充只剩「未唱/已唱」两态，看不到渐变。
    #[test]
    fn krc_word_dur_filled_from_next_start() {
        let lyric = parse_krc("[0,1000]<0,0>你<500,0>好");
        let ws = &lyric.lines[0].words;
        assert_eq!(ws.len(), 2);
        assert_eq!(ws[0].start_ms, 0);
        assert_eq!(ws[0].dur_ms, 500, "第一个字的时长应等于下一个字的开始时间");
        assert!(ws[1].dur_ms > 0, "行尾字也要有非零时长（用行时长兜底）");
    }

    /// 已经有合法时长的不动。
    #[test]
    fn krc_word_dur_kept_when_present() {
        let lyric = parse_krc("[0,1000]<0,300>你<300,300>好");
        assert_eq!(lyric.lines[0].words[0].dur_ms, 300);
        assert_eq!(lyric.lines[0].words[1].dur_ms, 300);
    }
}
