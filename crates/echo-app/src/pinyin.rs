//! 汉字 → 拼音首字母（纯前端，不走任何接口/词典文件）。
//!
//! 原理：GB2312 一级汉字（3755 个）是**按拼音排序**的，所以只要知道每个声母
//! 在 GBK 码表里的起始字节，就能用二分/顺序比较把汉字映射成首字母。
//! 二级汉字（生僻字，GBK `0xD8A1` 起）按部首排序，查不到首字母 —— 那些字直接
//! 跳过，搜索时仍能用原字命中原标题。
//!
//! 例：「错的人」→ GBK `B4ED B5C4 C8CB` → `cdr`。

/// 各声母在 GBK 里的起始码点（`0xB0A1` 起，一级汉字区 `0xB0A1..=0xD7F9`）。
/// 没有 i / u / v 开头的拼音，所以表里也没有它们。
const BOUNDS: [(u16, u8); 23] = [
    (0xB0A1, b'a'),
    (0xB0C5, b'b'),
    (0xB2C1, b'c'),
    (0xB4EE, b'd'),
    (0xB6EA, b'e'),
    (0xB7A2, b'f'),
    (0xB8C1, b'g'),
    (0xB9FE, b'h'),
    (0xBBF7, b'j'),
    (0xBFA6, b'k'),
    (0xC0AC, b'l'),
    (0xC2E8, b'm'),
    (0xC4C3, b'n'),
    (0xC5B6, b'o'),
    (0xC5BE, b'p'),
    (0xC6DA, b'q'),
    (0xC8BB, b'r'),
    (0xC8F6, b's'),
    (0xCBFA, b't'),
    (0xCDDA, b'w'),
    (0xCEF4, b'x'),
    (0xD1B9, b'y'),
    (0xD4D1, b'z'),
];

/// 整串 → 首字母串（ASCII 字母/数字原样保留并转小写）。
/// 生产路径批量建索引时用的是下面的 `append_initials`，这个留给单测和临时排查。
#[allow(dead_code)]
pub fn initials(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    append_initials(s, &mut out);
    out
}

/// 同上，但写进调用方给的缓冲区（批量建索引时省一次分配）。
pub fn append_initials(s: &str, out: &mut String) {
    let (bytes, _, _) = encoding_rs::GBK.encode(s);
    let b: &[u8] = &bytes;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c < 0x80 {
            if c == b'&' {
                // GBK 表示不了的字符会被编码成 `&#NNNN;`，整段跳过
                while i < b.len() && b[i] != b';' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            if c.is_ascii_alphanumeric() {
                out.push((c as char).to_ascii_lowercase());
            }
            i += 1;
        } else if i + 1 < b.len() {
            let code = ((c as u16) << 8) | b[i + 1] as u16;
            if let Some(l) = first_letter(code) {
                out.push(l as char);
            }
            i += 2;
        } else {
            break;
        }
    }
}

fn first_letter(code: u16) -> Option<u8> {
    if !(0xB0A1..=0xD7F9).contains(&code) {
        return None;
    }
    let mut hit = None;
    for (start, letter) in BOUNDS {
        if code >= start {
            hit = Some(letter);
        } else {
            break;
        }
    }
    hit
}

#[cfg(test)]
mod tests {
    use super::initials;

    #[test]
    fn common_words() {
        assert_eq!(initials("错的人"), "cdr");
        assert_eq!(initials("周杰伦"), "zjl");
        assert_eq!(initials("晴天"), "qt");
        assert_eq!(initials("Love Story"), "lovestory");
        assert_eq!(initials("无与伦比"), "wylb");
    }
}
