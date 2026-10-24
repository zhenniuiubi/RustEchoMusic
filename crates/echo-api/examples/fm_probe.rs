//! 私人FM / 音乐云盘取地址 探测。
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

    let auth = client.auth();
    let dev = client.device();
    let appid = client.appid_public();
    let clientver = client.clientver_public();

    // ===== 私人FM：/v2/personal_recommend（x-router: persnfm.service.kugou.com）=====
    let ct = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    let key = echo_api::crypto::sign_params_key(&ct.to_string(), appid, clientver, false);
    let body = serde_json::json!({
        "appid": appid,
        "clienttime": ct,
        "mid": dev.mid,
        "action": "play",
        "recommend_source_locked": 0,
        "song_pool_id": 0,
        "callerid": 0,
        "m_type": 1,
        "platform": "ios",
        "area_code": 1,
        "remain_songcnt": 0,
        "clientver": clientver,
        "is_overplay": 0,
        "mode": "normal",
        "fakem": "ca981cfc583a4c37f28d2d49000013c16a0a",
        "key": key,
        "userid": auth.userid,
        "kguid": auth.userid,
        "token": auth.token,
        "vip_type": 4,
    });
    let spec = RequestSpec {
        url: "/v2/personal_recommend".into(),
        x_router: Some("persnfm.service.kugou.com".into()),
        body: Some(body),
        encrypt_type: EncryptType::Android,
        ..Default::default()
    };
    match client.request(spec).await {
        Ok(v) => {
            let t = v.to_string();
            println!("fm OK len={} :: {}", t.len(), &t[..t.len().min(1500)]);
        }
        Err(e) => println!("fm ERR {e}"),
    }

    // ===== 云盘取地址：/bsstrackercdngz/v2/query_musicclound_url =====
    let hash = "f3f738e1483870ff4f4d32f31a7fc074";
    let mut params = std::collections::BTreeMap::new();
    params.insert("hash".to_string(), hash.to_string());
    params.insert("ssa_flag".to_string(), "is_fromtrack".to_string());
    params.insert("version".to_string(), "20102".to_string());
    params.insert("ssl".to_string(), "0".to_string());
    params.insert("album_audio_id".to_string(), "52592215".to_string());
    params.insert("pid".to_string(), "20026".to_string());
    params.insert("audio_id".to_string(), "23216520".to_string());
    params.insert("kv_id".to_string(), "1".to_string());
    params.insert(
        "key".to_string(),
        echo_api::crypto::sign_cloud_key(hash, "20026"),
    );
    params.insert("bucket".to_string(), "musicclound".to_string());
    params.insert("name".to_string(), "陈一发儿 - 童话镇.mp3".to_string());
    params.insert("with_res_tag".to_string(), "0".to_string());
    let spec = RequestSpec {
        url: "/bsstrackercdngz/v2/query_musicclound_url".into(),
        params,
        encrypt_type: EncryptType::Android,
        ..Default::default()
    };
    match client.request(spec).await {
        Ok(v) => {
            let t = v.to_string();
            println!("cloud_url OK :: {}", &t[..t.len().min(600)]);
        }
        Err(e) => println!("cloud_url ERR {e}"),
    }

    // ===== 普通取地址接口对云盘 hash 有没有效 =====
    match client.song_url(hash, "128", 52592215).await {
        Ok(u) => println!("song_url(hash) OK {u:?}"),
        Err(e) => println!("song_url(hash) ERR {e}"),
    }

    // ===== kv_id=2 的云盘歌曲用 kv_id=1 / 2 分别取一次 =====
    for kv in ["1", "2"] {
        let h2 = "e17b296cbc8b23329f52b445ba7a8e35";
        let mut p2 = std::collections::BTreeMap::new();
        p2.insert("hash".to_string(), h2.to_string());
        p2.insert("ssa_flag".to_string(), "is_fromtrack".to_string());
        p2.insert("version".to_string(), "20102".to_string());
        p2.insert("ssl".to_string(), "0".to_string());
        p2.insert("album_audio_id".to_string(), "507931889".to_string());
        p2.insert("pid".to_string(), "20026".to_string());
        p2.insert("audio_id".to_string(), "241032750".to_string());
        p2.insert("kv_id".to_string(), kv.to_string());
        p2.insert("key".to_string(), echo_api::crypto::sign_cloud_key(h2, "20026"));
        p2.insert("bucket".to_string(), "musicclound".to_string());
        p2.insert("name".to_string(), "涛涛 - 情深深雨蒙蒙 (原创).mp3".to_string());
        p2.insert("with_res_tag".to_string(), "0".to_string());
        let spec = RequestSpec {
            url: "/bsstrackercdngz/v2/query_musicclound_url".into(),
            params: p2,
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        match client.request(spec).await {
            Ok(v) => {
                let t = v.to_string();
                println!("cloud_url kv={kv} OK :: {}", &t[..t.len().min(200)]);
            }
            Err(e) => println!("cloud_url kv={kv} ERR {e}"),
        }
    }

    // ===== 已购单曲 pagesize 上限 =====
    for n in [10i64, 20, 30, 40, 50] {
        match client.purchased_songs(1, n).await {
            Ok(v) => println!("purchased pagesize={n} OK n={}", v.len()),
            Err(e) => println!("purchased pagesize={n} ERR {e}"),
        }
    }

    match client.personal_fm(0, 0, "play", "normal", 0).await {
        Ok(batch) => {
            let list = batch.songs;
            println!("personal_fm n={} cur_mark={}", list.len(), batch.cur_mark);
            for s in list.iter().take(5) {
                println!("  {} - {} hash={}", s.title, s.artist, s.hash);
            }
        }
        Err(e) => println!("personal_fm ERR {e}"),
    }
    // 第二种：action=login（新会话），连续两次
    for i in 1..=2 {
        match client.personal_fm(0, 0, "login", "peak", 0).await {
            Ok(b) => println!(
                "fm login#{i} n={} cur_mark={} first={}",
                b.songs.len(),
                b.cur_mark,
                b.songs.first().map(|s| s.title.clone()).unwrap_or_default()
            ),
            Err(e) => println!("fm login#{i} ERR {e}"),
        }
    }
    // 第三种：action=play 带上一批 cur_mark
    let first = client.personal_fm(0, 0, "play", "normal", 0).await?;
    match client
        .personal_fm(first.songs.len() as i64, first.cur_mark, "play", "normal", 0)
        .await
    {
        Ok(b) => println!("fm play#2 n={} cur_mark={}", b.songs.len(), b.cur_mark),
        Err(e) => println!("fm play#2 ERR {e}"),
    }

    // 云盘歌曲取地址（走 cloud_song_url）
    match client.cloud_songs(1, 30).await {
        Ok(list) => {
            println!("cloud n={}", list.len());
            for song in list.iter().take(2) {
                match client.cloud_song_url(song).await {
                    Ok(u) => println!("  cloud_url OK {} -> {}", song.title, u),
                    Err(e) => println!("  cloud_url ERR {} {e}", song.title),
                }
            }
        }
        Err(e) => println!("cloud ERR {e}"),
    }
    Ok(())
}
