//! 第二期接口批量探测。
use echo_api::KuGouClient;
use echo_core::DeviceIdentity;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let device = DeviceIdentity::generate();
    let client = KuGouClient::shared(device, true)?;
    let _ = client.register_device().await;

    // 排行榜
    match client.rank_list().await {
        Ok(v) => {
            println!("rank_list: {} 个", v.len());
            for r in v.iter().take(3) { println!("   {} | {} | {}", r.id, r.name, r.update_frequency); }
        }
        Err(e) => println!("rank_list ERR {e}"),
    }

    // 每日推荐
    match client.recommend_songs().await {
        Ok(v) => { println!("recommend_songs: {} 首", v.len()); if let Some(s)=v.first(){ println!("   {} - {} ({})", s.title, s.artist, s.duration); } }
        Err(e) => println!("recommend_songs ERR {e}"),
    }

    // 新歌
    match client.top_songs(1, 5).await {
        Ok(v) => { println!("top_songs: {} 首", v.len()); if let Some(s)=v.first(){ println!("   {} - {}", s.title, s.artist); } }
        Err(e) => println!("top_songs ERR {e}"),
    }

    // 新碟
    match client.top_albums(1, 5).await {
        Ok(v) => { println!("top_albums: {} 张", v.len()); if let Some(a)=v.first(){ println!("   {} - {} ({})", a.name, a.singer_name, a.id); } }
        Err(e) => println!("top_albums ERR {e}"),
    }

    // 歌单标签
    match client.playlist_tags().await {
        Ok(v) => {
            println!("playlist_tags: {} 组", v.len());
            for (g, tags) in v.iter().take(2) { println!("   {} -> {}", g, tags.iter().take(4).map(|t| format!("{}({})", t.1, t.0)).collect::<Vec<_>>().join(",")); }
        }
        Err(e) => println!("playlist_tags ERR {e}"),
    }

    // 综合搜索
    match client.search_complex("周杰伦", 1, 5).await {
        Ok(r) => println!("search_complex: songs={} playlists={} albums={} artists={}", r.songs.len(), r.playlists.len(), r.albums.len(), r.artists.len()),
        Err(e) => println!("search_complex ERR {e}"),
    }

    // 歌手 + 专辑（用搜索结果）
    if let Ok(r) = client.search_complex("周杰伦", 1, 1).await {
        if let Some(ar) = r.artists.first() {
            println!("artist id={} name={}", ar.id, ar.name);
            match client.artist_songs(&ar.id, 1, 3).await {
                Ok(v) => println!("artist_songs: {} 首", v.len()),
                Err(e) => println!("artist_songs ERR {e}"),
            }
            match client.artist_albums(&ar.id).await {
                Ok(v) => println!("artist_albums: {} 张", v.len()),
                Err(e) => println!("artist_albums ERR {e}"),
            }
        }
        if let Some(al) = r.albums.first() {
            println!("album id={} name={}", al.id, al.name);
            match client.album_songs(&al.id, 1, 3).await {
                Ok(v) => println!("album_songs: {} 首", v.len()),
                Err(e) => println!("album_songs ERR {e}"),
            }
        }
    }

    // FM
    match client.fm_songs(5).await {
        Ok(v) => println!("fm_songs: {} 首", v.len()),
        Err(e) => println!("fm_songs ERR {e}"),
    }
    Ok(())
}
