//! 东财 ulist.np 个股资金流解析（v0.3.0 需求 6）。
//! 样本取自 2026-09-16 盘中真实响应（现场固化，不发网络请求），
//! 恒等式（主力=超大+大；主力+中+小≈0）正是靠这份样本实测认定的。

use tickerglass_lib::parse::em_fundflow::parse_fund_flow;
use tickerglass_lib::parse::em_market::MarketParseError;

/// 2026-09-16 11:22 实测响应原文（茅台/宁德/天齐/腾讯四行）
const LIVE_LIKE: &str = r#"{"rc":0,"rt":11,"svr":183640595,"lt":1,"full":1,"dlmkts":"","dsc":"0","data":{"total":4,"diff":[
{"f2":125657,"f3":-127,"f12":"600519","f13":1,"f14":"贵州茅台","f62":-84637344.0,"f66":-118283504.0,"f72":33646160.0,"f78":84725328.0,"f84":-87970.0,"f124":1789528659},
{"f2":30371,"f3":-400,"f12":"300750","f13":0,"f14":"宁德时代","f62":-1924433728.0,"f66":-643782720.0,"f72":-1280651008.0,"f78":308015184.0,"f84":1616418448.0,"f124":1789528656},
{"f2":4233,"f3":55,"f12":"002466","f13":0,"f14":"天齐锂业","f62":-100233669.0,"f66":-18986849.0,"f72":-81246820.0,"f78":-2340640.0,"f84":102574298.0,"f124":1789528659},
{"f2":433000,"f3":-132,"f12":"00700","f13":116,"f14":"腾讯控股","f62":-145601620.0,"f66":-134920040.0,"f72":-10681580.0,"f78":99866512.0,"f84":3552140.0,"f124":1789528658}
]}}"#;

#[test]
fn rows_keyed_by_full_secid() {
    // f12 裸码跨市场不唯一（000001 沪深都有），必须拼回 "{f13}.{f12}" 做 key
    let flows = parse_fund_flow(LIVE_LIKE).expect("实测样本应解析成功");
    assert_eq!(flows.len(), 4);
    let keys: Vec<&str> = flows.iter().map(|(k, _)| k.as_str()).collect();
    assert!(keys.contains(&"1.600519"), "实际 {keys:?}");
    assert!(keys.contains(&"0.300750"), "实际 {keys:?}");
    assert!(keys.contains(&"0.002466"), "实际 {keys:?}");
    assert!(keys.contains(&"116.00700"), "港股市场号 116，实际 {keys:?}");
}

#[test]
fn identity_main_equals_extra_large_plus_large() {
    let flows = parse_fund_flow(LIVE_LIKE).unwrap();
    for (secid, f) in &flows {
        assert!(
            ((f.extra_large_net + f.large_net) - f.main_net).abs() < 1.0,
            "{secid} 主力净流入必须等于超大+大：{:?}",
            f
        );
    }
}

#[test]
fn identity_four_tiers_sum_to_about_zero_ashare_only() {
    // ⚠️ 实测发现（2026-09-16）："主力+中单+小单≈0" 只对 **A股** 成立
    // （茅台 +14 元、宁德/天齐恰好 0 —— 源头按元取整的噪声）。
    // 港股不封闭：腾讯控股四档和 = -4218 万 —— 港股没有逐笔买卖对称约束，
    // 东财对港股的四档划分口径不同。所以恒等式只拿来验证 A 股行。
    let flows = parse_fund_flow(LIVE_LIKE).unwrap();
    let ashare: Vec<_> = flows
        .iter()
        .filter(|(k, _)| k.starts_with("0.") || k.starts_with("1."))
        .collect();
    assert!(ashare.len() >= 3, "样本里 A 股行不该少于 3");
    for (secid, f) in ashare {
        let sum = f.main_net + f.medium_net + f.retail_net;
        assert!(
            sum.abs() < 100.0,
            "{secid} A股四档净额之和应≈0（每笔买必有卖），实际 {sum}"
        );
    }
}

#[test]
fn maotai_values_match_captured_sample() {
    let flows = parse_fund_flow(LIVE_LIKE).unwrap();
    let f = flows.iter().find(|(k, _)| k == "1.600519").unwrap();
    assert_eq!(f.1.main_net, -84637344.0);
    assert_eq!(f.1.retail_net, -87970.0);
    assert_eq!(f.1.medium_net, 84725328.0);
}

#[test]
fn dash_fields_row_is_skipped_not_zeroed() {
    // 美股不供资金流：字段是 "-"。跳过该行 → 调用方保持 fund_flow None，
    // 绝不能解析成 0（"净流入 0 元"与"无数据"是两种界面表现）
    let json = r#"{"rc":0,"data":{"diff":[
{"f12":"600519","f13":1,"f62":-84637344.0,"f66":-118283504.0,"f72":33646160.0,"f78":84725328.0,"f84":-87970.0},
{"f12":"AAPL","f13":105,"f62":"-","f66":"-","f72":"-","f78":"-","f84":"-"}
]}}"#;
    let flows = parse_fund_flow(json).unwrap();
    assert_eq!(flows.len(), 1, "字段为 - 的行必须跳过");
    assert_eq!(flows[0].0, "1.600519");
}

#[test]
fn empty_body_is_rate_limited() {
    // push2 静默限流的特征就是空响应体 —— 单独成错，让调用方说"稍后再试"
    assert!(matches!(parse_fund_flow("  "), Err(MarketParseError::RateLimited)));
    assert!(matches!(parse_fund_flow(""), Err(MarketParseError::RateLimited)));
}

#[test]
fn object_form_diff_is_accepted() {
    // ulist 的 data.diff 实测既可能是数组也可能是 "0"/"1" 为键的对象，
    // 对象形态必须按数值键排序（否则 10 会排到 2 前面）
    let json = r#"{"rc":0,"data":{"diff":{
"0":{"f12":"600519","f13":1,"f62":-1.0,"f66":-2.0,"f72":1.0,"f78":0.5,"f84":0.5},
"1":{"f12":"300750","f13":0,"f62":-3.0,"f66":-4.0,"f72":1.0,"f78":1.5,"f84":1.5}
}}}"#;
    let flows = parse_fund_flow(json).unwrap();
    assert_eq!(flows.len(), 2);
    assert_eq!(flows[0].0, "1.600519");
    assert_eq!(flows[1].0, "0.300750");
}
