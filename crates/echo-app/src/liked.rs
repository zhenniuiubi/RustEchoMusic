//! 远端「我喜欢的音乐」：加载、红心切换。
//!
//! 与本地 `library.json` 的收藏并存：
//! - 登录用户：红心以酷狗云歌单为准，切换走 `/v6/add_song` / `/v4/delete_songs`；
//! - 未登录：退化为本地收藏。

use std::sync::Arc;
use crate::state::AppStore;
use crate::ui;
use gpui::{App, AsyncApp, Entity};

/// 加载远端红心歌单（登录后调用）。
pub(crate) fn load_remote_liked(store: &Entity<AppStore>, force: bool, cx: &mut App) {
    let store_c = store.clone();
    if !force {
        let s = store.read(cx);
        if s.remote_liked_loaded || s.remote_liked_loading {
            return;
        }
        if s.session.get().userid == 0 {
            return;
        }
    }
    let t0 = std::time::Instant::now();
    store.update(cx, |s, cx| {
        s.remote_liked_loading = true;
        cx.notify();
    });

    let client = store.read(cx).client.clone();
    let task = cx.background_executor().spawn(async move {
        let pl = ui::run_tokio(client.liked_playlist()).unwrap_or_default();
        let listid = pl.as_ref().map(|p| p.listid.clone()).unwrap_or_default();
        // 拉全部页（红心歌单常有几百首）
        let mut songs = Vec::new();
        if !listid.is_empty() {
            for page in 1..=5 {
                let batch =
                    ui::run_tokio(client.user_playlist_tracks(&listid, page, 300)).unwrap_or_default();
                if batch.is_empty() {
                    break;
                }
                let n = batch.len();
                songs.extend(batch);
                if n < 300 {
                    break;
                }
            }
        }
        (listid, songs)
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let (listid, songs) = task.await;
        // 保证加载动画至少可见
        let min = std::time::Duration::from_millis(500);
        let elapsed = t0.elapsed();
        if elapsed < min {
            smol::Timer::after(min - elapsed).await;
        }
        let _ = store_c.update(cx, |s, cx| {
            s.remote_liked_loading = false;
            s.remote_liked_loaded = true;
            s.remote_liked_listid = listid;
            s.remote_liked_keys = songs.iter().map(|x| x.key()).collect();
            // hash → 云歌单条目 id（取消收藏时按 fileid 删）
            s.remote_liked_fileids = songs
                .iter()
                .filter_map(|x| {
                    x.cloud_audio_id
                        .parse::<i64>()
                        .ok()
                        .map(|id| (x.key(), id))
                })
                .collect();
            s.remote_liked = Arc::new(songs);
            // 数据源变了：重算「我最喜爱」的模糊搜索结果
            cx.notify();
        });
    })
    .detach();
}

/// 当前歌曲是否已在远端红心里。
pub(crate) fn is_remote_liked(s: &AppStore, song: &echo_core::Song) -> bool {
    if s.remote_liked_keys.is_empty() {
        return false;
    }
    s.remote_liked_keys.contains(&song.key())
}

/// 切换红心：登录用户走云歌单 API，未登录走本地收藏。
pub(crate) fn toggle_like(store: &Entity<AppStore>, song: &echo_core::Song, cx: &mut App) {
    let store_c = store.clone();
    let song_c = song.clone();

    let logged_in = store.read(cx).session.get().userid != 0;
    if !logged_in {
        // 本地收藏
        store.update(cx, |s, cx| {
            let now = s.library.toggle_favorite(&song_c);
            s.toast = if now { "已收藏（本地）".into() } else { "已取消收藏".into() };
            cx.notify();
        });
        return;
    }

    // 远端红心
    let listid = store.read(cx).remote_liked_listid.clone();
    if listid.is_empty() {
        store.update(cx, |s, cx| {
            s.toast = "红心歌单尚未加载，请稍后重试".into();
            cx.notify();
        });
        return;
    }

    let was_liked = store.read(cx).remote_liked_keys.contains(&song_c.key());
    store.update(cx, |s, cx| {
        s.toast = if was_liked {
            "正在取消收藏…".into()
        } else {
            "正在收藏…".into()
        };
        cx.notify();
    });

    let client = store.read(cx).client.clone();
    let client2 = client.clone();
    let listid_c = listid.clone();
    let song_api = song_c.clone();
    // 取消收藏要用云歌单里的条目 id（audio_id）
    let fileid = store
        .read(cx)
        .remote_liked_fileids
        .get(&song_c.key())
        .copied();
    // 必须走 run_tokio：reqwest 的超时依赖 tokio 定时器，
    // 直接丢给 gpui 的 background_executor（smol）会 panic
    // "there is no reactor running"，且在不可 unwind 的任务里直接崩溃进程。
    let task = cx.background_executor().spawn(async move {
        ui::run_tokio(async move {
            if was_liked {
                client
                    .unlike_song_with_fileid(&song_api, &listid_c, fileid)
                    .await
            } else {
                client
                    .like_song_with_fileid(&song_api, &listid_c, fileid)
                    .await
            }
        })
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let res = task.await;
        let _ = store_c.update(cx, |s, cx| {
            match res {
                Ok(()) => {
                    let key = song_c.key();
                    if was_liked {
                        s.remote_liked_keys.remove(&key);
                        Arc::make_mut(&mut s.remote_liked).retain(|x| x.key() != key);
                        s.remote_liked_fileids.remove(&key);
                        s.toast = "已取消收藏".into();
                    } else {
                        s.remote_liked_keys.insert(key.clone());
                        Arc::make_mut(&mut s.remote_liked).insert(0, song_c.clone());
                        s.toast = "已收藏".into();
                    }
                    // 同步本地收藏，保持离线可用
                    let _ = s.library.toggle_favorite(&song_c);
                }
                Err(e) => {
                    s.toast = format!("收藏操作失败：{e}");
                }
            }
            cx.notify();
        });
        // 收藏成功后重新拉一次列表：新条目的 fileid 只有服务端知道
        if !was_liked {
            let st = store_c.clone();
            let t = cx
                .background_executor()
                .spawn(async move { ui::run_tokio(client2.liked_playlist()) });
            cx.spawn(async move |cx: &mut AsyncApp| {
                if let Ok(Some(pl)) = t.await {
                    let _ = st.update(cx, |s, cx| {
                        s.remote_liked_listid = pl.listid;
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    })
    .detach();
}


// ==================== 歌单管理（第三期） ====================

