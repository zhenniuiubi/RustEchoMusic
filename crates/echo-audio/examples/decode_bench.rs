//! 解码开销基准：量化「把解码放在音频回调里」到底要花多少时间。
//!
//! 这正是**改造前**的架构：rodio 把 `Decoder` 当 Source 挂在 Sink 上，
//! 由 cpal 回调逐样本拉取，解码 + 重采样全在实时线程里跑。
//!
//! 本基准用和改造前一模一样的方式取数据（`Decoder` + `SampleRateConverter`
//! + `ChannelCountConverter`），只是不接设备，直接计时：
//! 「解码一个回调周期的音频要多久」vs「这个回调周期的预算」。
//!
//! 用法：
//! ```text
//! cargo run --release -p echo-audio --example decode_bench -- <file.mp3> [chunk_frames] [秒数]
//! ```

use rodio::conversions::{ChannelCountConverter, SampleRateConverter};
use rodio::{Decoder, Source};
use std::io::Cursor;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("用法: decode_bench <file> [chunk_frames] [秒数]");
    let chunk_frames: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(480);
    let seconds: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(60);

    let bytes = std::fs::read(&path).expect("读文件失败");
    let len = bytes.len() as u64;
    let mut dec = Decoder::builder()
        .with_data(Cursor::new(bytes))
        .with_byte_len(len)
        .with_seekable(true)
        .build()
        .expect("解码器初始化失败");

    let src_ch = dec.channels();
    let src_rate = dec.sample_rate();
    // 设备侧固定按 48kHz / 立体声（本机默认输出格式）
    let mut conv = SampleRateConverter::new(dec.by_ref(), src_rate, 48_000, src_ch);
    let mut chan = ChannelCountConverter::new(&mut conv, src_ch, 2);

    let per_chunk = chunk_frames * 2;
    let total_chunks = seconds * 48_000 / chunk_frames;
    let budget_us = chunk_frames as f64 / 48_000.0 * 1_000_000.0;

    let mut us: Vec<u32> = Vec::with_capacity(total_chunks);
    let mut samples = vec![0f32; per_chunk];

    for _ in 0..total_chunks {
        let t0 = Instant::now();
        let mut got = 0usize;
        while got < per_chunk {
            match chan.next() {
                Some(s) => {
                    samples[got] = s;
                    got += 1;
                }
                None => break,
            }
        }
        let el = t0.elapsed().as_micros() as u32;
        us.push(el);
        if got < per_chunk {
            break; // 文件放完了
        }
    }
    let _ = &samples; // 防优化

    if us.is_empty() {
        println!("[bench] 没有采集到样本");
        return;
    }
    us.sort_unstable();
    let n = us.len();
    let pick = |p: f64| us[((n as f64 - 1.0) * p) as usize];
    let mean = us.iter().map(|v| *v as f64).sum::<f64>() / n as f64;
    let over = |limit: u32| us.iter().filter(|v| **v > limit).count();

    println!(
        "[bench] chunks={n} chunk_frames={chunk_frames} 预算/回调={budget_us:.0}us \
         mean={mean:.0}us p50={}us p90={}us p99={}us max={}us \
         >2ms={} >5ms={} >预算={}",
        pick(0.50),
        pick(0.90),
        pick(0.99),
        us[n - 1],
        over(2000),
        over(5000),
        over(budget_us as u32)
    );
}
