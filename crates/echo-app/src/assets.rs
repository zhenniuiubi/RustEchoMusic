//! 资源源：内置 SVG 图标 + 基于 reqwest 的 HTTP 客户端（封面图加载）。

use futures::future::BoxFuture;
use gpui::http_client::{AsyncBody, HttpClient};
use gpui::{AssetSource, SharedString};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

/// 图标资源源。
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if let Some(svg) = crate::icons::icon_svg(path) {
            return Ok(Some(Cow::Borrowed(svg.as_bytes())));
        }
        // Lite 播放条「彩虹猫」形态的像素猫头（26×20 素材，2x 图）
        if path == "lite/nyan-cat.png" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/lite/nyan-cat.png"
            ))));
        }
        Ok(None)
    }

    fn list(&self, _path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

/// 基于 reqwest::blocking 的 HttpClient 实现（在后台线程池执行）。
///
/// gpui 的 smol 运行时无法直接驱动 reqwest(async)（需要 tokio reactor），
/// 这里用 `smol::unblock` 把同步请求丢到阻塞线程池，请求/响应体均在内存中缓冲。
pub struct ReqwestHttpClient {
    inner: reqwest::blocking::Client,
    user_agent: gpui::http_client::http::HeaderValue,
}

impl ReqwestHttpClient {
    pub fn new() -> anyhow::Result<Arc<Self>> {
        let inner = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        Ok(Arc::new(Self {
            inner,
            user_agent: gpui::http_client::http::HeaderValue::from_static("EchoMusic/1.0"),
        }))
    }
}

impl HttpClient for ReqwestHttpClient {
    fn type_name(&self) -> &'static str {
        "ReqwestHttpClient"
    }

    fn user_agent(&self) -> Option<&gpui::http_client::http::HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&gpui::http_client::Url> {
        None
    }

    fn send(
        &self,
        req: gpui::http_client::http::Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<gpui::http_client::http::Response<AsyncBody>>> {
        use futures::AsyncReadExt;

        let method = req.method().clone();
        let url = req.uri().to_string();
        let headers = req.headers().clone();
        let mut body = req.into_body();
        let client = self.inner.clone();

        Box::pin(smol::unblock(move || {
            // 读取请求体（封面加载均为 GET/空体，POST 场景体也很小）
            let mut body_bytes = Vec::new();
            futures::executor::block_on(async { body.read_to_end(&mut body_bytes).await })?;

            let mut r = client.request(
                reqwest::Method::from_bytes(method.as_str().as_bytes())
                    .unwrap_or(reqwest::Method::GET),
                &url,
            );
            for (k, v) in headers.iter() {
                if let Ok(vs) = v.to_str() {
                    r = r.header(k.as_str(), vs);
                }
            }
            if !body_bytes.is_empty() {
                r = r.body(body_bytes);
            }
            let resp = r.send()?;
            let status = resp.status();
            let resp_headers = resp.headers().clone();
            let bytes = resp.bytes()?;

            let mut builder = gpui::http_client::http::Response::builder().status(
                gpui::http_client::http::StatusCode::from_u16(status.as_u16())?,
            );
            for (k, v) in resp_headers.iter() {
                if let Ok(vs) = v.to_str() {
                    builder = builder.header(k.as_str(), vs);
                }
            }
            let response = builder.body(AsyncBody::from_bytes(bytes))?;
            Ok(response)
        }))
    }
}
