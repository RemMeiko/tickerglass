//! 腾讯日K解析。全部读固化 fixture，不发网络请求。
//!
//! 三个 fixture 都是 2026-08-27 抓的真实报文：
//! `tencent_kline_ashare.json`(sh600519) / `tencent_kline_us.json`(usAAPL.OQ) /
//! `tencent_kline_hk.json`(hk00700)。
//!
//! 这组测试盯住三个真实踩过的坑：
//! 1. 报文第 2 段是**收**不是高（`日期 开 收 高 低 量`）
//! 2. 美股不带 `.OQ`/`.N` 后缀会**静默**只返 2 条（不报错）
//! 3. 港股行末多挂一个公告对象，段数比其他市场多

mod common;

use common::read_text;
use tickerglass_lib::net::tencent_kline_code;
use tickerglass_lib::parse::em_kline::KlineParseError;
use tickerglass_lib::parse::tencent_kline::parse_tencent_kline;

// ───────── 字段顺序（第 2 段是收不是高）─────────

#[test]
fn second_field_is_close_not_high() {
    // 实测末根 sh600519：开 1300.000 收 1302.800 高 1314.450 低 1295.000。
    // 若按 OHLC 直觉读，会把 1302.800 当成最高价。
    let bars = parse_tencent_kline(&read_text("tencent_kline_ashare.json"), "sh600519")
        .expect("真实 A股日K 应解析成功");
    let last = bars.last().expect("应有至少一根");
    assert_eq!(last.date, "2026-08-26");
    assert!((last.open - 1300.0).abs() < 1e-6, "开={}", last.open);
    assert!((last.close - 1302.8).abs() < 1e-6, "收={}", last.close);
    assert!((last.high - 1314.45).abs() < 1e-6, "高={}", last.high);
    assert!((last.low - 1295.0).abs() < 1e-6, "低={}", last.low);
}

#[test]
fn high_is_max_and_low_is_min_across_all_bars() {
    // 结构性不变量：任何一根里 high 必须 ≥ max(open,close)、low ≤ min(open,close)。
    // 字段错位一定会破坏它 —— 比逐根写死数值更能防回归。
    for f in [
        "tencent_kline_ashare.json",
        "tencent_kline_us.json",
        "tencent_kline_hk.json",
    ] {
        let code = match f {
            "tencent_kline_ashare.json" => "sh600519",
            "tencent_kline_us.json" => "usAAPL.OQ",
            _ => "hk00700",
        };
        let bars = parse_tencent_kline(&read_text(f), code).expect("应解析成功");
        for b in &bars {
            assert!(
                b.high >= b.open.max(b.close) - 1e-6,
                "{f} {} high={} < max(open={},close={})",
                b.date,
                b.high,
                b.open,
                b.close
            );
            assert!(
                b.low <= b.open.min(b.close) + 1e-6,
                "{f} {} low={} > min(open={},close={})",
                b.date,
                b.low,
                b.open,
                b.close
            );
        }
    }
}

// ───────── 三市场都能解析 ─────────

#[test]
fn parses_all_three_markets_with_expected_bar_counts() {
    let a = parse_tencent_kline(&read_text("tencent_kline_ashare.json"), "sh600519").unwrap();
    let u = parse_tencent_kline(&read_text("tencent_kline_us.json"), "usAAPL.OQ").unwrap();
    let h = parse_tencent_kline(&read_text("tencent_kline_hk.json"), "hk00700").unwrap();
    // 请求 lmt=120，美股实测会多返 1 根（当日在内），故用范围断言
    assert!((115..=125).contains(&a.len()), "A股 {} 根", a.len());
    assert!((115..=125).contains(&u.len()), "美股 {} 根", u.len());
    assert!((115..=125).contains(&h.len()), "港股 {} 根", h.len());
}

#[test]
fn hk_rows_with_trailing_announcement_object_still_parse() {
    // 港股行末多挂 {cqr, FHcontent, HGcontent, ...} 公告对象 ——
    // 解析器按位置取前 6 段、不校验段数相等，故这些行必须照样出来
    let bars = parse_tencent_kline(&read_text("tencent_kline_hk.json"), "hk00700")
        .expect("港股应解析成功");
    let last = bars.last().unwrap();
    assert_eq!(last.date, "2026-08-26");
    assert!(last.close > 0.0 && last.volume > 0.0);
}

#[test]
fn dates_are_ascending() {
    // 腾讯日K 是最早在前。CandleChart 按数组顺序从左画到右，顺序反了图就反。
    let bars = parse_tencent_kline(&read_text("tencent_kline_ashare.json"), "sh600519").unwrap();
    let dates: Vec<&str> = bars.iter().map(|b| b.date.as_str()).collect();
    let mut sorted = dates.clone();
    sorted.sort_unstable();
    assert_eq!(dates, sorted, "日K 应按日期升序");
}

// ───────── 美股后缀推导（静默降级的防线）─────────

#[test]
fn us_code_gets_nasdaq_suffix_from_105_prefix() {
    assert_eq!(tencent_kline_code("usAAPL", Some("105.AAPL")), "usAAPL.OQ");
    assert_eq!(tencent_kline_code("usTSLA", Some("105.TSLA")), "usTSLA.OQ");
}

#[test]
fn us_code_gets_nyse_suffix_from_106_prefix() {
    // 实测 usJPM.OQ 只返 1 条、usJPM.N 正常 —— 二者不可互换
    assert_eq!(tencent_kline_code("usJPM", Some("106.JPM")), "usJPM.N");
    assert_eq!(tencent_kline_code("usKO", Some("106.KO")), "usKO.N");
}

#[test]
fn us_code_without_secid_falls_back_to_nasdaq() {
    // 退回 .OQ 而不是放弃：错了只是"数据少"，比直接不可用更可用
    assert_eq!(tencent_kline_code("usAAPL", None), "usAAPL.OQ");
    assert_eq!(tencent_kline_code("usAAPL", Some("999.AAPL")), "usAAPL.OQ");
}

#[test]
fn non_us_codes_pass_through_unchanged() {
    // A股/指数/港股不加后缀 —— 加了会请求失败
    assert_eq!(tencent_kline_code("sh600519", Some("1.600519")), "sh600519");
    assert_eq!(tencent_kline_code("sh000001", Some("1.000001")), "sh000001");
    assert_eq!(tencent_kline_code("hk00700", Some("116.00700")), "hk00700");
    assert_eq!(tencent_kline_code("sz000001", Some("0.000001")), "sz000001");
}

// ───────── 坏输入 ─────────

#[test]
fn rejects_non_json() {
    assert!(matches!(
        parse_tencent_kline("not json", "sh600519"),
        Err(KlineParseError::Malformed(_))
    ));
    assert!(matches!(
        parse_tencent_kline("", "sh600519"),
        Err(KlineParseError::Malformed(_))
    ));
}

#[test]
fn rejects_nonzero_code() {
    assert!(matches!(
        parse_tencent_kline(r#"{"code":-1,"msg":"param error","data":[]}"#, "sh600519"),
        Err(KlineParseError::Rejected(_))
    ));
}

#[test]
fn rejects_missing_node() {
    // 请求代码与响应键不匹配（如美股忘了带后缀）时必须报错，不能静默返空
    let err = parse_tencent_kline(&read_text("tencent_kline_ashare.json"), "usAAPL.OQ")
        .expect_err("键不匹配应报错");
    assert!(matches!(err, KlineParseError::Malformed(_)));
}

#[test]
fn accepts_day_key_when_not_adjusted() {
    // 不复权时键名是 day 而非 qfqday —— 两者都要认
    let json = r#"{"code":0,"data":{"sh600519":{"day":[
        ["2026-08-25","100.0","102.0","103.0","99.0","1000"],
        ["2026-08-26","102.0","101.0","104.0","100.5","2000"]
    ]}}}"#;
    let bars = parse_tencent_kline(json, "sh600519").expect("day 键应被接受");
    assert_eq!(bars.len(), 2);
    assert!((bars[1].close - 101.0).abs() < 1e-9);
}

#[test]
fn skips_malformed_rows_but_keeps_good_ones() {
    let json = r#"{"code":0,"data":{"sh600519":{"qfqday":[
        ["2026-08-25","100.0","102.0","103.0","99.0","1000"],
        ["too","few"],
        ["2026-08-26","0","102.0","103.0","99.0","1000"],
        ["2026-08-27","100.0","101.0","104.0","100.5","0"]
    ]}}}"#;
    let bars = parse_tencent_kline(json, "sh600519").expect("应保留可用行");
    // 第 2 行段数不足、第 3 行开盘价为 0 → 都剔除；第 4 行量为 0 是合法的（停牌）
    assert_eq!(bars.len(), 2);
    assert_eq!(bars[0].date, "2026-08-25");
    assert_eq!(bars[1].date, "2026-08-27");
    assert_eq!(bars[1].volume, 0.0);
}

#[test]
fn empty_array_reports_empty() {
    assert!(matches!(
        parse_tencent_kline(r#"{"code":0,"data":{"sh600519":{"qfqday":[]}}}"#, "sh600519"),
        Err(KlineParseError::Empty)
    ));
}
