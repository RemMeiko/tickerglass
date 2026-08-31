//! 日K 与 5 日分时解析测试。纯离线：不发任何网络请求。
//!
//! 报文形态取自 2026-08-24 对真实接口的实测（见 sprint-status.md「日K / 多周期数据源」），
//! 为保持测试自包含且聚焦，此处按实测结构构造最小样本，
//! 关键字段顺序与陷阱（开收高低、累计量、盘后点）与真实报文一致。

use tickerglass_lib::parse::em_kline::{moving_average, parse_em_kline, KlineParseError};
use tickerglass_lib::parse::tencent_5day::{parse_5day, FiveDayParseError};

/// 真实字段顺序：`日期,开,收,高,低,量,额,涨跌幅` —— 注意第 2 段是**收**不是高
const KLINE_OK: &str = r#"{"rc":0,"data":{"name":"苹果","klines":[
"2026-08-19,310.140,316.830,319.280,309.600,50505646,15950754816.000,3.12",
"2026-08-20,317.455,311.300,320.280,310.650,40959184,12864092160.000,3.04",
"2026-08-21,312.050,309.350,312.380,307.010,46876815,14517834752.000,1.73"
]}}"#;

#[test]
fn kline_parses_three_bars() {
    let bars = parse_em_kline(KLINE_OK).expect("应解析成功");
    assert_eq!(bars.len(), 3);
}

#[test]
fn kline_field_order_is_open_close_high_low_not_ohlc() {
    // 这是最容易错的一处：报文第 2 段是收盘价，第 3 段才是最高价。
    // 若按 OHLC 惯例直读，收盘价会被当成最高价。
    let bars = parse_em_kline(KLINE_OK).unwrap();
    let last = &bars[2];
    assert_eq!(last.date, "2026-08-21");
    assert_eq!(last.open, 312.050);
    assert_eq!(last.close, 309.350, "第 2 段必须映射到 close");
    assert_eq!(last.high, 312.380, "第 3 段必须映射到 high");
    assert_eq!(last.low, 307.010);
}

#[test]
fn kline_high_is_never_below_low() {
    let bars = parse_em_kline(KLINE_OK).unwrap();
    for b in &bars {
        assert!(b.high >= b.low, "{} 高低颠倒", b.date);
        assert!(b.high >= b.open && b.high >= b.close, "{} 最高价不是最高", b.date);
        assert!(b.low <= b.open && b.low <= b.close, "{} 最低价不是最低", b.date);
    }
}

#[test]
fn kline_keeps_volume_for_bars() {
    let bars = parse_em_kline(KLINE_OK).unwrap();
    assert_eq!(bars[2].volume, 46876815.0, "量柱要用它，不能丢");
}

#[test]
fn kline_skips_broken_row_but_keeps_others() {
    // 一根坏K线不该让整张图消失
    let text = r#"{"rc":0,"data":{"klines":[
"2026-08-19,310.140,316.830,319.280,309.600,50505646",
"坏行没有逗号分段",
"2026-08-21,312.050,309.350,312.380,307.010,46876815"
]}}"#;
    let bars = parse_em_kline(text).unwrap();
    assert_eq!(bars.len(), 2, "跳过坏行，保留其余");
}

#[test]
fn kline_rejects_row_with_high_below_low() {
    let text = r#"{"rc":0,"data":{"klines":[
"2026-08-21,312.050,309.350,300.000,307.010,46876815"
]}}"#;
    // high(300) < low(307) 属坏数据，丢弃后无可用行
    assert_eq!(parse_em_kline(text), Err(KlineParseError::Empty));
}

#[test]
fn kline_rejects_non_positive_price() {
    let text = r#"{"rc":0,"data":{"klines":[
"2026-08-21,0,0,0,0,100"
]}}"#;
    assert_eq!(parse_em_kline(text), Err(KlineParseError::Empty));
}

#[test]
fn kline_reports_rejected_on_nonzero_rc() {
    let text = r#"{"rc":1,"data":null}"#;
    assert!(matches!(
        parse_em_kline(text),
        Err(KlineParseError::Rejected(_))
    ));
}

#[test]
fn kline_reports_malformed_on_non_json() {
    assert!(matches!(
        parse_em_kline("<html>404</html>"),
        Err(KlineParseError::Malformed(_))
    ));
}

#[test]
fn kline_reports_malformed_when_klines_missing() {
    assert!(matches!(
        parse_em_kline(r#"{"rc":0,"data":{"name":"苹果"}}"#),
        Err(KlineParseError::Malformed(_))
    ));
}

#[test]
fn kline_reports_empty_on_empty_array() {
    assert_eq!(
        parse_em_kline(r#"{"rc":0,"data":{"klines":[]}}"#),
        Err(KlineParseError::Empty)
    );
}

#[test]
fn ma_is_none_until_window_is_full() {
    let bars = parse_em_kline(KLINE_OK).unwrap();
    let ma5 = moving_average(&bars, 5);
    assert_eq!(ma5.len(), 3, "长度必须与输入等长，前端按索引对齐");
    assert!(ma5.iter().all(|x| x.is_none()), "3 根不足 5 日窗口，不得编造均值");
}

#[test]
fn ma_computes_when_window_is_full() {
    let text = r#"{"rc":0,"data":{"klines":[
"2026-08-15,10,10,10,10,1",
"2026-08-16,10,20,20,10,1",
"2026-08-17,10,30,30,10,1"
]}}"#;
    let bars = parse_em_kline(text).unwrap();
    let ma3 = moving_average(&bars, 3);
    assert_eq!(ma3[0], None);
    assert_eq!(ma3[1], None);
    // (10+20+30)/3 = 20
    assert_eq!(ma3[2], Some(20.0));
}

#[test]
fn ma_window_slides_not_cumulates() {
    let text = r#"{"rc":0,"data":{"klines":[
"2026-08-15,1,1,1,1,1",
"2026-08-16,1,1,1,1,1",
"2026-08-17,1,4,4,1,1",
"2026-08-18,1,7,7,1,1"
]}}"#;
    let bars = parse_em_kline(text).unwrap();
    let ma2 = moving_average(&bars, 2);
    assert_eq!(ma2[1], Some(1.0));
    assert_eq!(ma2[2], Some(2.5), "(1+4)/2 —— 窗口要滑动，不是累计平均");
    assert_eq!(ma2[3], Some(5.5), "(4+7)/2");
}

// ───────────────────── 5 日分时 ─────────────────────

/// 行格式 `HHMM 价 累计量 累计额`。含 A股盘后重复点 1506/1530（须丢弃）。
const FIVEDAY_OK: &str = r#"{"code":0,"data":{"sh600519":{"data":[
{"date":"20260821","prec":"1291.50","data":[
"0930 1291.50 100 129150.00",
"0931 1285.00 300 385500.00",
"1130 1280.00 500 640000.00",
"1300 1282.00 700 897400.00",
"1500 1272.83 900 1145547.00",
"1506 1272.83 900 1145547.00",
"1530 1272.83 900 1145547.00"
]}]}}}"#;

/// A股栅格判定：0930-1130 与 1300-1500 之内才算交易时段
fn ashare_in_session(hhmm: u16) -> bool {
    (930..=1130).contains(&hhmm) || (1300..=1500).contains(&hhmm)
}

#[test]
fn fiveday_parses_one_day() {
    let days = parse_5day(FIVEDAY_OK, "sh600519", ashare_in_session).unwrap();
    assert_eq!(days.len(), 1);
    assert_eq!(days[0].date, "20260821");
}

#[test]
fn fiveday_reads_prev_close_from_prec() {
    let days = parse_5day(FIVEDAY_OK, "sh600519", ashare_in_session).unwrap();
    assert_eq!(days[0].prev_close, Some(1291.50), "每天各有自己的昨收基准");
}

#[test]
fn fiveday_drops_after_hours_points() {
    // 1506 与 1530 是盘后重复点，与当日分时同一个坑：不在栅格内须丢弃，
    // 否则会污染 Y 轴极值并在图右侧拖出一条平线
    let days = parse_5day(FIVEDAY_OK, "sh600519", ashare_in_session).unwrap();
    let hhmms: Vec<u16> = days[0].points.iter().map(|p| p.hhmm).collect();
    assert_eq!(hhmms, vec![930, 931, 1130, 1300, 1500]);
    assert!(!hhmms.contains(&1506));
    assert!(!hhmms.contains(&1530));
}

#[test]
fn fiveday_differentiates_cumulative_volume() {
    // 报文的量是**当日累计**，直接用会画出单调递增的斜坡而非量柱
    let days = parse_5day(FIVEDAY_OK, "sh600519", ashare_in_session).unwrap();
    let vols: Vec<f64> = days[0].points.iter().map(|p| p.volume).collect();
    assert_eq!(vols, vec![100.0, 200.0, 200.0, 200.0, 200.0]);
}

#[test]
fn fiveday_clamps_volume_regression_to_zero() {
    // 累计值回退（数据抖动）时该分钟量记 0，不产生负量柱
    let text = r#"{"code":0,"data":{"sh600519":{"data":[
{"date":"20260821","prec":"100","data":[
"0930 100 500 1",
"0931 101 300 1"
]}]}}}"#;
    let days = parse_5day(text, "sh600519", ashare_in_session).unwrap();
    assert_eq!(days[0].points[1].volume, 0.0);
}

#[test]
fn fiveday_skips_non_numeric_and_short_rows() {
    let text = r#"{"code":0,"data":{"sh600519":{"data":[
{"date":"20260821","prec":"100","data":[
"0930 100 10 1",
"0931 abc 20 1",
"0932",
"0933 103 40 1"
]}]}}}"#;
    let days = parse_5day(text, "sh600519", ashare_in_session).unwrap();
    let hhmms: Vec<u16> = days[0].points.iter().map(|p| p.hhmm).collect();
    assert_eq!(hhmms, vec![930, 933]);
}

#[test]
fn fiveday_reports_rejected_on_nonzero_code() {
    assert!(matches!(
        parse_5day(r#"{"code":-1,"data":null}"#, "sh600519", ashare_in_session),
        Err(FiveDayParseError::Rejected(_))
    ));
}

#[test]
fn fiveday_reports_malformed_when_code_node_missing() {
    // 请求 A 的代码但响应里只有 B —— 不能静默返回空图
    let text = r#"{"code":0,"data":{"sz000001":{"data":[]}}}"#;
    assert!(matches!(
        parse_5day(text, "sh600519", ashare_in_session),
        Err(FiveDayParseError::Malformed(_))
    ));
}

#[test]
fn fiveday_reports_empty_when_all_points_out_of_session() {
    // 全是盘后点 → 丢完后无可用数据，不能当成"这只股票没数据"
    let text = r#"{"code":0,"data":{"sh600519":{"data":[
{"date":"20260821","prec":"100","data":["1506 100 1 1","1530 100 1 1"]}]}}}"#;
    assert_eq!(
        parse_5day(text, "sh600519", ashare_in_session),
        Err(FiveDayParseError::Empty)
    );
}

#[test]
fn fiveday_handles_missing_prec() {
    let text = r#"{"code":0,"data":{"sh600519":{"data":[
{"date":"20260821","data":["0930 100 10 1"]}]}}}"#;
    let days = parse_5day(text, "sh600519", ashare_in_session).unwrap();
    assert_eq!(days[0].prev_close, None, "拿不到昨收就是 None，不编造");
}

#[test]
fn fiveday_hk_lunch_differs_from_ashare() {
    // 港股午休是 12:00-13:00，A股是 11:30-13:00。
    // 判定函数由调用方按市场传入，解析层不硬编码任何市场的时段。
    let text = r#"{"code":0,"data":{"hk00700":{"data":[
{"date":"20260821","prec":"451","data":[
"1145 452 10 1",
"1200 453 20 1",
"1230 454 30 1",
"1300 455 40 1"
]}]}}}"#;
    let hk_in_session =
        |h: u16| (930..=1200).contains(&h) || (1300..=1600).contains(&h);
    let days = parse_5day(text, "hk00700", hk_in_session).unwrap();
    let hhmms: Vec<u16> = days[0].points.iter().map(|p| p.hhmm).collect();
    // 1230 在港股午休内被丢；1145 与 1200 都在交易时段（A股规则下 1145 会被丢）
    assert_eq!(hhmms, vec![1145, 1200, 1300]);
}
