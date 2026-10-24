//! 无锁 SPSC（单生产 / 单消费）PCM 环形缓冲。
//!
//! 生产端是解码线程，消费端是音频回调线程。
//!
//! 设计约束（音频回调路径必须实时安全）：
//! - 定长 `f32` 交织样本，**构造时一次性分配**，之后读写都不再分配；
//! - 只有 head/tail 两个 `AtomicUsize`，纯 load/store、无 CAS、无锁；
//! - 索引单调递增（不取模），用 `wrapping_sub` 算占用量，避免「满/空」二义性；
//! - `flush` 给消费者一个「把已排队的样本整段丢掉」的开关：切歌 / seek 时
//!   旧音频绝不能串进新曲目，而清空动作交给消费者做，才不会破坏 SPSC 语义。

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// 单生产者 / 单消费者的 PCM 环形缓冲。
pub struct PcmRing {
    data: Box<[UnsafeCell<f32>]>,
    /// 容量 - 1，容量取 2 的幂，用位与代替取模
    mask: usize,
    /// 生产者写入位置（单调递增）
    write: AtomicUsize,
    /// 消费者读取位置（单调递增）
    read: AtomicUsize,
    /// 请求消费者清空
    flush: AtomicBool,
}

// 安全性：data 的并发访问由 head/tail 的 Acquire/Release 配对保护，
// 且生产者只写 [write, write+n)，消费者只读 [read, write)，区间不重叠。
unsafe impl Send for PcmRing {}
unsafe impl Sync for PcmRing {}

impl PcmRing {
    /// `min_cap` 是期望的最少样本数（会向上取到 2 的幂）。
    pub fn new(min_cap: usize) -> Self {
        let cap = min_cap.max(1024).next_power_of_two();
        let data = (0..cap)
            .map(|_| UnsafeCell::new(0.0f32))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            data,
            mask: cap - 1,
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            flush: AtomicBool::new(false),
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// 可读样本数。
    #[inline]
    pub fn available(&self) -> usize {
        self.write
            .load(Ordering::Acquire)
            .wrapping_sub(self.read.load(Ordering::Acquire))
    }

    /// 可写样本数。
    #[inline]
    pub fn free(&self) -> usize {
        self.capacity() - self.available()
    }

    /// 生产：写入最多 `src.len()` 个样本，返回实际写入数。零分配、无锁。
    pub fn push(&self, src: &[f32]) -> usize {
        let w = self.write.load(Ordering::Relaxed);
        let r = self.read.load(Ordering::Acquire);
        let free = self.capacity() - w.wrapping_sub(r);
        let n = src.len().min(free);
        if n == 0 {
            return 0;
        }
        let cap = self.capacity();
        let start = w & self.mask;
        let first = n.min(cap - start);
        // 安全性：见类型级注释，写区间与读区间不重叠
        unsafe {
            let base = self.data.as_ptr() as *mut f32;
            std::ptr::copy_nonoverlapping(src.as_ptr(), base.add(start), first);
            if n > first {
                std::ptr::copy_nonoverlapping(src.as_ptr().add(first), base, n - first);
            }
        }
        self.write.store(w.wrapping_add(n), Ordering::Release);
        n
    }

    /// 消费：把最多 `dst.len()` 个样本读进 `dst`，返回实际读到的数量。零分配、无锁。
    pub fn pop_into(&self, dst: &mut [f32]) -> usize {
        let r = self.read.load(Ordering::Relaxed);
        let w = self.write.load(Ordering::Acquire);
        let avail = w.wrapping_sub(r);
        let n = dst.len().min(avail);
        if n == 0 {
            return 0;
        }
        let cap = self.capacity();
        let start = r & self.mask;
        let first = n.min(cap - start);
        // 安全性：读的样本都已由生产者 Release 写入
        unsafe {
            let base = self.data.as_ptr() as *const f32;
            std::ptr::copy_nonoverlapping(base.add(start), dst.as_mut_ptr(), first);
            if n > first {
                std::ptr::copy_nonoverlapping(base, dst.as_mut_ptr().add(first), n - first);
            }
        }
        self.read.store(r.wrapping_add(n), Ordering::Release);
        n
    }

    /// 生产端请求清空（切歌 / seek 时调用）。
    pub fn request_flush(&self) {
        self.flush.store(true, Ordering::Release);
    }

    /// 消费端执行清空，返回是否真的清过。只在音频回调里调用。
    #[inline]
    pub fn apply_flush(&self) -> bool {
        if !self.flush.swap(false, Ordering::AcqRel) {
            return false;
        }
        let w = self.write.load(Ordering::Acquire);
        self.read.store(w, Ordering::Release);
        true
    }

    /// 直接丢弃全部数据。**只能在消费者没有运行时调用**（初始化 / 拆流）。
    pub fn clear(&self) {
        let w = self.write.load(Ordering::Acquire);
        self.read.store(w, Ordering::Release);
        self.flush.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn push_pop_roundtrip() {
        let r = PcmRing::new(16);
        assert_eq!(r.capacity(), 1024, "最小容量兜底到 1024");
        let src: Vec<f32> = (0..900).map(|i| i as f32).collect();
        assert_eq!(r.push(&src), 900);
        assert_eq!(r.available(), 900);
        let mut out = vec![0f32; 400];
        assert_eq!(r.pop_into(&mut out), 400);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[399], 399.0);
        assert_eq!(r.available(), 500);
    }

    #[test]
    fn wraps_around() {
        let r = PcmRing::new(1024);
        // 先把读写指针推到接近末尾，制造回绕
        let big: Vec<f32> = vec![0.0; 1000];
        r.push(&big);
        let mut sink = vec![0f32; 1000];
        r.pop_into(&mut sink);
        let src: Vec<f32> = (0..500).map(|i| (i + 1) as f32).collect();
        assert_eq!(r.push(&src), 500);
        let mut out = vec![0f32; 500];
        assert_eq!(r.pop_into(&mut out), 500);
        assert_eq!(out[0], 1.0);
        assert_eq!(out[499], 500.0);
    }

    #[test]
    fn full_and_empty_bounds() {
        let r = PcmRing::new(1024);
        let src = vec![1.0f32; 2000];
        assert_eq!(r.push(&src), 1024, "满了只写能写的");
        assert_eq!(r.push(&src), 0, "满时写不进");
        let mut out = vec![0f32; 2048];
        assert_eq!(r.pop_into(&mut out), 1024);
        assert_eq!(r.pop_into(&mut out), 0);
    }

    #[test]
    fn flush_drops_queued() {
        let r = PcmRing::new(1024);
        r.push(&[1.0; 300]);
        r.request_flush();
        assert!(r.apply_flush());
        assert_eq!(r.available(), 0);
        assert!(!r.apply_flush(), "没有新请求时不该重复清");
    }

    #[test]
    fn concurrent_spsc_preserves_order() {
        let ring = Arc::new(PcmRing::new(1024));
        let total = 200_000usize;
        let prod = {
            let ring = ring.clone();
            std::thread::spawn(move || {
                let mut i = 0usize;
                while i < total {
                    let chunk: Vec<f32> = (0..64).map(|k| (i + k) as f32).collect();
                    let n = ring.push(&chunk);
                    i += n;
                    if n == 0 {
                        std::thread::yield_now();
                    }
                }
            })
        };
        let mut got = 0usize;
        let mut buf = vec![0f32; 64];
        while got < total {
            let n = ring.pop_into(&mut buf);
            for k in 0..n {
                assert_eq!(buf[k], (got + k) as f32, "顺序错乱 @{got}+{k}");
            }
            got += n;
            if n == 0 {
                std::thread::yield_now();
            }
        }
        prod.join().unwrap();
    }
}
