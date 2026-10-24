//! 第三期接口探测：歌词下载（KRC/LRC）、歌词搜索、免费听、歌曲 MV。
use echo_api::client::{EncryptType, RequestSpec};
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
    let _ = client.refresh_token().await;

    // ===== 免费听 =====
    {
        let spec = RequestSpec {
            url: "/concepts/v1/ai/recommend_song".into(),
            body: Some(serde_json::json!({"apiver": 2, "source": 2, "pagesize": 20, "page": 1})),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        match client.request(spec).await {
            Ok(v) => {
                let t = v.to_string();
                println!("free_listen OK :: {}", &t[..t.len().min(400)]);
            }
            Err(e) => println!("free_listen ERR {e}"),
        }
    }

    // ===== 取一首歌（搜索）→ 歌词 =====
    let songs = client
        .search_songs("周杰伦", 1, 5)
        .await
        .unwrap_or_default();
    let first = songs.first().cloned();
    if let Some(song) = first {
        println!(
            "song: {} - {} hash={} aa={} dur={}",
            song.title, song.artist, song.hash, song.album_audio_id, song.duration
        );
        let aa = song.play_album_audio_id();

        // user_verify → auth（猜测 lyrics download 的 accesskey 就是它）
        let auth = client.user_verify().await.unwrap_or_default();
        println!("user_verify auth len={}", auth.len());

        for (id_name, id) in [("aa", aa.to_string()), ("audio_id", song.audio_id.clone())] {
            for (ak_name, ak) in [("auth", auth.clone()), ("token", session.token.clone())] {
                let mut spec = RequestSpec {
                    url: "/download".into(),
                    base_url: Some("https://lyrics.kugou.com".into()),
                    encrypt_type: EncryptType::Android,
                    ..Default::default()
                };
                spec.params.insert("ver".into(), "1".into());
                spec.params.insert("client".into(), "android".into());
                spec.params.insert("id".into(), id.clone());
                spec.params.insert("accesskey".into(), ak.clone());
                spec.params.insert("fmt".into(), "krc".into());
                spec.params.insert("charset".into(), "utf8".into());
                match client.request(spec).await {
                    Ok(v) => {
                        let ct = v.pointer("/contenttype").and_then(|x| x.as_i64());
                        let content = v.pointer("/content").and_then(|x| x.as_str()).unwrap_or("");
                        println!(
                            "lyric id={id_name}({id}) ak={ak_name} contenttype={:?} content_len={} head={}",
                            ct,
                            content.len(),
                            &content[..content.len().min(40)]
                        );
                    }
                    Err(e) => println!("lyric id={id_name} ak={ak_name} ERR {e}"),
                }
            }
        }

        // ===== 歌词搜索（lyrics.kugou.com/v1/search，不签名 + 清默认参数）=====
        {
            let mut spec = RequestSpec {
                url: "/v1/search".into(),
                base_url: Some("https://lyrics.kugou.com".into()),
                encrypt_type: EncryptType::None,
                skip_user_auth: true,
                clear_default_params: true,
                ..Default::default()
            };
            spec.params.insert("appid".into(), client.appid_public().to_string());
            spec.params.insert("clientver".into(), client.clientver_public().to_string());
            spec.params.insert("album_audio_id".into(), aa.to_string());
            spec.params.insert("duration".into(), song.duration.to_string());
            spec.params.insert("hash".into(), song.hash.clone());
            spec.params.insert("keyword".into(), format!("{} - {}", song.title, song.artist));
            spec.params.insert("lrctxt".into(), "1".into());
            spec.params.insert("man".into(), "no".into());

            match client.request_text(spec).await {
                Ok(text) => {
                    println!("lyric_search RAW :: {}", &text[..text.len().min(700)]);
                    // 取第一个候选的 id / accesskey
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        let cands = v.pointer("/candidates").and_then(|x| x.as_array()).cloned();
                        if let Some(c) = cands.and_then(|a| a.first().cloned()) {
                            let id = c.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
                            let ak = c.get("accesskey").and_then(|x| x.as_str()).unwrap_or("");
                            println!("candidate id={id} accesskey={}", &ak[..ak.len().min(24)]);
                            let mut dspec = RequestSpec {
                                url: "/download".into(),
                                base_url: Some("https://lyrics.kugou.com".into()),
                                encrypt_type: EncryptType::Android,
                                skip_user_auth: true,
                                ..Default::default()
                            };
                            dspec.params.insert("ver".into(), "1".into());
                            dspec.params.insert("client".into(), "android".into());
                            dspec.params.insert("id".into(), id.to_string());
                            dspec.params.insert("accesskey".into(), ak.to_string());
                            dspec.params.insert("fmt".into(), "krc".into());
                            dspec.params.insert("charset".into(), "utf8".into());
                            match client.request_text(dspec).await {
                                Ok(t2) => println!("lyric_download RAW :: {}", &t2[..t2.len().min(300)]),
                                Err(e) => println!("lyric_download ERR {e}"),
                            }
                        }
                    }
                }
                Err(e) => println!("lyric_search ERR {e}"),
            }
        }

        // ===== MV：/kmr/v1/audio/mv =====
        {
            let mut spec = RequestSpec {
                url: "/kmr/v1/audio/mv".into(),
                body: Some(serde_json::json!({
                    "data": [{"album_audio_id": aa.to_string()}],
                    "fields": ""
                })),
                encrypt_type: EncryptType::Android,
                ..Default::default()
            };
            spec.x_router = Some("openapi.kugou.com".into());
            spec.extra_headers.push(("KG-TID".into(), "38".into()));
            match client.request(spec).await {
                Ok(v) => {
                    let t = v.to_string();
                    println!("audio_mv OK :: {}", &t[..t.len().min(400)]);
                }
                Err(e) => println!("audio_mv ERR {e}"),
            }
        }
    }
    Ok(())
}
