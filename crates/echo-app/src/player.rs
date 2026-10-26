//! 播放器状态：播放队列、当前曲目（对齐 stores/player 的第一期子集）。
//! 播放动作（解析 URL → 引擎加载）在 `ui::actions` 中实现，本模块只保存状态。
use echo_core::{PlayMode, Song};

/// 播放器运行状态。
#[derive(Debug, Default)]
pub struct PlayerState {
    /// 播放队列（当前上下文歌曲列表）
    pub queue: Vec<Song>,
    /// 队列标题（用于展示播放来源）
    pub queue_title: String,
    /// 当前曲目在队列中的下标
    pub index: Option<usize>,
    pub playing: bool,
    pub loading: bool,
    /// 引擎里是否已加载当前曲目。
    ///
    /// 重启 App 还原队列时引擎是空的，此时点播放必须先重新解析地址，
    /// 否则 `engine.play()` 打在一个空引擎上，什么都不会发生。
    pub loaded: bool,
    /// 当前位置（秒）
    pub position: f64,
    /// 时长（秒，取歌曲元数据）
    pub duration: f64,
    pub volume: f64,
    pub play_mode: PlayMode,
    /// 播放请求 id（竞态守卫：过期解析结果被丢弃）
    pub request_id: u64,
    /// 当前队列是不是私人 FM（播完要自动续推下一批）
    pub fm_queue: bool,
}

impl PlayerState {
    pub fn current(&self) -> Option<&Song> {
        self.index.and_then(|i| self.queue.get(i))
    }

    pub fn current_mut(&mut self) -> Option<&mut Song> {
        self.index.and_then(|i| self.queue.get_mut(i))
    }

    pub fn is_current(&self, song: &Song) -> bool {
        self.current().map(|c| c.key() == song.key()).unwrap_or(false)
    }

    /// 设置队列并从 start_index 开始播放。
    pub fn set_queue(&mut self, title: &str, songs: Vec<Song>, start: usize) {
        self.queue = songs;
        self.queue_title = title.to_string();
        self.index = if self.queue.is_empty() { None } else { Some(start.min(self.queue.len() - 1)) };
        self.loaded = false;
        // 换队列默认不是 FM；`play_from` 会在来源是 FM 时把它置回来
        self.fm_queue = false;
    }

    /// 清空队列并停止播放。
    pub fn clear_queue(&mut self) {
        self.queue.clear();
        self.queue_title.clear();
        self.index = None;
        self.playing = false;
        self.loading = false;
        self.loaded = false;
        self.position = 0.0;
        self.duration = 0.0;
        self.fm_queue = false;
    }

    /// 计算下一曲目下标（按播放模式），无下一首返回 None。
    pub fn next_index(&self) -> Option<usize> {
        let cur = self.index?;
        match self.play_mode {
            PlayMode::Single => Some(cur),
            PlayMode::Random => {
                if self.queue.len() <= 1 {
                    Some(cur)
                } else {
                    use rand::Rng;
                    let mut rng = rand::rng();
                    let mut next = cur;
                    while next == cur {
                        next = rng.random_range(0..self.queue.len());
                    }
                    Some(next)
                }
            }
            PlayMode::List => {
                if cur + 1 < self.queue.len() {
                    Some(cur + 1)
                } else {
                    Some(0)
                }
            }
            PlayMode::Sequential => {
                if cur + 1 < self.queue.len() {
                    Some(cur + 1)
                } else {
                    None
                }
            }
        }
    }

    /// 计算上一曲目下标。
    pub fn prev_index(&self) -> Option<usize> {
        let cur = self.index?;
        Some(if cur == 0 { self.queue.len().saturating_sub(1) } else { cur - 1 })
    }
}
