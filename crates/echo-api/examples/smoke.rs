//! 真实网络冒烟测试：搜索 → 播放地址。
use echo_api::KuGouClient;
use echo_core::DeviceIdentity;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let device = DeviceIdentity::generate();
    let client = KuGouClient::shared(device, true)?;
    let _ = client.register_device().await;

    let songs = client.search_songs("晴天", 1, 3).await?;
    println!("搜索到 {} 首", songs.len());
    for s in &songs {
        println!("  {} / {} hash={}", s.title, s.artist, &s.hash[..10.min(s.hash.len())]);
    }
    let first = &songs[0];
    let aa: i64 = first.album_audio_id.parse().unwrap_or(0);
    match client.song_url(&first.hash, "128", aa).await {
        Ok(urls) => {
            println!("song_url OK: {}", urls[0]);
            // 试拉前 64KB 验证可下载
            let resp = reqwest::get(&urls[0]).await?;
            println!("  http status={}", resp.status());
            let bytes = resp.bytes().await?;
            println!("  bytes={}", bytes.len());
        }
        Err(e) => println!("song_url FAILED: {e}"),
    }
    Ok(())
}
