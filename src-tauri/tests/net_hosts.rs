//! 域名白名单与 URL 编码（纯函数，**不发任何网络请求**）。
//!
//! 这组测试的存在理由：Sprint 2A 的搜索域名若漏进白名单，
//! `get_bytes` 会直接 `Blocked` 而**编译期毫无提示**，标准 2 在运行时才挂。

use tickerglass_lib::net::{allowed_hosts, host_of, is_allowed_host, urlencode};

// ---------- 1. 搜索域名必须在白名单内（S2 的对症断言）----------

#[test]
fn search_primary_host_is_whitelisted() {
    assert!(
        is_allowed_host("searchapi.eastmoney.com"),
        "搜索主源不在白名单 → 标准 2 运行时必挂"
    );
}

#[test]
fn search_fallback_host_is_whitelisted() {
    assert!(
        is_allowed_host("search-codetable.eastmoney.com"),
        "搜索兜底源不在白名单 → 限流时无法降级"
    );
}

#[test]
fn sprint1_quote_hosts_remain_whitelisted() {
    // 回归防线：补新域名时别把老的挤掉
    assert!(is_allowed_host("qt.gtimg.cn"));
    assert!(is_allowed_host("web.ifzq.gtimg.cn"));
    assert!(is_allowed_host("push2delay.eastmoney.com"));
}

#[test]
fn whitelist_has_exactly_six_hosts() {
    // 锁总数：新增外联域名必须是显式决策，不能悄悄溜进来。
    //
    // 变更记录（改这个数字前请先在此登记理由）：
    // - 5 → 6（2026-08-24）：新增 `push2his.eastmoney.com` 作日K 主源。
    //   它是**独立域名**，不同于已有的 `push2delay`；选它而非腾讯 `fqkline` 的理由见
    //   `parse::em_kline` 模块文档（腾讯美股须带 `.OQ` 后缀，而 `.OQ` 不能作身份键；
    //   且腾讯日K 不带成交量，画不了量柱）。
    //   同步已补 `spec.md` 标准 21 的隐私域名清单，否则 Sprint 3 会误判有未声明外联。
    // - 6 → 7（2026-08-27）：新增 `push2.eastmoney.com` 作大盘面板源。
    //   非它不可的理由：当时实测"只有它带 f113/f114/f115"。
    // - 7 → 6（2026-11-18，v0.4.0）：**移除** `push2.eastmoney.com`。
    //   当天的旧结论被盘中实测推翻：push2delay 的 `stock/get` 同样带
    //   f113/f114/f115 与 f135~f149（沪 459/1836/60、恒等式成立），
    //   大盘面板整体迁 delay 域。push2 对轮询的 host 级 TLS 掐断惩罚
    //   使它不再值得留在请求路径里（详见 net::fetch_em_market_stats 文档）。
    assert_eq!(allowed_hosts().len(), 6, "实际: {:?}", allowed_hosts());
}

#[test]
fn market_panel_host_is_whitelisted() {
    // v0.4.0：大盘面板（家数/资金流）与美股分时、个股资金流同走 push2delay。
    // 漏加会让大盘面板被 get_bytes 硬拒，且编译期完全看不出来
    assert!(
        is_allowed_host("push2delay.eastmoney.com"),
        "大盘面板源不在白名单 → 面板不可用"
    );
    // push2（非 delay）已随 v0.4.0 退役，不得再悄悄溜回白名单
    assert!(
        !is_allowed_host("push2.eastmoney.com"),
        "push2 已退役，出现即说明有人把它接回了请求路径（需重新评审）"
    );
}

#[test]
fn kline_host_is_whitelisted() {
    // 漏加会让日K 被 get_bytes 硬拒，表现为大图窗「日K」页永远报错
    assert!(
        is_allowed_host("push2his.eastmoney.com"),
        "日K 主源不在白名单 → 大图窗日K 页不可用"
    );
    // 它与 push2delay 是两个域名，别混为一谈
    assert_ne!("push2his.eastmoney.com", "push2delay.eastmoney.com");
}

#[test]
fn unknown_host_is_rejected() {
    assert!(!is_allowed_host("evil.example.com"));
    assert!(!is_allowed_host(""));
}

#[test]
fn lookalike_host_is_rejected() {
    // 子域名/近似域名不得因"包含"关系被放行
    assert!(!is_allowed_host("searchapi.eastmoney.com.evil.com"));
    assert!(!is_allowed_host("evil-qt.gtimg.cn"));
}

// ---------- 2. host 提取 ----------

#[test]
fn host_is_extracted_from_full_url() {
    assert_eq!(
        host_of("https://searchapi.eastmoney.com/api/suggest/get?input=x"),
        "searchapi.eastmoney.com"
    );
}

#[test]
fn host_extraction_stops_at_first_slash() {
    assert_eq!(host_of("https://qt.gtimg.cn/q=sh600519"), "qt.gtimg.cn");
}

#[test]
fn malformed_url_yields_empty_host_which_is_rejected() {
    let h = host_of("not a url");
    assert!(!is_allowed_host(&h));
}

// ---------- 3. URL 编码：中文查询词与注入防护 ----------

#[test]
fn chinese_query_is_percent_encoded() {
    // 「茅台」直接拼进 URL 会得到非法 URL
    assert_eq!(urlencode("茅台"), "%E8%8C%85%E5%8F%B0");
}

#[test]
fn ascii_alphanumeric_query_is_unchanged() {
    assert_eq!(urlencode("600519"), "600519");
    assert_eq!(urlencode("GZMT"), "GZMT");
}

#[test]
fn unreserved_characters_are_not_encoded() {
    assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
}

#[test]
fn query_param_injection_characters_are_encoded() {
    // 用户输入 `a&type=99` 不得篡改后续查询参数
    let e = urlencode("a&type=99");
    assert!(!e.contains('&'), "实际: {e}");
    assert_eq!(e, "a%26type%3D99");
}

#[test]
fn space_and_special_chars_are_encoded() {
    assert_eq!(urlencode(" "), "%20");
    assert_eq!(urlencode("?"), "%3F");
    assert_eq!(urlencode("#"), "%23");
}

#[test]
fn empty_query_encodes_to_empty() {
    assert_eq!(urlencode(""), "");
}

#[test]
fn long_input_does_not_panic() {
    // 标准 2 的异常输入要求：200 字符不崩
    let long = "茅".repeat(200);
    let e = urlencode(&long);
    assert_eq!(e.len(), 200 * 9);
}
