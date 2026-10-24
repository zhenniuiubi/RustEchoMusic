//! 轻量诊断计数。
//!
//! 全部是 `Atomic*`：音频回调里只做 `fetch_add` / `fetch_max` / `store`，
//! 无锁、无分配、无格式化。快照由控制线程按 1Hz 取走并通过事件回 UI
//! （实时线程里绝不碰 GPUI，也不写日志）。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// 实时线程写、控制线程读的计数器集合。
#[derive(Debug, Default)]
pub struct Diag {
    /// 回调被调用的总次数
    pub callbacks: AtomicU64,
    /// 送进设备缓冲的真实 PCM 帧数（不含补的静音）
    pub played_frames: AtomicU64,
    /// 欠载次数（一次回调中环里样本不够算一次）
    pub underruns: AtomicU64,
    /// 因欠载补出去的静音样本数
    pub silent_samples: AtomicU64,
    /// 回调耗时最大值（微秒）
    pub max_cb_us: AtomicU32,
    /// 清空（切歌 / seek）次数
    pub flushes: AtomicU64,
    /// 实时优先级是否提升成功
    pub rt_ok: AtomicBool,
    /// 最近一次回调拿到的帧数（用来核对设备实际周期）
    pub cb_frames: AtomicU32,
}

impl Diag {
    #[inline]
    pub fn bump_callbacks(&self) {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn add_played_frames(&self, frames: u64) {
        self.played_frames.fetch_add(frames, Ordering::Relaxed);
    }

    #[inline]
    pub fn bump_underrun(&self, silent_samples: u64) {
        self.underruns.fetch_add(1, Ordering::Relaxed);
        self.silent_samples
            .fetch_add(silent_samples, Ordering::Relaxed);
    }

    #[inline]
    pub fn note_callback_time(&self, us: u32) {
        self.max_cb_us.fetch_max(us, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> DiagSnapshot {
        DiagSnapshot {
            cb_frames: self.cb_frames.load(Ordering::Relaxed),
            target_ms: 0,
            callbacks: self.callbacks.load(Ordering::Relaxed),
            played_frames: self.played_frames.load(Ordering::Relaxed),
            underruns: self.underruns.load(Ordering::Relaxed),
            silent_samples: self.silent_samples.load(Ordering::Relaxed),
            max_cb_us: self.max_cb_us.load(Ordering::Relaxed),
            flushes: self.flushes.load(Ordering::Relaxed),
            rt_ok: self.rt_ok.load(Ordering::Relaxed),
        }
    }
}

/// 诊断快照（`PlayerEvent::Diagnostics` 的载荷，限频 1Hz）。
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct DiagSnapshot {
    /// 设备实际回调周期（帧）
    pub cb_frames: u32,
    /// 当前自适应预缓冲目标（毫秒），由控制线程填
    pub target_ms: u32,
    pub callbacks: u64,
    pub played_frames: u64,
    pub underruns: u64,
    pub silent_samples: u64,
    /// 单次回调最长耗时（微秒）—— 实时安全的核心指标之一
    pub max_cb_us: u32,
    pub flushes: u64,
    pub rt_ok: bool,
}

impl DiagSnapshot {
    /// 供日志/UI 用的一行摘要（**只在非实时线程调用**）。
    pub fn summary(&self) -> String {
        format!(
            "cb={} 回调帧={} 预缓冲={}ms 欠载={} 静音={}样本 最大回调={}us 清缓冲={} 实时优先级={}",
            self.callbacks,
            self.cb_frames,
            self.target_ms,
            self.underruns,
            self.silent_samples,
            self.max_cb_us,
            self.flushes,
            if self.rt_ok { "已启用" } else { "未启用" }
        )
    }
}
