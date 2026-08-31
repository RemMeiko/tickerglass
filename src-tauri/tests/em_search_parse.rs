//! 搜索结果解析（两个源，结构完全不同 → 两个独立解析器）。
//!
//! 全部读固化 fixture，**不发网络请求**。fixture 来历见 `tests/fixtures/search/README.md`。
//!
//! 断言点逐个独立命名，不把多个判据塞进一个 `#[test]`。

mod common;

use common::read_text;
use tickerglass_lib::domain::market::Market;
use tickerglass_lib::parse::em_search::{
    dedup_by_identity, is_allowed_type_name, parse_codetable, parse_searchapi, tencent_code_for,
    tencent_prefix_for_market, SearchError, ALLOWED_TYPE_NAMES, KNOWN_NOISE_TYPE_NAMES,
};

fn sa(name: &str) -> String {
    read_text(&format!("search/{name}"))
}

// ---------- 1. 市场号 → 前缀映射（遗留 3：美股前缀不止 105）----------

#[test]
fn us_market_105_nasdaq_maps_to_us_prefix() {
    assert_eq!(tencent_prefix_for_market(105), Some("us"));
}

#[test]
fn us_market_106_nyse_maps_to_us_prefix() {
    // 遗留 3 的正题：只硬编码 105 时，106 的美股取不到分时
    assert_eq!(tencent_prefix_for_market(106), Some("us"));
}

#[test]
fn us_market_107_amex_maps_to_us_prefix() {
    assert_eq!(tencent_prefix_for_market(107), Some("us"));
}

#[test]
fn shanghai_and_shenzhen_prefixes_differ() {
    assert_eq!(tencent_prefix_for_market(1), Some("sh"));
    assert_eq!(tencent_prefix_for_market(0), Some("sz"));
}

#[test]
fn hk_market_116_maps_to_hk_prefix() {
    assert_eq!(tencent_prefix_for_market(116), Some("hk"));
}

#[test]
fn markets_without_tencent_identity_are_rejected() {
    // 2=中证指数、150=场外基金、155=英股、177=韩股：腾讯报价无对应代码，
    // 加进自选股只会永远显示「—」，故在源头排除
    assert_eq!(tencent_prefix_for_market(2), None);
    assert_eq!(tencent_prefix_for_market(150), None);
    assert_eq!(tencent_prefix_for_market(155), None);
    assert_eq!(tencent_prefix_for_market(177), None);
}

// ---------- 2. 身份键推导：同一裸码 + 不同市场 → 不同键 ----------

#[test]
fn same_bare_code_different_market_yields_different_identity() {
    // 这是「删平安银行连带删上证指数」的根因所在
    assert_eq!(tencent_code_for(0, "000001").unwrap(), "sz000001");
    assert_eq!(tencent_code_for(1, "000001").unwrap(), "sh000001");
    assert_ne!(
        tencent_code_for(0, "000001").unwrap(),
        tencent_code_for(1, "000001").unwrap()
    );
}

#[test]
fn identity_derivation_rejects_empty_code() {
    assert_eq!(tencent_code_for(1, ""), None);
}

#[test]
fn identity_derivation_rejects_unsupported_market() {
    assert_eq!(tencent_code_for(2, "H30263"), None);
}

// ---------- 3. 主源 searchapi：标准 2 的三种查询词 ----------

#[test]
fn searchapi_finds_maotai_by_name() {
    let hits = parse_searchapi(&sa("searchapi_maotai.json")).expect("应解析成功");
    let h = hits
        .iter()
        .find(|h| h.tencent_code == "sh600519")
        .expect("茅台应在结果中");
    assert_eq!(h.name, "贵州茅台");
    assert_eq!(h.market_label, "沪A");
}

#[test]
fn searchapi_finds_maotai_by_code() {
    let hits = parse_searchapi(&sa("searchapi_600519.json")).expect("应解析成功");
    assert!(hits.iter().any(|h| h.tencent_code == "sh600519"));
}

#[test]
fn searchapi_finds_maotai_by_pinyin_gzmt() {
    // 判据是 GZMT（茅台拼音），**不是「mt」** —— 后者实测搜不到茅台
    let hits = parse_searchapi(&sa("searchapi_gzmt.json")).expect("应解析成功");
    assert!(hits.iter().any(|h| h.tencent_code == "sh600519"));
}

#[test]
fn searchapi_maotai_market_is_ashare() {
    let hits = parse_searchapi(&sa("searchapi_maotai.json")).expect("应解析成功");
    let h = hits.iter().find(|h| h.tencent_code == "sh600519").unwrap();
    assert_eq!(h.market, Market::Ashare);
    assert!(!h.is_index);
}

#[test]
fn searchapi_secid_comes_from_quote_id_verbatim() {
    // QuoteID 就是东财 secid 原文，直接取用 → 彻底不需要猜前缀
    let hits = parse_searchapi(&sa("searchapi_maotai.json")).expect("应解析成功");
    let h = hits.iter().find(|h| h.tencent_code == "sh600519").unwrap();
    assert_eq!(h.em_secid.as_deref(), Some("1.600519"));
}

// ---------- 4. 主源：`指数` 白名单必须通过（S1 的对症断言）----------

#[test]
fn searchapi_index_szzs_passes_whitelist() {
    // 漏了「指数」白名单，上证指数（Sprint 1 默认四只之一）就加不回来 —— 功能回归
    let hits = parse_searchapi(&sa("searchapi_szzs.json")).expect("应解析成功");
    let h = hits
        .iter()
        .find(|h| h.tencent_code == "sh000001")
        .expect("上证指数必须能被搜到");
    assert_eq!(h.name, "上证指数");
    assert_eq!(h.market_label, "指数");
}

#[test]
fn searchapi_index_szzs_is_flagged_as_index() {
    let hits = parse_searchapi(&sa("searchapi_szzs.json")).expect("应解析成功");
    let h = hits.iter().find(|h| h.tencent_code == "sh000001").unwrap();
    assert!(h.is_index, "securityTypeName=指数 必须落成 is_index");
    assert_eq!(h.market, Market::IndexCn);
}

#[test]
fn searchapi_index_secid_is_market_1() {
    let hits = parse_searchapi(&sa("searchapi_szzs.json")).expect("应解析成功");
    let h = hits.iter().find(|h| h.tencent_code == "sh000001").unwrap();
    assert_eq!(h.em_secid.as_deref(), Some("1.000001"));
}

#[test]
fn searchapi_excludes_fund_noise_from_index_query() {
    // 「上证指数」一查带出 8 条 ETF/联接基金，全部应被排除
    let hits = parse_searchapi(&sa("searchapi_szzs.json")).expect("应解析成功");
    assert!(
        hits.iter().all(|h| h.market_label != "基金"),
        "基金必须被排除，实际: {:?}",
        hits.iter().map(|h| &h.market_label).collect::<Vec<_>>()
    );
}

// ---------- 5. 主源：美股与债券噪声 ----------

#[test]
fn searchapi_aapl_real_stock_is_kept() {
    let hits = parse_searchapi(&sa("searchapi_aapl.json")).expect("应解析成功");
    let h = hits
        .iter()
        .find(|h| h.tencent_code == "usAAPL")
        .expect("苹果正股应在结果中");
    assert_eq!(h.name, "苹果");
    assert_eq!(h.market, Market::Us);
    assert_eq!(h.em_secid.as_deref(), Some("105.AAPL"));
}

#[test]
fn searchapi_excludes_bond_notes_despite_us_type_name() {
    // 实测陷阱：`Apple Inc Notes 2022` 的 SecurityTypeName **也是「美股」**，
    // 仅靠类型名白名单会把 8 条债券放进自选股。判据是 SecurityType=7
    let hits = parse_searchapi(&sa("searchapi_aapl.json")).expect("应解析成功");
    assert!(
        hits.iter().all(|h| !h.name.contains("Notes")),
        "债券必须被排除，实际留下: {:?}",
        hits.iter().map(|h| &h.name).collect::<Vec<_>>()
    );
}

#[test]
fn searchapi_aapl_query_keeps_only_the_real_stock() {
    // 10 条里只有 1 条正股（其余 8 债券 + 1 杠杆 ETF 均为 SecurityType=7）
    let hits = parse_searchapi(&sa("searchapi_aapl.json")).expect("应解析成功");
    assert_eq!(hits.len(), 1, "实际: {:?}", hits);
}

// ---------- 6. 主源：组合键歧义与各类噪声 ----------

#[test]
fn searchapi_000001_returns_both_pingan_and_index_as_distinct_keys() {
    // 同一裸码 000001 在两个市场是两个不同标的，**两条都要保留**
    let hits = parse_searchapi(&sa("searchapi_000001.json")).expect("应解析成功");
    assert!(hits.iter().any(|h| h.tencent_code == "sz000001"));
    assert!(hits.iter().any(|h| h.tencent_code == "sh000001"));
}

#[test]
fn searchapi_000001_pingan_is_ashare_not_index() {
    let hits = parse_searchapi(&sa("searchapi_000001.json")).expect("应解析成功");
    let pingan = hits.iter().find(|h| h.tencent_code == "sz000001").unwrap();
    assert_eq!(pingan.name, "平安银行");
    assert_eq!(pingan.market, Market::Ashare);
    assert!(!pingan.is_index, "sz000001 是平安银行，不是指数");
}

#[test]
fn searchapi_excludes_uk_shares_market_155() {
    let hits = parse_searchapi(&sa("searchapi_000001.json")).expect("应解析成功");
    assert!(hits.iter().all(|h| h.market_label != "英股"));
}

#[test]
fn searchapi_hk_tencent_uses_market_116() {
    let hits = parse_searchapi(&sa("searchapi_tencent.json")).expect("应解析成功");
    let h = hits
        .iter()
        .find(|h| h.tencent_code == "hk00700")
        .expect("腾讯控股应在结果中");
    assert_eq!(h.market, Market::Hk);
    assert_eq!(h.em_secid.as_deref(), Some("116.00700"));
}

#[test]
fn searchapi_excludes_korean_shares_market_177() {
    let hits = parse_searchapi(&sa("searchapi_tencent.json")).expect("应解析成功");
    assert!(hits.iter().all(|h| h.market_label != "韩股"));
}

// ---------- 7. 主源：空结果是 `Data: null` 而非 `[]` ----------

#[test]
fn searchapi_empty_result_is_data_null_not_error() {
    // 按 `[]` 解析会得到结构错误；真实空结果是 Data: null
    let hits = parse_searchapi(&sa("searchapi_empty.json")).expect("Data:null 应解析为空列表");
    assert!(hits.is_empty());
}

#[test]
fn searchapi_empty_result_is_not_rate_limited() {
    // 关键区分：0 结果的响应**仍带** QuotationCodeTable 键，不能误判为限流
    let r = parse_searchapi(&sa("searchapi_empty.json"));
    assert_ne!(r, Err(SearchError::RateLimited));
}

// ---------- 8. 限流判据：缺 `QuotationCodeTable` 键 ----------

#[test]
fn missing_quotation_code_table_key_is_rate_limited() {
    // 单次请求即可判定，不需要比对历史 md5
    let r = parse_searchapi(&sa("searchapi_rate_limited.json"));
    assert_eq!(r, Err(SearchError::RateLimited));
}

#[test]
fn rate_limited_error_message_is_not_no_result_wording() {
    // 限流必须提示"暂不可用"，不得当成"无匹配结果"呈现
    let msg = SearchError::RateLimited.to_string();
    assert!(msg.contains("暂不可用"), "实际文案: {msg}");
    assert!(!msg.contains("无匹配"), "不得说成无匹配结果: {msg}");
}

#[test]
fn html_error_page_is_not_json() {
    assert_eq!(
        parse_searchapi("<html><body>502</body></html>"),
        Err(SearchError::NotJson)
    );
}

#[test]
fn empty_body_is_not_json() {
    assert_eq!(parse_searchapi(""), Err(SearchError::NotJson));
}

// ---------- 9. 兜底源 codetable：结构与主源不同 ----------

#[test]
fn codetable_result_is_a_bare_array_and_parses() {
    // `result` 直接是数组，没有第二层包装（与 searchapi 不同）
    let hits = parse_codetable(&sa("codetable_szzs.json")).expect("应解析成功");
    assert!(!hits.is_empty());
}

#[test]
fn codetable_index_szzs_passes_whitelist() {
    let hits = parse_codetable(&sa("codetable_szzs.json")).expect("应解析成功");
    let h = hits
        .iter()
        .find(|h| h.tencent_code == "sh000001")
        .expect("兜底源也必须能搜到上证指数");
    assert_eq!(h.name, "上证指数");
    assert!(h.is_index);
}

#[test]
fn codetable_handles_integer_array_security_type_without_panic() {
    // securityType 是整数数组 `[401,101,5,501,301]`，按标量解析必崩。
    // 这条测试锁的是"能正常解析出结果"，即没有把数组当标量读
    let hits = parse_codetable(&sa("codetable_szzs.json")).expect("整数数组不应导致解析失败");
    assert!(hits.iter().any(|h| h.tencent_code == "sh000001"));
}

#[test]
fn codetable_top_level_code_string_is_not_mistaken_for_a_security() {
    // 顶层 `code` 是状态串 "0"，与条目内证券 code 撞名。
    // 若误把它当证券码，会冒出一条 sh0 / sz0 之类的垃圾项
    let hits = parse_codetable(&sa("codetable_szzs.json")).expect("应解析成功");
    assert!(
        hits.iter().all(|h| h.raw_code != "0"),
        "顶层状态串被误当成证券码: {:?}",
        hits.iter().map(|h| &h.raw_code).collect::<Vec<_>>()
    );
}

#[test]
fn codetable_excludes_fund_noise() {
    let hits = parse_codetable(&sa("codetable_szzs.json")).expect("应解析成功");
    assert!(hits.iter().all(|h| h.market_label != "基金"));
}

#[test]
fn codetable_000001_yields_both_markets_as_distinct_keys() {
    let hits = parse_codetable(&sa("codetable_000001.json")).expect("应解析成功");
    assert!(hits.iter().any(|h| h.tencent_code == "sz000001"));
    assert!(hits.iter().any(|h| h.tencent_code == "sh000001"));
}

#[test]
fn codetable_excludes_bonds_by_status_not_by_type_array() {
    // codetable 侧**不能**用「数组含 7」判债券 —— 真身 AAPL 的数组同样含 7。
    // 实测判据是 status=20。这条同时锁住"正股没被误杀"与"债券被排除"
    let hits = parse_codetable(&sa("codetable_aapl.json")).expect("应解析成功");
    assert!(
        hits.iter().any(|h| h.tencent_code == "usAAPL"),
        "苹果正股不可被误杀"
    );
    assert!(
        hits.iter().all(|h| !h.name.contains("Notes")),
        "债券必须被排除: {:?}",
        hits.iter().map(|h| &h.name).collect::<Vec<_>>()
    );
}

#[test]
fn codetable_hk_market_116_is_parsed() {
    let hits = parse_codetable(&sa("codetable_00700.json")).expect("应解析成功");
    let h = hits.iter().find(|h| h.tencent_code == "hk00700").unwrap();
    assert_eq!(h.name, "腾讯控股");
    assert_eq!(h.em_secid.as_deref(), Some("116.00700"));
}

#[test]
fn codetable_market_2_index_is_excluded_for_lack_of_identity() {
    // H30263 是中证指数（market=2），腾讯没有对应代码 → 无身份键 → 不能加自选股
    let hits = parse_codetable(&sa("codetable_h30263.json")).expect("应解析成功");
    assert!(
        hits.is_empty(),
        "market=2 无腾讯身份键，应被排除: {:?}",
        hits
    );
}

#[test]
fn codetable_empty_result_is_empty_array_not_null() {
    // 与主源不同：codetable 空结果是 `result: []`
    let hits = parse_codetable(&sa("codetable_empty.json")).expect("应解析成功");
    assert!(hits.is_empty());
}

#[test]
fn codetable_rejects_non_json() {
    assert_eq!(
        parse_codetable("<html>error</html>"),
        Err(SearchError::NotJson)
    );
}

// ---------- 10. 白名单与噪声值自证 ----------

#[test]
fn whitelist_contains_index_type() {
    // 独立锁一条：白名单必须含「指数」
    assert!(is_allowed_type_name("指数"));
    assert!(ALLOWED_TYPE_NAMES.contains(&"指数"));
}

#[test]
fn whitelist_contains_four_market_types() {
    assert!(is_allowed_type_name("沪A"));
    assert!(is_allowed_type_name("深A"));
    assert!(is_allowed_type_name("港股"));
    assert!(is_allowed_type_name("美股"));
}

#[test]
fn whitelist_rejects_all_known_noise_types() {
    for n in KNOWN_NOISE_TYPE_NAMES {
        assert!(!is_allowed_type_name(n), "{n} 不应在白名单内");
    }
}

#[test]
fn whitelist_rejects_fund_and_industry_explicitly() {
    // revision 1 的合同曾错把这两个当允许项
    assert!(!is_allowed_type_name("基金"));
    assert!(!is_allowed_type_name("行业"));
}

// ---------- 11. 去重按身份键 ----------

#[test]
fn dedup_keeps_two_markets_of_same_bare_code() {
    // sz000001 与 sh000001 **不是**重复项，去重不许把它们合成一条
    let hits = parse_searchapi(&sa("searchapi_000001.json")).expect("应解析成功");
    let n_before = hits.len();
    let after = dedup_by_identity(hits);
    assert_eq!(after.len(), n_before, "不同身份键不应被去重合并");
}

#[test]
fn dedup_removes_repeated_identity() {
    let mut hits = parse_searchapi(&sa("searchapi_maotai.json")).expect("应解析成功");
    let dup = hits[0].clone();
    hits.push(dup);
    let after = dedup_by_identity(hits);
    assert_eq!(
        after.iter().filter(|h| h.tencent_code == "sh600519").count(),
        1
    );
}
