//! 腾讯批量报价解析：4 市场真实样本 + 异常样本 + 请求集对账 + 身份键 + 时间戳三格式。
//! 全部读固化 fixture，不发网络请求。

mod common;

use common::{read_bytes, read_text};
use tickerglass_lib::domain::market::Market;
use tickerglass_lib::model::QuoteParseOutcome;
use tickerglass_lib::parse::gbk::decode_gbk;
use tickerglass_lib::parse::tencent_quote::{parse_batch, parse_line, reconcile};
use tickerglass_lib::parse::timestamp::parse_quote_ts;

fn decode(name: &str) -> String {
    decode_gbk(&read_bytes(name))
}

fn only_ok(name: &str) -> tickerglass_lib::model::Quote {
    let text = decode(name);
    match parse_line(text.trim()) {
        QuoteParseOutcome::Ok(q) => q,
        other => panic!("{name} 期望 Ok，实际 {other:?}"),
    }
}

// ---------- 1. 四市场真实样本 ----------

#[test]
fn ashare_name_is_correct_chinese() {
    let q = only_ok("quote_ashare.txt");
    // GBK 解码验证点：名称必须是正确中文而非乱码
    assert_eq!(q.name, "贵州茅台");
    assert_eq!(q.code, "sh600519");
    assert_eq!(q.market, Market::Ashare);
}

#[test]
fn ashare_prices_parse_correctly() {
    let q = only_ok("quote_ashare.txt");
    assert_eq!(q.price, Some(1272.83));
    assert_eq!(q.prev_close, Some(1291.50));
    assert_eq!(q.change.amount, Some(-18.67));
    assert_eq!(q.change.percent, Some(-1.45));
    // A 股报文 2 位小数
    assert_eq!(q.price_decimals, Some(2));
}

#[test]
fn index_is_classified_as_index_cn() {
    let q = only_ok("quote_index.txt");
    assert_eq!(q.name, "上证指数");
    assert_eq!(q.market, Market::IndexCn);
    assert!(q.market.is_index());
    assert_eq!(q.price, Some(3905.20));
}

#[test]
fn hk_quote_keeps_three_decimals() {
    let q = only_ok("quote_hk.txt");
    assert_eq!(q.name, "腾讯控股");
    assert_eq!(q.market, Market::Hk);
    assert_eq!(q.price, Some(457.000));
    // 港股报文是 3 位小数，按 2 位显示会与东财网页不一致
    assert_eq!(q.price_decimals, Some(3));
}

#[test]
fn us_quote_parses_with_price() {
    let q = only_ok("quote_us.txt");
    assert_eq!(q.market, Market::Us);
    assert!(q.price.is_some_and(|p| p > 0.0));
    assert!(q.prev_close.is_some_and(|p| p > 0.0));
}

#[test]
fn batch_of_four_markets_all_ok() {
    let outcomes = parse_batch(&decode("quote_mixed_4.txt"));
    assert_eq!(outcomes.len(), 4);
    assert!(
        outcomes
            .iter()
            .all(|o| matches!(o, QuoteParseOutcome::Ok(_))),
        "四市场合并报文应全部解析成功，实际 {outcomes:?}"
    );
    let markets: Vec<Market> = outcomes
        .iter()
        .filter_map(|o| match o {
            QuoteParseOutcome::Ok(q) => Some(q.market),
            _ => None,
        })
        .collect();
    assert_eq!(
        markets,
        vec![Market::IndexCn, Market::Ashare, Market::Hk, Market::Us]
    );
}

// ---------- 2. 异常样本 ----------

#[test]
fn empty_payload_is_nodata_not_zero() {
    let text = decode("quote_empty.txt");
    match parse_line(text.trim()) {
        QuoteParseOutcome::NoData { code } => assert_eq!(code, "sh600519"),
        other => panic!("空串应为 NoData，实际 {other:?}"),
    }
}

#[test]
fn none_match_response_is_nodata() {
    // 实测：单个无效代码的真实响应是 v_pv_none_match="1";
    let text = decode("quote_none_match.txt");
    assert!(matches!(
        parse_line(text.trim()),
        QuoteParseOutcome::NoData { .. }
    ));
}

#[test]
fn short_fields_is_nodata() {
    let text = decode("quote_short_fields.txt");
    assert!(
        matches!(parse_line(text.trim()), QuoteParseOutcome::NoData { .. }),
        "字段不足 5 个时现价不可得，应为 NoData"
    );
}

#[test]
fn nonnumeric_price_is_nodata_and_does_not_produce_zero() {
    let text = decode("quote_nonnumeric.txt");
    match parse_line(text.trim()) {
        QuoteParseOutcome::NoData { code } => assert_eq!(code, "sh600519"),
        // 关键：绝不能是 Ok(price=0.0)
        other => panic!("非数字现价应为 NoData，实际 {other:?}"),
    }
}

#[test]
fn truncated_line_is_malformed_without_panic() {
    let text = decode("quote_truncated.txt");
    assert!(
        matches!(parse_line(text.trim()), QuoteParseOutcome::Malformed { .. }),
        "行尾缺 \"; 应为 Malformed"
    );
}

#[test]
fn one_broken_line_does_not_break_the_others() {
    let outcomes = parse_batch(&decode("quote_mixed_one_broken.txt"));
    let ok_count = outcomes
        .iter()
        .filter(|o| matches!(o, QuoteParseOutcome::Ok(_)))
        .count();
    assert_eq!(ok_count, 3, "4 行中 1 行损坏，其余 3 行仍应 Ok：{outcomes:?}");
    assert!(outcomes
        .iter()
        .any(|o| matches!(o, QuoteParseOutcome::Malformed { .. })));
}

// ---------- 3. 请求集 vs 返回行集对账 ----------

#[test]
fn reconcile_fills_missing_line_so_the_row_never_disappears() {
    // 实测 q=sh999999,sz000001 的响应里 sh999999 整行不出现（不是空串）
    let requested = ["sh999999", "sz000001"];
    let parsed = parse_batch(&decode("quote_missing_line.txt"));
    assert_eq!(parsed.len(), 1, "响应里只有 1 行");

    let out = reconcile(&requested, parsed);
    assert_eq!(out.len(), 2, "对账后必须与请求集等长，缺失代码不能消失");
    assert!(
        matches!(&out[0], QuoteParseOutcome::NoData { code } if code == "sh999999"),
        "缺失的 sh999999 应补 NoData，实际 {:?}",
        out[0]
    );
    assert!(
        matches!(&out[1], QuoteParseOutcome::Ok(q) if q.code == "sz000001"),
        "实际 {:?}",
        out[1]
    );
}

#[test]
fn reconcile_preserves_requested_order() {
    let requested = ["hk00700", "sh600519", "usAAPL", "sh000001"];
    let parsed = parse_batch(&decode("quote_mixed_4.txt")); // 响应顺序与请求不同
    let out = reconcile(&requested, parsed);
    let codes: Vec<&str> = out.iter().filter_map(|o| o.code()).collect();
    assert_eq!(codes, requested.to_vec(), "输出顺序必须与请求集一致");
}

// ---------- 4. 身份键铁律 ----------

#[test]
fn identity_key_comes_from_prefix_never_from_field_2() {
    let q = only_ok("quote_us_suffix.txt");
    // 实测 usAAPL 的 [2] 是 AAPL.OQ（带交易所后缀），与前缀键不同
    assert_eq!(q.code, "usAAPL", "身份键唯一来源是 v_{{code}}= 前缀");
    assert_eq!(q.raw_symbol, "AAPL.OQ", "[2] 字段仅作展示");
    assert_ne!(
        q.code, q.raw_symbol,
        "二者不相等，故 [2] 永不能用于与自选股匹配"
    );
}

#[test]
fn hk_field_2_also_differs_from_identity_key() {
    let q = only_ok("quote_hk.txt");
    assert_eq!(q.code, "hk00700");
    assert_eq!(q.raw_symbol, "00700");
    assert_ne!(q.code, q.raw_symbol);
}

// ---------- 5. 时间戳三格式 ----------

#[test]
fn timestamp_compact_format_for_ashare() {
    // A 股/指数：20260821161402
    let dt = parse_quote_ts("20260821161402", Market::IndexCn).expect("紧凑格式应解析成功");
    assert_eq!(dt.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-08-21 16:14:02");
}

#[test]
fn timestamp_slash_format_for_hk() {
    // 港股：2026/08/21 16:08:14
    let dt = parse_quote_ts("2026/08/21 16:08:14", Market::Hk).expect("斜杠格式应解析成功");
    assert_eq!(dt.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-08-21 16:08:14");
}

#[test]
fn timestamp_dash_format_for_us() {
    // 美股：2026-08-21 10:20:41
    let dt = parse_quote_ts("2026-08-21 10:20:41", Market::Us).expect("横杠格式应解析成功");
    assert_eq!(dt.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-08-21 10:20:41");
}

#[test]
fn timestamp_three_formats_from_real_merged_report() {
    // 三市场合并报文里三种格式各一条，全部要解析成功
    let outcomes = parse_batch(&decode("quote_ts_formats.txt"));
    assert_eq!(outcomes.len(), 3);
    for oc in &outcomes {
        let QuoteParseOutcome::Ok(q) = oc else {
            panic!("期望 Ok，实际 {oc:?}");
        };
        assert!(
            q.quote_time.is_some(),
            "{} 的时间戳 {:?} 解析失败 —— 单一格式解析会在这里暴露",
            q.code,
            q.raw_timestamp
        );
    }
    // 确认三条确实是三种不同格式的原文
    let raws: Vec<&str> = outcomes
        .iter()
        .filter_map(|o| match o {
            QuoteParseOutcome::Ok(q) => Some(q.raw_timestamp.as_str()),
            _ => None,
        })
        .collect();
    assert!(raws[0].len() == 14 && !raws[0].contains(' '), "指数应为紧凑格式: {}", raws[0]);
    assert!(raws[1].contains('/'), "港股应为斜杠格式: {}", raws[1]);
    assert!(raws[2].contains('-'), "美股应为横杠格式: {}", raws[2]);
}

#[test]
fn timestamp_falls_back_to_other_formats() {
    // 按市场选定的格式失败时，其余两种各试一次（接口换格式不至于全盘失效）
    let dt = parse_quote_ts("2026/08/21 16:08:14", Market::Ashare)
        .expect("A 股首选紧凑格式失败后应回退到斜杠格式");
    assert_eq!(dt.format("%H:%M").to_string(), "16:08");
    assert!(parse_quote_ts("", Market::Ashare).is_none());
    assert!(parse_quote_ts("not-a-time", Market::Us).is_none());
}

#[test]
fn html_or_garbage_input_never_panics() {
    // 防御性：任何输入都不应 panic
    let garbage = read_text("minute_html_error.html");
    let _ = parse_batch(&garbage);
    let _ = parse_line("");
    let _ = parse_line("完全不相关的一行文本");
    let _ = parse_line("v_=\"\";");
}

// ---------- 成交额（v0.3.0 需求 5）：[38] 万元 → 元 ----------

#[test]
fn amount_field_38_converts_wan_to_yuan() {
    // 实测茅台 2026-09-16 11:22：[37]=17359（手）[38]=219045（万元）
    // 与分时接口累计额 2,190,454,599 元对上（219045万 × 10000）
    let mut f: Vec<&str> = vec!["1"; 39];
    f[1] = "贵州茅台";
    f[2] = "600519";
    f[3] = "1700.00";
    f[4] = "1690.00";
    f[5] = "1695.00";
    f[30] = "20260916112239";
    f[31] = "10.00";
    f[32] = "0.59";
    f[33] = "1710.00";
    f[34] = "1690.00";
    f[38] = "219045";
    let line = format!("v_sh600519=\"{}\";", f.join("~"));
    match parse_line(&line) {
        QuoteParseOutcome::Ok(q) => {
            assert_eq!(q.amount, Some(2_190_450_000.0), "万元×10000 折元");
        }
        other => panic!("应解析成功，实际 {other:?}"),
    }
}

#[test]
fn amount_missing_or_dash_is_none() {
    // 港股/美股部分形态不带 [38]，或值是 "-"：必须是 None 而不是 0
    let mut f: Vec<&str> = vec!["1"; 34]; // 只有 34 段：[38] 越界
    f[1] = "腾讯控股";
    f[2] = "00700";
    f[3] = "438.800";
    f[4] = "440.000";
    let line = format!("v_hk00700=\"{}\";", f.join("~"));
    match parse_line(&line) {
        QuoteParseOutcome::Ok(q) => assert_eq!(q.amount, None),
        other => panic!("应解析成功，实际 {other:?}"),
    }

    let mut f2: Vec<&str> = vec!["1"; 39];
    f2[3] = "438.800";
    f2[38] = "-";
    let line2 = format!("v_hk00700=\"{}\";", f2.join("~"));
    match parse_line(&line2) {
        QuoteParseOutcome::Ok(q) => assert_eq!(q.amount, None, "占位符 - 必须是 None"),
        other => panic!("应解析成功，实际 {other:?}"),
    }
}
