//! 主题系统：色板 token、深浅色模式、主题色。
//! 对齐原项目 Tailwind 主题变量（浅色参考 screenshots/home.png，深色参考 lyric.png）。

use gpui::Hsla;

/// 封面图统一圆角（专辑 / 歌单 / 歌曲缩略图 / 播放条封面）。
pub const COVER_RADIUS: f32 = 8.0;
/// 卡片（面板）圆角。
pub const CARD_RADIUS: f32 = 12.0;
/// 小控件（按钮 / 输入框）圆角。
pub const CONTROL_RADIUS: f32 = 10.0;

/// 主题色板（对齐 EchoMusic CSS 变量）。
#[derive(Debug, Clone)]
pub struct Theme {
    pub dark: bool,
    /// 主题色（默认 #f55e55）
    pub accent: Hsla,
    /// 窗口背景（内容区）
    pub bg_main: Hsla,
    /// 侧边栏背景
    pub bg_sidebar: Hsla,
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

/// 主题配色过渡时长（秒）。
pub const THEME_TRANSITION_SECS: f32 = 0.26;

//// 开光灯圆形揭示时长（秒）。
pub const THEME_REVEAL_SECS: f32 = 0.55;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_hsla(a: Hsla, b: Hsla, t: f32) -> Hsla {
    // 在 sRGB 空间插值：色相在浅色(暖)/深色(冷)之间插值会经过黄绿，观感很脏
    let ra = a.to_rgb();
    let rb = b.to_rgb();
    gpui::Rgba {
        r: lerp(ra.r, rb.r, t),
        g: lerp(ra.g, rb.g, t),
        b: lerp(ra.b, rb.b, t),
        a: lerp(ra.a, rb.a, t),
    }
    .into()
}

impl Theme {
    /// 两套配色之间的插值（用于浅色/深色切换动画）。
    pub fn blend(from: &Theme, to: &Theme, t: f32) -> Theme {
        let t = t.clamp(0.0, 1.0);
        Theme {
            dark: if t < 0.5 { from.dark } else { to.dark },
            accent: lerp_hsla(from.accent, to.accent, t),
            bg_main: lerp_hsla(from.bg_main, to.bg_main, t),
            bg_sidebar: lerp_hsla(from.bg_sidebar, to.bg_sidebar, t),
            bg_panel: lerp_hsla(from.bg_panel, to.bg_panel, t),
            bg_hover: lerp_hsla(from.bg_hover, to.bg_hover, t),
            text: lerp_hsla(from.text, to.text, t),
            text_secondary: lerp_hsla(from.text_secondary, to.text_secondary, t),
            text_tertiary: lerp_hsla(from.text_tertiary, to.text_tertiary, t),
            border: lerp_hsla(from.border, to.border, t),
            progress_track: lerp_hsla(from.progress_track, to.progress_track, t),
        }
    }
}

impl Theme {
    pub fn light(accent: Hsla) -> Self {
        Self {
            dark: false,
            accent,
            bg_main: hex(0xfaf7f2),
            bg_sidebar: hex(0xf3efe9),
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
            dark: true,
            accent,
            bg_main: hex(0x1b222b),
            bg_sidebar: hex(0x161d25),
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

