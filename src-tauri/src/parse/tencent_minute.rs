// 接口用法参考 StockPet (MIT)，实现为本项目原创
//
// `minute/query?code={code}` 返回 JSON：
//   data.{code}.data.data — 分钟点数组，每项 `"HHMM price volume [amount]"`（空格分隔）
//   data.{code}.qt.{code} — 与批量报价同构的字段数组，昨收在 [4]（分时基准线来源）

use crate::domain::market::classify;
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

/// 解析一项 `"HHMM price volume ..."`。价格非法/为 0 时返回 `None`，
/// 由 `attach_grid` 统一丢弃（0 价会把 Y 轴压到 0）。
///
/// 注意必须按**单空格**切分而不是 `split_whitespace()`：后者会把连续空格折叠，
/// 于是价格字段为空的点（`"0941  91500375 …"`）会把**累计成交量**当成价格读进来，
/// 得到一个天文数字的假价格。按位置切分才能让空价格如实变成 `None`。
fn parse_point(item: &str) -> Option<RawPoint> {
    let mut seg = item.split(' ');
    let hhmm = seg.next()?.trim().parse::<u16>().ok()?;
    let price = seg.next()?.trim().parse::<f64>().ok()?;
    if !price.is_finite() || price <= 0.0 {
        return None;
    }
    Some(RawPoint {
        hhmm,
        price,
        // 第 3 段是累计成交量而非均价，均价线属 Sprint 4，此处不臆造
        avg: None,
    })
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
        .filter_map(parse_point)
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
        .filter_map(parse_point)
        .collect())
}
