//! AMLL（Apple Music-like Lyrics）歌词渲染。
//!
//! 只有两件事：
//! - **逐字注水**：一个字唱多久就填多久（唱到一半就填一半），字内填充边界连续右移；
//! - **换行弹性**：一行被唱到时整行 spring 放大一下。
//!
//! gpui 里做「字内部分填充」的唯一可靠办法是**裁剪**：文字走 `paint_layer`
//! （scene layer + scissor），吃祖先的 ContentMask，所以「覆盖层 + 宽度=进度
//! 的裁剪盒」就能得到水位线效果。早先「颜色插值」之所以看不出渐变，是因为
//! 插值中间色（高亮度低饱和）和未唱的灰几乎一样；而「整字换色」本身也压根
//! 不是卡拉 OK 的效果。

use gpui::{div, px, prelude::*, Animation, AnimationExt, Div, Hsla, Pixels, SharedString};

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

/// 换行弹性的幅度与时长：峰值放大 18%，460ms 收住。
const SPRING_GAIN: f32 = 0.18;
const SPRING_MS: u64 = 460;

/// 阻尼脉冲波形：t∈[0,1]，从 0 起振、约一个周期后衰减回 0。
fn spring_wave(t: f32) -> f32 {
    (1.0 - t).powi(2) * (t * std::f32::consts::PI * 2.4).sin()
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

/// 渲染一行（宽度由内容撑开 —— 桌面歌词居中用）。
pub fn line(words: impl IntoIterator<Item = (String, f32)>, cfg: &AmllConfig) -> Div {
    line_with_width(words, cfg, None)
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

/// 给一行套上「换行弹性」。
///
/// 动画 id 带行号：行变了 id 就变，gpui 会为新 id 重新起表，于是 spring
/// 只在这一行被唱到的那一下跑一次（`AnimationElement` 的状态按元素 id 缓存）。
pub fn with_line_spring(
    row: Div,
    cfg: &AmllConfig,
    id_prefix: &str,
    line_index: usize,
) -> gpui::AnyElement {
    let base = f32::from(cfg.text_size);
    row.with_animation(
        SharedString::from(format!("{id_prefix}-spring-{line_index}")),
        Animation::new(std::time::Duration::from_millis(SPRING_MS)),
        move |el, t| el.text_size(px(base * (1.0 + SPRING_GAIN * spring_wave(t)))),
    )
    .into_any_element()
}

/// 「当前正在唱的这行」一步到位：逐字注水 + 换行弹性。
///
/// 播放页和桌面歌词都走这一个入口，避免两边各算一套进度。
pub fn current_line(
    line: &echo_core::LyricLine,
    t_ms: f64,
    cfg: &AmllConfig,
    id_prefix: &str,
    line_index: usize,
    width: Option<Pixels>,
) -> gpui::AnyElement {
    with_line_spring(
        line_with_width(line_words(line, t_ms), cfg, width),
        cfg,
        id_prefix,
        line_index,
    )
}
