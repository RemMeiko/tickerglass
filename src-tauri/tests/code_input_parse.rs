//! 直接输码的解析（纯函数，不发网络）。
//!
//! 段位表的每条断言都对应 2026-08-27 实测过的真实标的 —— 见
//! `domain::code_input` 模块文档的表格。写死这些是为了防"顺手改段位表"把
//! 已验证的映射改错：段位错了不会崩，只会让用户加进一只错的股票。

use tickerglass_lib::domain::code_input::{parse_code_input, CodeInputError};
use tickerglass_lib::domain::market::Market;

/// 断言输入唯一解析到某个腾讯代码。
fn only(input: &str) -> String {
    let c = parse_code_input(input).unwrap_or_else(|e| panic!("{input} 应解析成功，实际 {e:?}"));
    assert_eq!(c.len(), 1, "{input} 应只有 1 个候选，实际 {c:?}");
    c[0].tencent_code.clone()
}

fn codes(input: &str) -> Vec<String> {
    parse_code_input(input)
        .expect("应解析成功")
        .iter()
        .map(|c| c.tencent_code.clone())
        .collect()
}

// ───────── 段位 → 交易所（全部实测样本）─────────

#[test]
fn shanghai_main_board_segments() {
    assert_eq!(only("600519"), "sh600519"); // 贵州茅台
    assert_eq!(only("601318"), "sh601318"); // 中国平安
    assert_eq!(only("603288"), "sh603288"); // 海天味业
}

#[test]
fn shanghai_star_market_and_b_shares() {
    assert_eq!(only("688111"), "sh688111"); // 金山办公（科创板）
    assert_eq!(only("900901"), "sh900901"); // 云赛Ｂ股（沪B）
}

#[test]
fn shenzhen_segments() {
    assert_eq!(only("002415"), "sz002415"); // 海康威视
    assert_eq!(only("300750"), "sz300750"); // 宁德时代（创业板）
    assert_eq!(only("301269"), "sz301269"); // 华大九天
    assert_eq!(only("200011"), "sz200011"); // 深物业B
    assert_eq!(only("399001"), "sz399001"); // 深证成指
}

#[test]
fn beijing_exchange_segments() {
    // 北交所必须给 bj 前缀：实测 sh430047/sz430047 都是 pv_none_match，
    // 只有 bj430047 有数据。段位落到 sh/sz 会让用户查无此码。
    assert_eq!(only("430047"), "bj430047"); // 诺思兰德
    assert_eq!(only("920819"), "bj920819"); // 颖泰生物
}

#[test]
fn etf_segments_split_by_exchange() {
    assert_eq!(only("510300"), "sh510300"); // 沪深300ETF华泰柏瑞
    assert_eq!(only("159915"), "sz159915"); // 创业板ETF易方达
}

// ───────── 歧义码：必须给两个候选 ─────────

#[test]
fn bare_000001_yields_both_markets() {
    // 项目里反复强调的坑：sz000001 是平安银行、sh000001 是上证指数，两者都真实存在。
    // 这里**必须**两个都给，不能猜一个 —— 猜错时用户毫无感知。
    let got = codes("000001");
    assert_eq!(got.len(), 2, "000001 应给两个候选，实际 {got:?}");
    assert!(got.contains(&"sh000001".to_string()), "缺沪指候选: {got:?}");
    assert!(got.contains(&"sz000001".to_string()), "缺深A候选: {got:?}");
}

#[test]
fn index_candidate_comes_first_for_000_segment() {
    // 000300 沪深300 / 000001 上证指数是高频输入，指数排前面减少一次点击
    assert_eq!(codes("000300")[0], "sh000300");
}

#[test]
fn explicit_prefix_suppresses_the_other_candidate() {
    // 用户明确写了 sz，就不该再给 sh 候选 —— 显式输入优先于段位推导
    assert_eq!(only("sz000001"), "sz000001");
    assert_eq!(only("sh000001"), "sh000001");
}

// ───────── 输入写法容错 ─────────

#[test]
fn prefix_is_case_insensitive() {
    assert_eq!(only("SH600519"), "sh600519");
    assert_eq!(only("Sz000001"), "sz000001");
    assert_eq!(only("BJ430047"), "bj430047");
}

#[test]
fn wind_style_dotted_suffix_is_accepted() {
    // 从 Wind/同花顺导出的表格是这个格式，用户很可能直接粘进来
    assert_eq!(only("600519.SH"), "sh600519");
    assert_eq!(only("000001.sz"), "sz000001");
    assert_eq!(only("430047.BJ"), "bj430047");
}

#[test]
fn whitespace_is_stripped() {
    // 从表格复制常带前后空白，甚至中间被断开
    assert_eq!(only("  600519  "), "sh600519");
    assert_eq!(only("600 519"), "sh600519");
    assert_eq!(only("\tsh600519\n"), "sh600519");
}

// ───────── secid 与市场推导 ─────────

#[test]
fn shanghai_secid_uses_market_1_shenzhen_uses_0() {
    let sh = parse_code_input("sh600519").unwrap();
    assert_eq!(sh[0].em_secid, "1.600519");
    let sz = parse_code_input("sz000001").unwrap();
    assert_eq!(sz[0].em_secid, "0.000001");
}

#[test]
fn beijing_secid_uses_market_0_not_1() {
    // 实测不对称：腾讯前缀是 bj，但东财 secid 前缀是 0（1.430047 返 rc=100）。
    // 这条最容易写错 —— 直觉会想给北交所一个独立市场号。
    let bj = parse_code_input("bj430047").unwrap();
    assert_eq!(bj[0].em_secid, "0.430047", "北交所 secid 必须是 0. 前缀");
}

#[test]
fn index_flag_matches_classify() {
    let sh_idx = parse_code_input("sh000001").unwrap();
    assert!(sh_idx[0].is_index);
    assert_eq!(sh_idx[0].market, Market::IndexCn);

    let sz_idx = parse_code_input("399001").unwrap();
    assert!(sz_idx[0].is_index);
    assert_eq!(sz_idx[0].market, Market::IndexCn);

    // sz000001 是平安银行不是指数 —— 只按"含 000001"判会错分
    let sz_stock = parse_code_input("sz000001").unwrap();
    assert!(!sz_stock[0].is_index, "sz000001 是平安银行，不是指数");
    assert_eq!(sz_stock[0].market, Market::Ashare);
}

#[test]
fn beijing_is_treated_as_ashare_market() {
    // 北交所与 A 股同交易时段，栅格/时段判定直接复用 Ashare 是对的
    let bj = parse_code_input("bj430047").unwrap();
    assert_eq!(bj[0].market, Market::Ashare);
    assert!(!bj[0].is_index);
}

#[test]
fn watch_item_carries_secid_as_some() {
    // em_secid 是 Option：这里推导出来的是确定值，必须是 Some 而不是 None
    let it = parse_code_input("600519").unwrap()[0].to_watch_item();
    assert_eq!(it.tencent_code, "sh600519");
    assert_eq!(it.em_secid.as_deref(), Some("1.600519"));
}

// ───────── 坏输入：分类要能区分，不能一律"格式错误" ─────────

#[test]
fn empty_input_is_empty_error() {
    assert_eq!(parse_code_input(""), Err(CodeInputError::Empty));
    assert_eq!(parse_code_input("   "), Err(CodeInputError::Empty));
}

#[test]
fn chinese_name_tells_user_to_use_search() {
    // 用户打「茅台」时不该说"位数不对"，该指向名称搜索
    assert_eq!(parse_code_input("茅台"), Err(CodeInputError::NotNumeric));
    assert_eq!(parse_code_input("GZMT"), Err(CodeInputError::NotNumeric));
}

#[test]
fn wrong_digit_count_reports_actual_length() {
    assert_eq!(parse_code_input("60051"), Err(CodeInputError::BadLength(5)));
    assert_eq!(
        parse_code_input("6005199"),
        Err(CodeInputError::BadLength(7))
    );
    assert_eq!(
        parse_code_input("sh60519"),
        Err(CodeInputError::BadLength(5))
    );
}

#[test]
fn hk_and_us_are_explicitly_unsupported_not_silently_ashare() {
    // 本期只做 A 股。把 hk00700 静默当成 A 股会构造出一个不存在的 shk0070…
    assert_eq!(
        parse_code_input("hk00700"),
        Err(CodeInputError::UnsupportedMarket("HK".into()))
    );
    assert_eq!(
        parse_code_input("usAAPL"),
        Err(CodeInputError::UnsupportedMarket("US".into()))
    );
    assert_eq!(
        parse_code_input("00700.HK"),
        Err(CodeInputError::UnsupportedMarket("HK".into()))
    );
}

#[test]
fn error_messages_are_human_readable_chinese() {
    // 这些串会直接显示在设置窗里，不能是 Debug 输出
    assert!(CodeInputError::Empty.to_string().contains("请输入"));
    assert!(CodeInputError::NotNumeric.to_string().contains("搜索"));
    assert!(CodeInputError::BadLength(5).to_string().contains('5'));
    assert!(CodeInputError::UnsupportedMarket("HK".into())
        .to_string()
        .contains("HK"));
}

// ───────── 不 panic（防御多字节输入）─────────

#[test]
fn multibyte_input_does_not_panic_on_slicing() {
    // `&s[..3]` 遇到中文会 panic on char boundary —— 这条锁住那个坑
    for bad in ["中", "中文六位码", "6005中9", "０００００１", "🙂🙂🙂🙂🙂🙂"] {
        let _ = parse_code_input(bad); // 只要不 panic
    }
}

#[test]
fn unknown_segment_falls_back_to_both_exchanges() {
    // 段位表不可能穷尽（新板块随时开）。未知段位给两个候选、由接口淘汰，
    // 比直接报"不支持"更可用。
    let got = codes("777777");
    assert_eq!(got.len(), 2, "未知段位应给两个候选，实际 {got:?}");
    assert!(got.contains(&"sh777777".to_string()));
    assert!(got.contains(&"sz777777".to_string()));
}
