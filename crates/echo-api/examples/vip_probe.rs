//! VIP 状态 / 领取接口实测：打印原始响应，确认字段语义。
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

    // 与 App 启动流程一致：先注册设备再刷新 token
    let _ = client.register_device().await;
    match client.refresh_token().await {
        Ok(r) => println!("refresh OK vip_type={}", r.vip_type),
        Err(e) => println!("refresh ERR {e}"),
    }

    match client.user_detail().await {
        Ok(u) => println!("user_detail: nick={} vip_type={}", u.nickname, u.vip_type),
        Err(e) => println!("user_detail ERR {e}"),
    }

    match client.vip_detail().await {
        Ok(v) => println!("vip_detail: {}", serde_json::to_string_pretty(&v)?),
        Err(e) => println!("vip_detail ERR {e}"),
    }

    match client.vip_status().await {
        Ok(st) => println!(
            "vip_status: type={} product={} end={} active={}",
            st.vip_type,
            st.product,
            st.end_time,
            st.active()
        ),
        Err(e) => println!("vip_status ERR {e}"),
    }

    match client.vip_claim_records().await {
        Ok(v) => println!("month_record: {}", serde_json::to_string_pretty(&v)?),
        Err(e) => println!("month_record ERR {e}"),
    }

    for day in ["2026-10-24", "2026-10-23"] {
        match client.claim_daily_vip(day).await {
            Ok(v) => println!("claim {day}: OK {}", serde_json::to_string_pretty(&v)?),
            Err(e) => println!(
                "claim {day}: ERR {e} → 分类 {:?}",
                echo_api::classify_claim_error(&e)
            ),
        }
    }

    match client.vip_status().await {
        Ok(st) => println!(
            "after claim vip_status: type={} product={} end={} active={}",
            st.vip_type,
            st.product,
            st.end_time,
            st.active()
        ),
        Err(e) => println!("after claim vip_status ERR {e}"),
    }
    Ok(())
}
