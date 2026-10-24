//! 无界面音频探针：起引擎 → 播 URL → 打诊断，用于验收测试。
//!
//! 用法：
//! ```text
//! cargo run --release -p echo-audio --example engine_probe -- <url> [秒数] [音量]
//! ```
//! 默认跑 30 秒、音量 0（静音，测试时不吵），每秒打一行诊断，
//! 结束时打一行 `[summary]` 便于脚本抓取。

use echo_audio::{AudioEngine, DiagSnapshot, PlaybackState, PlayerEvent};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

fn main() {
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .expect("用法: engine_probe <url> [秒数] [音量 0..1]");
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let vol: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);

    let (tx, rx) = channel();
    let engine = AudioEngine::new(tx).expect("init audio engine");
    engine.set_volume(vol);

    let started = Instant::now();
    let t_load = Instant::now();
    engine.load(&url);

    let mut first_sound: Option<u128> = None;
    let mut last: Option<DiagSnapshot> = None;
    let deadline = started + Duration::from_secs(secs);

    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(PlayerEvent::StateChanged(PlaybackState::Playing)) if first_sound.is_none() => {
                let ms = t_load.elapsed().as_millis();
                first_sound = Some(ms);
                println!("[probe] 首次出声: {ms}ms");
            }
            Ok(PlayerEvent::Diagnostics(d)) => {
                last = Some(d);
                println!(
                    "[probe] t={:>3}s 欠载={} 预缓冲={}ms 回调帧={} 最大回调={}us",
                    started.elapsed().as_secs(),
                    d.underruns,
                    d.target_ms,
                    d.cb_frames,
                    d.max_cb_us
                );
            }
            Ok(PlayerEvent::Error(e)) => println!("[probe] 错误: {e}"),
            Ok(PlayerEvent::Ended) => {
                println!("[probe] 播放结束");
                break;
            }
            Ok(_) | Err(_) => {}
        }
    }

    match last {
        Some(d) => println!(
            "[summary] underruns={} max_cb_us={} callbacks={} played_frames={} target_ms={} cb_frames={} rt_ok={} first_sound_ms={}",
            d.underruns,
            d.max_cb_us,
            d.callbacks,
            d.played_frames,
            d.target_ms,
            d.cb_frames,
            d.rt_ok,
            first_sound.unwrap_or(0)
        ),
        None => println!("[summary] 没有收到诊断事件（引擎未启动？）"),
    }

    // 引擎各线程不参与 join，直接退出
    std::process::exit(0);
}
