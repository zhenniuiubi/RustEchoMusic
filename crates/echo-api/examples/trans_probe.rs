//! 探测酷狗歌词翻译机制（结论：fmt=trans 无效、候选列表不含翻译版；
//! 翻译走的独立接口需要抓包校准，暂未接入 —— 见 docs/plan.md 第三期降级表）。
use echo_api::client::{EncryptType, RequestSpec};
use echo_api::KuGouClient;
use echo_core::Song;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("EchoMusic");
    let device = echo_core::DeviceIdentity::load_or_create(&dir)?;
    let client = KuGouClient::shared(device, false)?;

    let song = Song {
        title: "晴天".into(),
        artist: "周杰伦".into(),
        duration: 269,
        hash: "e9e6225c1d1c4f1b96a9f9b7d5aa44d3".into(),
        ..Default::default()
    };

    // 1) 搜索候选：打印第一条的完整 JSON（看有没有 trans/罗马音字段）
    let mut spec = RequestSpec {
        url: "/search".into(),
        base_url: Some("https://lyrics.kugou.com".into()),
        encrypt_type: EncryptType::None,
        skip_user_auth: true,
        clear_default_params: true,
        ..Default::default()
    };
    spec.params.insert("ver".into(), "1".into());
    spec.params.insert("man".into(), "yes".into());
    spec.params.insert("client".into(), "pc".into());
    spec.params.insert("keyword".into(), "周杰伦 晴天".into());
    spec.params.insert("duration".into(), "269000".into());
    let text = client.request_text(spec).await?;
    let v: serde_json::Value = serde_json::from_str(&text)?;
    let first = v
        .pointer("/candidates/0")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    println!("=== candidate[0] 全字段 ===");
    println!("{}", serde_json::to_string_pretty(&first)?);
    let id = first
        .get("id")
        .and_then(|x| x.as_i64().map(|n| n.to_string()).or_else(|| x.as_str().map(str::to_string)))
        .unwrap_or_default();
    let key = first.get("accesskey").and_then(|x| x.as_str()).unwrap_or("").to_string();
    println!("id={id} accesskey={key}");

    // 2) 尝试 fmt=trans / fmt=krc 的下载响应顶层字段
    for fmt in ["trans", "krc", "lrc"] {
        let mut spec = RequestSpec {
            url: "/download".into(),
            base_url: Some("https://lyrics.kugou.com".into()),
            encrypt_type: EncryptType::Android,
            skip_user_auth: true,
            ..Default::default()
        };
        spec.params.insert("ver".into(), "1".into());
        spec.params.insert("client".into(), "android".into());
        spec.params.insert("id".into(), id.clone());
        spec.params.insert("accesskey".into(), key.clone());
        spec.params.insert("fmt".into(), fmt.into());
        spec.params.insert("charset".into(), "utf8".into());
        match client.request_text(spec).await {
            Ok(t) => {
                let v: serde_json::Value = serde_json::from_str(&t)?;
                let top: Vec<String> = v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
                let status = v.get("status").and_then(|x| x.as_i64()).unwrap_or(-1);
                let ec = v.get("error_code").and_then(|x| x.as_i64()).unwrap_or(-1);
                let content_len = v.get("content").and_then(|x| x.as_str()).map(|s| s.len()).unwrap_or(0);
                println!("fmt={fmt}: keys={top:?} status={status} err={ec} content_len={content_len}");
                if fmt == "trans" && content_len > 0 {
                    // 解出前几行看看是什么
                    let content = v.get("content").and_then(|x| x.as_str()).unwrap_or("");
                    use base64::Engine as _;
                    let bytes = base64::engine::general_purpose::STANDARD.decode(content.trim().as_bytes()).unwrap_or_default();
                    let text = String::from_utf8_lossy(&bytes);
                    for line in text.lines().take(6) {
                        println!("  | {line}");
                    }
                }
            }
            Err(e) => println!("fmt={fmt}: ERR {e}"),
        }
    }
    Ok(())
}
