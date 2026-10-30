//! AMLL（Apple Music-like Lyrics）歌词渲染。
//!
//! 只做一件事：**逐字注水** —— 一个字唱多久就填多久（唱到一半就填一半），
//! 字内填充边界连续右移。
//!
//! （播放页那套「换行时整行 spring 放大」在 Lite 里不要：迷你面板只有一行，
//! 整行缩放看着就是一跳一跳。）
//! gpui 里做「字内部分填充」的唯一可靠办法是**裁剪**：文字走 `paint_layer`
//! （scene layer + scissor），吃祖先的 ContentMask，所以「覆盖层 + 宽度=进度
//! 的裁剪盒」就能得到水位线效果。早先「颜色插值」之所以看不出渐变，是因为
//! 插值中间色（高亮度低饱和）和未唱的灰几乎一样；而「整字换色」本身也压根
//! 不是卡拉 OK 的效果。

use gpui::{div, prelude::*, Div, Hsla, Pixels};

/// 一行歌词的渲染配置。
#[derive(Clone, Copy)]
pub struct AmllConfig {
    /// 未唱颜色
    pub base: Hsla,
    /// 已唱颜色
    pub accent: Hsla,
    /// 字号（换行弹性的基准）
    pub text_size: Pixels,
}

impl AmllConfig {
    pub fn new(base: Hsla, accent: Hsla, text_size: Pixels) -> Self {
        Self {
            base,
            accent,
            text_size,
        }
    }
}

/// 一行歌词里每个字的填充进度（0..1）。
///
/// **严格按各自的演唱时长**：某个字 `dur` 毫秒，唱到 `dur/2` 就是 0.5。
///
/// KRC 的逐字标签时长经常是 0（`<121,0>`），解析层已经用「下一个字的开始时间」
/// 补好了（`echo_api::lyrics::parse_krc`），这里只按比例算。
pub fn line_words(line: &echo_core::LyricLine, t_ms: f64) -> Vec<(String, f32)> {
    if line.words.is_empty() {
        return vec![(line.text.clone(), 1.0)];
    }
    line.words
        .iter()
        .map(|w| {
            let frac = if w.dur_ms > 1 {
                ((t_ms - w.start_ms as f64) / w.dur_ms as f64).clamp(0.0, 1.0)
            } else if t_ms >= w.start_ms as f64 {
                1.0
            } else {
                0.0
            };
            (w.text.clone(), frac as f32)
        })
        .collect()
}

/// 渲染一行，可指定宽度（播放页要把长句折到固定列宽里）。
pub fn line_with_width(
    words: impl IntoIterator<Item = (String, f32)>,
    cfg: &AmllConfig,
    width: Option<Pixels>,
) -> Div {
    let mut row = div().flex().flex_wrap().text_size(cfg.text_size);
    if let Some(w) = width {
        row = row.w(w);
    }
    for (text, frac) in words {
        row = row.child(word(&text, frac, cfg));
    }
    row
}

/// 单个词：词内再按字符切分，每个字有自己的水位线。
fn word(text: &str, frac: f32, cfg: &AmllConfig) -> gpui::AnyElement {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len().max(1) as f32;

    let mut row = div().flex().flex_shrink_0();
    for (ci, ch) in chars.iter().enumerate() {
        // 第 ci 个字在词内的进度：词进度落在 [ci/n, (ci+1)/n] 之间就是它正在唱
        let cf = ((frac - ci as f32 / n) * n).clamp(0.0, 1.0);
        let glyph = ch.to_string();
        // 底色字负责撑出这个字的固有宽度
        let cell = div()
            .relative()
            .flex_shrink_0()
            .text_color(cfg.base)
            .child(glyph.clone());
        let cell = if cf > 0.001 {
            cell.child(
                // 水位线：宽度 = 字宽 × 进度，超出部分被 overflow_hidden 裁掉
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .h_full()
                    .w(gpui::relative(cf))
                    .overflow_hidden()
                    .child(
                        // 同字的已唱色副本：nowrap 不收缩，按固有宽度铺开后被裁
                        div()
                            .whitespace_nowrap()
                            .flex_shrink_0()
                            .text_color(cfg.accent)
                            .child(glyph),
                    ),
            )
        } else {
            cell
        };
        row = row.child(cell);
    }
    row.into_any_element()
}
