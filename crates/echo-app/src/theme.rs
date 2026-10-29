//! 主题系统：色板 token、深浅色模式、主题色。
//! 对齐原项目 Tailwind 主题变量（浅色参考 screenshots/home.png，深色参考 lyric.png）。

use gpui::Hsla;

/// 主题色板（对齐 EchoMusic CSS 变量）。
#[derive(Debug, Clone)]
pub struct Theme {
    /// 主题色（默认 #f55e55）
    pub accent: Hsla,
    /// 面板/卡片背景
    pub bg_panel: Hsla,
    /// 悬停背景
    pub bg_hover: Hsla,
    /// 主文字
    pub text: Hsla,
    /// 次级文字
    pub text_secondary: Hsla,
    /// 弱化文字
    pub text_tertiary: Hsla,
    /// 边框
    pub border: Hsla,
    /// 播放条进度槽
    pub progress_track: Hsla,
}

fn hex(h: u32) -> Hsla {
    gpui::rgb(h).into()
}

impl Theme {
    pub fn light(accent: Hsla) -> Self {
        Self {
            accent,
            bg_panel: hex(0xffffff),
            bg_hover: hex(0x000000).opacity(0.05),
            text: hex(0x333333),
            text_secondary: hex(0x7a7a7a),
            text_tertiary: hex(0xb3b3b3),
            border: hex(0x000000).opacity(0.08),
            progress_track: hex(0x000000).opacity(0.12),
        }
    }

    pub fn dark(accent: Hsla) -> Self {
        Self {
            accent,
            bg_panel: hex(0x232b36),
            bg_hover: hex(0xffffff).opacity(0.07),
            text: hex(0xf0f0f0),
            text_secondary: hex(0xa8aeb5),
            text_tertiary: hex(0x7c8791),
            border: hex(0xffffff).opacity(0.10),
            progress_track: hex(0xffffff).opacity(0.15),
        }
    }
}

/// "#f55e55" → Hsla。
pub fn parse_color(s: &str) -> Hsla {
    let s = s.trim_start_matches('#');
    u32::from_str_radix(s, 16).map(hex).unwrap_or_else(|_| hex(0xf55e55))
}

