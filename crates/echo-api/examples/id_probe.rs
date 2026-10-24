//! 用 /kmr/v2/audio 把 album_audio_id / mixsongid 反查成歌曲，验证播放地址接口拿到的 id 是否正确。
use echo_api::client::{EncryptType, RequestSpec};
use echo_api::KuGouClient;
use serde_json::json;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("EchoMusic");
    let device = echo_core::DeviceIdentity::load_or_create(&dir)?;
    let client = KuGouClient::shared(device, false)?;

    let ids: Vec<i64> = std::env::args()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();

    // 不带参数时：搜索一首歌，用解析出来的 album_audio_id 反查，验证不会串歌
    if ids.is_empty() {
        for kw in ["别叹 黄霄雲", "晴天 周杰伦"] {
            let songs = client.search_songs(kw, 1, 3).await?;
            for s in songs {
                println!(
                    "[{}] title={} hash={} aa={} audio_id={} mix={}",
                    kw, s.title, s.hash, s.album_audio_id, s.audio_id, s.mix_song_id
                );
                let id = s.play_album_audio_id();
                probe_one(&client, id).await;
            }
        }
        return Ok(());
    }
    for id in ids {
        probe_one(&client, id).await;
    }
    Ok(())
}

async fn probe_one(client: &KuGouClient, id: i64) {
    {
        let spec = RequestSpec {
            url: "/kmr/v2/audio".into(),
            encrypt_type: EncryptType::Android,
            body: Some(json!({ "data": [{ "entity_id": id }], "fields": "base" })),
            extra_headers: vec![
                ("x-router".into(), "openapi.kugou.com".into()),
                ("KG-TID".into(), "238".into()),
            ],
            ..Default::default()
        };
        match client.request(spec).await {
            Ok(v) => {
                let arr = v
                    .pointer("/data")
                    .and_then(|d| d.as_array())
                    .cloned()
                    .unwrap_or_default();
                if arr.is_empty() {
                    println!("{id}: 无结果 {}", &v.to_string()[..200.min(v.to_string().len())]);
                }
                for item in arr {
                    let name = item
                        .pointer("/base/audio_name")
                        .or_else(|| item.pointer("/base/songname"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("-");
                    let hash = item
                        .pointer("/base/audio_hash")
                        .or_else(|| item.pointer("/base/hash"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("-");
                    println!("{id}: {name}  hash={hash}");
                }
            }
            Err(e) => println!("{id}: ERR {e}"),
        }
    }
}
