//! cpal 输出层：设备打开、输出流构建、以及**实时安全**的数据回调。
//!
//! 回调里只做四件事：
//! 1. 处理 `flush`（切歌 / seek 时把旧数据整段丢掉）；
//! 2. 暂停时输出静音、且**不消费**环形缓冲；
//! 3. 从 SPSC 环里 `copy` 出 PCM，乘以原子音量；
//! 4. 环空了就补静音并给欠载计数 +1。
//!
//! 无 Mutex、无堆分配、无 `format!`、无文件/网络 IO、无阻塞等待。
//! 这里也是唯一提升线程实时优先级的入口（cpal 内部线程，我们拿不到句柄，
//! 只能在第一次回调时提升一次，结果存 `AtomicBool` 交给控制线程打日志）。

use crate::diag::Diag;
use crate::ring::PcmRing;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// 回调与控制线程共享的控制量（全部原子）。
pub struct CallbackCtl {
    /// 暂停：回调输出静音且不消费环形缓冲
    pub paused: AtomicBool,
    /// 音量（f32 的 bit 表示）
    pub volume: AtomicU32,
    /// 淡入淡出当前增益（0..1，f32 bit）
    pub fade: AtomicU32,
    /// 淡入淡出目标增益（f32 bit）
    pub fade_target: AtomicU32,
    /// 每帧向目标移动的步长（f32 bit）；0 表示瞬时
    pub fade_step: AtomicU32,
    /// 播放倍速（f32 bit，1.0 = 原速）
    pub speed: AtomicU32,
    /// 实时优先级是否已尝试过提升（只试一次）
    pub rt_tried: AtomicBool,
    /// 曲目已解码完、环形缓冲正在放水（此时的「空」是正常收尾，不算欠载）
    pub draining: AtomicBool,
    /// 输出流报错（设备断开 / 失效等）：由流错误回调置位，控制线程消费后自动暂停
    pub stream_failed: AtomicBool,
    /// 实时频谱：16 个对数频段的归一化能量（f32 bits，0..1，仅回调线程写）
    pub spec_pow: [AtomicU32; 16],
    /// Goertzel 滤波器组状态（s1/s2 交错，f32 bits；仅回调线程读写）
    spec_st: [AtomicU32; 32],
    /// 滤波器组已推进的样本数（半采样率计数）
    spec_n: AtomicU32,
    /// Goertzel 系数（2cos w，构造时按 24kHz 半采样率算好）
    spec_coeff: [f32; 16],
    pub diag: Diag,
}

/// 频谱频段中心频率（Hz，对数分布 45Hz–11kHz；按半采样率 24kHz 设计）。
const SPEC_FREQS: [f32; 16] = [
    45.0, 66.0, 96.0, 140.0, 205.0, 299.0, 437.0, 638.0, 932.0, 1361.0, 1987.0, 2901.0, 4235.0,
    6184.0, 9028.0, 11000.0,
];
/// 每段窗口长度（推进样本数，半采样率下 ≈ 21ms 窗）
const SPEC_WINDOW: u32 = 512;

impl CallbackCtl {
    pub fn new(volume: f64) -> Self {
        Self {
            paused: AtomicBool::new(true),
            volume: AtomicU32::new((volume.clamp(0.0, 1.0) as f32).to_bits()),
            fade: AtomicU32::new(1.0f32.to_bits()),
            fade_target: AtomicU32::new(1.0f32.to_bits()),
            fade_step: AtomicU32::new(0.0f32.to_bits()),
            speed: AtomicU32::new(1.0f32.to_bits()),
            rt_tried: AtomicBool::new(false),
            draining: AtomicBool::new(false),
            stream_failed: AtomicBool::new(false),
            spec_pow: std::array::from_fn(|_| AtomicU32::new(0)),
            spec_st: std::array::from_fn(|_| AtomicU32::new(0)),
            spec_n: AtomicU32::new(0),
            spec_coeff: std::array::from_fn(|i| {
                2.0 * (std::f32::consts::TAU * SPEC_FREQS[i] / 48_000.0).cos()
            }),
            diag: Diag::default(),
        }
    }

    pub fn set_volume(&self, v: f64) {
        self.volume
            .store((v.clamp(0.0, 1.0) as f32).to_bits(), Ordering::Relaxed);
    }

    /// 淡入/淡出：在 `secs` 秒内把增益线性移到 `target`。
    pub fn fade_to(&self, target: f32, secs: f32) {
        self.fade_target.store(target.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        // 步长按 48kHz 单声道估算即可，偏差只影响淡变时长
        let step = if secs <= 0.0 {
            1.0
        } else {
            1.0 / (secs * 48_000.0)
        };
        self.fade_step.store(step.to_bits(), Ordering::Relaxed);
    }

    /// 立刻结束淡变（直接到达目标）
    pub fn fade_now(&self, value: f32) {
        self.fade
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        self.fade_target
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn set_speed(&self, speed: f32) {
        self.speed
            .store(speed.clamp(0.5, 2.0).to_bits(), Ordering::Relaxed);
    }

    pub fn speed(&self) -> f32 {
        f32::from_bits(self.speed.load(Ordering::Relaxed))
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// 实时频谱：喂一个样本进 Goertzel 滤波器组（调用方按半采样率节流）。
    ///
    /// 每段窗口走满后取能量（对数刻度 0..1，带峰值衰减），供 UI 画柱。
    /// 全程只碰原子量、无分配，RT 安全。
    fn spec_feed(&self, v: f32) {
        let mut power = [0f32; 16];
        let mut done = false;
        for i in 0..16 {
            let s1 = f32::from_bits(self.spec_st[i * 2].load(Ordering::Relaxed));
            let s2 = f32::from_bits(self.spec_st[i * 2 + 1].load(Ordering::Relaxed));
            let s0 = v + self.spec_coeff[i] * s1 - s2;
            self.spec_st[i * 2].store(s1.to_bits(), Ordering::Relaxed);
            self.spec_st[i * 2 + 1].store(s0.to_bits(), Ordering::Relaxed);
            power[i] = s0 * s0 + s1 * s1 - self.spec_coeff[i] * s0 * s1;
        }
        let n = self.spec_n.fetch_add(1, Ordering::Relaxed) + 1;
        if n >= SPEC_WINDOW {
            self.spec_n.store(0, Ordering::Relaxed);
            done = true;
        }
        if done {
            for i in 0..16 {
                // 对数映射：功率 1e-9..1e-2 → 0..1；峰值缓降避免闪烁
                let lvl = ((power[i] + 1e-12).log10() + 9.0) / 7.0;
                let lvl = lvl.clamp(0.0, 1.0);
                let prev = f32::from_bits(self.spec_pow[i].load(Ordering::Relaxed));
                let next = lvl.max(prev * 0.78);
                self.spec_pow[i].store(next.to_bits(), Ordering::Relaxed);
                self.spec_st[i * 2].store(0, Ordering::Relaxed);
                self.spec_st[i * 2 + 1].store(0, Ordering::Relaxed);
            }
        }
    }

    /// 读取当前频谱（16 段 0..1）。
    pub fn spectrum(&self) -> [f32; 16] {
        std::array::from_fn(|i| {
            f32::from_bits(self.spec_pow[i].load(Ordering::Relaxed))
        })
    }
}

/// 已打开的输出设备（持有 `Stream` 保活，drop 即停）。
pub struct OutputDevice {
    pub stream: cpal::Stream,
    pub sample_rate: u32,
    pub channels: u16,
    /// 实际生效的设备缓冲帧数（Fixed 申请失败时会回落到设备默认值，这里记为 0 表示默认）
    pub buffer_frames: u32,
    /// 与回调共享的 PCM 环形缓冲（容量按设备采样率换算）
    pub ring: Arc<PcmRing>,
}

/// 打开默认输出设备并启动数据流。
///
/// 环形缓冲在这里创建：容量需要按设备采样率/声道数换算（默认装 2 秒）。
pub fn open(ctl: Arc<CallbackCtl>, want_frames: u32, ring_secs: f32) -> Result<OutputDevice, String> {
    open_named(ctl, want_frames, ring_secs, None)
}

/// 按设备名打开输出设备（`None` = 系统默认；找不到同名设备时回退默认）。
pub fn open_named(
    ctl: Arc<CallbackCtl>,
    want_frames: u32,
    ring_secs: f32,
    device_name: Option<&str>,
) -> Result<OutputDevice, String> {
    let host = cpal::default_host();
    let device = match device_name {
        Some(name) if !name.is_empty() => {
            let found = host
                .output_devices()
                .ok()
                .and_then(|mut ds| ds.find(|d| d.name().ok().as_deref() == Some(name)));
            match found {
                Some(d) => d,
                None => {
                    log::warn!("输出设备「{name}」不存在，回退系统默认");
                    host.default_output_device()
                        .ok_or_else(|| "找不到默认输出设备".to_string())?
                }
            }
        }
        _ => host
            .default_output_device()
            .ok_or_else(|| "找不到默认输出设备".to_string())?,
    };
    let supported = device
        .default_output_config()
        .map_err(|e| format!("读取默认输出配置失败: {e}"))?;
    let format = supported.sample_format();
    let mut cfg: cpal::StreamConfig = supported.config();
    let sample_rate = cfg.sample_rate.0;
    let channels = cfg.channels;

    let cap = (sample_rate as f32 * ring_secs * channels as f32).ceil() as usize;
    let ring = Arc::new(PcmRing::new(cap));

    // 先按固定缓冲申请（R4.1：2048~4096 帧起步），失败再回落到设备默认值
    let mut used_frames = 0u32;
    let mut stream = None;
    if want_frames > 0 {
        cfg.buffer_size = cpal::BufferSize::Fixed(want_frames);
        match build(&device, &cfg, format, ring.clone(), ctl.clone(), channels) {
            Ok(s) => {
                used_frames = want_frames;
                stream = Some(s);
            }
            Err(e) => {
                log::warn!("固定缓冲 {want_frames} 帧创建失败（{e}），回退设备默认缓冲");
            }
        }
    }
    let stream = match stream {
        Some(s) => s,
        None => {
            cfg.buffer_size = cpal::BufferSize::Default;
            build(&device, &cfg, format, ring.clone(), ctl.clone(), channels)
                .map_err(|e| format!("创建输出流失败（format={format:?}）: {e}"))?
        }
    };

    stream
        .play()
        .map_err(|e| format!("启动输出流失败: {e}"))?;

    log::info!(
        "audio output: rate={sample_rate} ch={channels} format={format:?} buffer={}",
        if used_frames > 0 {
            format!("{used_frames} 帧")
        } else {
            "设备默认".to_string()
        }
    );

    Ok(OutputDevice {
        stream,
        sample_rate,
        channels,
        buffer_frames: used_frames,
        ring,
    })
}

fn build(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    ring: Arc<PcmRing>,
    ctl: Arc<CallbackCtl>,
    channels: u16,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    // 错误回调：设备断开/失效等。立刻静音并置标志，控制线程消费后自动暂停
    // （P1「设备断开自动暂停」；不做自动重连，恢复由用户在设置里重选设备）
    let err_ctl = ctl.clone();
    let on_error = move |e: cpal::StreamError| {
        log::error!("audio stream error: {e}");
        err_ctl.stream_failed.store(true, Ordering::Relaxed);
        err_ctl.set_paused(true);
    };
    match format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            cfg,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| fill(data, &ring, &ctl, channels),
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            cfg,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| fill(data, &ring, &ctl, channels),
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_output_stream(
            cfg,
            move |data: &mut [u16], _: &cpal::OutputCallbackInfo| fill(data, &ring, &ctl, channels),
            on_error,
            None,
        ),
        other => {
            log::warn!("不支持的设备采样格式 {other:?}，尝试按 f32 打开");
            let err_ctl = ctl.clone();
            let on_error = move |e: cpal::StreamError| {
                log::error!("audio stream error: {e}");
                err_ctl.stream_failed.store(true, Ordering::Relaxed);
                err_ctl.set_paused(true);
            };
            device.build_output_stream(
                cfg,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    fill(data, &ring, &ctl, channels)
                },
                on_error,
                None,
            )
        }
    }
}

/// 音频回调主体。**实时安全**：无锁 / 无分配 / 无日志 / 无 IO / 不阻塞。
fn fill<T>(data: &mut [T], ring: &PcmRing, ctl: &CallbackCtl, channels: u16)
where
    T: SizedSample + FromSample<f32>,
{
    let started = Instant::now();
    ctl.diag.bump_callbacks();
    if channels > 0 {
        ctl.diag
            .cb_frames
            .store((data.len() / channels as usize) as u32, Ordering::Relaxed);
    }

    // 实时优先级：只在本线程第一次进来时提升，结果交给控制线程打日志
    if !ctl.rt_tried.swap(true, Ordering::Relaxed) {
        let ok = crate::priority::promote_current_thread().is_ok();
        ctl.diag.rt_ok.store(ok, Ordering::Relaxed);
    }

    if ring.apply_flush() {
        ctl.diag.flushes.fetch_add(1, Ordering::Relaxed);
    }

    // 暂停：只出静音，不动环形缓冲（否则一暂停就把已解码的数据啃光，恢复必爆音）
    if ctl.paused.load(Ordering::Relaxed) {
        for s in data.iter_mut() {
            *s = T::EQUILIBRIUM;
        }
        ctl.diag.note_callback_time(started.elapsed().as_micros() as u32);
        return;
    }

    let vol = f32::from_bits(ctl.volume.load(Ordering::Relaxed));
    let speed = ctl.speed();
    // 栈上临时缓冲，避免在回调里分配（倍速时需要更大的源窗口）
    let mut scratch = [0f32; 2048];
    let mut written = 0usize;
    let mut played_samples = 0usize;

    // 淡入淡出：取出当前值，本次回调内逐样本推进
    let mut fade = f32::from_bits(ctl.fade.load(Ordering::Relaxed));
    let fade_target = f32::from_bits(ctl.fade_target.load(Ordering::Relaxed));
    let fade_step = f32::from_bits(ctl.fade_step.load(Ordering::Relaxed));

    while written < data.len() {
        // 源窗口：倍速 > 1 时同样长的输出需要啃掉更多样本
        let want = (data.len() - written).min(1024);
        let need = (((want as f32) * speed).ceil() as usize).min(scratch.len());
        let got = ring.pop_into(&mut scratch[..need]);
        if got == 0 {
            for i in 0..want {
                data[written + i] = T::EQUILIBRIUM;
            }
            if !ctl.draining.load(Ordering::Relaxed) {
                ctl.diag.bump_underrun(want as u64);
            }
            break;
        }
        // 线性重采样：把 got 个源样本铺到 want 个输出样本上
        let ratio = got as f32 / want as f32;
        for i in 0..want {
            let p = i as f32 * ratio;
            let i0 = p as usize;
            let frac = p - i0 as f32;
            let a = scratch[i0.min(got - 1)];
            let b = scratch[(i0 + 1).min(got - 1)];
            let v = a + (b - a) * frac;
            // 频谱：每 2 个输出样本喂 1 个（半采样率足够 16 段可视化）
            if (written + i) & 1 == 0 {
                ctl.spec_feed(v);
            }
            // 淡变（逐样本推进，步长极小，听感上是平滑的）
            if fade != fade_target {
                if fade < fade_target {
                    fade = (fade + fade_step).min(fade_target);
                } else {
                    fade = (fade - fade_step).max(fade_target);
                }
            }
            data[written + i] = T::from_sample(v * vol * fade);
        }
        played_samples += got;
        if got < need {
            // 欠载：源不够，剩下的按静音补（环已空）
            for i in got..need {
                let _ = i;
            }
            if !ctl.draining.load(Ordering::Relaxed) {
                ctl.diag.bump_underrun((need - got) as u64);
            }
            break;
        }
        written += want;
    }
    ctl.fade.store(fade.to_bits(), Ordering::Relaxed);

    if channels > 0 {
        ctl.diag.add_played_frames((played_samples / channels as usize) as u64);
    }
    ctl.diag
        .note_callback_time(started.elapsed().as_micros() as u32);
}


/// 列出所有可用的音频输出设备名（第一个是系统默认）。
pub fn output_devices() -> Vec<String> {
    let host = cpal::default_host();
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|d| d.name().ok())
        .collect()
}

/// 当前默认输出设备的名字。
pub fn default_output_device() -> String {
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default()
}
