//! 内置 SVG 图标（24x24 viewBox，stroke 风格，颜色跟随 text_color）。

/// 生成 stroke 风格 SVG 包装。
macro_rules! icon {
    ($body:expr) => {
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"#,
            $body,
            "</svg>"
        )
    };
}

/// 资源路径 → SVG 字节。load 时按需返回。
pub fn icon_svg(name: &str) -> Option<&'static str> {
    Some(match name {
        "icons/arrow-left.svg" => icon!(r#"<path d="M19 12H5"/><path d="M12 19l-7-7 7-7"/>"#),
        "icons/arrow-right.svg" => icon!(r#"<path d="M5 12h14"/><path d="M12 5l7 7-7 7"/>"#),
        "icons/refresh.svg" => icon!(r#"<path d="M21 12a9 9 0 1 1-2.64-6.36"/><path d="M21 3v6h-6"/>"#),
        "icons/search.svg" => icon!(r#"<circle cx="11" cy="11" r="8"/><path d="M21 21l-4.35-4.35"/>"#),
        "icons/mic.svg" => icon!(r#"<rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 10a7 7 0 0 0 14 0"/><path d="M12 19v3"/>"#),
        "icons/list-music.svg" => icon!(r#"<path d="M3 6h12"/><path d="M3 12h12"/><path d="M3 18h8"/><circle cx="17" cy="16" r="3"/><path d="M20 16V7l3-1"/>"#),
        "icons/play.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M8 5.14v13.72c0 .96 1.05 1.55 1.87 1.05l10.9-6.86a1.24 1.24 0 0 0 0-2.1L9.87 4.09A1.24 1.24 0 0 0 8 5.14z"/></svg>"#,
        "icons/pause.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" stroke="none"><rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/></svg>"#,
        "icons/prev.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M6 5a1 1 0 0 1 2 0v14a1 1 0 0 1-2 0z"/><path d="M20 5.87v12.26c0 .99-1.1 1.58-1.93 1.03L8.6 13.03a1.24 1.24 0 0 1 0-2.07l9.47-6.12c.83-.55 1.93.04 1.93 1.03z"/></svg>"#,
        "icons/next.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M16 5a1 1 0 0 1 2 0v14a1 1 0 0 1-2 0z"/><path d="M4 5.87v12.26c0 .99 1.1 1.58 1.93 1.03l9.47-6.13a1.24 1.24 0 0 0 0-2.07L5.93 4.84C5.1 4.29 4 4.88 4 5.87z"/></svg>"#,
        "icons/heart.svg" => icon!(r#"<path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7z"/>"#),
        "icons/heart-filled.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7z"/></svg>"#,
        "icons/cloud.svg" => icon!(r#"<path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9z"/>"#),
        "icons/history.svg" => icon!(r#"<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l3 3"/>"#),
        "icons/settings.svg" => icon!(r#"<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>"#),
        "icons/shirt.svg" => icon!(r#"<path d="M20.38 3.46 16 2a4 4 0 0 1-8 0L3.62 3.46a2 2 0 0 0-1.34 2.23l.58 3.47a1 1 0 0 0 .99.84H6v10c0 1.1.9 2 2 2h8a2 2 0 0 0 2-2V10h2.15a1 1 0 0 0 .99-.84l.58-3.47a2 2 0 0 0-1.34-2.23z"/>"#),
        "icons/sparkles.svg" => icon!(r#"<path d="M12 3l1.9 5.8a2 2 0 0 0 1.3 1.3L21 12l-5.8 1.9a2 2 0 0 0-1.3 1.3L12 21l-1.9-5.8a2 2 0 0 0-1.3-1.3L3 12l5.8-1.9a2 2 0 0 0 1.3-1.3z"/>"#),
        "icons/compass.svg" => icon!(r#"<circle cx="12" cy="12" r="10"/><path d="m16.24 7.76-2.12 6.36-6.36 2.12 2.12-6.36z"/>"#),
        "icons/star.svg" => icon!(r#"<path d="m12 2 3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01z"/>"#),
        "icons/vinyl.svg" => icon!(r#"<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="2"/><path d="M12 5a7 7 0 0 1 7 7"/>"#),
        "icons/cassette.svg" => icon!(r#"<rect x="2" y="5" width="20" height="14" rx="2"/><circle cx="8" cy="12" r="2"/><circle cx="16" cy="12" r="2"/><path d="M8 14h8"/>"#),
        "icons/moon.svg" => icon!(r#"<path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/>"#),
        "icons/sun.svg" => icon!(r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/>"#),
        "icons/chevron-up.svg" => icon!(r#"<path d="m18 15-6-6-6 6"/>"#),
        "icons/radio.svg" => icon!(r#"<circle cx="12" cy="12" r="2"/><path d="M16.24 7.76a6 6 0 0 1 0 8.49"/><path d="M7.76 16.24a6 6 0 0 1 0-8.49"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/><path d="M4.93 19.07a10 10 0 0 1 0-14.14"/>"#),
        "icons/heart-sparkle.svg" => icon!(r#"<path d="M12 20.3 4.9 13.2a4.5 4.5 0 0 1 6.4-6.4l.7.7.7-.7a4.5 4.5 0 0 1 3.4-1.3"/><path d="M18.6 3.2v4"/><path d="M16.6 5.2h4"/>"#),
        // 私人漫游：耳机 + 声波
        "icons/headphones.svg" => icon!(r#"<path d="M4 15v-3a8 8 0 0 1 16 0v3"/><path d="M4 15h2.5a1.5 1.5 0 0 1 1.5 1.5v3A1.5 1.5 0 0 1 6.5 21H6a2 2 0 0 1-2-2z"/><path d="M20 15h-2.5a1.5 1.5 0 0 0-1.5 1.5v3a1.5 1.5 0 0 0 1.5 1.5h.5a2 2 0 0 0 2-2z"/>"#),
        "icons/bag.svg" => icon!(r#"<path d="M5 8h14l1 12H4z"/><path d="M9 8V6a3 3 0 0 1 6 0v2"/>"#),
        "icons/sort.svg" => icon!(r#"<path d="M6 4v14"/><path d="m2.5 14.5 3.5 3.5 3.5-3.5"/><path d="M12.5 7h9"/><path d="M12.5 12h6"/><path d="M12.5 17h3"/>"#),
        "icons/qr.svg" => icon!(r#"<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><path d="M14 14h3v3h-3z"/><path d="M20 14v3"/><path d="M14 20h3"/><path d="M20 20h1"/>"#),
        "icons/phone.svg" => icon!(r#"<rect x="7" y="2" width="10" height="20" rx="2.5"/><path d="M11 18.5h2"/>"#),
        "icons/lock.svg" => icon!(r#"<rect x="4" y="10" width="16" height="11" rx="2.5"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/>"#),
        "icons/calendar.svg" => icon!(r#"<rect x="3" y="5" width="18" height="16" rx="2"/><path d="M8 3v4"/><path d="M16 3v4"/><path d="M3 11h18"/>"#),
        "icons/repeat.svg" => icon!(r#"<path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14"/><path d="m7 22-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/>"#),
        "icons/repeat-one.svg" => icon!(r#"<path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14"/><path d="m7 22-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/><path d="M11 10h1v4"/>"#),
        "icons/shuffle.svg" => icon!(r#"<path d="M2 18h1.4c1.3 0 2.5-.6 3.3-1.7l6.1-8.6c.8-1.1 2-1.7 3.3-1.7H22"/><path d="m18 2 4 4-4 4"/><path d="M2 6h1.9c1.5 0 2.9.9 3.6 2.2"/><path d="M22 18h-5.9c-1.3 0-2.6-.7-3.3-1.8l-.5-.8"/><path d="m18 14 4 4-4 4"/>"#),
        "icons/volume.svg" => icon!(r#"<path d="M11 5 6 9H2v6h4l5 4z"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/>"#),
        "icons/volume-mute.svg" => icon!(r#"<path d="M11 5 6 9H2v6h4l5 4z"/><path d="M16 9l5 6"/><path d="M21 9l-5 6"/>"#),
        "icons/mute.svg" => icon!(r#"<path d="M11 5 6 9H2v6h4l5 4z"/><path d="m22 9-6 6"/><path d="m16 9 6 6"/>"#),
        "icons/queue.svg" => icon!(r#"<path d="M3 6h18"/><path d="M3 12h18"/><path d="M3 18h10"/><path d="M17 16v6"/><path d="M14 19h6"/>"#),
        "icons/more.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" stroke="none"><circle cx="5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="19" cy="12" r="1.8"/></svg>"#,
        "icons/minimize.svg" => icon!(r#"<path d="M5 12h14"/>"#),
        "icons/maximize.svg" => icon!(r#"<rect x="5" y="5" width="14" height="14" rx="1"/>"#),
        // 还原（最大化后显示）：两个错开的方框
        "icons/restore.svg" => icon!(
            r#"<rect x="8" y="8" width="12" height="12" rx="1"/><path d="M16 8V5.5A1.5 1.5 0 0 0 14.5 4h-9A1.5 1.5 0 0 0 4 5.5v9A1.5 1.5 0 0 0 5.5 16H8"/>"#
        ),
        "icons/close.svg" => icon!(r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#),
        "icons/plus.svg" => icon!(r#"<path d="M12 5v14"/><path d="M5 12h14"/>"#),
        "icons/chevron-down.svg" => icon!(r#"<path d="m6 9 6 6 6-6"/>"#),
        "icons/music-note.svg" => icon!(r#"<circle cx="8" cy="18" r="3"/><circle cx="20" cy="16" r="3"/><path d="M11 18V5l12-2v13"/>"#),
        "icons/comment.svg" => icon!(r#"<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>"#),
        "icons/timer.svg" => icon!(r#"<circle cx="12" cy="13" r="8"/><path d="M12 9v4l2 2"/><path d="M9 2h6"/>"#),
        "icons/eq.svg" => icon!(r#"<path d="M6 20V10"/><path d="M12 20V4"/><path d="M18 20v-6"/>"#),
        "icons/download.svg" => icon!(r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/><path d="M12 15V3"/>"#),
        "icons/user.svg" => icon!(r#"<circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 3.6-6 8-6s8 2 8 6"/>"#),
        "icons/clock.svg" => icon!(r#"<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>"#),
        _ => return None,
    })
}
