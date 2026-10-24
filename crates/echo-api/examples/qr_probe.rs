//! 扫码 / 验证码登录接口探测。
use echo_api::KuGouClient;
use echo_core::DeviceIdentity;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let device = DeviceIdentity::generate();
    let client = KuGouClient::shared(device, true)?;
    let _ = client.register_device().await;

    match client.qr_key().await {
        Ok((key, url)) => {
            println!("qr_key OK: key={key}");
            println!("           url={url}");
            for i in 0..3 {
                match client.qr_check(&key).await {
                    Ok(st) => println!(
                        "  check#{i}: status={} token_len={} userid={}",
                        st.status,
                        st.token.len(),
                        st.userid
                    ),
                    Err(e) => println!("  check#{i} ERR {e}"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
        Err(e) => println!("qr_key ERR {e}"),
    }

    // 只验证接口可达性，不发真实短信
    match client.send_sms_code("13800000000").await {
        Ok(()) => println!("send_sms_code OK"),
        Err(e) => println!("send_sms_code ERR {e}"),
    }

    Ok(())
}
