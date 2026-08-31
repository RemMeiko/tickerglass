//! 5 日分时解析：腾讯路径（A股/指数/港股）+ 东财路径（美股）。
//!
//! 全部读固化 fixture，不发网络请求。三个 fixture 都是 2026-08-26 抓的真实报文：
//! - `tencent_5day_ashare.json`      —— 腾讯 sh600519，5 天，验排序与差分
//! - `tencent_5day_us_rejected.json` —— 腾讯 usAAPL，`code:-1 param error`，验拒绝
//! - `em_trends_5day_us.json`        —— 东财 105.AAPL ndays=5，1682 点，验美东分组
//!
//! 这组测试的核心是两个**曾经真实存在的 bug**：
//! 1. 腾讯返回最新在前，链路上无任何排序 → 5 日图时间轴倒着画
//! 2. 腾讯不供美股 5 日 → 苹果那档永远「获取失败」

mod common;

use common::read_text;
use tickerglass_lib::parse::em_trends::parse_trends_5day;
use tickerglass_lib::parse::tencent_5day::{parse_5day, FiveDayParseError};

/// A股交易时段（含午休），用于替代 domain::session 的注入 ——
/// 测试里显式写出来，避免断言随 domain 改动而漂移。
fn ashare_session(hhmm: u16) -> bool {
    (930..=1130).contains(&hhmm) || (1300..=1500).contains(&hhmm)
}

/// 美股 09:30–16:00 连续时段（美东刻度）
fn us_session(hhmm: u16) -> bool {
    (930..=1600).contains(&hhmm)
}

// ───────── 腾讯路径：排序（这是修过的 bug）─────────

#[test]
fn tencent_5day_returns_days_in_ascending_date_order() {
    // 报文原序是最新在前（20260826, 20260825, ...），而前端 segments[0] 画最左侧。
    // 不排序的话 5 日图左边是今天、右边是最早那天，时间轴整体倒着走。
    let days = parse_5day(
        &read_text("tencent_5day_ashare.json"),
        "sh600519",
        ashare_session,
    )
    .expect("真实 A股报文应解析成功");

    let dates: Vec<&str> = days.iter().map(|d| d.date.as_str()).collect();
    let mut sorted = dates.clone();
    sorted.sort_unstable();
    assert_eq!(dates, sorted, "5 日必须按日期升序，否则前端画反");
}

#[test]
fn tencent_5day_first_segment_is_oldest_day() {
    let days = parse_5day(
        &read_text("tencent_5day_ashare.json"),
        "sh600519",
        ashare_session,
    )
    .expect("应解析成功");
    let first = &days[0].date;
    let last = &days[days.len() - 1].date;
    assert!(first < last, "首段({first}) 应早于末段({last})");
}

// ───────── 腾讯路径：不供美股（这是用户报的 bug 的根因）─────────

#[test]
fn tencent_5day_rejects_us_code_with_nonzero_code() {
    // 腾讯这条通道不供美股：实测六种代码写法全失败。
    // 解析器必须把 code!=0 判成 Rejected 而不是退化成空数据或 panic。
    let err = parse_5day(
        &read_text("tencent_5day_us_rejected.json"),
        "usAAPL",
        us_session,
    )
    .expect_err("美股在腾讯 5 日通道上必须失败");

    match err {
        FiveDayParseError::Rejected(msg) => {
            assert!(msg.contains("-1"), "应带上 code=-1，实际: {msg}");
        }
        other => panic!("应为 Rejected，实际: {other:?}"),
    }
}

// ───────── 东财路径：美股 5 日（新增通道）─────────

#[test]
fn em_5day_groups_into_five_us_east_trading_days() {
    // 报文里是 6 个**北京**日期，但只有 5 个**美东**交易日 ——
    // 一个美股交易日跨两个北京日期（美东 09:30 = 北京 21:30，收盘落到次日 04:00）。
    // 按北京日期分组会得到 6 段且把相邻两日的尾头拼一起，所以这条断言是防线。
    let days = parse_trends_5day(&read_text("em_trends_5day_us.json"), us_session)
        .expect("真实东财 5 日报文应解析成功");
    assert_eq!(days.len(), 5, "应为 5 个美东交易日");
}

#[test]
fn em_5day_dates_are_ascending_and_skip_weekend() {
    let days = parse_trends_5day(&read_text("em_trends_5day_us.json"), us_session)
        .expect("应解析成功");
    let dates: Vec<&str> = days.iter().map(|d| d.date.as_str()).collect();
    // 8/22 是周六、8/23 周日 —— 报文里没有，分组结果也不该凭空补出来
    assert_eq!(
        dates,
        vec!["20260820", "20260821", "20260824", "20260825", "20260826"]
    );
}

#[test]
fn em_5day_complete_sessions_have_full_391_points() {
    // 美股栅格是 391 格（09:30–16:00 含两端）。
    // 前四天是完整交易日，实测各 391 点；末日是当天，进行中故不足。
    let days = parse_trends_5day(&read_text("em_trends_5day_us.json"), us_session)
        .expect("应解析成功");
    for d in &days[..4] {
        assert_eq!(d.points.len(), 391, "{} 应是完整交易日", d.date);
    }
    assert!(
        days[4].points.len() < 391,
        "末日是当天，应不足 391（实际 {}）",
        days[4].points.len()
    );
}

#[test]
fn em_5day_points_span_open_to_close() {
    let days = parse_trends_5day(&read_text("em_trends_5day_us.json"), us_session)
        .expect("应解析成功");
    let d = &days[0];
    assert_eq!(d.points[0].hhmm, 930, "首点应是开盘 09:30");
    assert_eq!(
        d.points[d.points.len() - 1].hhmm,
        1600,
        "完整交易日末点应是收盘 16:00"
    );
}

#[test]
fn em_5day_points_are_sorted_within_day() {
    let days = parse_trends_5day(&read_text("em_trends_5day_us.json"), us_session)
        .expect("应解析成功");
    for d in &days {
        let ticks: Vec<u16> = d.points.iter().map(|p| p.hhmm).collect();
        let mut sorted = ticks.clone();
        sorted.sort_unstable();
        assert_eq!(ticks, sorted, "{} 日内刻度应升序", d.date);
    }
}

#[test]
fn em_5day_volume_is_per_minute_not_cumulative() {
    // **与腾讯路径相反**：东财 trends2 的量已是逐分钟值。
    // 腾讯那条是当日累计值必须差分；这里若照抄差分会得到一堆 0 与负数。
    // 判据：累计值必然单调不减，逐分钟值不会。
    let days = parse_trends_5day(&read_text("em_trends_5day_us.json"), us_session)
        .expect("应解析成功");
    let vols: Vec<f64> = days[0].points.iter().map(|p| p.volume).collect();
    let monotonic = vols.windows(2).all(|w| w[0] <= w[1]);
    assert!(
        !monotonic,
        "东财量应是逐分钟值（非单调）；若单调递增说明把累计值当逐分钟用了"
    );
    assert!(vols.iter().all(|v| *v >= 0.0), "量不得为负");
}

#[test]
fn em_5day_prev_close_chains_from_previous_session_last_price() {
    // trends2 响应**不含 preClose**，故第 2~5 天的昨收用前一交易日最后一个价推导。
    let days = parse_trends_5day(&read_text("em_trends_5day_us.json"), us_session)
        .expect("应解析成功");
    for i in 1..days.len() {
        let expect = days[i - 1].points.last().unwrap().price;
        let got = days[i].prev_close.expect("第 2 天起应有昨收");
        assert!(
            (got - expect).abs() < 1e-9,
            "{} 的昨收应等于前一日末价 {expect}，实际 {got}",
            days[i].date
        );
    }
}

#[test]
fn em_5day_oldest_day_has_no_prev_close() {
    // 最早那天没有前一日可依据 —— 记 None 而不是猜一个值（前端会退化用首点当基准）
    let days = parse_trends_5day(&read_text("em_trends_5day_us.json"), us_session)
        .expect("应解析成功");
    assert_eq!(days[0].prev_close, None, "最早一天不该凭空造昨收");
}

#[test]
fn em_5day_drops_out_of_session_points() {
    // 注入一个"全都不在时段内"的判定 —— 所有点都该被丢弃，进而返回 Err 而不是空数组
    let err = parse_trends_5day(&read_text("em_trends_5day_us.json"), |_| false)
        .expect_err("全部点被丢弃时应报错而不是返回空");
    let msg = format!("{err:?}");
    assert!(msg.contains("无可用点"), "实际: {msg}");
}

// ───────── 东财路径：坏输入 ─────────

#[test]
fn em_5day_rejects_non_json() {
    assert!(parse_trends_5day("not json at all", us_session).is_err());
    assert!(parse_trends_5day("", us_session).is_err());
}

#[test]
fn em_5day_rejects_null_data() {
    // 无效 secid 时东财返回 data:null —— 必须报错，不能退化成造点
    let err = parse_trends_5day(r#"{"rc":0,"data":null}"#, us_session)
        .expect_err("data 为 null 应报错");
    let msg = format!("{err:?}");
    assert!(msg.contains("null"), "实际: {msg}");
}

#[test]
fn em_5day_rejects_missing_trends() {
    let err = parse_trends_5day(r#"{"rc":0,"data":{"code":"AAPL"}}"#, us_session)
        .expect_err("缺 trends 应报错");
    let msg = format!("{err:?}");
    assert!(msg.contains("trends"), "实际: {msg}");
}

#[test]
fn em_5day_skips_malformed_rows_but_keeps_good_ones() {
    // 字段不足 / 时间不可解析 / 价格非正 的行逐条跳过，不整体失败
    let json = r#"{"rc":0,"data":{"trends":[
        "2026-08-25 21:30,1,310.5,1,1,100,1,1",
        "too,few",
        "bad-time,1,311.0,1,1,100,1,1",
        "2026-08-25 21:31,1,-5,1,1,100,1,1",
        "2026-08-25 21:32,1,311.5,1,1,200,1,1"
    ]}}"#;
    let days = parse_trends_5day(json, us_session).expect("应保留可用行");
    assert_eq!(days.len(), 1);
    // 5 行里只有第 1、5 行可用（第 4 行价格为负被剔除）
    assert_eq!(days[0].points.len(), 2);
    assert_eq!(days[0].points[0].hhmm, 930);
    assert_eq!(days[0].points[1].hhmm, 932);
}
