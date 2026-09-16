//! 个股当日资金流解析：东财 `ulist.np` 一次请求带全部自选股 secid。
//!
//! # 为什么用 `ulist.np` 而不是逐股 `stock/get`
//!
//! `ulist.np` 支持多 secid 一次返回 —— 50 只自选股 = **1 个请求**。
//! 而 `push2` host 是间隔型限流（约每分钟只容一个请求，见 `net` 的闸门），
//! 逐股取资金流等于 50 请求/轮，必被限流。这也是大盘面板轮转设计
//! （`ipc::fetch_market_overview`）的同一条约束。
//!
//! # 字段含义（2026-09-16 盘中实测认定，恒等式验证）
//!
//! - `f12` 裸码 / `f13` 市场号 → 拼 `"{f13}.{f12}"` 还原 secid，
//!   与 `WatchItem.em_secid` 精确匹配（裸码跨市场不唯一，`000001` 沪深都有）
//! - `f62` 主力净流入（元）= `f66` 超大单净 + `f72` 大单净（实测相等，沪深港都成立）
//! - `f78` 中单净流入、`f84` 小单净流入（散户）
//! - ⚠️ "主力+中单+小单≈0" 的封闭恒等式**只对 A 股成立**（实测 A 股误差≤14 元）；
//!   港股不封闭（00700 四档和 -4218 万）—— 港股没有逐笔买卖对称约束，
//!   档位口径与 A 股不同。**不要**拿这条恒等式去校验港股行。
//!
//! 不供资金流的标的（美股等）这些字段是 `"-"` 或缺失 → 该行被跳过，
//! 调用方保持该行 `fund_flow: None`。

use crate::model::FundFlow;
use serde_json::Value;

pub use super::em_market::MarketParseError;

/// 取浮点字段；`"-"`、缺失、非数字一律 `None`（绝不 `unwrap_or(0.0)`：
/// "净流入 0 元"与"没有数据"在界面上是两种表现）。
fn f64_of(row: &Value, key: &str) -> Option<f64> {
    let v = row.get(key)?;
    if let Some(f) = v.as_f64() {
        return f.is_finite().then_some(f);
    }
    v.as_str()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
}

/// 解析 `ulist.np` 资金流响应 → `(secid, FundFlow)` 列表。
///
/// 响应外层与 `em_market::parse_markets_batch` 同构（`rc`/`data.diff`，
/// `diff` 既可能是数组也可能是 `"0"/"1"` 为键的对象）。
/// 空响应体（限流特征）返回 `RateLimited`；行缺关键字段的**跳过该行**而不是
/// 整体报错 —— 一批里混着美股（无资金流）与 A 股是常态。
pub fn parse_fund_flow(text: &str) -> Result<Vec<(String, FundFlow)>, MarketParseError> {
    if text.trim().is_empty() {
        return Err(MarketParseError::RateLimited);
    }
    let root: Value =
        serde_json::from_str(text).map_err(|_| MarketParseError::NotJson)?;
    if let Some(rc) = root.get("rc").and_then(|v| v.as_i64()) {
        if rc != 0 {
            return Err(MarketParseError::Rejected(rc));
        }
    }
    let diff = root
        .get("data")
        .and_then(|d| d.get("diff"))
        .ok_or_else(|| MarketParseError::Structure("缺 data.diff".into()))?;

    let rows: Vec<&Value> = match diff {
        Value::Array(a) => a.iter().collect(),
        Value::Object(m) => {
            let mut kv: Vec<(usize, &Value)> = m
                .iter()
                .filter_map(|(k, v)| k.parse::<usize>().ok().map(|i| (i, v)))
                .collect();
            kv.sort_by_key(|(i, _)| *i);
            kv.into_iter().map(|(_, v)| v).collect()
        }
        _ => {
            return Err(MarketParseError::Structure(
                "diff 既非数组也非对象".into(),
            ))
        }
    };

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        // secid 还原：f13 市场号 + f12 裸码。缺任一无法归属，跳过。
        let Some(code) = r.get("f12").and_then(|v| v.as_str()) else {
            continue;
        };
        let mkt = match r.get("f13") {
            Some(v) if v.is_number() => v.as_i64().unwrap_or(-1).to_string(),
            Some(v) if v.is_string() => v.as_str().unwrap_or("").trim().to_string(),
            _ => continue,
        };
        if mkt.is_empty() || mkt == "-1" {
            continue;
        }
        // 五个净额字段缺任一就跳过该行：半套数据会让"主力=超大+大"
        // 的恒等式在界面上算出错值（与 em_market 的严口径一致）
        let (Some(main_net), Some(retail_net), Some(medium_net), Some(extra_large_net), Some(large_net)) = (
            f64_of(r, "f62"),
            f64_of(r, "f84"),
            f64_of(r, "f78"),
            f64_of(r, "f66"),
            f64_of(r, "f72"),
        ) else {
            continue;
        };
        out.push((
            format!("{mkt}.{code}"),
            FundFlow {
                main_net,
                retail_net,
                medium_net,
                extra_large_net,
                large_net,
            },
        ));
    }
    Ok(out)
}
