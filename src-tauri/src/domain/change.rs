use serde::{Deserialize, Serialize};

/// 涨跌额与涨跌幅。两者都是 `Option`：拿不到就是拿不到，
/// 不用 0 冒充（0 在界面上意味着"平盘"，与"无数据"是两件事）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub amount: Option<f64>,
    pub percent: Option<f64>,
}

/// 报文自带字段优先，缺失时按昨收计算兜底。
///
/// - `prev_close <= 0`：不做除法，`percent` 返回 `None`（除零保护）
/// - `cur <= 0`（停牌无成交）：不产出任何涨跌
pub fn resolve_change(
    cur: Option<f64>,
    prev_close: Option<f64>,
    raw_amount: Option<f64>,
    raw_percent: Option<f64>,
) -> Change {
    let cur = match cur {
        Some(v) if v > 0.0 => v,
        _ => return Change::default(),
    };

    let amount = raw_amount.or_else(|| prev_close.map(|pc| cur - pc));

    let percent = raw_percent.or_else(|| {
        prev_close.and_then(|pc| {
            if pc > 0.0 {
                Some((cur - pc) / pc * 100.0)
            } else {
                None
            }
        })
    });

    Change { amount, percent }
}
