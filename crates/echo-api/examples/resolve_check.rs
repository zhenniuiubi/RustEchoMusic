//! 取一首歌的播放地址（从 queue.json / library.json 里按标题找），用于验证「不会串歌」。
use echo_api::KuGouClient;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("EchoMusic");
    let session: echo_core::Session =
        serde_json::from_str(&std::fs::read_to_string(dir.join("session.json"))?)?;
    let device = echo_core::DeviceIdentity::load_or_create(&dir)?;
    let client = KuGouClient::shared(device, false)?;
    client.set_auth(&session.token, session.userid);
    let _ = client.register_device().await;

    let queue: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("queue.json"))?)?;
    let kw = std::env::args().nth(1).unwrap_or_else(|| "别叹".into());
    let songs: Vec<echo_core::Song> = serde_json::from_value(queue["songs"].clone())?;
    let Some(song) = songs.into_iter().find(|s| s.title.contains(&kw)) else {
        println!("队列里没有 {kw}");
        return Ok(());
    };

    println!(
        "song: {} - {} hash={} aa={} audio_id={} mix={} duration={}s",
        song.title,
        song.artist,
        song.hash,
        song.album_audio_id,
        song.audio_id,
        song.mix_song_id,
        song.duration
    );
    println!("play_album_audio_id={}", song.play_album_audio_id());

    let quality = std::env::args().nth(2).unwrap_or_else(|| "128".into());
    match client
        .song_url(&song.hash, &quality, song.play_album_audio_id())
        .await
    {
        Ok(urls) => {
            for u in urls {
                println!("url: {u}");
            }
        }
        Err(e) => println!("song_url ERR {e}"),
    }
    Ok(())
}
