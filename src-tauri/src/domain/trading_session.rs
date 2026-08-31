//! 交易时段判断（提醒功能需要：非交易时段不提醒，否则半夜会被叫醒）。
//!
//! 纯函数，但**允许 use chrono_tz** —— 这是架构不变量 3 的第二个（也是最后一个）
//! 放开点，理由：美股时段必须按美东本地时间判断，而京美时差随夏令时在 12h/13h 之间变，
//! 硬编码必错。放开范围限本文件，不得扩散到第三处。
//!
//! 判定只用「当地时间的星期与 HHMM」，不涉及节假日日历 ——
//! 免费接口拿不到交易日历，且节假日会因数据不刷新而自然进入"过期"态，
//! 而过期数据本就不参与提醒判定（见 `alert::AlertInput::fresh`）。

use crate::domain::market::Market;
use chrono::{Datelike, TimeZone, Timelike, Weekday};
use chrono_tz::Tz;

/// 各市场所在时区。
fn tz_of(market: Market) -> Tz {
    match market {
        // A股与港股都按北京时间（港股交易时间与北京时间一致）
        Market::Ashare | Market::IndexCn | Market::Hk => chrono_tz::Asia::Shanghai,
        // 美股按美东，夏令时切换由时区库处理
        Market::Us => chrono_tz::America::New_York,
    }
}

/// 该市场当日的交易时段（当地时间 `HHMM` 闭区间）。
fn windows_of(market: Market) -> &'static [(u16, u16)] {
    match market {
        // A股/指数：09:30-11:30 + 13:00-15:00
        Market::Ashare | Market::IndexCn => &[(930, 1130), (1300, 1500)],
        // 港股：09:30-12:00 + 13:00-16:00（午休是 12:00 不是 11:30）
        Market::Hk => &[(930, 1200), (1300, 1600)],
        // 美股常规时段：09:30-16:00 美东（不含盘前盘后）
        Market::Us => &[(930, 1600)],
    }
}

/// 现在是否处于该市场的交易时段。`now_utc_ms` 为 UTC 毫秒时间戳。
///
/// 周末一律返回 false。节假日不判（见模块文档）。
pub fn in_session(market: Market, now_utc_ms: i64) -> bool {
    let tz = tz_of(market);
    let Some(local) = chrono::Utc
        .timestamp_millis_opt(now_utc_ms)
        .single()
        .map(|t| t.with_timezone(&tz))
    else {
        // 时间戳非法时**不提醒**（宁可漏提醒，不要半夜误响）
        return false;
    };

    if matches!(local.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }

    let hhmm = (local.hour() as u16) * 100 + local.minute() as u16;
    windows_of(market)
        .iter()
        .any(|(lo, hi)| hhmm >= *lo && hhmm <= *hi)
}

/// 供测试与诊断：返回该市场当地时间的 `HHMM`。
pub fn local_hhmm(market: Market, now_utc_ms: i64) -> Option<u16> {
    let tz = tz_of(market);
    chrono::Utc
        .timestamp_millis_opt(now_utc_ms)
        .single()
        .map(|t| t.with_timezone(&tz))
        .map(|l| (l.hour() as u16) * 100 + l.minute() as u16)
}
