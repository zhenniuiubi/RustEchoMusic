//! 探测新碟结构 + 分类搜索接口。
use echo_api::client::{EncryptType, KuGouClient, RequestSpec};
use echo_core::DeviceIdentity;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let device = DeviceIdentity::generate();
    let client = KuGouClient::shared(device, true)?;
    let _ = client.register_device().await;

    let spec = RequestSpec {
        url: "/musicadservice/v1/mobile_newalbum_sp".into(),
        body: Some(json!({"apiver":5,"token":"","page":1,"pagesize":3,"withpriv":1})),
        encrypt_type: EncryptType::Android,
        ..Default::default()
    };
    if let Ok(v) = client.request(spec).await {
        for region in ["chn", "eur", "jpn", "kor"] {
            let p = format!("/data/{region}");
            if let Some(arr) = v.pointer(&p) {
                if let Some(first) = arr.get(0) {
                    println!("[{region}] len={} sample={}", arr.as_array().map(|a| a.len()).unwrap_or(0),
                        first.to_string().chars().take(400).collect::<String>());
                }
            }
        }
    }

    // 分类搜索探测
    for (name, url) in [
        ("special", "/v2/search/special"),
        ("album", "/v2/search/album"),
        ("singer", "/v2/search/singer"),
    ] {
        let mut spec = RequestSpec {
            url: url.into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("keyword".into(), "周杰伦".into());
        spec.params.insert("page".into(), "1".into());
        spec.params.insert("pagesize".into(), "3".into());
        spec.params.insert("platform".into(), "AndroidFilter".into());
        spec.params.insert("area_code".into(), "1".into());
        spec.x_router = Some("complexsearch.kugou.com".into());
        match client.request(spec).await {
            Ok(v) => {
                let d = v.get("data").cloned().unwrap_or_default();
                let first = d.pointer("/lists/0").or_else(|| d.pointer("/info/0")).map(|x| x.to_string().chars().take(220).collect::<String>());
                println!("[{name}] data_keys={:?} first={:?}",
                    d.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
                    first);
            }
            Err(e) => println!("[{name}] ERR {e}"),
        }
    }
    Ok(())
}
