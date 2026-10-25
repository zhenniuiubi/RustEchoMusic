//! 应用全局状态：路由栈、页面数据、设置、API 客户端。
use crate::player::PlayerState;
use crate::theme::{parse_color, Theme};
use echo_api::KuGouClient;
use echo_audio::AudioEngine;
use echo_core::{Album, Lyric, MvInfo, Playlist, SettingsStore, Song};
use gpui::Global;
use std::sync::Arc;

/// 页面（对齐 hash 路由）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    Home,
    Explore,
    Ranking,
    DailyRecommend,
    Favorites,
    /// 私人 FM（按听歌口味推的电台）
    Fm,
    /// 歌词页（当前播放曲目的歌词）
    Lyric,
    Cloud,
    History,
    Purchased,
    Profile,
    Settings,
    Login,
    /// 搜索页（当前关键词）
    Search(String),
    /// 歌单详情（global_collection_id, 名称）
    Playlist(String, String),
    /// 歌手详情（author_id, 名称）
    Artist(String, String),
    /// 专辑详情（album_id, 名称）
    Album(String, String),
    /// 榜单详情（rankid, 名称）
    Rank(String, String),
    /// 占位页（后续期实现的功能入口）
    Placeholder(&'static str),
}

impl Page {
    /// 该页面是否必须登录（未登录时自动改道登录页）。
    ///
    /// 「我最喜爱」不在此列：未登录时展示本地收藏；播放历史也是本地的。
    /// 云盘 / 已购音乐 / 个人中心 完全依赖登录态。
    pub fn requires_login(&self) -> bool {
        matches!(self, Page::Cloud | Page::Purchased | Page::Profile)
    }

    pub fn title(&self) -> String {
        match self {
            Page::Home => "首页".into(),
            Page::Explore => "探索发现".into(),
            Page::Ranking => "排行榜".into(),
            Page::DailyRecommend => "每日推荐".into(),
            Page::Favorites => "我最喜爱".into(),
            Page::Fm => "私人FM".into(),
            Page::Lyric => "歌词".into(),
            Page::Cloud => "音乐云盘".into(),
            Page::History => "播放历史".into(),
            Page::Purchased => "已购音乐".into(),
            Page::Profile => "个人中心".into(),
            Page::Settings => "设置".into(),
            Page::Login => "登录".into(),
            Page::Search(kw) => format!("搜索：{kw}"),
            Page::Playlist(_, name) => format!("歌单：{name}"),
            Page::Artist(_, name) => format!("歌手：{name}"),
            Page::Album(_, name) => format!("专辑：{name}"),
            Page::Rank(_, name) => format!("榜单：{name}"),
            Page::Placeholder(name) => (*name).to_string(),
        }
    }
}

/// 路由栈（前进/后退）。
#[derive(Debug, Default)]
pub struct RouteState {
    pub stack: Vec<Page>,
    pub index: usize,
}

impl RouteState {
    pub fn current(&self) -> Option<&Page> {
        self.stack.get(self.index)
    }

    pub fn navigate(&mut self, page: Page) {
        // 相同页面不重复压栈
        if self.current() == Some(&page) {
            return;
        }
        self.stack.truncate(self.index + 1);
        self.stack.push(page);
        self.index = self.stack.len() - 1;
    }

    pub fn back(&mut self) -> bool {
        if self.index > 0 {
            self.index -= 1;
            return true;
        }
        false
    }

    pub fn forward(&mut self) -> bool {
        if self.index + 1 < self.stack.len() {
            self.index += 1;
            return true;
        }
        false
    }

    pub fn can_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.index + 1 < self.stack.len()
    }
}

/// 首页数据（对齐 views/Home.vue）。
#[derive(Debug, Default, Clone)]
pub struct HomeData {
    pub loaded: bool,
    /// 风格推荐：分类（含标签）
    pub style_categories: Vec<echo_api::StyleCategory>,
    /// 风格推荐：歌曲
    pub style_songs: Arc<Vec<Song>>,
    /// 当前选中的分类下标
    pub active_category: usize,
    /// 已选标签 id
    pub selected_tags: Vec<String>,
    /// 风格加载状态
    pub style_loading: bool,
    /// 推荐歌单
    pub recommend_playlists: Vec<Playlist>,
    /// 编辑精选
    pub editor_playlists: Vec<Playlist>,
}

impl HomeData {
    /// 当前分类的标签列表。
    pub fn active_tags(&self) -> &[echo_api::StyleTag] {
        self.style_categories
            .get(self.active_category)
            .map(|c| c.tags.as_slice())
            .unwrap_or(&[])
    }
}

/// 歌单详情数据（Playlist + 歌曲列表）。
pub type PlaylistDetail = (echo_core::Playlist, Arc<Vec<Song>>);

/// 搜索数据。
#[derive(Debug, Default, Clone)]
pub struct SearchData {
    pub keyword: String,
    pub loaded: bool,
    /// 单曲搜索进行中（用于加载动画）
    pub loading: bool,
    pub songs: Arc<Vec<Song>>,
    /// 0 单曲 / 1 歌单 / 2 歌手 / 3 专辑 / 4 歌词 / 5 MV
    pub tab: usize,
    pub playlists: Vec<Playlist>,
    pub artists: Vec<echo_core::Artist>,
    pub albums: Vec<echo_core::Album>,
    /// MV 搜索结果（第三期搜索补全）
    pub mvs: Vec<echo_core::MvMeta>,
    /// 歌词搜索候选（第三期搜索补全）
    pub lyrics: Vec<echo_api::lyrics::LyricCandidate>,
    /// 已加载过的分类 tab（避免重复请求）
    pub loaded_tabs: Vec<usize>,
}

/// 探索页数据（5 Tab）。
#[derive(Debug, Default, Clone)]
pub struct ExploreData {
    pub loaded: bool,
    /// 0=歌单 1=排行榜 2=新歌 3=新碟
    pub tab: usize,
    pub loading: bool,
    pub tags: Vec<(String, Vec<(String, String)>)>,
    pub active_tag: Option<String>,
    pub playlists: Vec<Playlist>,
    pub ranks: Vec<echo_core::RankMeta>,
    pub new_songs: Arc<Vec<Song>>,
    pub new_albums: Vec<echo_core::Album>,
}

/// 登录方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoginMethod {
    /// 扫码登录（默认）
    #[default]
    Qr,
    /// 手机验证码
    Sms,
    /// 账号密码
    Password,
}

/// 登录表单状态。
#[derive(Debug, Clone)]
pub struct LoginForm {
    pub method: LoginMethod,
    pub username: String,
    pub password: String,
    pub busy: bool,
    pub error: String,
    /// 二维码 key / 内容 URL
    pub qr_key: String,
    pub qr_url: String,
    /// 0 过期 / 1 等待扫码 / 2 待确认 / 4 成功
    pub qr_status: i32,
    pub qr_loading: bool,
    /// 轮询代次（切换 tab / 刷新时自增，用于终止旧轮询）
    pub qr_gen: u64,
    /// 验证码表单
    pub mobile: String,
    pub code: String,
    pub sms_busy: bool,
    /// 重发倒计时（秒）
    pub sms_countdown: u32,
}

impl Default for LoginForm {
    fn default() -> Self {
        Self {
            method: LoginMethod::Qr,
            username: String::new(),
            password: String::new(),
            busy: false,
            error: String::new(),
            qr_key: String::new(),
            qr_url: String::new(),
            qr_status: 1,
            qr_loading: false,
            qr_gen: 0,
            mobile: String::new(),
            code: String::new(),
            sms_busy: false,
            sms_countdown: 0,
        }
    }
}

/// 歌手详情数据。
#[derive(Debug, Clone, Default)]
pub struct ArtistView {
    pub artist: echo_core::Artist,
    pub songs: Arc<Vec<Song>>,
    pub albums: Vec<echo_core::Album>,
}

/// 专辑详情数据。
#[derive(Debug, Clone, Default)]
pub struct AlbumView {
    pub album: echo_core::Album,
    pub songs: Arc<Vec<Song>>,
}

/// 榜单详情数据。
#[derive(Debug, Clone, Default)]
pub struct RankView {
    pub rank_id: String,
    pub ranks: Vec<echo_core::RankMeta>,
    pub songs: Arc<Vec<Song>>,
}

/// 应用状态（单 Entity，所有 UI 读取它）。
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
    pub routes: RouteState,
    pub home: HomeData,
    pub search: SearchData,
    /// 歌单详情页数据
    pub playlist_detail: Option<PlaylistDetail>,
    /// 已加载过的歌单 id
    pub playlists_loaded: std::collections::HashSet<String>,
    /// 登录用户的自建歌单（侧栏「自建歌单」）
    pub user_playlists: Vec<Playlist>,
    pub user_playlists_loaded: bool,
    pub search_hot: Vec<String>,
    /// 登录会话
    pub session: echo_core::SessionStore,
    /// 当前登录用户
    pub user: Option<echo_core::User>,
    pub login: LoginForm,
    /// 探索页
    pub explore: ExploreData,
    /// 每日推荐
    pub daily: Arc<Vec<Song>>,
    pub daily_loaded: bool,
    pub daily_loading: bool,
    /// 歌手 / 专辑 / 榜单详情
    pub artist_view: Option<ArtistView>,
    pub album_view: Option<AlbumView>,
    pub rank_view: RankView,
    pub artist_loading: bool,
    pub album_loading: bool,
    pub rank_loading: bool,
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
    /// 播放条「更多」菜单是否展开
    pub player_menu: bool,
    /// 睡眠定时器的截止时刻（unix 秒），None = 未设置
    pub sleep_until: Option<i64>,
    /// 当前曲目的 MV
    pub mv: Option<MvInfo>,
    pub mv_loading: bool,
    /// 私人 FM 当前这一批歌曲
    pub fm: Arc<Vec<Song>>,
    /// 私人 FM 频道：normal 红心 / small 小众 / peak 速览 / radio 电台
    pub fm_mode: String,
    /// 私人 FM 推荐来源：0 口味 / 1 风格 / 2 探索
    pub fm_pool: i64,
    pub fm_loaded: bool,
    pub fm_loading: bool,
    /// 音乐云盘歌曲（登录后拉取）
    pub cloud: Arc<Vec<Song>>,
    pub cloud_loaded: bool,
    pub cloud_loading: bool,
    /// 已购音乐：单曲 + 专辑
    pub purchased_songs: Arc<Vec<Song>>,
    pub purchased_albums: Arc<Vec<Album>>,
    pub purchased_loaded: bool,
    pub purchased_loading: bool,
    /// 已购页 tab：0 单曲 / 1 专辑
    pub purchased_tab: usize,
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
    /// 播放队列面板是否展开
    pub queue_open: bool,
    /// 音量调节面板是否展开
    pub volume_open: bool,
    /// 播放进度条是否正在拖拽
    pub seeking: bool,
    /// 拖拽目标位置（秒）
    pub seek_target: f64,
    /// 顶栏搜索框文本
    pub search_input: String,
    /// 全局提示（toast 文本，空则不显示）
    pub toast: String,
    /// 最近一次 Position 事件的时间（歌词扫光按耗时外推，30fps 平滑推进）
    pub position_at: std::time::Instant,
    /// 播放条进度条 bounds（canvas 记录，点击 seek 用）
    pub progress_bounds_slot: std::sync::Arc<parking_lot::Mutex<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// 音量滑条 bounds（canvas 记录）
    pub volume_bounds_slot: std::sync::Arc<parking_lot::Mutex<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// 播放队列滚动句柄（uniform_list，行高固定，滚动条可拖拽）
    pub queue_scroll: gpui::UniformListScrollHandle,
    /// 队列滚动条拖拽中：(按下时鼠标 y, 按下时的滚动像素偏移)
    pub queue_drag: Option<(f32, f32)>,
    /// 主内容区歌曲列表的滚动句柄。
    ///
    /// 用 `uniform_list`（行高固定 52px）而非 `gpui::list`：
    /// 后者只测量可见项，`max_offset_for_scrollbar` 不完整，滚动条既算不准也没法拖；
    /// 前者的内容高度 = 行数 × 行高，滚动手柄的 `max_offset` 是精确值，
    /// 因此滚动条可以做到 1:1 跟手。
    pub song_scroll: gpui::UniformListScrollHandle,
    /// 列表滚动条拖拽中：(按下时鼠标 y, 按下时的滚动像素偏移)
    pub list_drag: Option<(f32, f32)>,
    /// 播放队列面板的区域（用于点击外部自动关闭）
    pub queue_bounds: Arc<parking_lot::Mutex<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// 顶栏搜索框区域（用于点击外部时让输入框失焦）
    pub search_bounds: Arc<parking_lot::Mutex<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// 「我最喜爱」页搜索框区域（同上）
    pub fav_search_bounds: Arc<parking_lot::Mutex<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// 「我最喜爱」前端模糊搜索关键词
    pub fav_filter: String,
    /// 模糊搜索结果（关键词非空时的展示/播放基准，避免每帧重算）
    pub fav_filtered: Arc<Vec<Song>>,
    pub sidebar_collapsed: bool,
    /// 侧栏「发现音乐」分组展开
    pub sidebar_found_open: bool,
    /// 侧栏「我的乐库」分组展开
    pub sidebar_lib_open: bool,
    /// 侧栏歌单 Tab：0 自建歌单 / 1 收藏歌单
    pub sidebar_tab: usize,
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
    /// 未登录时点开的「需要登录」页面，登录成功后跳回去
    pub login_redirect: Option<Page>,
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
        // 开光灯揭示进行中：保持旧主题（圆盖满后由 finish 切换）
        if let Some(r) = &self.theme_reveal {
            return r.hold.clone();
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

    /// 是否存在任一列表加载态（驱动加载动画持续重绘）。
    pub fn any_loading(&self) -> bool {
        self.fm_loading
            || self.cloud_loading
            || self.purchased_loading
            || self.explore.loading
            || self.daily_loading
            || self.remote_liked_loading
            || self.artist_loading
            || self.album_loading
            || self.rank_loading
            || self.search.loading
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

    /// 按当前关键词重算「我最喜爱」的模糊搜索结果。
    ///
    /// 只在关键词变化 / 数据源变化时调用，渲染与播放都读 `fav_filtered`，
    /// 避免每帧对上千首歌做一遍匹配。
    pub fn refresh_fav_filter(&mut self) {
        let kw = self.fav_filter.trim().to_lowercase();
        if kw.is_empty() {
            self.fav_filtered = Arc::new(Vec::new());
            return;
        }
        let source = self.favorites_source();
        let hits: Vec<Song> = source
            .iter()
            .filter(|s| {
                // 歌名/歌手允许跳字（更「模糊」），专辑只做子串匹配：
                // 专辑名往往很长，子序列匹配会把一堆不相干的歌捞进来。
                crate::ui::fuzzy_match(&s.title, &kw)
                    || crate::ui::fuzzy_match(&s.artist, &kw)
                    || s.album.to_lowercase().contains(&kw)
            })
            .cloned()
            .collect();
        self.fav_filtered = Arc::new(hits);
    }

    /// 「我最喜爱」页面当前应该展示的列表（有搜索词时用过滤结果）。
    pub fn favorites_view(&self) -> Arc<Vec<Song>> {
        if self.fav_filter.trim().is_empty() {
            self.favorites_source()
        } else {
            self.fav_filtered.clone()
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
