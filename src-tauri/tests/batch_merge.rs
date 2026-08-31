//! 分批结果汇总（纯函数）。锁的是**架构不变量 5 的分批延伸**：
//! 整批失败时该批所有标的须进失败集（→ 界面上标记为过期/无数据），
//! **不得从界面消失**。只靠运行时观察证不了这条（失败批本来就少见）。

use std::collections::HashMap;
use tickerglass_lib::domain::change::Change;
use tickerglass_lib::domain::market::Market;
use tickerglass_lib::model::{Quote, QuoteParseOutcome};
use tickerglass_lib::scheduler::merge_batch_results;

fn ok_quote(code: &str) -> QuoteParseOutcome {
    QuoteParseOutcome::Ok(Quote {
        code: code.to_string(),
        name: "测试".to_string(),
        raw_symbol: code.to_string(),
        market: Market::Ashare,
        price: Some(1.0),
        prev_close: Some(1.0),
        open: Some(1.0),
        high: Some(1.0),
        low: Some(1.0),
        change: Change {
            amount: Some(0.0),
            percent: Some(0.0),
        },
        price_decimals: Some(2),
        raw_timestamp: String::new(),
        quote_time: None,
    })
}

fn codes(n: usize, offset: usize) -> Vec<String> {
    (0..n).map(|i| format!("sh6{:05}", i + offset)).collect()
}

// ---------- 1. 全部成功 ----------

#[test]
fn all_batches_ok_yields_no_failures() {
    let b1 = codes(40, 0);
    let b2 = codes(10, 40);
    let r1: Vec<QuoteParseOutcome> = b1.iter().map(|c| ok_quote(c)).collect();
    let r2: Vec<QuoteParseOutcome> = b2.iter().map(|c| ok_quote(c)).collect();
    let (by_code, failed) = merge_batch_results(vec![(b1, Some(r1)), (b2, Some(r2))]);
    assert!(failed.is_empty());
    assert_eq!(by_code.len(), 50);
}

#[test]
fn merged_index_is_keyed_lowercase() {
    // 腾讯响应里 usAAPL 的大小写与请求可能不一致，键必须规范化
    let batch = vec!["usAAPL".to_string()];
    let out = vec![ok_quote("usAAPL")];
    let (by_code, _) = merge_batch_results(vec![(batch, Some(out))]);
    assert!(by_code.contains_key("usaapl"), "键应为小写: {:?}", by_code.keys());
}

// ---------- 2. 整批失败 → 全批进失败集（不变量 5 的机器验证）----------

#[test]
fn whole_batch_failure_puts_every_code_of_that_batch_into_failed_set() {
    let b1 = codes(40, 0);
    let (_, failed) = merge_batch_results(vec![(b1.clone(), None)]);
    assert_eq!(failed.len(), 40, "失败批的 40 个代码必须一个不漏");
    for c in &b1 {
        assert!(
            failed.contains(&c.to_ascii_lowercase()),
            "{c} 未进失败集 → 界面上这一行会消失"
        );
    }
}

#[test]
fn second_batch_failure_does_not_affect_first_batch_results() {
    // 一批失败不该拖累另一批（50 只场景：第 2 批挂了，前 40 只仍要正常显示）
    let b1 = codes(40, 0);
    let b2 = codes(10, 40);
    let r1: Vec<QuoteParseOutcome> = b1.iter().map(|c| ok_quote(c)).collect();
    let (by_code, failed) = merge_batch_results(vec![(b1, Some(r1)), (b2.clone(), None)]);
    assert_eq!(by_code.len(), 40, "第 1 批的结果必须完好");
    assert_eq!(failed.len(), 10, "只有第 2 批的 10 个进失败集");
    for c in &b2 {
        assert!(failed.contains(&c.to_ascii_lowercase()));
    }
}

#[test]
fn first_batch_failure_does_not_affect_second_batch_results() {
    let b1 = codes(40, 0);
    let b2 = codes(10, 40);
    let r2: Vec<QuoteParseOutcome> = b2.iter().map(|c| ok_quote(c)).collect();
    let (by_code, failed) = merge_batch_results(vec![(b1.clone(), None), (b2, Some(r2))]);
    assert_eq!(by_code.len(), 10);
    assert_eq!(failed.len(), 40);
    assert!(failed.contains(&b1[0].to_ascii_lowercase()));
}

#[test]
fn all_batches_failing_puts_all_fifty_codes_into_failed_set() {
    let b1 = codes(40, 0);
    let b2 = codes(10, 40);
    let (by_code, failed) = merge_batch_results(vec![(b1, None), (b2, None)]);
    assert!(by_code.is_empty());
    assert_eq!(failed.len(), 50, "全挂时 50 个标的都要留在界面上（带标记）");
}

#[test]
fn no_code_is_both_resolved_and_failed() {
    // 同一代码不该同时出现在两个集合里（否则界面状态会打架）
    let b1 = codes(40, 0);
    let b2 = codes(10, 40);
    let r1: Vec<QuoteParseOutcome> = b1.iter().map(|c| ok_quote(c)).collect();
    let (by_code, failed) = merge_batch_results(vec![(b1, Some(r1)), (b2, None)]);
    for f in &failed {
        assert!(!by_code.contains_key(f), "{f} 同时在成功与失败集里");
    }
}

// ---------- 3. NoData 行的传递 ----------

#[test]
fn nodata_outcome_is_preserved_in_the_merged_index() {
    // reconcile 产出的 NoData（响应里整行缺失）必须能传到调用方，
    // 让它走"保留上次值 + 标记"而不是被当成"没请求过"
    let batch = vec!["sh999999".to_string()];
    let out = vec![QuoteParseOutcome::NoData {
        code: "sh999999".to_string(),
    }];
    let (by_code, failed) = merge_batch_results(vec![(batch, Some(out))]);
    assert!(failed.is_empty(), "NoData 不等于整批失败");
    assert!(matches!(
        by_code.get("sh999999"),
        Some(QuoteParseOutcome::NoData { .. })
    ));
}

#[test]
fn malformed_outcome_without_code_is_skipped_without_panic() {
    // Malformed 无法归属到具体标的（code() 返回 None），跳过即可，不能 panic
    let batch = vec!["sh600519".to_string()];
    let out = vec![QuoteParseOutcome::Malformed {
        raw_prefix: "garbage".to_string(),
    }];
    let (by_code, failed) = merge_batch_results(vec![(batch, Some(out))]);
    assert!(by_code.is_empty());
    assert!(failed.is_empty());
}

// ---------- 4. 边界 ----------

#[test]
fn empty_result_list_yields_empty_maps() {
    let (by_code, failed): (HashMap<_, _>, Vec<_>) = merge_batch_results(vec![]);
    assert!(by_code.is_empty());
    assert!(failed.is_empty());
}

#[test]
fn empty_batch_with_none_adds_no_failures() {
    let (_, failed) = merge_batch_results(vec![(vec![], None)]);
    assert!(failed.is_empty());
}
