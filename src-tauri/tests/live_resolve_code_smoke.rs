//! **联网**冒烟测试：直接输码解析（`ipc::resolve_code_inner`）。
//!
//! 全部标 `#[ignore]`，`cargo test` 默认不跑。
//!
//! ```text
//! cargo test --test live_resolve_code_smoke -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! # 存在理由
//!
//! `code_input_parse.rs` 已覆盖段位表，但覆盖不了三件只有真机能验的事：
//!
//! 1. **存在性过滤真的生效**：`sh605888` 段位合法但无此股，必须被淘汰。
//!    纯离线测试看不出来 —— 它不知道哪个码真实存在。
//! 2. **真名取得到**：自选股列表要显示名字，得确认 GBK 解码后不是乱码。
//! 3. **歧义码两条都活着**：`000001` 必须同时返回沪指与平安银行。
//!    这条是整个功能的核心主张，只有打真接口才能证明。
//!
//! 另外这里也是白名单的运行时检查：`qt.gtimg.cn` 若漏了，编译期看不出来。

use tickerglass_lib::ipc::{resolve_code_inner, ResolveResponse};
use tickerglass_lib::model::WatchItem;

const GAP: std::time::Duration = std::time::Duration::from_secs(3);

/// 清掉代理环境变量，**每个联网用例开头都必须调**。
///
/// 存在理由（2026-08-27 血的教训，本项目犯过两次）：带 `http_proxy` 的 shell 里
/// `reqwest` 会读这些变量，于是测试全绿、我据此报"已验证"。
/// 但 **app 自己没有代理** —— 用户点开时全是失败的。
/// 冒烟测试的意义是复现 app 的真实网络环境，所以必须主动摘掉代理。
fn clear_proxy() {
    for k in [
        "http_proxy",
        "https_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
    ] {
        std::env::remove_var(k);
    }
}

fn run(input: &str, current: &[WatchItem]) -> ResolveResponse {
    clear_proxy();
    let rt = tokio::runtime::Runtime::new().expect("建 runtime");
    let r = rt
        .block_on(resolve_code_inner(input, current))
        .unwrap_or_else(|e| panic!("{input} 解析应不返 Err，实际 {e}"));
    std::thread::sleep(GAP);
    r
}

fn expect_ok(input: &str) -> Vec<(String, String)> {
    match run(input, &[]) {
        ResolveResponse::Ok { items } => items
            .into_iter()
            .map(|i| (i.tencent_code, i.name))
            .collect(),
        other => panic!("{input} 应返 Ok，实际 {other:?}"),
    }
}

// ───────── 真实名称与存在性 ─────────

#[test]
#[ignore]
fn live_resolves_bare_ashare_code_with_real_name() {
    let got = expect_ok("600519");
    assert_eq!(got.len(), 1, "600519 应只有沪市一条，实际 {got:?}");
    assert_eq!(got[0].0, "sh600519");
    // 名字必须是可读中文，不是乱码 —— GBK 解码若走 .text() 这里会是「è´µå·žèŒ…å°」
    assert!(
        got[0].1.contains("茅台"),
        "名称应含「茅台」，实际 {:?}（乱码说明 GBK 解码路径出问题）",
        got[0].1
    );
}

#[test]
#[ignore]
fn live_resolves_star_market_and_chinext() {
    let star = expect_ok("688111");
    assert_eq!(star[0].0, "sh688111");
    assert!(!star[0].1.is_empty(), "科创板应有名称");

    let cyb = expect_ok("300750");
    assert_eq!(cyb[0].0, "sz300750");
    assert!(cyb[0].1.contains("宁德"), "实际 {:?}", cyb[0].1);
}

#[test]
#[ignore]
fn live_resolves_beijing_exchange() {
    // 北交所是最容易被段位表漏掉的一段：实测 sh430047/sz430047 都不存在，
    // 只有 bj430047 有数据。这条同时验证了段位表与 bj 前缀。
    let got = expect_ok("430047");
    assert_eq!(got.len(), 1, "430047 应只有北交所一条，实际 {got:?}");
    assert_eq!(got[0].0, "bj430047");
    assert!(!got[0].1.is_empty(), "北交所应有名称");
}

#[test]
#[ignore]
fn live_resolves_etf() {
    let got = expect_ok("510300");
    assert_eq!(got[0].0, "sh510300");
    assert!(got[0].1.contains("300"), "ETF 名称应含「300」，实际 {:?}", got[0].1);
}

// ───────── 歧义码：核心主张 ─────────

#[test]
#[ignore]
fn live_ambiguous_000001_returns_both_markets_alive() {
    // 整个"候选 + 接口淘汰"设计的核心论据：这两个码**都真实存在**，
    // 所以任何"猜一个"的实现都会有一半时候是错的。
    let got = expect_ok("000001");
    assert_eq!(got.len(), 2, "000001 应返两条（沪指 + 平安银行），实际 {got:?}");

    let sh = got.iter().find(|(c, _)| c == "sh000001").expect("缺沪指");
    let sz = got.iter().find(|(c, _)| c == "sz000001").expect("缺平安银行");
    assert!(sh.1.contains("上证"), "沪 000001 应是上证指数，实际 {:?}", sh.1);
    assert!(sz.1.contains("平安"), "深 000001 应是平安银行，实际 {:?}", sz.1);
}

#[test]
#[ignore]
fn live_explicit_prefix_narrows_ambiguous_code_to_one() {
    let got = expect_ok("sz000001");
    assert_eq!(got.len(), 1, "显式指定 sz 后不应再给沪市候选");
    assert_eq!(got[0].0, "sz000001");
    assert!(got[0].1.contains("平安"), "实际 {:?}", got[0].1);
}

// ───────── 存在性过滤：不存在的码必须被淘汰 ─────────

#[test]
#[ignore]
fn live_nonexistent_code_reports_not_found_not_ok() {
    // sh605888：605 段位合法（605xxx 是沪市主板），但实测无此股
    // （返 `v_pv_none_match="1"`）。若这里返回 Ok，说明存在性过滤没生效，
    // 用户会加进一只永远显示「—」的死行。
    match run("605888", &[]) {
        ResolveResponse::NotFound { message } => {
            assert!(message.contains("605888"), "提示应含用户输入的码: {message}");
        }
        other => panic!("605888 应返 NotFound，实际 {other:?}"),
    }
}

#[test]
#[ignore]
fn live_unknown_segment_gets_filtered_to_nothing() {
    // 777777 两个候选都不存在 → NotFound（而不是返回两条死行）
    match run("777777", &[]) {
        ResolveResponse::NotFound { message } => {
            // 提示里应把两个都试过的码列出来，便于用户判断
            assert!(message.contains("sh777777") && message.contains("sz777777"), "{message}");
        }
        other => panic!("777777 应返 NotFound，实际 {other:?}"),
    }
}

// ───────── 不打网络的分支 ─────────

#[test]
#[ignore]
fn live_invalid_input_short_circuits_without_network() {
    // 格式就不对时不该发请求。这里只能验行为（返 Invalid），
    // 但配合 `resolve_code_inner` 的实现可知它在 parse 失败时直接返回。
    for bad in ["茅台", "60051", "hk00700"] {
        match run(bad, &[]) {
            ResolveResponse::Invalid { message } => {
                assert!(!message.is_empty(), "{bad} 的提示不应为空");
            }
            other => panic!("{bad} 应返 Invalid，实际 {other:?}"),
        }
    }
}

// ───────── already_added 标记 ─────────

#[test]
#[ignore]
fn live_marks_already_added_when_in_watchlist() {
    let current = vec![WatchItem {
        tencent_code: "sh600519".to_string(),
        em_secid: Some("1.600519".to_string()),
        market: tickerglass_lib::domain::market::Market::Ashare,
        is_index: false,
        // 隐藏与"是否已在自选股中"无关：藏起来的标的**仍然算已添加**，
        // 否则用户会重复添加同一只
        hidden: false,
    }];
    match run("600519", &current) {
        ResolveResponse::Ok { items } => {
            assert_eq!(items.len(), 1);
            assert!(items[0].already_added, "已在自选股中的标的必须标记出来");
        }
        other => panic!("应返 Ok，实际 {other:?}"),
    }
}
