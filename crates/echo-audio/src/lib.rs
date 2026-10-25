//! EchoMusic 音频播放引擎。
//!
//! # 线程模型
//!
//! ```text
//!  UI / GPUI 主线程
//!    │  有界命令通道（SyncSender<Cmd>，满了丢弃并告警，UI 永不阻塞）
//!    ▼
//!  audio-control  ──▶  audio-download    整文件下载（reqwest::blocking）
//!    │                    │
//!    │                    └── Cmd::LoadDone（原始字节）
//!    ├──▶  audio-decode      Decoder → 声道/采样率归一 → 自适应预缓冲
//!    │            │
//!    │      SPSC PCM 环形缓冲（无锁，2 秒容量）
//!    │            │
//!    └──▶  【cpal 回调线程】只做 flush 判断 + copy + 补静音 + 计数
//!    │
//!    └── 事件通道（PlayerEvent，进度 4Hz / 诊断 1Hz）──▶ UI
//! ```
//!
//! 关键约束：
//! - **解码、重采样都不在音频回调里**，回调只碰环形缓冲和原子量；
//! - 回调路径无锁、无堆分配、无日志、无 IO；
//! - 切歌 / seek 时 `request_flush()` 让消费者把旧数据整段丢掉，不会串音；
//! - 对外 API 与事件与原实现保持一致（只多了 `Diagnostics`）。

mod decode;
mod diag;
pub mod output;
mod priority;
pub mod ring;

use cpal::traits::StreamTrait;
use parking_lot::Mutex;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use diag::DiagSnapshot;
pub use output::{default_output_device, output_devices, CallbackCtl};

/// 环形缓冲容量（秒）。够装下 1.5s 的最大预缓冲目标。
const RING_SECS: f32 = 2.0;
/// 空闲多久后暂停设备流（省 CPU，R4.3）。
const IDLE_STREAM_PAUSE: Duration = Duration::from_secs(10);
/// 定时线程周期（进度回报 / 结束检测 / 诊断）。
const TICK_INTERVAL: Duration = Duration::from_millis(250);
/// seek 之后的重新预滚深度：取 `min(目标深度, 150ms)`。
/// seek 会清空环形缓冲，如果立刻放行就必然欠载一次（听得见的「咔」），
/// 这里等一小段再继续；150ms 足够解码线程补上，用户也感觉不到停顿。
const SEEK_PREBUF_MS: u32 = 150;

/// 播放状态（对齐原 enginePlayback.status）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackState {
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
    Stopped,
    Error,
}

/// 播放器事件（对齐 preload `window.electron.player` 事件回调子集）。
#[derive(Debug, Clone, serde::Serialize)]
pub enum PlayerEvent {
    /// 状态变化
    StateChanged(PlaybackState),
    /// 当前位置（秒）
    Position(f64),
    /// 自然播放结束（仅当前请求）
    Ended,
    /// 错误（仅当前请求）
    Error(String),
    /// 轻量诊断（1Hz 限频，实时线程不参与格式化）
    Diagnostics(DiagSnapshot),
}

/// 内部命令（UI → control）。
enum Cmd {
    Load { urls: Vec<String>, request_id: u64 },
    LoadDone { bytes: Vec<u8>, request_id: u64 },
    LoadFailed { error: String, request_id: u64 },
    Play,
    Pause,
    Stop,
    Seek(f64),
    /// 运行中切换输出设备（P0：不再需要重启）。`None` = 回到系统默认。
    SwitchDevice(Option<String>),
    Tick,
}

/// 音频引擎句柄。
pub struct AudioEngine {
    /// 有界命令通道：满了就丢（UI 线程绝不阻塞）
    cmd_tx: SyncSender<Cmd>,
    request_seq: Mutex<u64>,
    /// 当前活动请求 id（竞态守卫）
    active_request: Arc<Mutex<u64>>,
    /// 音量 / 暂停等即时控制量（回调直接读原子，不排队）
    ctl: Arc<CallbackCtl>,
}

impl AudioEngine {
    /// 创建引擎：启动控制线程（持有 cpal 输出流）与定时线程。
    pub fn new(event_tx: Sender<PlayerEvent>) -> anyhow::Result<Self> {
        // 命令通道有界：突发命令最多积压 64 条
        let (cmd_tx, cmd_rx) = std::sync::mpsc::sync_channel::<Cmd>(64);
        let active_request: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
        let ctl = Arc::new(CallbackCtl::new(0.8));

        {
            let active = active_request.clone();
            let event_tx = event_tx.clone();
            let cmd_tx = cmd_tx.clone();
            let ctl = ctl.clone();
            std::thread::Builder::new()
                .name("audio-control".into())
                .spawn(move || {
                    let _ = control_loop(cmd_rx, cmd_tx, event_tx, active, ctl);
                })?;
        }

        // 定时线程：250ms 一次 tick（进度回报 / 结束检测 / 诊断）
        {
            let cmd_tx = cmd_tx.clone();
            let _ = std::thread::Builder::new()
                .name("audio-tick".into())
                .spawn(move || loop {
                    std::thread::sleep(TICK_INTERVAL);
                    if cmd_tx.try_send(Cmd::Tick).is_err() {
                        // 通道满（极端情况）继续跑，命令通道断开时再退出
                        continue;
                    }
                });
        }

        Ok(Self {
            cmd_tx,
            request_seq: Mutex::new(0),
            active_request,
            ctl,
        })
    }

    /// 加载音源 URL（默认自动播放），返回本次请求 id。
    pub fn load(&self, url: &str) -> u64 {
        self.load_multi(&[url.to_string()])
    }

    /// 加载音源（多候选 URL，逐个回退）。
    pub fn load_multi(&self, urls: &[String]) -> u64 {
        let mut seq = self.request_seq.lock();
        *seq += 1;
        let request_id = *seq;
        *self.active_request.lock() = request_id;
        self.send(Cmd::Load {
            urls: urls.to_vec(),
            request_id,
        });
        request_id
    }

    pub fn play(&self) {
        self.send(Cmd::Play);
    }

    pub fn pause(&self) {
        self.send(Cmd::Pause);
    }

    pub fn stop(&self) {
        self.send(Cmd::Stop);
    }

    /// 跳转到指定秒数。
    pub fn seek(&self, seconds: f64) {
        self.send(Cmd::Seek(seconds.max(0.0)));
    }

    /// 运行中切换输出设备（`None`/空 = 回到系统默认）。
    ///
    /// 切换会重建输出流；正在播放的曲目用内存里缓存的音频字节在新设备上
    /// 从当前位置继续（采样率不同的设备会重新解码归一）。
    pub fn switch_device(&self, name: Option<&str>) {
        self.send(Cmd::SwitchDevice(name.map(str::to_string)));
    }

    /// 设置音量 0.0 - 1.0（直接写原子，回调下一帧生效）。
    pub fn set_volume(&self, volume: f64) {
        self.ctl.set_volume(volume);
    }

    /// 播放倍速（0.5–2.0）。
    ///
    /// 实现方式是「按倍率啃环形缓冲 + 线性重采样回设备帧数」，
    /// 属于变速**变调**（后续可换 WSOLA 做变速不变调）。
    pub fn set_speed(&self, speed: f64) {
        self.ctl.set_speed(speed.clamp(0.5, 2.0) as f32);
    }

    pub fn speed(&self) -> f64 {
        self.ctl.speed() as f64
    }

    /// 当前实时频谱（16 段 0..1；第四期可视化）。
    pub fn spectrum(&self) -> [f32; 16] {
        self.ctl.spectrum()
    }

    /// 淡入（新曲目开始时调用，避免开头爆音）。
    pub fn fade_in(&self, secs: f32) {
        self.ctl.fade_to(1.0, secs);
    }

    /// 淡出（切歌/停止前调用）。返回后需要等待 `secs` 再真正 stop，
    /// 否则淡出还没走完就被切断。
    pub fn fade_out(&self, secs: f32) {
        self.ctl.fade_to(0.0, secs);
    }

    /// 立刻取消淡变（增益回到 1）。
    pub fn fade_reset(&self) {
        self.ctl.fade_now(1.0);
    }

    fn send(&self, cmd: Cmd) {
        if let Err(e) = self.cmd_tx.try_send(cmd) {
            log::warn!("audio cmd 投递失败（通道满？）: {e}");
        }
    }
}

/// 解码线程句柄。
struct DecoderHandle {
    shared: Arc<decode::DecodeShared>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl DecoderHandle {
    /// 取消并等待解码线程退出。
    fn stop(&mut self) {
        self.shared.cancel.store(true, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for DecoderHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 期望的设备缓冲帧数（可用环境变量覆盖）。
fn want_buffer_frames() -> u32 {
    std::env::var("ECHO_AUDIO_BUFFER_FRAMES")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|v| *v >= 256)
        .unwrap_or(2048)
}

/// 控制线程主循环。
fn control_loop(
    cmd_rx: Receiver<Cmd>,
    cmd_tx: SyncSender<Cmd>,
    event_tx: Sender<PlayerEvent>,
    active_request: Arc<Mutex<u64>>,
    ctl: Arc<CallbackCtl>,
) -> anyhow::Result<()> {
    let mut out = match output::open(ctl.clone(), want_buffer_frames(), RING_SECS) {
        Ok(o) => o,
        Err(e) => {
            log::error!("音频输出初始化失败: {e}");
            let _ = event_tx.send(PlayerEvent::Error(format!("音频输出初始化失败: {e}")));
            return Ok(());
        }
    };
    let mut ring = out.ring.clone();
    log::info!(
        "audio engine ready: {}Hz {}ch ring={} 样本",
        out.sample_rate,
        out.channels,
        ring.capacity()
    );

    let mut state = PlaybackState::Idle;
    let mut want_play = false;
    let mut decoder: Option<DecoderHandle> = None;
    // 当前曲目的音频字节缓存：设备切换后用它在新设备上原地续播
    let mut loaded_bytes: Option<Vec<u8>> = None;
    // 当前曲目已经播放的起点（seek 后会变化），进度 = base + played/rate
    let mut seek_base = 0.0f64;
    let mut target_ms = decode::TARGET_MIN_MS;
    // 起播/seek 前置预滚深度（毫秒）
    let mut prebuf_ms = target_ms;
    let mut last_underruns = 0u64;
    let mut last_decay = Instant::now();
    let mut last_diag = Instant::now();
    let mut idle_since: Option<Instant> = None;
    let mut stream_paused = false;
    let mut rt_reported = false;
    // ECHO_AUDIO_DIAG=1：把 1Hz 诊断打进日志（R5.4 可开关的诊断模式）
    let diag_log = std::env::var("ECHO_AUDIO_DIAG").is_ok();

    macro_rules! emit_state {
        ($s:expr) => {{
            state = $s;
            let _ = event_tx.send(PlayerEvent::StateChanged($s));
        }};
    }

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            Cmd::Load { urls, request_id } => {
                if let Some(mut d) = decoder.take() {
                    d.stop();
                }
                ring.clear();
                ctl.set_paused(true);
                ctl.draining.store(false, Ordering::Relaxed);
                seek_base = 0.0;
                want_play = true;
                ctl.diag.played_frames.store(0, Ordering::Relaxed);
                emit_state!(PlaybackState::Loading);

                let load_tx = cmd_tx.clone();
                let load_ctl = ctl.clone();
                let _ = std::thread::Builder::new()
                    .name("audio-download".into())
                    .spawn(move || {
                        let mut last_err = "no url".to_string();
                        let t0 = Instant::now();
                        for url in &urls {
                            match fetch_bytes(url) {
                                Ok(bytes) => {
                                    log::info!(
                                        "audio download OK: {} bytes / {}ms",
                                        bytes.len(),
                                        t0.elapsed().as_millis()
                                    );
                                    let _ = load_tx.send(Cmd::LoadDone { bytes, request_id });
                                    return;
                                }
                                Err(e) => {
                                    log::warn!("audio download 失败 {url}: {e}");
                                    last_err = e.to_string();
                                }
                            }
                        }
                        let _ = load_tx.send(Cmd::LoadFailed {
                            error: last_err,
                            request_id,
                        });
                        let _ = load_ctl;
                    });
            }

            Cmd::LoadDone { bytes, request_id } => {
                if *active_request.lock() != request_id {
                    continue; // 过期请求，丢弃
                }
                if let Some(mut d) = decoder.take() {
                    d.stop();
                }
                ring.clear();
                // 保持暂停，等预滚到目标深度再放行（避免开局就欠载）
                ctl.set_paused(true);
                ctl.draining.store(false, Ordering::Relaxed);
                ctl.diag.played_frames.store(0, Ordering::Relaxed);
                seek_base = 0.0;
                last_underruns = ctl.diag.underruns.load(Ordering::Relaxed);

                prebuf_ms = target_ms;
                let shared = Arc::new(decode::DecodeShared::new(
                    out.channels,
                    out.sample_rate,
                    target_ms,
                ));
                loaded_bytes = Some(bytes.clone());
                let join = decode::spawn(bytes, shared.clone(), ring.clone());
                decoder = Some(DecoderHandle {
                    shared,
                    join: Some(join),
                });
                emit_state!(PlaybackState::Loading);
            }

            Cmd::LoadFailed { error, request_id } => {
                if *active_request.lock() != request_id {
                    continue;
                }
                emit_state!(PlaybackState::Error);
                let _ = event_tx.send(PlayerEvent::Error(error));
            }

            Cmd::Play => {
                if decoder.is_none() {
                    continue;
                }
                want_play = true;
                // 预滚还没完成就继续等，由 tick 放行
                if state != PlaybackState::Loading {
                    ctl.set_paused(false);
                    emit_state!(PlaybackState::Playing);
                }
            }

            Cmd::Pause => {
                want_play = false;
                ctl.set_paused(true);
                if matches!(state, PlaybackState::Playing | PlaybackState::Loading) {
                    emit_state!(PlaybackState::Paused);
                }
            }

            Cmd::Stop => {
                want_play = false;
                if let Some(mut d) = decoder.take() {
                    d.stop();
                }
                loaded_bytes = None;
                ring.clear();
                ctl.set_paused(true);
                seek_base = 0.0;
                ctl.diag.played_frames.store(0, Ordering::Relaxed);
                emit_state!(PlaybackState::Stopped);
            }

            Cmd::Seek(seconds) => {
                let Some(d) = decoder.as_ref() else {
                    log::warn!("audio seek 忽略：没有加载曲目");
                    continue;
                };
                seek_base = seconds;
                ctl.diag.played_frames.store(0, Ordering::Relaxed);
                d.shared.request_seek(seconds);
                // seek 清了缓冲：先暂停消费、重新预滚，避免松手瞬间欠载
                prebuf_ms = target_ms.min(SEEK_PREBUF_MS);
                ctl.set_paused(true);
                ctl.draining.store(false, Ordering::Relaxed);
                emit_state!(PlaybackState::Loading);
                let _ = event_tx.send(PlayerEvent::Position(seconds));
            }

            Cmd::SwitchDevice(name) => {
                // 记录现场：位置 + 是否需要续播
                let played = ctl.diag.played_frames.load(Ordering::Relaxed);
                let pos = seek_base + played as f64 / out.sample_rate as f64;
                let resume = want_play && decoder.is_some();
                // 旧解码线程写的是旧环形缓冲，必须先停
                if let Some(mut d) = decoder.take() {
                    d.stop();
                }
                ctl.set_paused(true);
                // 先开新设备再丢旧的：开失败可以原样保留当前输出
                match output::open_named(
                    ctl.clone(),
                    want_buffer_frames(),
                    RING_SECS,
                    name.as_deref(),
                ) {
                    Ok(new_out) => {
                        log::info!(
                            "输出设备已切换: {}（{}Hz {}ch）",
                            name.as_deref().unwrap_or("系统默认"),
                            new_out.sample_rate,
                            new_out.channels
                        );
                        out = new_out;
                        ring = out.ring.clone();
                        idle_since = None;
                        stream_paused = false;
                        if let Some(bytes) = loaded_bytes.clone() {
                            // 新设备采样率可能不同：用缓存字节重建解码，从原位置继续
                            let shared = Arc::new(decode::DecodeShared::new(
                                out.channels,
                                out.sample_rate,
                                target_ms,
                            ));
                            let join = decode::spawn(bytes, shared.clone(), ring.clone());
                            decoder = Some(DecoderHandle {
                                shared,
                                join: Some(join),
                            });
                            seek_base = pos;
                            ctl.diag.played_frames.store(0, Ordering::Relaxed);
                            prebuf_ms = target_ms.min(SEEK_PREBUF_MS);
                            ctl.draining.store(false, Ordering::Relaxed);
                            if resume {
                                emit_state!(PlaybackState::Loading); // tick 预滚完成后自动放行
                            } else {
                                emit_state!(PlaybackState::Paused);
                            }
                        } else if resume {
                            // 没有可续播的字节（不该发生），退回暂停
                            want_play = false;
                            emit_state!(PlaybackState::Paused);
                        } else if matches!(state, PlaybackState::Stopped | PlaybackState::Idle) {
                            // 空闲切换：状态不变，只换设备
                        }
                    }
                    Err(e) => {
                        log::warn!("切换输出设备失败（保留当前设备）: {e}");
                        let _ = event_tx.send(PlayerEvent::Error(format!("切换输出设备失败: {e}")));
                        if resume && decoder.is_none() {
                            // 解码已停但新设备没开成：有缓存字节就在旧设备上重建
                            if let Some(bytes) = loaded_bytes.clone() {
                                let shared = Arc::new(decode::DecodeShared::new(
                                    out.channels,
                                    out.sample_rate,
                                    target_ms,
                                ));
                                let join = decode::spawn(bytes, shared.clone(), ring.clone());
                                decoder = Some(DecoderHandle {
                                    shared,
                                    join: Some(join),
                                });
                                seek_base = pos;
                                prebuf_ms = target_ms.min(SEEK_PREBUF_MS);
                                emit_state!(PlaybackState::Loading);
                            }
                        }
                    }
                }
            }

            Cmd::Tick => {
                // ---- 输出流故障（设备断开/失效）→ 自动暂停 ----
                if ctl.stream_failed.swap(false, Ordering::Relaxed) {
                    want_play = false;
                    ctl.set_paused(true);
                    if matches!(state, PlaybackState::Playing | PlaybackState::Loading) {
                        emit_state!(PlaybackState::Paused);
                    }
                    let _ = event_tx.send(PlayerEvent::Error(
                        "音频输出设备已断开或异常，播放已自动暂停".into(),
                    ));
                }

                // ---- 解码错误 ----
                if let Some(d) = decoder.as_ref() {
                    if let Some(err) = d.shared.error.lock().unwrap().take() {
                        emit_state!(PlaybackState::Error);
                        let _ = event_tx.send(PlayerEvent::Error(err));
                    }
                }

                // ---- 预滚完成 → 放行 ----
                if state == PlaybackState::Loading {
                    if let Some(d) = decoder.as_ref() {
                        let ready = ring.available() >= d.shared.samples_for_ms(prebuf_ms.max(1))
                            || d.shared.eof.load(Ordering::Acquire);
                        if ready {
                            if want_play {
                                ctl.set_paused(false);
                                emit_state!(PlaybackState::Playing);
                            } else {
                                emit_state!(PlaybackState::Paused);
                            }
                        }
                    }
                }

                // ---- 自然结束 ----
                if state == PlaybackState::Playing {
                    if let Some(d) = decoder.as_ref() {
                        if d.shared.eof.load(Ordering::Acquire) {
                            // 已到文件末尾：之后的「空」是正常放水，不算欠载
                            ctl.draining.store(true, Ordering::Relaxed);
                        }
                        if d.shared.eof.load(Ordering::Acquire) && ring.available() == 0 {
                            want_play = false;
                            ctl.set_paused(true);
                            emit_state!(PlaybackState::Stopped);
                            let _ = event_tx.send(PlayerEvent::Ended);
                        }
                    }
                }

                // ---- 进度 ----
                if state == PlaybackState::Playing {
                    let played = ctl.diag.played_frames.load(Ordering::Relaxed);
                    let pos = seek_base + played as f64 / out.sample_rate as f64;
                    let _ = event_tx.send(PlayerEvent::Position(pos));
                }

                // ---- 自适应预缓冲深度 ----
                let underruns = ctl.diag.underruns.load(Ordering::Relaxed);
                if underruns > last_underruns {
                    last_underruns = underruns;
                    last_decay = Instant::now();
                    if target_ms < decode::TARGET_MAX_MS {
                        target_ms = (target_ms + decode::TARGET_STEP_MS).min(decode::TARGET_MAX_MS);
                        if let Some(d) = decoder.as_ref() {
                            d.shared.target_ms.store(target_ms, Ordering::Relaxed);
                        }
                        log::warn!("jitter buffer 加深 → {target_ms}ms（检测到欠载）");
                    }
                } else if target_ms > decode::TARGET_MIN_MS
                    && last_decay.elapsed() >= Duration::from_secs(10)
                {
                    target_ms = (target_ms - decode::TARGET_DECAY_MS).max(decode::TARGET_MIN_MS);
                    if let Some(d) = decoder.as_ref() {
                        d.shared.target_ms.store(target_ms, Ordering::Relaxed);
                    }
                    last_decay = Instant::now();
                }

                // ---- 诊断（1Hz，限频；实时线程只写原子）----
                if last_diag.elapsed() >= Duration::from_secs(1) {
                    last_diag = Instant::now();
                    let mut snap = ctl.diag.snapshot();
                    snap.target_ms = target_ms;
                    // 实时优先级结果：在这里（非实时线程）提示一次
                    if ctl.rt_tried.load(Ordering::Relaxed) && !rt_reported {
                        rt_reported = true;
                        if snap.rt_ok {
                            log::info!("音频线程已提升为 MMCSS 实时优先级");
                        } else {
                            log::warn!(
                                "音频线程未能提升实时优先级（继续以普通优先级运行；\n                                 Linux 需要 rtprio/CAP_SYS_NICE，Windows 需要 MMPROFILE 未被策略禁用）"
                            );
                        }
                    }
                    if diag_log {
                        log::info!("audio diag: {}", snap.summary());
                    }
                    let _ = event_tx.send(PlayerEvent::Diagnostics(snap));
                }

                // ---- 空闲时暂停设备流（省 CPU）----
                if state == PlaybackState::Playing {
                    idle_since = None;
                    if stream_paused {
                        if let Err(e) = out.stream.play() {
                            log::warn!("恢复输出流失败: {e}");
                        }
                        stream_paused = false;
                    }
                } else {
                    let since = *idle_since.get_or_insert_with(Instant::now);
                    if !stream_paused && since.elapsed() >= IDLE_STREAM_PAUSE {
                        if let Err(e) = out.stream.pause() {
                            log::warn!("暂停输出流失败: {e}");
                        }
                        stream_paused = true;
                    }
                }
            }
        }
    }

    Ok(())
}

/// 同步拉取音源字节（独立线程中调用）。
fn fetch_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    log::info!("fetch_bytes: {url}");
    let resp = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(15))
        .build()?
        .get(url)
        .header("User-Agent", "EchoMusic/1.0")
        .send()?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status} while fetching audio");
    }
    let bytes = resp.bytes()?.to_vec();
    log::info!("fetch_bytes: OK {} bytes", bytes.len());
    Ok(bytes)
}
