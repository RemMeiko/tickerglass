//! 腾讯 5 日分时解析（`web.ifzq.gtimg.cn/appstock/app/day/query`）。
//!
//! 纯函数：不 import reqwest/tauri，可离线单测（架构不变量 2）。
//! 东财没有对应的 5 日分时通道，故这条只有腾讯一个源。
//!
//! # 响应结构（实测 2026-08-24，`sh600519`）
//!
//! ```json
//! { "code": 0, "data": { "sh600519": {
//!     "data": [ { "date": "20260821",
//!                 "data": ["0930 1291.50 167 21568050.00", ...],
//!                 "prec": "1291.50" }, ... ],
//!     "qt": {...} } } }
//! ```
//!
//! 每天一个对象，`data` 是行数组，每行空格分隔四段：
//! `HHMM 价 累计成交量 累计成交额`。`prec` 是**当日昨收**（每天各有一个）。
//!
//! # 两个实测坑
//!
//! 1. **量与额是当日累计值不是单笔** —— 画量柱要做逐分钟差分，直接用会得到单调递增的斜坡。
//! 2. **A股会带盘后重复点**（`1506`~`1530`，价格恒等于收盘价）。这些点不在交易栅格内，
//!    与 Sprint 1 在当日分时上踩的是同一个坑，须丢弃且不参与 Y 轴极值。

use serde::Serialize;

/// 5 日分时里的一分钟。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayMinutePoint {
    /// `HHMM` 数值形式，如 `930`、`1130`、`1500`
    pub hhmm: u16,
    pub price: f64,
    /// 本分钟成交量（已由累计值差分得出，非报文原值）
    pub volume: f64,
}

/// 一个交易日的分时序列。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DaySeries {
    /// `YYYYMMDD`
    pub date: String,
    /// 当日昨收，用于画该日的基准线
    pub prev_close: Option<f64>,
    pub points: Vec<DayMinutePoint>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FiveDayParseError {
    Malformed(String),
    Rejected(String),
    Empty,
}

/// 解析 5 日分时。`code` 用于从 `data` 对象里取出对应节点（响应以代码为键）。
///
/// `session_hhmm` 判定某个 `HHMM` 是否属于该市场的交易时段 ——
/// 由调用方传入而非在此硬编码，因为交易时段属 `domain` 层职责，
/// 且前端不得出现时段常量（架构不变量 4）。
pub fn parse_5day(
    text: &str,
    code: &str,
    in_session: impl Fn(u16) -> bool,
) -> Result<Vec<DaySeries>, FiveDayParseError> {
    let v: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| FiveDayParseError::Malformed(format!("非 JSON: {e}")))?;

    if let Some(c) = v.get("code").and_then(|x| x.as_i64()) {
        if c != 0 {
            return Err(FiveDayParseError::Rejected(format!("code={c}")));
        }
    }

    let node = v
        .get("data")
        .and_then(|d| d.get(code))
        .ok_or_else(|| FiveDayParseError::Malformed(format!("缺 data.{code}")))?;

    let days = node
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| FiveDayParseError::Malformed("缺 data 数组".into()))?;

    let mut series: Vec<DaySeries> = days
        .iter()
        .filter_map(|day| parse_one_day(day, &in_session))
        .collect();

    if series.is_empty() {
        return Err(FiveDayParseError::Empty);
    }

    // 按日期升序（最早在前）。
    //
    // 腾讯返回的是**最新在前**（实测 20260826, 20260825, 20260824, 20260821, 20260820），
    // 而前端 `IntradayLarge` 把 `segments[0]` 画在**最左侧** ——
    // 不排序的话 5 日图会左边是今天、右边是最早那天，时间轴整体倒着走。
    // 链路上原先没有任何一处排序（解析器 / ipc / 前端都没有），所以这是真的画反了。
    // `YYYYMMDD` 的字典序即时序，直接按字符串排即可。
    series.sort_by(|a, b| a.date.cmp(&b.date));
    Ok(series)
}

fn parse_one_day(day: &serde_json::Value, in_session: &impl Fn(u16) -> bool) -> Option<DaySeries> {
    let date = day.get("date").and_then(|x| x.as_str())?.trim().to_string();
    if date.is_empty() {
        return None;
    }
    let prev_close = day
        .get("prec")
        .and_then(|x| x.as_str())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|x| *x > 0.0 && x.is_finite());

    let rows = day.get("data").and_then(|x| x.as_array())?;

    // 先原样解析成 (hhmm, price, 累计量)，再差分
    let mut raw: Vec<(u16, f64, f64)> = Vec::with_capacity(rows.len());
    for r in rows {
        let line = match r.as_str() {
            Some(s) => s,
            None => continue,
        };
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3 {
            continue;
        }
        let hhmm = match f[0].trim().parse::<u16>() {
            Ok(x) => x,
            Err(_) => continue,
        };
        // 盘后重复点（A股 1506~1530）不在交易栅格内，丢弃 —— 与当日分时同一处理
        if !in_session(hhmm) {
            continue;
        }
        let price = match f[1].trim().parse::<f64>() {
            Ok(x) if x > 0.0 && x.is_finite() => x,
            _ => continue,
        };
        let cum_vol = f[2].trim().parse::<f64>().unwrap_or(0.0);
        raw.push((hhmm, price, cum_vol));
    }
    if raw.is_empty() {
        return None;
    }

    // 累计量 → 逐分钟量。首点的量就是它自己的累计值。
    // 若出现累计值回退（数据抖动），该分钟量记 0 而不是负数。
    let mut points = Vec::with_capacity(raw.len());
    let mut prev_cum = 0.0;
    for (hhmm, price, cum) in raw {
        let vol = (cum - prev_cum).max(0.0);
        prev_cum = cum;
        points.push(DayMinutePoint {
            hhmm,
            price,
            volume: vol,
        });
    }

    Some(DaySeries {
        date,
        prev_close,
        points,
    })
}
