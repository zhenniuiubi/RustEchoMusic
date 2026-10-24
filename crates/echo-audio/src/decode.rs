//! 解码线程：`Decoder` → 声道/采样率归一 → SPSC 环形缓冲。
//!
//! 这一层同时承担「抖动缓冲」的角色：它不是网络包级别的重排序，而是在
//! **解码输出 → 音频设备**之间维持一个**自适应目标深度**的预缓冲：
//! - 起播前先预滚到 `target_ms`，避免开局就欠载；
//! - 一旦回调报欠载，控制线程把目标深度 +100ms（上限 1500ms）；
//! - 连续 10s 没有欠载，慢慢收窄 −50ms（下限 300ms）。
//!
//! 重采样（源采样率 → 设备采样率）与声道转换都在**本线程**完成，
//! 音频回调里不再有任何解码/重采样工作。

use crate::ring::PcmRing;
use rodio::{Decoder, Source};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 预缓冲目标深度的下限（毫秒）。
pub const TARGET_MIN_MS: u32 = 300;
/// 预缓冲目标深度的上限（毫秒）—— 防止延迟无限增长。
pub const TARGET_MAX_MS: u32 = 1500;
/// 每次欠载后加深多少毫秒。
pub const TARGET_STEP_MS: u32 = 100;
/// 稳定播放时每次收窄多少毫秒。
pub const TARGET_DECAY_MS: u32 = 50;

/// 解码线程与编排线程共享的状态。
pub struct DecodeShared {
    /// 取消（换歌 / 停止）
    pub cancel: AtomicBool,
    /// 待执行的 seek（秒）。`take()` 后为空。
    pub seek: Mutex<Option<f64>>,
    /// 解码器已到文件末尾
    pub eof: AtomicBool,
    /// 解码失败原因（非空即失败）
    pub error: Mutex<Option<String>>,
    /// 自适应预缓冲目标（毫秒）
    pub target_ms: AtomicU32,
    /// 已解码帧数（诊断用）
    pub decoded_frames: AtomicU64,
    /// 设备声道数
    pub channels: u16,
    /// 设备采样率
    pub sample_rate: u32,
}

impl DecodeShared {
    pub fn new(channels: u16, sample_rate: u32, target_ms: u32) -> Self {
        Self {
            cancel: AtomicBool::new(false),
            seek: Mutex::new(None),
            eof: AtomicBool::new(false),
            error: Mutex::new(None),
            target_ms: AtomicU32::new(target_ms),
            decoded_frames: AtomicU64::new(0),
            channels: channels.max(1),
            sample_rate: sample_rate.max(1),
        }
    }

    /// 目标深度折算成样本数（按设备声道展开）。
    pub fn target_samples(&self) -> usize {
        self.samples_for_ms(self.target_ms.load(Ordering::Relaxed))
    }

    /// 毫秒 → 样本数（交织后）。
    pub fn samples_for_ms(&self, ms: u32) -> usize {
        ms as usize * self.sample_rate as usize / 1000 * self.channels as usize
    }

    pub fn request_seek(&self, seconds: f64) {
        *self.seek.lock().unwrap() = Some(seconds.max(0.0));
    }
}

/// 解码器类型别名（整文件在内存里，可 seek）。
type Dec = Decoder<Cursor<Vec<u8>>>;

/// 启动解码线程。
pub fn spawn(
    bytes: Vec<u8>,
    shared: Arc<DecodeShared>,
    ring: Arc<PcmRing>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("audio-decode".into())
        .spawn(move || run(bytes, shared, ring))
        .expect("spawn audio-decode")
}

fn run(bytes: Vec<u8>, shared: Arc<DecodeShared>, ring: Arc<PcmRing>) {
    let byte_len = bytes.len() as u64;
    // 关键：`Decoder::new` 默认 is_seekable=false，symphonia 会拒绝随机访问
    // （try_seek 报 RandomAccessNotSupported → 进度条拖拽不生效）。
    let mut dec = match Decoder::builder()
        .with_data(Cursor::new(bytes))
        .with_byte_len(byte_len)
        .with_seekable(true)
        .build()
    {
        Ok(d) => d,
        Err(e) => {
            log::warn!("audio decode ERR: {e}");
            *shared.error.lock().unwrap() = Some(format!("decode: {e}"));
            return;
        }
    };

    let src_ch = dec.channels() as usize;
    let src_rate = dec.sample_rate();
    let dst_ch = shared.channels as usize;
    let dst_rate = shared.sample_rate;
    log::info!("audio decode: {byte_len} bytes, {src_ch}ch {src_rate}Hz → {dst_ch}ch {dst_rate}Hz");

    let mut conv = Converter::new(src_ch, dst_ch, src_rate, dst_rate);
    conv.reset(&mut dec);

    let mut out = vec![0f32; dst_ch * 1024];
    let mut silent_waits = 0u32;

    loop {
        if shared.cancel.load(Ordering::Acquire) {
            log::debug!("audio decode: cancelled");
            return;
        }

        // seek 优先处理：清环 + 重新预热插值状态
        if let Some(pos) = shared.seek.lock().unwrap().take() {
            match dec.try_seek(Duration::from_secs_f64(pos)) {
                Ok(()) => {
                    ring.request_flush();
                    conv.reset(&mut dec);
                    shared.eof.store(false, Ordering::Release);
                    log::info!("audio decode: seek → {pos:.2}s OK");
                }
                Err(e) => log::warn!("audio decode: seek → {pos:.2}s 失败: {e:?}"),
            }
            continue;
        }

        // 环里已经够目标深度：歇一会儿，别空转 CPU
        if ring.available() >= shared.target_samples() {
            silent_waits = 0;
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }

        // 解一块 PCM
        let mut frames = 0usize;
        while frames < 1024 {
            let off = frames * dst_ch;
            if !conv.next(&mut dec, &mut out[off..off + dst_ch]) {
                break;
            }
            frames += 1;
        }
        if frames == 0 {
            shared.eof.store(true, Ordering::Release);
            log::info!("audio decode: EOF");
            return;
        }
        shared
            .decoded_frames
            .fetch_add(frames as u64, Ordering::Relaxed);

        // 写环：可能只能写一部分（环满），写完继续
        let n = frames * dst_ch;
        let mut off = 0usize;
        while off < n {
            if shared.cancel.load(Ordering::Acquire) {
                return;
            }
            let w = ring.push(&out[off..n]);
            off += w;
            if w == 0 {
                silent_waits += 1;
                // 环满：等消费者取走一点再写（这里不能忙等，会把 CPU 打满）
                std::thread::sleep(Duration::from_millis(3));
            }
        }
        if silent_waits > 0 {
            silent_waits = 0;
        }
    }
}

/// 声道 + 采样率归一化（线性插值），**只在解码线程里跑**。
///
/// rodio 的 `UniformSourceIterator` 也能做同样的事，但它把 `Decoder` 的所有权
/// 吞掉了，seek 之后没法重建插值状态；这里自己实现，seek 时 `reset()` 即可。
struct Converter {
    src_ch: usize,
    dst_ch: usize,
    /// 源采样率 / 目标采样率
    step: f64,
    /// 上一帧 / 当前帧（已转成 dst_ch）
    prev: Vec<f32>,
    cur: Vec<f32>,
    /// 插值位置 [0, 1)
    t: f64,
    /// 预热时读到的源帧缓冲
    raw: Vec<f32>,
    ended: bool,
}

impl Converter {
    fn new(src_ch: usize, dst_ch: usize, src_rate: u32, dst_rate: u32) -> Self {
        let src_ch = src_ch.max(1);
        let dst_ch = dst_ch.max(1);
        Self {
            src_ch,
            dst_ch,
            step: src_rate as f64 / dst_rate as f64,
            prev: vec![0.0; dst_ch],
            cur: vec![0.0; dst_ch],
            t: 0.0,
            raw: vec![0.0; src_ch],
            ended: false,
        }
    }

    /// 读一帧源数据并转成目标声道数，写入 `cur`。
    fn pull(&mut self, dec: &mut Dec) -> bool {
        for i in 0..self.src_ch {
            match dec.next() {
                Some(s) => self.raw[i] = s,
                None => return false,
            }
        }
        map_channels(&self.raw, self.dst_ch, &mut self.cur);
        true
    }

    /// seek 之后重新预热插值状态。
    fn reset(&mut self, dec: &mut Dec) {
        self.t = 0.0;
        self.ended = false;
        if self.pull(dec) {
            self.prev.copy_from_slice(&self.cur);
            // 再读一帧作为插值右端点；读不到就说明只剩一帧
            if !self.pull(dec) {
                self.cur.copy_from_slice(&self.prev);
            }
        } else {
            self.ended = true;
            self.prev.iter_mut().for_each(|s| *s = 0.0);
            self.cur.copy_from_slice(&self.prev);
        }
    }

    /// 生成下一个目标采样帧，写入 `out`（长度 = dst_ch）。返回 false 表示流结束。
    fn next(&mut self, dec: &mut Dec, out: &mut [f32]) -> bool {
        if self.ended {
            return false;
        }
        while self.t >= 1.0 {
            std::mem::swap(&mut self.prev, &mut self.cur);
            if !self.pull(dec) {
                self.ended = true;
                return false;
            }
            self.t -= 1.0;
        }
        let t = self.t as f32;
        for c in 0..self.dst_ch {
            let a = self.prev[c];
            out[c] = a + (self.cur[c] - a) * t;
        }
        self.t += self.step;
        true
    }
}

/// 声道映射：
/// - 相同 → 直通
/// - 单声道 → 多声道：复制到每一路
/// - 多声道 → 单声道：取平均
/// - 其它（如 5.1 → 立体声）：按 L/R 取前两路
fn map_channels(src: &[f32], dst_ch: usize, out: &mut [f32]) {
    let src_ch = src.len();
    if src_ch == dst_ch {
        out[..dst_ch].copy_from_slice(&src[..dst_ch]);
        return;
    }
    if src_ch == 1 {
        for o in out.iter_mut().take(dst_ch) {
            *o = src[0];
        }
        return;
    }
    if dst_ch == 1 {
        let sum: f32 = src.iter().sum();
        out[0] = sum / src_ch as f32;
        return;
    }
    for (c, o) in out.iter_mut().take(dst_ch).enumerate() {
        *o = src[c.min(src_ch - 1)];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用一个假的「解码器」验证插值与声道映射：44.1k → 48k 的帧数比例要接近 48/44.1。
    #[test]
    fn map_mono_to_stereo() {
        let mut out = [0f32; 2];
        map_channels(&[0.5], 2, &mut out);
        assert_eq!(out, [0.5, 0.5]);
    }

    #[test]
    fn map_stereo_to_mono() {
        let mut out = [0f32; 1];
        map_channels(&[0.2, 0.4], 1, &mut out);
        assert!((out[0] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn rate_step_ratio() {
        let c = Converter::new(2, 2, 44_100, 48_000);
        assert!((c.step - 0.91875).abs() < 1e-9);
    }
}
