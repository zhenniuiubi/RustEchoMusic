//! 有上限的封面图缓存（LRU）。
//!
//! gpui 0.2.2 里图片有两条「只增不减」的路径：
//! - `RetainAllImageCache`：文档说 LRU，实现是纯 `HashMap`，永不淘汰；
//! - `Window::use_asset`：解码结果留在 `App::loading_assets` 里，直到进程退出。
//!
//! 列表页一次会显示几十张封面（歌单卡 200×200 的 RGBA 就是 160KB），
//! 浏览几个列表后常驻内存能涨几十 MB。这里用带上限的 LRU 顶替：
//! 超过 `cap` 就把最久未用的那张 `drop_image`（同时释放 CPU 位图与 GPU 纹理）。

use futures::FutureExt;
use gpui::{
    App, Asset, AssetLogger, ImageAssetLoader, ImageCache, ImageCacheError, ImageCacheItem,
    RenderImage, Resource, Window, hash,
};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// 默认缓存上限（张）。
///
/// 一屏最多几十张封面，150 张约有 3 屏余量；按平均 100KB/张（120~200px RGBA）
/// 估算常驻上限约 15MB，滚回原位置也能命中而不必重新下载。
pub const DEFAULT_CAP: usize = 150;

/// LRU 封面缓存。
pub struct CoverCache {
    cap: usize,
    map: HashMap<u64, ImageCacheItem>,
    /// 访问顺序：尾部最新，头部最久未用
    order: VecDeque<u64>,
}

impl CoverCache {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(16),
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// 把 key 移到队尾（最近使用）。
    fn touch(&mut self, key: u64) {
        if let Some(pos) = self.order.iter().position(|k| *k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key);
    }

    /// 超出上限时淘汰最久未用的条目。
    fn evict(&mut self, window: &mut Window, cx: &mut App) {
        // 少量冗余：避免每次 insert 都算一遍
        while self.map.len() > self.cap {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            let Some(mut item) = self.map.remove(&key) else {
                continue;
            };
            // 已解码的图要显式回收纹理；仍在下载中的直接丢弃任务即可
            if let Some(Ok(image)) = item.get() {
                cx.drop_image(image, Some(window));
            }
        }
    }
}

impl ImageCache for CoverCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = hash(resource);

        let mut hit = None;
        if let Some(item) = self.map.get_mut(&key) {
            hit = item.get();
        }
        if let Some(res) = hit {
            self.touch(key);
            return Some(res);
        }

        // 未命中（或仍在加载）：起一个加载任务
        if !self.map.contains_key(&key) {
            let fut = AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx);
            let task = cx.background_executor().spawn(fut).shared();
            self.map.insert(key, ImageCacheItem::Loading(task.clone()));
            self.order.push_back(key);

            // 加载完成后触发一次重绘
            let view = window.current_view();
            window
                .spawn(cx, async move |cx| {
                    let _ = task.await;
                    cx.on_next_frame(move |_, cx| cx.notify(view));
                })
                .detach();
        }

        self.evict(window, cx);
        None
    }
}
