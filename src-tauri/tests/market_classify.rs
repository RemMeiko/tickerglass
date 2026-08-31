//! 市场分类 6 个用例逐条枚举。
//!
//! 用例 2 与 3 是这组测试的价值所在：只按"含 000001"判指数会把深市平安银行
//! 错分成指数，进而带错涨跌配色与小数位规则。

use tickerglass_lib::domain::market::{classify, Market};

#[test]
fn case1_sh600519_is_ashare() {
    // 沪市个股：sh 前缀但非 000xxx 指数段
    assert_eq!(classify("sh600519"), Market::Ashare);
    assert!(!classify("sh600519").is_index());
}

#[test]
fn case2_sz000001_is_ashare_not_index() {
    // 深市 000001 是平安银行，**不是**指数 —— 与用例 3 构成"同数字段不同市"的边界
    assert_eq!(classify("sz000001"), Market::Ashare);
    assert!(!classify("sz000001").is_index());
}

#[test]
fn case3_sh000001_is_index_cn() {
    // 上证指数：sh + 000xxx 才是指数
    assert_eq!(classify("sh000001"), Market::IndexCn);
    assert!(classify("sh000001").is_index());
}

#[test]
fn case4_sz399001_is_index_cn() {
    // 深证成指：sz + 399xxx 指数段
    assert_eq!(classify("sz399001"), Market::IndexCn);
    assert!(classify("sz399001").is_index());
    // 创业板指同段
    assert_eq!(classify("sz399006"), Market::IndexCn);
}

#[test]
fn case5_hk00700_is_hk() {
    assert_eq!(classify("hk00700"), Market::Hk);
    assert!(!classify("hk00700").is_index());
}

#[test]
fn case6_usaapl_is_us() {
    // us 前缀 + 字母代码（身份键不带 .OQ 后缀）
    assert_eq!(classify("usAAPL"), Market::Us);
    assert!(!classify("usAAPL").is_index());
}

#[test]
fn classify_never_panics_on_odd_input() {
    // 短于前缀长度、空串等都不应 panic
    for c in ["", "s", "sh", "us", "x", "中文"] {
        let _ = classify(c);
    }
}
