//! EchoMusic 酷狗 API 客户端。
//!
//! 直接以 Rust 实现酷狗网关协议（签名/加密/设备身份），
//! 替代原项目的 Electron 本地服务（KuGouMusicApi JS 子模块）。

pub mod api;
pub mod client;
pub mod crypto;
pub mod normalize;
pub mod lyrics;
pub mod userlib;
pub mod vip;

pub use api::{
    resolve_urls_from_response, FmBatch, LoginResult, QrState,
};
pub use client::{
    is_auth_expired, ApiError, AuthState, EncryptType, KuGouClient, RequestSpec,
};
pub use vip::{classify_claim_error, fmt_ts, ClaimError, VipProduct, VipStatus};
