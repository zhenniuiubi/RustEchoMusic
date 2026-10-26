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
    RenderImage, Resource, SharedUri, Window, hash,
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

// ==================== 播放页背景：封面的高斯模糊版 ====================
//
// gpui 0.2.2 没有元素级的 `backdrop-filter`/高斯模糊（只有窗口级的
// `WindowBackgroundAppearance::Blurred`），要「毛玻璃背景」只能自己算。
//
// 做法：复用 gpui 自己的图片加载器拿到已解码的封面（BGRA 位图），在后台线程里
// **先缩到很小再高斯模糊**，最后交给 GPU 拉伸铺满——双线性放大再糊一层。
// 直接对原图做 sigma=20 的模糊是 O(像素×sigma)，慢一个数量级；先缩小再模糊
// 视觉效果几乎一样（背景本来就要糊到看不出细节）。
//
// 注意通道顺序：gpui 的 `RenderImage` 存的是 BGRA（见 `swap_rgba_pa_to_bgra`），
// 但高斯模糊是逐通道独立的线性运算，通道顺序不影响结果，不用先转回 RGBA。

/// 模糊图的宽度（像素）。96 是「够糊但颜色还在」的平衡点：再小下去放大到
/// 上千像素时会看到色块台阶，再大就把源图的结构留下来了。
const BLUR_W: u32 = 96;
/// 高斯 sigma（在缩小后的图上做，等效到原图约 ×(原图宽/96)，铺满窗口后
/// 远大于这个数）。2.4 只能糊掉细节、人形轮廓还看得清，往上加到 5.5。
const BLUR_SIGMA: f32 = 5.5;

/// 背景模糊图缓存条目。
///
/// `Loading` 只用来「别重复起任务」——等待和写回都由外面那个 task 负责，
/// 所以这里不需要存 task 本身。
enum BlurItem {
    Loading,
    Ready(Arc<RenderImage>),
    Failed,
}

/// 播放页背景用的模糊封面缓存（容量很小：一张 72px 的图才 20KB）。
pub struct BlurCache {
    cap: usize,
    map: HashMap<u64, BlurItem>,
    /// 访问顺序：尾部最新
    order: VecDeque<u64>,
}

impl BlurCache {
    fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(4),
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn touch(&mut self, key: u64) {
        if let Some(pos) = self.order.iter().position(|k| *k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key);
    }

    fn evict(&mut self) {
        while self.map.len() > self.cap {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            self.map.remove(&key);
        }
    }
}

static BLUR_CACHE: std::sync::OnceLock<parking_lot::Mutex<BlurCache>> =
    std::sync::OnceLock::new();

fn blur_cache() -> &'static parking_lot::Mutex<BlurCache> {
    BLUR_CACHE.get_or_init(|| parking_lot::Mutex::new(BlurCache::new(8)))
}

/// 取封面的高斯模糊版本。
///
/// 第一次调用返回 `None` 并在后台起任务（下载 → 解码 → 缩小 → 模糊），
/// 完成后会触发一次重绘；调用方在 `None` 时回退到「原图 + 压暗」即可。
pub fn blurred_cover(url: &str, window: &mut Window, cx: &mut App) -> Option<Arc<RenderImage>> {
    if url.is_empty() {
        return None;
    }
    let key = hash(&url.to_string());
    {
        let mut cache = blur_cache().lock();
        match cache.map.get(&key) {
            Some(BlurItem::Ready(img)) => {
                let img = img.clone();
                cache.touch(key);
                return Some(img);
            }
            // 还在加载 / 之前失败过：不再重复起任务
            Some(_) => return None,
            None => {}
        }
    }

    let resource = Resource::Uri(SharedUri::from(url.to_string()));
    let fut = AssetLogger::<ImageAssetLoader>::load(resource, cx);
    let task = cx
        .background_executor()
        .spawn(async move {
            let src = fut.await.ok()?;
            blur_render_image(&src)
        })
        .shared();

    {
        let mut cache = blur_cache().lock();
        cache.map.insert(key, BlurItem::Loading);
        cache.order.push_back(key);
        cache.evict();
    }

    // 算完要**把结果写回缓存**（否则条目永远停在 Loading，背景永远是回退图），
    // 再通知重绘（否则要等下一次交互才刷新）
    let view = window.current_view();
    window
        .spawn(cx, async move |cx| {
            let res = task.await;
            {
                let mut cache = blur_cache().lock();
                cache.map.insert(
                    key,
                    match res {
                        Some(img) => BlurItem::Ready(img),
                        None => BlurItem::Failed,
                    },
                );
            }
            cx.on_next_frame(move |_, cx| cx.notify(view));
        })
        .detach();
    None
}

/// 把已解码的封面做成小尺寸高斯模糊图。
fn blur_render_image(src: &RenderImage) -> Option<Arc<RenderImage>> {
    let size = src.size(0);
    let (w, h) = (size.width.0 as u32, size.height.0 as u32);
    if w == 0 || h == 0 {
        return None;
    }
    let bytes = src.as_bytes(0)?;
    // 数据在 gpui 里是 BGRA，但模糊逐通道独立，套 Rgba 的壳直接算即可
    let buf = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(w, h, bytes.to_vec())?;
    let nw = BLUR_W;
    let nh = ((h as u32 * nw) / w).max(1);
    let small = image::imageops::resize(&buf, nw, nh, image::imageops::FilterType::Triangle);
    // `image::imageops::blur` 把越界采样当成**透明黑**：直接在原图上模糊，四条
    // 边会被拉黑一圈。铺满整个窗口之后，那一圈就是左右各一条黑边（用户报的
    // 「背景两边有黑色」）。所以先按边缘像素外扩再模糊，最后把外扩裁掉。
    let pad = (BLUR_SIGMA * 3.0).ceil() as u32;
    let padded = pad_edge(&small, pad);
    let blurred = image::imageops::blur(&padded, BLUR_SIGMA);
    let mut cropped = image::imageops::crop_imm(&blurred, pad, pad, nw, nh).to_image();
    // 大 sigma 之后各通道被平均，颜色会往灰里跑；稍微提一点饱和度，
    // 铺满窗口才有「专辑配色」的感觉（0.62 的压暗会再吃掉一部分）。
    boost_saturation(&mut cropped, 1.35);
    let out = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(nw, nh, cropped.into_raw())?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(out)])))
}

/// 提高饱和度（往亮度轴外推）。字节序是 BGRA。
fn boost_saturation(img: &mut image::RgbaImage, k: f32) {
    for px in img.pixels_mut() {
        let (b, g, r) = (px[0] as f32, px[1] as f32, px[2] as f32);
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        px[0] = (luma + (b - luma) * k).clamp(0.0, 255.0) as u8;
        px[1] = (luma + (g - luma) * k).clamp(0.0, 255.0) as u8;
        px[2] = (luma + (r - luma) * k).clamp(0.0, 255.0) as u8;
    }
}

/// 边缘像素外扩（clamp 到边界），避免高斯模糊在图像边缘吸到「透明黑」。
fn pad_edge(src: &image::RgbaImage, pad: u32) -> image::RgbaImage {
    let (w, h) = src.dimensions();
    if pad == 0 {
        return src.clone();
    }
    let mut out = image::RgbaImage::new(w + pad * 2, h + pad * 2);
    for y in 0..out.height() {
        let sy = y.saturating_sub(pad).min(h - 1);
        for x in 0..out.width() {
            let sx = x.saturating_sub(pad).min(w - 1);
            out.put_pixel(x, y, *src.get_pixel(sx, sy));
        }
    }
    out
}
