// 接口用法参考 StockPet (MIT)，实现为本项目原创
//
// `minute/query?code={code}` 返回 JSON：
//   data.{code}.data.data — 分钟点数组，每项 `"HHMM price volume [amount]"`（空格分隔）
//   data.{code}.qt.{code} — 与批量报价同构的字段数组，昨收在 [4]（分时基准线来源）

use crate::domain::market::{classify, Market};
use crate::domain::session::{attach_grid, grid_len};
use crate::model::{MinuteSeries, ParseError, RawPoint};
use serde_json::Value;

/// 从 `qt.{code}[4]` 取昨收作为基准线来源。取不到返回 `None`（不画基准线，不编造）。
fn prev_close_from_qt(node: &Value, code: &str) -> Option<f64> {
    let arr = node.get("qt")?.get(code)?.as_array()?;
    arr.get(4)?
        .as_str()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
}

/// 解析一项 `"HHMM price cum_volume cum_amount"`。价格非法/为 0 时返回 `None`，
/// 由 `attach_grid` 统一丢弃（0 价会把 Y 轴压到 0）。
///
/// 注意必须按**单空格**切分而不是 `split_whitespace()`：后者会把连续空格折叠，
/// 于是价格字段为空的点（`"0941  91500375 …"`）会把**累计成交量**当成价格读进来，
/// 得到一个天文数字的假价格。按位置切分才能让空价格如实变成 `None`。
///
/// # 均价线（v0.3.0 需求 1）—— 量纲是**按市场**实测认定的，不能一处通用
///
/// 报文后两段是累计成交量与累计成交额（2026-09-16 盘中实测，茅台首点
/// `74手×100×1273.93 = 9,427,082元` 与报文分毫不差）：
/// - A 股：量纲**手**（×100 股），均价 = 累计额 / (累计量 × 100)
/// - 港股：量纲**股**（00700 首点 `935777股`、`411306252元`，除出来 439.75
///   落在当日价格区间内），均价 = 累计额 / 累计量
/// - 指数：额/量 得到的"平均股价"与指数点位**不同量纲**（上证首点 1415.88
///   vs 点位 3861），画上去会压垮 Y 轴 —— 指数一律 `avg: None`，不臆造。
fn parse_point(item: &str, market: Market) -> Option<RawPoint> {
    let mut seg = item.split(' ');
    let hhmm = seg.next()?.trim().parse::<u16>().ok()?;
    let price = seg.next()?.trim().parse::<f64>().ok()?;
    if !price.is_finite() || price <= 0.0 {
        return None;
    }
    let cum_vol = seg
        .next()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0);
    let cum_amt = seg
        .next()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0);
    // 指数不算均价（见函数文档）；量或额缺失时该点均价留空，价格线照画
    let avg = match (market, cum_vol, cum_amt) {
        (Market::Ashare, Some(v), Some(a)) => Some(a / (v * 100.0)),
        (Market::Hk, Some(v), Some(a)) => Some(a / v),
        _ => None,
    }
    .filter(|v| v.is_finite() && *v > 0.0);
    Some(RawPoint { hhmm, price, avg })
}

/// 腾讯当日分时解析。
///
/// 容错要求：`data` 为空数组（盘前）返回**空序列而非 Err**（昨收仍可用于画基准线）；
/// 缺 `data.{code}` 返回 `Structure`；非 JSON 返回 `NotJson`。任何情况不 panic。
pub fn parse_minute(json_text: &str, code: &str) -> Result<MinuteSeries, ParseError> {
    let trimmed = json_text.trim_start_matches('\u{feff}').trim();
    if !trimmed.starts_with('{') {
        return Err(ParseError::NotJson);
    }
    let root: Value = serde_json::from_str(trimmed).map_err(|_| ParseError::NotJson)?;

    let node = root
        .get("data")
        .and_then(|d| d.get(code))
        .ok_or_else(|| ParseError::Structure(format!("缺少 data.{code}")))?;

    let market = classify(code);
    let prev_close = prev_close_from_qt(node, code);

    // `data.data` 缺失视为结构错误；存在但为空数组视为盘前（合法空序列）
    let items = node
        .get("data")
        .and_then(|d| d.get("data"))
        .ok_or_else(|| ParseError::Structure(format!("缺少 data.{code}.data.data")))?
        .as_array()
        .ok_or_else(|| ParseError::Structure("data.data 不是数组".to_string()))?;

    let raw: Vec<RawPoint> = items
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(|s| parse_point(s, market))
        .collect();

    Ok(MinuteSeries {
        code: code.to_string(),
        market,
        grid_len: grid_len(market),
        prev_close,
        points: attach_grid(market, &raw),
    })
}

/// 落栅格之前的裸点（供测试对比"原始点数 vs 落格后点数"）。
pub fn raw_points(json_text: &str, code: &str) -> Result<Vec<RawPoint>, ParseError> {
    let trimmed = json_text.trim_start_matches('\u{feff}').trim();
    if !trimmed.starts_with('{') {
        return Err(ParseError::NotJson);
    }
    let root: Value = serde_json::from_str(trimmed).map_err(|_| ParseError::NotJson)?;
    let items = root
        .get("data")
        .and_then(|d| d.get(code))
        .and_then(|n| n.get("data"))
        .and_then(|d| d.get("data"))
        .and_then(|a| a.as_array())
        .ok_or_else(|| ParseError::Structure(format!("缺少 data.{code}.data.data")))?;
    Ok(items
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(|s| parse_point(s, classify(code)))
        .collect())
}
