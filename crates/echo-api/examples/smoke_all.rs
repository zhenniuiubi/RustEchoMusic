//! 全接口冒烟：标准版客户端（lite=false）下逐个验证。
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
    println!("lite={} register={:?}", client.lite_public(), client.register_device().await.map(|_| "ok"));

    macro_rules! probe {
        ($name:expr, $e:expr) => {
            match $e.await {
                Ok(v) => println!("{:22} OK  n={}", $name, v.len()),
                Err(e) => println!("{:22} ERR {}", $name, e),
            }
        };
    }

    probe!("search_songs", client.search_songs("周杰伦", 1, 10));
    probe!("search_playlists", client.search_playlists("轻音乐", 1, 10));
    probe!("search_albums", client.search_albums("晴天", 1, 10));
    probe!("search_artists", client.search_artists("林俊杰", 1, 10));
    probe!("search_hot", client.search_hot());
    probe!("recommend_playlists", client.recommend_playlists(0, 1, 10));
    probe!("top_ip", client.top_ip());
    match client.style_recommend("1,2").await {
        Ok(s) => println!("{:22} OK  songs={} cats={}", "style_recommend", s.songs.len(), s.categories.len()),
        Err(e) => println!("{:22} ERR {}", "style_recommend", e),
    }
    probe!("rank_list", client.rank_list());
    probe!("top_songs", client.top_songs(1, 10));
    probe!("top_albums", client.top_albums(1, 10));
    probe!("playlist_tags", client.playlist_tags());
    probe!("recommend_songs", client.recommend_songs());
    probe!("fm_songs", client.fm_songs(10));
    match client.user_detail().await {
        Ok(u) => println!("{:22} OK  nick={} vip_type={} pic={}", "user_detail", u.nickname, u.vip_type, &u.pic[..u.pic.len().min(50)]),
        Err(e) => println!("{:22} ERR {}", "user_detail", e),
    }
    probe!("user_playlists", client.user_playlists(1, 30));
    probe!("liked_songs", client.liked_songs());

    // 播放地址：免费 + VIP
    let songs = client.recommend_songs().await.unwrap_or_default();
    for (label, want) in [("free", 0u8), ("vip", 10u8)] {
        if let Some(s) = songs.iter().find(|x| x.privilege == want as i64) {
            let aa: i64 = s.album_audio_id.parse().unwrap_or(0);
            match client.song_url(&s.hash, "128", aa).await {
                Ok(u) => println!("url[{label}]            OK  n={} {}", u.len(), &u[0][..60.min(u[0].len())]),
                Err(e) => println!("url[{label}]            ERR {e}"),
            }
        }
    }

    // 歌单详情与曲目
    if let Ok(pl) = client.recommend_playlists(0, 1, 3).await {
        if let Some(p) = pl.first() {
            println!("playlist_detail({}) {:?}", p.id, client.playlist_detail(&p.id).await.map(|x| x.is_some()));
            probe!("playlist_tracks", client.playlist_tracks(&p.id, 1, 10));
        }
    }
    if let Ok(ranks) = client.rank_list().await {
        if let Some(r) = ranks.first() {
            match client.rank_songs(&r.id, 1, 10).await {
                Ok(v) => println!("{:22} OK  n={} (rankid={})", "rank_songs", v.len(), r.id),
                Err(e) => println!("{:22} ERR {}", "rank_songs", e),
            }
        }
    }

    Ok(())
}