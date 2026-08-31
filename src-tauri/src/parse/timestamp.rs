use crate::domain::market::Market;
use chrono::NaiveDateTime;

/// 实测同一批响应里 `[30]` 字段有三种格式：
/// - A 股/指数 `20260821161402`
/// - 港股 `2026/08/21 16:08:14`
/// - 美股 `2026-08-21 10:20:41`
const FMT_COMPACT: &str = "%Y%m%d%H%M%S";
const FMT_SLASH: &str = "%Y/%m/%d %H:%M:%S";
const FMT_DASH: &str = "%Y-%m-%d %H:%M:%S";

/// 按市场选首选格式，失败时把其余两种各试一次
/// （接口哪天换格式不至于全盘失效），全失败返回 `None`。
pub fn parse_quote_ts(raw: &str, market: Market) -> Option<NaiveDateTime> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let preferred = match market {
        Market::Ashare | Market::IndexCn => FMT_COMPACT,
        Market::Hk => FMT_SLASH,
        Market::Us => FMT_DASH,
    };

    let mut order = vec![preferred];
    for f in [FMT_COMPACT, FMT_SLASH, FMT_DASH] {
        if f != preferred {
            order.push(f);
        }
    }

    order
        .into_iter()
        .find_map(|f| NaiveDateTime::parse_from_str(raw, f).ok())
}

/// `HH:MM` 展示形式。
pub fn format_hhmm(dt: &NaiveDateTime) -> String {
    dt.format("%H:%M").to_string()
}
