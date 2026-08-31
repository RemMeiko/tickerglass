use serde::{Deserialize, Serialize};

/// 标的所属市场。指数单独成一类，因为它的小数位与栅格选择与个股一致，
/// 但后续 Sprint 的显示规则（换手率/市盈率等字段无意义）需要区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Market {
    Ashare,
    IndexCn,
    Hk,
    Us,
}

impl Market {
    pub fn is_index(self) -> bool {
        matches!(self, Market::IndexCn)
    }
}

/// 代码 → 市场。身份键一律是腾讯代码格式（`sh600519` / `hk00700` / `usAAPL`）。
///
/// 指数识别的关键：`sh` 前缀 + `000xxx` 才是指数（上证指数 `sh000001`），
/// 而 `sz000001` 是深市平安银行**不是**指数 —— 只按"含 000001"判会错分。
pub fn classify(code: &str) -> Market {
    // 全程走字节：代码前缀与数字段都是 ASCII，而 `&s[2..]` 这类切片在遇到
    // 多字节字符（如误输入中文）时会 panic on char boundary。
    let bytes = code.as_bytes();
    let prefix: [u8; 2] = match bytes {
        [a, b, ..] => [a.to_ascii_lowercase(), b.to_ascii_lowercase()],
        // 短于 2 字节：按 A 股处理（不 panic）。真实自选股来源永远带前缀。
        _ => return Market::Ashare,
    };
    let rest = &bytes[2..];

    match &prefix {
        // 沪市指数段：000xxx（上证系列）
        b"sh" if rest.starts_with(b"000") => Market::IndexCn,
        b"sh" => Market::Ashare,
        // 深市指数段：399xxx（深证成指/创业板指等）
        b"sz" if rest.starts_with(b"399") => Market::IndexCn,
        b"sz" => Market::Ashare,
        b"hk" => Market::Hk,
        b"us" => Market::Us,
        _ => Market::Ashare,
    }
}
