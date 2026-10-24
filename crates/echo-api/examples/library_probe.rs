//! 已购音乐 / 音乐云盘接口探测。
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
    let refresh = client.refresh_token().await;
    let a = client.auth();
    println!(
        "probe auth: userid={} token_len={} play_auth_len={} refresh={:?}",
        a.userid,
        a.token.len(),
        a.auth.len(),
        refresh.map(|r| r.userid)
    );

    // 原始响应（排查字段结构）
    {
        use echo_api::client::{EncryptType, RequestSpec};
        let auth = client.auth();
        let spec = RequestSpec {
            url: "/openapi/copyright/v1/audio/get_goods".into(),
            encrypt_type: EncryptType::Android,
            body: Some(serde_json::json!({
                "appid": client.appid_public(),
                "userid": auth.userid,
                "token": auth.token,
                "page": 1,
                "pagesize": 10,
                "clientver": client.clientver_public().to_string(),
                "deleted": 0,
                "need_audio_info": 1,
                "area_code": "1",
            })),
            ..Default::default()
        };
        match client.request(spec).await {
            Ok(v) => println!("raw_gooods: {}", &v.to_string()[..600.min(v.to_string().len())]),
            Err(e) => println!("raw_gooods ERR {e}"),
        }
    }

    match client.purchased_songs(1, 100).await {
        Ok(v) => {
            println!("purchased_songs n={}", v.len());
            for s in v.iter().take(5) {
                println!("  {} - {} hash={} aa={}", s.title, s.artist, s.hash, s.album_audio_id);
            }
        }
        Err(e) => println!("purchased_songs ERR {e}"),
    }

    match client.purchased_albums(1, 10).await {
        Ok(v) => {
            println!("purchased_albums n={}", v.len());
            for a in v.iter().take(5) {
                println!("  {} id={}", a.name, a.id);
            }
        }
        Err(e) => println!("purchased_albums ERR {e}"),
    }

    match client.cloud_songs(1, 10).await {
        Ok(v) => {
            println!("cloud_songs n={}", v.len());
            for s in v.iter().take(5) {
                println!("  {} - {} hash={}", s.title, s.artist, s.hash);
            }
        }
        Err(e) => println!("cloud_songs ERR {e}"),
    }

    // 云盘原始响应（解密后的 JSON 文本）
    {
        use base64::Engine as _;
        let auth = client.auth();
        let plain = serde_json::json!({"page":1,"pagesize":10,"getkmr":1}).to_string();
        let (seed, cipher_b64) = echo_api::crypto::playlist_aes_encrypt(&plain);
        let p_plain = serde_json::json!({"aes": seed, "uid": auth.userid, "token": auth.token}).to_string();
        let p = echo_api::crypto::rsa_encrypt_hex(p_plain.as_bytes(), false)?.to_uppercase();
        let clienttime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
            .to_string();
        let key = echo_api::crypto::sign_params_key(&clienttime, client.appid_public(), client.clientver_public(), false);
        let body = base64::engine::general_purpose::STANDARD.decode(cipher_b64.as_bytes())?;
        let params: Vec<(String, String)> = vec![
            ("clienttime".into(), clienttime.clone()),
            ("mid".into(), client.device().mid),
            ("key".into(), key),
            ("clientver".into(), client.clientver_public().to_string()),
            ("appid".into(), client.appid_public().to_string()),
            ("p".into(), p),
        ];
        match client
            .post_cloud_raw("https://mcloudservice.kugou.com/v1/get_list", &params, body)
            .await
        {
            Ok(resp) => {
                let resp: Vec<u8> = resp;
                println!("cloud raw bytes = {}", resp.len());
                let b64 = base64::engine::general_purpose::STANDARD.encode(&resp);
                match echo_api::crypto::playlist_aes_decrypt(&seed, &b64) {
                    Ok(bytes) => {
                        let t = String::from_utf8_lossy(&bytes).to_string();
                        println!("cloud plain: {}", &t[..t.len().min(1200)]);
                    }
                    Err(e) => {
                        println!("cloud decrypt ERR {e}");
                        println!("cloud raw as text: {}", String::from_utf8_lossy(&resp[..resp.len().min(400)]));
                    }
                }
            }
            Err(e) => println!("cloud raw ERR {e}"),
        }
    }
    Ok(())
}
