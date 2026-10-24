//! 会员（VIP）相关接口：每日畅听 VIP 领取、听歌上报、升级奖励、领取记录。
//!
//! 端点对齐 `_kgapi/KuGouMusicApi-main/module/youth_*.js`
//! 与参考项目 `_ref/EchoMusic-main/src/renderer/api/user.ts`。
//!
//! 注意：这些接口都**需要登录**，token 过期时统一返回 `20018`（请重新登录）。

use crate::client::{EncryptType, KuGouClient, RequestSpec};
use serde_json::{json, Value};

/// 单个已开通的会员产品（一个账号可能同时有「超级VIP」和「畅听VIP」）。
#[derive(Debug, Clone, Default)]
pub struct VipProduct {
    /// 产品线：svip / tvip / dvip / qvip
    pub product: String,
    /// 展示名：超级VIP / 畅听VIP …
    pub label: String,
    pub vip_type: i64,
    /// 到期时间（unix 秒），0 = 接口没给
    pub end_time: i64,
}

/// 会员状态快照（含到期时间）。
#[derive(Debug, Clone, Default)]
pub struct VipStatus {
    /// 最高等级：0 非会员 / 1 普通 / 2 畅听 / 4 超级
    pub vip_type: i64,
    /// 最高等级对应的产品线（svip / tvip / dvip / qvip …）
    pub product: String,
    /// 所有产品里最晚的到期时间（unix 秒），0 = 接口没给
    pub end_time: i64,
    /// 当前仍有效的产品列表（按等级从高到低）
    pub products: Vec<VipProduct>,
}

impl VipStatus {
    /// 是否当前有效（有到期时间时按时间判断）。
    pub fn active(&self) -> bool {
        if self.vip_type <= 0 {
            return false;
        }
        self.end_time == 0 || self.end_time > chrono::Local::now().timestamp()
    }

    /// 展示用文案：`超级VIP 至 2026-10-25 10:34:28 ｜ 畅听VIP 至 2027-01-30 19:33:35`。
    pub fn detail_text(&self) -> String {
        if self.products.is_empty() {
            return if self.vip_type > 0 {
                "VIP".into()
            } else {
                "非VIP".into()
            };
        }
        self.products
            .iter()
            .map(|p| {
                if p.end_time > 0 {
                    format!(
                        "{} 至 {}",
                        p.label,
                        crate::vip::fmt_ts(p.end_time, "%Y-%m-%d %H:%M:%S")
                    )
                } else {
                    p.label.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ｜ ")
    }
}

/// unix 秒 → 本地时间字符串。
pub fn fmt_ts(ts: i64, fmt: &str) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.with_timezone(&chrono::Local).format(fmt).to_string())
        .unwrap_or_else(|| "-".into())
}

/// 领取接口的业务错误分类（码值实测于 2026-10）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    /// 今天已经领过了（`131001`）
    Already,
    /// 该日期不是可领取的日期（`304003`）
    WrongDay,
    /// 登录态失效
    AuthExpired,
    /// 其它
    Other,
}

/// 解析「领取每日 VIP」失败的语义。
pub fn classify_claim_error(e: &anyhow::Error) -> ClaimError {
    use crate::client::ApiError;
    match e.downcast_ref::<ApiError>() {
        Some(ApiError::Business { error_code, .. }) => match *error_code {
            131001 => ClaimError::Already,
            304003 => ClaimError::WrongDay,
            code if crate::client::AUTH_EXPIRED_CODES.contains(&code) => ClaimError::AuthExpired,
            _ => ClaimError::Other,
        },
        _ => ClaimError::Other,
    }
}

impl KuGouClient {
    /// 拉一次会员状态（`kugouvip.kugou.com/v1/get_union_vip`）。
    ///
    /// 该接口同时给出 `busi_vip[]`（各业务线会员）与到期时间，是展示
    /// 「VIP 到期时间（精确到秒）」的唯一来源；`user_detail` 只有等级。
    pub async fn vip_status(&self) -> anyhow::Result<VipStatus> {
        let detail = self.vip_detail().await?;
        Ok(parse_vip_status(&detail))
    }
}

/// 从 `/v1/get_union_vip` 的响应里解析会员等级与到期时间。
///
/// 字段在不同账号/产品线下不一定齐全，这里按「业务线会员 > 顶层会员」取值，
/// 时间支持 unix 秒与 `YYYY-MM-DD HH:MM:SS` 两种形态。
fn parse_vip_status(detail: &Value) -> VipStatus {
    let mut products: Vec<VipProduct> = Vec::new();

    // 顶层：传统会员（只有等级，没有独立到期时间时靠 su_vip_end_time 兜）
    let top_level = {
        let mut t = detail.get("vip_type").and_then(|v| v.as_i64()).unwrap_or(0);
        if detail.get("is_vip").and_then(|v| v.as_i64()).unwrap_or(0) == 1 {
            t = t.max(1);
        }
        t
    };
    let top_end = ["vip_end_time", "su_vip_end_time", "end_time", "expire_time"]
        .iter()
        .map(|k| parse_time(detail.get(*k)))
        .max()
        .unwrap_or(0);
    if top_level > 0 {
        products.push(VipProduct {
            product: "vip".into(),
            label: vip_label(top_level, "vip"),
            vip_type: top_level,
            end_time: top_end,
        });
    }

    // 业务线会员：svip（超级）/ tvip（畅听）/ dvip / qvip
    if let Some(arr) = detail.get("busi_vip").and_then(|v| v.as_array()) {
        for item in arr {
            if item.get("is_vip").and_then(|v| v.as_i64()).unwrap_or(0) != 1 {
                continue;
            }
            let pt = item
                .get("product_type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let level = match pt.as_str() {
                "svip" | "super_vip" => 4,
                "tvip" | "qvip" | "vip" => 2,
                _ => 1,
            };
            let end = [
                "vip_end_time",
                "end_time",
                "expire_time",
                "paid_vip_expire_time",
            ]
            .iter()
            .map(|k| parse_time(item.get(*k)))
            .max()
            .unwrap_or(0);
            // 到期时间已过的产品不再算「有效」
            if end > 0 && end <= chrono::Local::now().timestamp() {
                continue;
            }
            products.push(VipProduct {
                label: vip_label(level, &pt),
                product: pt,
                vip_type: level,
                end_time: end,
            });
        }
    }

    // 等级高的排前面；同等级按到期时间靠后的排前面
    products.sort_by(|a, b| {
        b.vip_type
            .cmp(&a.vip_type)
            .then(b.end_time.cmp(&a.end_time))
    });

    let vip_type = products.first().map(|p| p.vip_type).unwrap_or(0);
    let product = products.first().map(|p| p.product.clone()).unwrap_or_default();
    let end_time = products.iter().map(|p| p.end_time).max().unwrap_or(0);

    VipStatus {
        vip_type,
        product,
        end_time,
        products,
    }
}

/// 会员产品展示名。
fn vip_label(level: i64, product: &str) -> String {
    match product {
        "svip" | "super_vip" => "超级VIP".into(),
        "tvip" => "畅听VIP".into(),
        "dvip" => "豪华VIP".into(),
        "qvip" => "概念VIP".into(),
        _ => match level {
            4 => "超级VIP".into(),
            3 => "豪华VIP".into(),
            2 => "畅听VIP".into(),
            _ => "VIP".into(),
        },
    }
}

/// 到期时间解析：支持 unix 秒 / 毫秒 / `YYYY-MM-DD HH:MM:SS` / 纯数字字符串。
fn parse_time(v: Option<&Value>) -> i64 {
    let Some(v) = v else { return 0 };
    let raw = match v {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                return 0;
            }
            if let Ok(n) = s.parse::<i64>() {
                n
            } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
                // 服务端给的是北京时间；按 +08:00 解释，避免整体偏移 8 小时
                match chrono::FixedOffset::east_opt(8 * 3600) {
                    Some(off) => dt.and_local_timezone(off).single().map(|d| d.timestamp()).unwrap_or(0),
                    None => dt.and_utc().timestamp(),
                }
            } else if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                let t = d.and_hms_opt(0, 0, 0);
                match (t, chrono::FixedOffset::east_opt(8 * 3600)) {
                    (Some(t), Some(off)) => t.and_local_timezone(off).single().map(|d| d.timestamp()).unwrap_or(0),
                    _ => 0,
                }
            } else {
                return 0;
            }
        }
        _ => return 0,
    };
    // 毫秒 → 秒
    if raw > 100_000_000_000 {
        raw / 1000
    } else {
        raw
    }
}

impl KuGouClient {
    /// 领取「每日畅听 VIP」（`/youth/v1/recharge/receive_vip_listen_song`）。
    ///
    /// `receive_day` 是**日期字符串** `YYYY-MM-DD`（见 KuGouMusicApi
    /// `interface.d.ts::YouthDayVipParams`），传当天日期即可领当天那一份。
    pub async fn claim_daily_vip(&self, receive_day: &str) -> anyhow::Result<Value> {
        let mut spec = RequestSpec {
            url: "/youth/v1/recharge/receive_vip_listen_song".into(),
            encrypt_type: EncryptType::Android,
            // 该接口是 POST + 空表单体：body 给空串才能让底层发 POST
            body: Some(Value::String(String::new())),
            ..Default::default()
        };
        spec.params.insert("source_id".into(), "90139".into());
        spec.params.insert("receive_day".into(), receive_day.to_string());

        let res = self.request(spec).await?;
        Ok(res.get("data").cloned().unwrap_or(res))
    }

    /// 本月 VIP 领取记录（`/youth/v1/activity/get_month_vip_record`）。
    ///
    /// 返回里带已领取的日期 / 天数，可用来判断今天是否已经领过。
    pub async fn vip_claim_records(&self) -> anyhow::Result<Value> {
        let mut spec = RequestSpec {
            url: "/youth/v1/activity/get_month_vip_record".into(),
            encrypt_type: EncryptType::Android,
            ..Default::default()
        };
        spec.params.insert("latest_limit".into(), "100".into());

        let res = self.request(spec).await?;
        Ok(res.get("data").cloned().unwrap_or(res))
    }

    /// 升级为「概念会员」的每日奖励（`/youth/v1/listen_song/upgrade_vip_reward`）。
    pub async fn claim_upgrade_reward(&self) -> anyhow::Result<Value> {
        let userid = self.auth().userid;
        let mut spec = RequestSpec {
            url: "/youth/v1/listen_song/upgrade_vip_reward".into(),
            encrypt_type: EncryptType::Android,
            body: Some(Value::String(String::new())),
            ..Default::default()
        };
        spec.params.insert("kugouid".into(), userid.to_string());
        spec.params.insert("ad_type".into(), "1".into());

        let res = self.request(spec).await?;
        Ok(res.get("data").cloned().unwrap_or(res))
    }

    /// 听歌上报（`/youth/v2/report/listen_song`）：累计听歌时长可兑换 VIP 天数。
    ///
    /// `mixsongid` 为歌曲的 mixsongid（`album_audio_id` 优先，缺失时用 hash 查询到的值）。
    pub async fn report_listen_song(&self, mixsongid: i64) -> anyhow::Result<Value> {
        let mut spec = RequestSpec {
            url: "/youth/v2/report/listen_song".into(),
            encrypt_type: EncryptType::Android,
            body: Some(json!({ "mixsongid": mixsongid })),
            user_agent: Some(
                "Android13-1070-10566-201-0-ReportPlaySongToServerProtocol-wifi".into(),
            ),
            ..Default::default()
        };
        spec.params.insert("clientver".into(), "10566".into());

        let res = self.request(spec).await?;
        Ok(res.get("data").cloned().unwrap_or(res))
    }

    /// 开屏广告播放上报（`/youth/v1/ad/play_report`）：看广告也能领 VIP。
    ///
    /// 上报的是一次 30 秒的播放窗口；调用方应真实等过这段时长再上报。
    pub async fn report_vip_ad(&self) -> anyhow::Result<Value> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        let spec = RequestSpec {
            url: "/youth/v1/ad/play_report".into(),
            encrypt_type: EncryptType::Android,
            body: Some(json!({
                "ad_id": 12307537187i64,
                "play_end": now,
                "play_start": now - 30_000,
            })),
            ..Default::default()
        };
        let res = self.request(spec).await?;
        Ok(res.get("data").cloned().unwrap_or(res))
    }
}
