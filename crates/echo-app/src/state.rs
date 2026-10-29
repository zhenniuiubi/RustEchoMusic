//! 应用全局状态：播放队列、设置、登录会话、曲库。
use crate::player::PlayerState;
use crate::theme::{parse_color, Theme};
use echo_api::KuGouClient;
use echo_audio::AudioEngine;
use echo_core::{Lyric, Playlist, SettingsStore, Song};
use gpui::Global;
use std::sync::Arc;

/// 扫码登录的运行时状态（Lite 只支持扫码，没有账号/短信表单了）。
#[derive(Debug, Clone)]
pub struct LoginForm {
    pub error: String,
    /// 二维码 key / 内容 URL
    pub qr_key: String,
    pub qr_url: String,
    /// 0 过期 / 1 等待扫码 / 2 待确认 / 4 成功
    pub qr_status: i32,
    pub qr_loading: bool,
    /// 轮询代次（刷新/取消时自增，用于终止旧轮询）
    pub qr_gen: u64,
}

impl Default for LoginForm {
    fn default() -> Self {
        Self {
            error: String::new(),
            qr_key: String::new(),
            qr_url: String::new(),
            qr_status: 1,
            qr_loading: false,
            qr_gen: 0,
        }
    }
}

pub struct AppStore {
    pub settings: SettingsStore,
    /// 当前生效的背景特效模式：启动时 = 设置值（可被 ECHO_BACKDROP 环境变量
    /// 覆盖），切换设置时由 `set_backdrop_mode` 同步更新。
    /// UI 配色和 DWM 应用都必须读它，否则环境变量覆盖时会出现
    /// 「DWM 已关材质、UI 还在半透明」的壁纸透底。
    pub backdrop_effective: echo_core::BackdropMode,
    pub client: Arc<KuGouClient>,
    pub engine: Arc<AudioEngine>,
    pub player: PlayerState,
    /// 登录用户的自建歌单（侧栏「自建歌单」）
    pub user_playlists: Vec<Playlist>,
    pub user_playlists_loaded: bool,
    /// 登录会话
    pub session: echo_core::SessionStore,
    /// 当前登录用户
    pub user: Option<echo_core::User>,
    pub login: LoginForm,
    /// 收藏与历史
    pub library: echo_core::LibraryStore,
    /// 播放队列的落盘快照（关掉 App 再打开可还原）
    pub queue_store: echo_core::QueueStore,
    /// 音频引擎诊断快照（欠载次数 / 缓冲深度 / 回调耗时，1Hz 更新）
    pub audio_diag: Option<echo_audio::DiagSnapshot>,
    /// 当前曲目的歌词
    pub lyric: Option<Lyric>,
    /// 歌词归属的歌曲 key（避免切歌时串台）
    pub lyric_key: String,
    pub lyric_loading: bool,
    /// 睡眠定时器的截止时刻（unix 秒），None = 未设置
    pub sleep_until: Option<i64>,
    /// 私人 FM 当前这一批歌曲
    pub fm: Arc<Vec<Song>>,
    /// 私人 FM 频道：normal 红心 / small 小众 / peak 速览 / radio 电台
    pub fm_mode: String,
    /// 私人 FM 推荐来源：0 口味 / 1 风格 / 2 探索
    pub fm_pool: i64,
    pub fm_loaded: bool,
    pub fm_loading: bool,
    /// 上一批 FM 的游标（`cur_mark`）：续推下一批必须带回去，否则服务端返回空列表
    pub fm_cur_mark: i64,
    /// 正在续推下一批 FM（防止一批播完时重复发起）
    pub fm_extending: bool,
    /// 私人 FM 本批曲目的 hash：用来判断「这一批是不是都播放过了」
    ///
    /// 注意这里记的是**队列里这一批**（play_from / fm_extend 时写入），
    /// 不是 `fm` 那份预览列表 —— 用户点圆盘插播时队列可能和预览不同。
    pub fm_batch: Vec<String>,
    /// 私人 FM 本批已经播放过的 hash（播到就记，不要求播完）
    pub fm_heard: std::collections::HashSet<String>,
    /// 远端「我喜欢的音乐」歌单（登录后从酷狗云歌单拉取）
    pub remote_liked: Arc<Vec<Song>>,
    /// 远端红心歌单是否已加载
    pub remote_liked_loaded: bool,
    /// 远端红心歌单加载中
    pub remote_liked_loading: bool,
    /// 远端红心歌单的 listid（加歌/删歌需要）
    pub remote_liked_listid: String,
    /// 远端红心歌曲的 key 集合（快速判断是否已红心）
    pub remote_liked_keys: std::collections::HashSet<String>,
    /// hash → 云歌单条目 id（audio_id），取消收藏时需要
    pub remote_liked_fileids: std::collections::HashMap<String, i64>,
    /// 播放进度条是否正在拖拽
    pub seeking: bool,
    /// 拖拽目标位置（秒）
    pub seek_target: f64,
    /// 全局提示（toast 文本，空则不显示）
    pub toast: String,
    /// 最近一次 Position 事件的时间（歌词扫光按耗时外推，30fps 平滑推进）
    pub position_at: std::time::Instant,
    /// 播放条进度条 bounds（canvas 记录，点击 seek 用）
    pub progress_bounds_slot: std::sync::Arc<parking_lot::Mutex<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// 音量滑条 bounds（canvas 记录）
    pub volume_bounds_slot: std::sync::Arc<parking_lot::Mutex<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// 主题切换动画：切换前的配色 + 起始时间
    pub theme_anim: Option<(Theme, std::time::Instant)>,
    /// 开光灯圆形揭示进行中（此时 theme() 返回 hold 旧主题）
    pub theme_reveal: Option<ThemeReveal>,
    /// 每日 VIP 领取中
    pub vip_claiming: bool,
    /// 每日 VIP 领取状态文案（空则不显示）
    pub vip_note: String,
    /// 服务端判定登录态已失效（token 过期 → 20018），需要重新登录
    pub session_expired: bool,
    /// 上次成功刷新 token 的时间（unix 秒）：运行期保活按它算下一次续期
    pub token_refresh_at: i64,
}

impl Global for AppStore {}

impl AppStore {
    /// 目标配色（不做过渡）。
    pub(crate) fn target_theme(&self) -> Theme {
        let s = self.settings.get();
        let accent = parse_color(&s.theme_color);
        match s.theme_mode {
            echo_core::ThemeMode::Light => Theme::light(accent),
            echo_core::ThemeMode::Dark => Theme::dark(accent),
            echo_core::ThemeMode::System => {
                // 第一期跟随系统：Windows 深色模式检测暂缺，默认浅色
                Theme::light(accent)
            }
        }
    }

    /// 当前配色（浅/深切换时在 `THEME_TRANSITION_SECS` 内平滑过渡）。
    pub fn theme(&self) -> Theme {
        // 开光灯揭示进行中：整棵 UI 树的颜色从旧主题插值到新主题。
        //
        // gpui 0.2.2 没有圆形裁剪（`ContentMask` 只有矩形，见 Windows 后端的
        // `distance_from_clip_rect`），也没有整页快照，所以拿不到 Web 上 View
        // Transitions 那种「圆内新 UI、圆外旧 UI」的双层。折中做法：**圆面扩散
        // 期间底色保持旧主题**，等圆面盖满窗口后，圆面淡出与底色收口同时进行
        // ——两件事反向交叉，视觉上就是「扫过的地方已经换好了」。
        //
        // 千万不要让底色全程跟着插值：那样圆内圆外底色一样，圆面看不出来，
        // 整窗还会长时间停在灰蒙蒙的中间色（实测过，观感最差的就是这个）。
        if let Some(r) = &self.theme_reveal {
            let t = (r.started.elapsed().as_secs_f32() / crate::theme::THEME_REVEAL_SECS)
                .min(1.0f32);
            let from = crate::theme::THEME_REVEAL_GROW_FRAC;
            let k = ((t - from) / (1.0 - from)).clamp(0.0, 1.0);
            let e = k * k * (3.0 - 2.0 * k); // smoothstep：快速但不硬切
            return Theme::blend(&r.hold, &self.target_theme(), e);
        }
        let target = self.target_theme();
        match &self.theme_anim {
            Some((from, started)) => {
                let t = started.elapsed().as_secs_f32() / crate::theme::THEME_TRANSITION_SECS;
                if t >= 1.0 {
                    target
                } else {
                    // ease-out：起步快、收尾稳
                    let e = 1.0 - (1.0 - t).powi(2);
                    Theme::blend(from, &target, e)
                }
            }
            None => target,
        }
    }

    /// 记录切换起点（在修改 theme_mode 之前调用）。
    pub fn begin_theme_anim(&mut self) {
        let from = self.theme();
        self.theme_anim = Some((from, std::time::Instant::now()));
    }

    /// 动画结束，清理状态。
    pub fn finish_theme_anim(&mut self) {
        self.theme_anim = None;
    }

    /// 系统材质（Mica / Acrylic / 磨砂降级）是否真的垫在主窗口后面。
    ///
    /// true 时主窗口的基础背景画成半透明，透明像素透出系统材质；
    /// false（关闭特效 / 系统不支持）保持不透明配色。
    pub fn backdrop_active(&self) -> bool {
        crate::windows_backdrop::material_visible(self.backdrop_effective)
    }

    /// 材质窗口的基础底色透明度。
    ///
    /// 0.45 能明显看出壁纸被磨砂透出来；`backdrop_dense`（「开始菜单风格」）
    /// 时拉到 0.80——Win11 开始菜单就是这个观感：底层确实是亚克力，但染色很浓，
    /// 只泛一点壁纸的色调，内容对比度也更高。
    pub fn backdrop_tint_alpha(&self) -> f32 {
        if self.settings.get().backdrop_dense {
            0.80
        } else {
            0.45
        }
    }

    /// 材质窗口上「面板类」表面（播放条、侧栏）的透明度。
    ///
    /// 面板本来就该比底色更实一点（分层），浓染色模式下再收一点，避免
    /// 与底色糊成一片、看不出层级。
    pub fn backdrop_panel_alpha(&self) -> f32 {
        if self.settings.get().backdrop_dense {
            0.92
        } else {
            0.82
        }
    }

    /// 是否存在任一列表加载态（驱动加载动画持续重绘）。
    pub fn any_loading(&self) -> bool {
        self.fm_loading
            || self.remote_liked_loading
            || self.lyric_loading
            // 切歌/缓冲时也要转：播放键上的加载圈按墙钟推进，需要持续重绘
            || self.player.loading
    }

    /// 把当前队列快照落盘（切歌 / 换队列 / 清空时调用）。
    pub fn persist_queue(&self) {
        self.queue_store
            .save(&self.player.queue_title, &self.player.queue, self.player.index);
    }

    /// 「我最喜爱」的完整数据源：登录用户用云歌单，未登录用本地收藏。
    pub fn favorites_source(&self) -> Arc<Vec<Song>> {
        if self.session.get().userid != 0 {
            self.remote_liked.clone()
        } else {
            self.library.get().favorites.clone()
        }
    }

}

/// 全局弱引用（跨组件回调访问 store）。
pub struct AppGlobal {
    #[allow(dead_code)]
    pub store: gpui::WeakEntity<AppStore>,
    /// 主窗口句柄：设置页切换背景特效时，从点击回调（只有 `&mut App`）
    /// 里拿它回到窗口线程改 DWM 属性。
    pub main_window: Option<gpui::AnyWindowHandle>,
}

impl Global for AppGlobal {}

/// 主题「开关灯」圆形揭示过渡（起点 = 切换开关位置）。
#[derive(Clone)]
pub struct ThemeReveal {
    /// 圆心（窗口逻辑坐标）
    pub origin: gpui::Point<gpui::Pixels>,
    /// 揭示期间保持的旧主题（结束时清空，新主题接管）
    pub hold: Theme,
    pub started: std::time::Instant,
}
