//! 腾讯日K解析（`web.ifzq.gtimg.cn/appstock/app/fqkline/get`）。
//!
//! 纯函数：不 import reqwest/tauri，可离线单测（架构不变量 2）。
//! 输出复用 `em_kline::KlineBar`，下游 `CandleChart` 与均线计算完全不动。
//!
//! # 为什么日K从东财换到腾讯（2026-08-27）
//!
//! 东财的历史数据只在 `push2his.eastmoney.com` 上，而**这台机器直连不通那个域名**：
//! 根路径能返 404（DNS/TCP/TLS 都通），但 `/api/qt/stock/kline/get` 一律 0.06s 内
//! 被重置（`error sending request`）。经 Clash 代理则 200。
//! `push2delay` 虽可直连，但它是**延时报价**主机、不带历史 —— 实测 `klines` 恒为空
//! （且 HTTP 200 掩盖了 body 里的 `rc=102`，看起来像"成功但没数据"）。
//! 用户报的「第二次刷新容易日K请求失败」就是这条。
//!
//! # 响应结构（实测 2026-08-27）
//!
//! ```json
//! { "code": 0, "data": { "sh600519": {
//!     "qfqday": [ ["2026-08-26","1300.000","1302.800","1314.450","1295.000","21731.000"], ... ]
//! } } }
//! ```
//!
//! # 三个实测坑
//!
//! 1. **第 2 段是「收」不是「高」** —— 顺序为 `日期 开 收 高 低 量`。
//!    与东财那条同一个坑（见 `em_kline`），照 OHLC 直觉读会把收盘价当最高价。
//! 2. **美股必须带交易所后缀**：`usAAPL` 只返 2 条，`usAAPL.OQ` 返 121 条。
//!    纳斯达克 `.OQ`、纽交所 `.N`，二者不可互换（`usJPM.OQ` 只返 1 条、`usJPM.N` 正常）。
//!    后缀由 `em_secid` 前缀推导（105→`.OQ`，106→`.N`），见 `net::tencent_kline_code`。
//! 3. **键名随复权方式变**：`qfq` 参数下是 `qfqday`，不复权是 `day`。两者都要认。
//!    港股行末还会多一个对象（回购/派息公告），故按**位置取前 6 段**、不校验段数相等。

use super::em_kline::{KlineBar, KlineParseError};
use serde_json::Value;

/// 解析腾讯日K。`code` 是**带后缀**的请求代码（如 `usAAPL.OQ`），
/// 因为响应以请求代码为键。
pub fn parse_tencent_kline(text: &str, code: &str) -> Result<Vec<KlineBar>, KlineParseError> {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if !trimmed.starts_with('{') {
        return Err(KlineParseError::Malformed("非 JSON".to_string()));
    }
    let root: Value =
        serde_json::from_str(trimmed).map_err(|e| KlineParseError::Malformed(format!("非 JSON: {e}")))?;

    if let Some(c) = root.get("code").and_then(|x| x.as_i64()) {
        if c != 0 {
            return Err(KlineParseError::Rejected(format!("code={c}")));
        }
    }

    let node = root
        .get("data")
        .and_then(|d| d.get(code))
        .ok_or_else(|| KlineParseError::Malformed(format!("缺 data.{code}")))?;

    // 复权用 qfqday，不复权用 day —— 两者都认，避免换参数就解析失败
    let rows = node
        .get("qfqday")
        .or_else(|| node.get("day"))
        .and_then(|x| x.as_array())
        .ok_or_else(|| KlineParseError::Malformed("缺 qfqday/day 数组".to_string()))?;

    let mut bars = Vec::with_capacity(rows.len());
    for r in rows {
        let Some(f) = r.as_array() else { continue };
        // 港股行末多挂一个公告对象，故只要前 6 段够用就行，不要求长度相等
        if f.len() < 6 {
            continue;
        }
        let s = |i: usize| -> Option<&str> { f.get(i).and_then(|x| x.as_str()) };
        let n = |i: usize| -> Option<f64> {
            s(i)
                .and_then(|x| x.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v > 0.0)
        };

        let Some(date) = s(0).map(|x| x.trim().to_string()) else {
            continue;
        };
        if date.is_empty() {
            continue;
        }
        // 报文顺序：日期 开 收 高 低 量 —— 第 2 段是收、第 3 段是高
        let (Some(open), Some(close), Some(high), Some(low)) = (n(1), n(2), n(3), n(4)) else {
            continue;
        };
        // 量可以为 0（停牌日），故不套 >0 过滤
        let volume = s(5)
            .and_then(|x| x.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(0.0);

        bars.push(KlineBar {
            date,
            open,
            high,
            low,
            close,
            volume,
        });
    }

    if bars.is_empty() {
        return Err(KlineParseError::Empty);
    }
    Ok(bars)
}
