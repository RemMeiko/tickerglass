// 接口用法参考 StockPet (MIT)，实现为本项目原创
//
// `trends2/get?secid={quoteID}` 返回 JSON：
//   data.trends — 字符串数组，每项逗号分隔 `时间,开,收,高,低,成交量,成交额,均价`
//   时间格式 `yyyy-MM-dd HH:mm`
//
// **本文件是全项目唯一允许 use chrono_tz 的地方**（合同决策 13 的范围限定）。
// 原因见下方 bj_to_us_east_hhmm。

use crate::domain::market::Market;
use crate::domain::session::{attach_grid, grid_len};
use crate::model::{MinuteSeries, ParseError, RawPoint};
// 复用腾讯 5 日路径的输出类型：两条源产出同构结果，前端只有一套渲染
use crate::parse::tencent_5day::{DayMinutePoint, DaySeries};
use chrono::{NaiveDateTime, TimeZone, Timelike};
use chrono_tz::{America::New_York, Asia::Shanghai};
use serde_json::Value;

const FMT_EM: &str = "%Y-%m-%d %H:%M";

/// 东财 trends2 的时间戳是**北京时间**（实测：首点 `2026-08-21 21:30` 即美东 09:30 EDT 开盘），
/// 而美股栅格是 391 格**美东**刻度（09:30–16:00）。不换算的话所有点都落在栅格外被丢弃，
/// 苹果那一行就画不出曲线。
///
/// 换算走 `America/New_York` 时区数据库，**不硬编码时差** —— 京美时差夏令时 12h、
/// 冬令时 13h，写死 `-12h` 会让 11 月起的美股曲线整体横向偏移 60 格。
/// 因此本函数里不出现任何月份或小时数常量。
pub fn bj_to_us_east_hhmm(raw: &str) -> Option<u16> {
    let naive = NaiveDateTime::parse_from_str(raw.trim(), FMT_EM).ok()?;
    // 北京时间无夏令时，不存在歧义/不存在时刻，single() 足够
    let bj = Shanghai.from_local_datetime(&naive).single()?;
    let ny = bj.with_timezone(&New_York);
    Some(ny.hour() as u16 * 100 + ny.minute() as u16)
}

/// 同 [`bj_to_us_east_hhmm`]，但连**美东日期**一起返回（`YYYYMMDD`, `HHMM`）。
///
/// 5 日分时必须按**美东日期**分组，不能按报文里的北京日期分：
/// 一个美股交易日（美东 09:30–16:00）跨两个北京日期
/// （实测 `2026-08-25 21:30` 开盘 → `2026-08-26 04:00` 收盘）。
/// 按北京日期分组会把同一个交易日劈成两段、并把相邻两日的尾头拼到一起。
fn bj_to_us_east_date_hhmm(raw: &str) -> Option<(String, u16)> {
    let naive = NaiveDateTime::parse_from_str(raw.trim(), FMT_EM).ok()?;
    let bj = Shanghai.from_local_datetime(&naive).single()?;
    let ny = bj.with_timezone(&New_York);
    Some((
        ny.format("%Y%m%d").to_string(),
        ny.hour() as u16 * 100 + ny.minute() as u16,
    ))
}

/// 东财 trends2 的 5 日分时解析（**美股专用**）。
///
/// 产出与腾讯 5 日路径**同构**的 `Vec<DaySeries>`，前端 `IntradayLarge` 完全复用，
/// 不为美股开第二条渲染路径。
///
/// # 与腾讯路径的三处实测差异（2026-08-26，`105.AAPL`）
///
/// 1. **成交量是逐分钟值，不是累计值。** 实测首四点 784496 / 370330 / 155142 / 133847
///    上下起伏。腾讯那条是累计值必须差分 —— 这里若照抄差分会得到一堆 0 和负数。
/// 2. **响应不含 `preClose`。** 腾讯每天带 `prec` 字段；这里没有，故第 2~5 天的基准线
///    用**前一交易日的最后一个价**推导。最早那天没有前一日可依据，`prev_close` 记 `None`
///    （前端会退化用该段首点当基准），不猜。
/// 3. **时间戳是北京时间**，且一个美股交易日跨两个北京日期 —— 见 [`bj_to_us_east_date_hhmm`]。
///
/// 返回按日期**升序**（最早的在前），与前端 `segments[0]` 画在最左侧的约定一致。
///
/// `in_session` 由调用方注入（交易时段属 domain 层职责，本文件不得硬编码时段）。
pub fn parse_trends_5day(
    json_text: &str,
    in_session: impl Fn(u16) -> bool,
) -> Result<Vec<DaySeries>, ParseError> {
    let trimmed = json_text.trim_start_matches('\u{feff}').trim();
    if !trimmed.starts_with('{') {
        return Err(ParseError::NotJson);
    }
    let root: Value = serde_json::from_str(trimmed).map_err(|_| ParseError::NotJson)?;

    // 无效 secid 时 data 为 null —— 与 raw_points 同样的防线，不退化为造点
    let data = root
        .get("data")
        .filter(|v| !v.is_null())
        .ok_or_else(|| ParseError::Structure("data 为 null".to_string()))?;
    let trends = data
        .get("trends")
        .and_then(|t| t.as_array())
        .ok_or_else(|| ParseError::Structure("缺少 data.trends".to_string()))?;

    // 按美东日期分组。BTreeMap 天然按日期字符串升序（YYYYMMDD 字典序即时序）
    let mut by_day: std::collections::BTreeMap<String, Vec<DayMinutePoint>> =
        std::collections::BTreeMap::new();

    for item in trends.iter().filter_map(|v| v.as_str()) {
        let parts: Vec<&str> = item.split(',').collect();
        if parts.len() < 6 {
            continue;
        }
        let Some((date, hhmm)) = bj_to_us_east_date_hhmm(parts[0]) else {
            continue;
        };
        // 栅格外的点丢弃（与当日分时同一处理）
        if !in_session(hhmm) {
            continue;
        }
        // 价格取「收」字段（索引 2），与 parse_trend_item 保持一致
        let Ok(price) = parts[2].trim().parse::<f64>() else {
            continue;
        };
        if !price.is_finite() || price <= 0.0 {
            continue;
        }
        // 索引 5 是成交量，已是逐分钟值，**不要差分**
        let volume = parts[5]
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(0.0);

        by_day.entry(date).or_default().push(DayMinutePoint {
            hhmm,
            price,
            volume,
        });
    }

    if by_day.is_empty() {
        return Err(ParseError::Structure("trends 无可用点".to_string()));
    }

    // 逐日装配，并用前一日最后一个价当本日昨收
    let mut out: Vec<DaySeries> = Vec::with_capacity(by_day.len());
    let mut prev_day_last: Option<f64> = None;
    for (date, mut points) in by_day {
        // 同一日内按刻度升序（报文本身有序，但不依赖这个假设）
        points.sort_by_key(|p| p.hhmm);
        let last = points.last().map(|p| p.price);
        out.push(DaySeries {
            date,
            prev_close: prev_day_last,
            points,
        });
        prev_day_last = last;
    }

    Ok(out)
}

/// 解析一项 `时间,开,收,高,低,成交量,成交额,均价`。价格取「收」字段（索引 2）。
fn parse_trend_item(item: &str) -> Option<RawPoint> {
    let parts: Vec<&str> = item.split(',').collect();
    if parts.len() < 3 {
        return None;
    }
    let hhmm = bj_to_us_east_hhmm(parts[0])?;
    let price = parts[2].trim().parse::<f64>().ok()?;
    if !price.is_finite() || price <= 0.0 {
        return None;
    }
    // 第 8 段（索引 7）是均价，存在则带上（Sprint 1 下发但不绘制）
    let avg = parts
        .get(7)
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0);
    Some(RawPoint { hhmm, price, avg })
}

/// 落栅格之前的裸点（标准 4 要求记录「东财原始返回点数」，且落格断言需要它）。
pub fn raw_points(json_text: &str) -> Result<Vec<RawPoint>, ParseError> {
    let trimmed = json_text.trim_start_matches('\u{feff}').trim();
    if !trimmed.starts_with('{') {
        return Err(ParseError::NotJson);
    }
    let root: Value = serde_json::from_str(trimmed).map_err(|_| ParseError::NotJson)?;

    // 实测无效 secid 时 `data` 为 null —— 必须走 Structure 而不是退化为造点
    let data = root
        .get("data")
        .filter(|v| !v.is_null())
        .ok_or_else(|| ParseError::Structure("data 为 null".to_string()))?;
    let trends = data
        .get("trends")
        .and_then(|t| t.as_array())
        .ok_or_else(|| ParseError::Structure("缺少 data.trends".to_string()))?;

    Ok(trends
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(parse_trend_item)
        .collect())
}

/// 东财 trends2 分时解析。输出与腾讯路径**同构**的 `MinuteSeries`，
/// 下游 `session` 与前端绘制完全复用，不为美股开第二条渲染路径。
///
/// `secid` 仅用于校验/日志；产出的 `code` 用调用方给的腾讯身份键。
pub fn parse_trends(json_text: &str, code: &str) -> Result<MinuteSeries, ParseError> {
    let raw = raw_points(json_text)?;
    let market = Market::Us;
    Ok(MinuteSeries {
        code: code.to_string(),
        market,
        grid_len: grid_len(market),
        // trends2 响应不含昨收，基准线由报价通道的昨收补（在 scheduler 装配处）
        prev_close: None,
        points: attach_grid(market, &raw),
    })
}
