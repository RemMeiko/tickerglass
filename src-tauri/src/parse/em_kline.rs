//! 东财日K 解析（`push2his.eastmoney.com/api/qt/stock/kline/get`）。
//!
//! 纯函数：不 import reqwest/tauri，可离线单测（架构不变量 2）。
//!
//! # 为什么用东财而不是腾讯做日K主源
//!
//! 腾讯 `fqkline/get` 有两个问题：
//! 1. **美股必须带 `.OQ` 后缀** —— `usAAPL` 只返 2 条，`usAAPL.OQ` 才返 60 条。
//!    而 `.OQ` 是腾讯报价字段 `[2]` 的值，Sprint 1 已明确它**不能作身份键**；
//!    走腾讯日K 等于让日K 依赖报价先到才能拼出代码，多一层时序耦合。
//! 2. **不带成交量** —— 行格式只到 `[日期,开,收,高,低,量]` 的"量"字段在部分市场缺失，
//!    画不了量柱。
//!
//! 东财 `klines` 用已存在 `WatchItem.em_secid` 的 secid（如 `105.AAPL`），
//! 且每行自带成交量与成交额。
//!
//! # 响应结构（实测 2026-08-24）
//!
//! ```json
//! { "rc": 0, "data": { "name": "苹果", "klines": [
//!     "2026-08-21,312.050,309.350,312.380,307.010,46876815,14517834752.000,1.73"
//! ] } }
//! ```
//!
//! 逗号分隔 8 段：`日期,开,收,高,低,成交量,成交额,涨跌幅`。
//! ⚠️ 注意 **第 2 段是收盘价、第 3 段才是最高价** —— 顺序是「开收高低」不是「开高低收」，
//! 按 OHLC 惯例直读会把收盘价当最高价。

use serde::Serialize;

/// 一根日K。字段顺序按 OHLC 惯例排列（与报文的「开收高低」顺序不同，解析时已换位）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KlineBar {
    /// `YYYY-MM-DD`
    pub date: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    /// 成交量（手/股，随市场而异，只用于画相对高度的量柱，不做单位换算）
    pub volume: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KlineParseError {
    /// 响应不是 JSON，或顶层结构不符
    Malformed(String),
    /// 接口返回了错误码
    Rejected(String),
    /// 结构合法但没有任何可用K线
    Empty,
}

/// 解析东财日K 响应。
///
/// 宽容原则：单行残缺（段数不足、非数字）**跳过该行而不是整批失败** ——
/// 一根坏K线不该让整张图消失。但若所有行都不可用则返回 `Empty`，
/// 避免把空图当成"这只股票没有历史"。
pub fn parse_em_kline(text: &str) -> Result<Vec<KlineBar>, KlineParseError> {
    let v: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| KlineParseError::Malformed(format!("非 JSON: {e}")))?;

    // rc 非 0 视为接口拒绝（实测正常为 0）
    if let Some(rc) = v.get("rc").and_then(|x| x.as_i64()) {
        if rc != 0 {
            return Err(KlineParseError::Rejected(format!("rc={rc}")));
        }
    }

    let klines = v
        .get("data")
        .and_then(|d| d.get("klines"))
        .and_then(|k| k.as_array())
        .ok_or_else(|| KlineParseError::Malformed("缺 data.klines 数组".into()))?;

    let bars: Vec<KlineBar> = klines
        .iter()
        .filter_map(|row| row.as_str())
        .filter_map(parse_kline_row)
        .collect();

    if bars.is_empty() {
        return Err(KlineParseError::Empty);
    }
    Ok(bars)
}

/// 解析单行。段序是 `日期,开,收,高,低,量,额,涨跌幅`（**开收高低**，非 OHLC 顺序）。
fn parse_kline_row(row: &str) -> Option<KlineBar> {
    let f: Vec<&str> = row.split(',').collect();
    if f.len() < 6 {
        return None;
    }
    let date = f[0].trim();
    if date.is_empty() {
        return None;
    }
    let open = num(f[1])?;
    let close = num(f[2])?; // ← 第 2 段是收，不是高
    let high = num(f[3])?;
    let low = num(f[4])?;
    let volume = num(f[5]).unwrap_or(0.0);

    // 价格必须为正；高低颠倒的行视为坏数据丢弃（宁可少一根，不画出错的影线）
    if open <= 0.0 || close <= 0.0 || high <= 0.0 || low <= 0.0 || high < low {
        return None;
    }
    Some(KlineBar {
        date: date.to_string(),
        open,
        high,
        low,
        close,
        volume,
    })
}

fn num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() || t == "-" {
        return None;
    }
    t.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// 简单移动均线。返回与输入等长的序列，前 `period-1` 位为 `None`（不足窗口不编造值）。
pub fn moving_average(bars: &[KlineBar], period: usize) -> Vec<Option<f64>> {
    if period == 0 {
        return vec![None; bars.len()];
    }
    let mut out = Vec::with_capacity(bars.len());
    let mut sum = 0.0;
    for (i, b) in bars.iter().enumerate() {
        sum += b.close;
        if i + 1 > period {
            sum -= bars[i - period].close;
        }
        out.push(if i + 1 >= period {
            Some(sum / period as f64)
        } else {
            None
        });
    }
    out
}
