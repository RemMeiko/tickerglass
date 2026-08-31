//! 腾讯分时解析：正常 / 午休 / 空 data / 0 价 / 缺 key / 非 JSON / 盘后裁点 / 港股午休。
//! 全部读固化 fixture，不发网络请求。

mod common;

use common::read_text;
use tickerglass_lib::domain::market::Market;
use tickerglass_lib::domain::session::index_of;
use tickerglass_lib::model::ParseError;
use tickerglass_lib::parse::tencent_minute::{parse_minute, raw_points};

#[test]
fn normal_series_parses_full_day() {
    let s = parse_minute(&read_text("minute_normal.json"), "sh000001").expect("正常样本应解析成功");
    assert_eq!(s.market, Market::IndexCn);
    assert_eq!(s.grid_len, 242);
    // 实测 sh000001 当日 242 点，首点 0930 末点 1500，全部落在栅格内
    assert_eq!(s.points.len(), 242, "242 个盘中点应全部落格");
    assert_eq!(s.points.first().map(|p| p.hhmm), Some(930));
    assert_eq!(s.points.last().map(|p| p.hhmm), Some(1500));
    assert_eq!(s.points.first().map(|p| p.grid_index), Some(0));
    assert_eq!(s.points.last().map(|p| p.grid_index), Some(241));
    // 昨收从 qt 节点取到，用于画基准线
    assert!(s.prev_close.is_some_and(|v| v > 0.0), "昨收应取到");
    assert!(s.points.iter().all(|p| p.price > 0.0));
}

#[test]
fn lunch_break_has_no_interpolated_points() {
    // 真实抓取的 11:28-13:02 片段（6 点：1128 1129 1130 1300 1301 1302）
    let s = parse_minute(&read_text("minute_with_lunch.json"), "sh600519").expect("应解析成功");
    let times: Vec<u16> = s.points.iter().map(|p| p.hhmm).collect();
    assert!(times.contains(&1130) && times.contains(&1300));
    assert!(
        !times.iter().any(|&t| t > 1130 && t < 1300),
        "11:30 与 13:00 之间不应有任何插值点，实际 {times:?}"
    );
}

#[test]
fn lunch_break_is_marked_by_break_before() {
    let s = parse_minute(&read_text("minute_with_lunch.json"), "sh600519").expect("应解析成功");
    // 13:00 那点的前一个有效点是 11:30，grid_index 不连续 → break_before = true
    let p1300 = s.points.iter().find(|p| p.hhmm == 1300).expect("应有 1300 点");
    assert!(
        p1300.break_before,
        "13:00 点必须标 break_before，前端据此断线而不是画横跨斜线"
    );
    // 序列首点恒为 false，其余连续点也为 false
    assert!(!s.points[0].break_before, "序列首点恒为 false");
    let breaks = s.points.iter().filter(|p| p.break_before).count();
    assert_eq!(breaks, 1, "该片段只应有午休这一个断点");
}

#[test]
fn ashare_lunch_grid_indexes_are_adjacent() {
    // 栅格上 1130 与 1300 是相邻刻度（午休不占格）
    let s = parse_minute(&read_text("minute_with_lunch.json"), "sh600519").expect("应解析成功");
    let i1130 = s.points.iter().find(|p| p.hhmm == 1130).unwrap().grid_index;
    let i1300 = s.points.iter().find(|p| p.hhmm == 1300).unwrap().grid_index;
    assert_eq!(i1130 + 1, i1300, "栅格上午休两端应相邻");
}

#[test]
fn empty_data_returns_empty_series_not_error() {
    // 盘前：data.data 为 []，应返回空序列而非 Err（昨收仍可用于画基准线）
    let s = parse_minute(&read_text("minute_empty_data.json"), "sh000001")
        .expect("空 data 应返回 Ok 而不是 Err");
    assert!(s.points.is_empty());
    assert!(s.prev_close.is_some_and(|v| v > 0.0), "昨收仍应可用");
    assert!(s.price_range().is_none(), "空序列无极值");
}

#[test]
fn zero_price_points_are_dropped_without_flattening_y_axis() {
    let s = parse_minute(&read_text("minute_zero_price.json"), "sh000001").expect("应解析成功");
    assert!(
        s.points.iter().all(|p| p.price > 0.0),
        "0 价 / 空价点必须被丢弃，否则 Y 轴被压到 0"
    );
    let (lo, _hi) = s.price_range().expect("应有极值");
    assert!(lo > 1000.0, "上证指数的最低价不应被 0 价点拉到 0，实际 {lo}");
    // fixture 里减法处理了 2 个点，故比 242 少 2
    assert_eq!(s.points.len(), 240);
}

#[test]
fn missing_data_key_returns_structure_error() {
    let err = parse_minute(&read_text("minute_missing_key.json"), "sh600519")
        .expect_err("缺 data.{code} 应返回 Structure");
    assert!(matches!(err, ParseError::Structure(_)), "实际 {err:?}");
}

#[test]
fn html_error_page_returns_not_json() {
    let err = parse_minute(&read_text("minute_html_error.html"), "sh600519")
        .expect_err("HTML 错误页应返回 NotJson");
    assert_eq!(err, ParseError::NotJson);
}

#[test]
fn after_close_points_are_trimmed_to_grid() {
    // 实测 sh600519 全量 267 点、范围 0930-1530，15:06-15:30 是盘后重复点
    let text = read_text("minute_after_close.json");
    let raw = raw_points(&text, "sh600519").expect("裸点应解析成功");
    assert_eq!(raw.len(), 267, "东财/腾讯原始返回 267 点");

    let s = parse_minute(&text, "sh600519").expect("应解析成功");
    assert!(s.points.len() <= 242, "落格后不应超过栅格长度");
    assert_eq!(s.points.last().map(|p| p.hhmm), Some(1500), "末点应为 1500");
    assert!(
        !s.points.iter().any(|p| p.hhmm > 1500),
        "15:06-15:30 的盘后点必须全部丢弃，否则曲线尾部拖出平直尾巴"
    );
    // 被丢弃的点数 = 原始 267 - 落格 242
    assert_eq!(raw.len() - s.points.len(), 25, "应丢弃 25 个盘后点");
}

#[test]
fn after_close_points_do_not_affect_y_axis_extremes() {
    let text = read_text("minute_after_close.json");
    let s = parse_minute(&text, "sh600519").expect("应解析成功");
    let (lo, hi) = s.price_range().expect("应有极值");

    // 手工只在盘中点上算一遍极值，两者必须完全相等
    let raw = raw_points(&text, "sh600519").unwrap();
    let intraday: Vec<f64> = raw
        .iter()
        .filter(|p| index_of(Market::Ashare, p.hhmm).is_some())
        .map(|p| p.price)
        .collect();
    let man_lo = intraday.iter().cloned().fold(f64::INFINITY, f64::min);
    let man_hi = intraday.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    assert_eq!(
        (lo, hi),
        (man_lo, man_hi),
        "Y 轴极值必须与仅含盘中点时完全相等（盘后重复点不参与）"
    );
}

#[test]
fn hk_lunch_break_is_at_1200_not_1130() {
    // 港股午休是 12:00-13:00（不是 A 股的 11:30-13:00）
    let s = parse_minute(&read_text("minute_hk_lunch.json"), "hk00700").expect("应解析成功");
    assert_eq!(s.market, Market::Hk);
    assert_eq!(s.grid_len, 332);

    let breaks: Vec<u16> = s
        .points
        .iter()
        .filter(|p| p.break_before)
        .map(|p| p.hhmm)
        .collect();
    assert_eq!(breaks, vec![1300], "唯一断点应在 1300（其前一点是 1200）");

    let p1300 = s.points.iter().find(|p| p.hhmm == 1300).unwrap();
    assert!(p1300.break_before);
    // 11:30 在港股是连续交易时段，不应是断点
    let p1130 = s.points.iter().find(|p| p.hhmm == 1130).expect("港股 11:30 仍在交易");
    assert!(!p1130.break_before, "港股 11:30 不是午休，不应断线");
    assert!(
        !s.points.iter().any(|p| p.hhmm > 1200 && p.hhmm < 1300),
        "12:00-13:00 之间不应有点"
    );
}

#[test]
fn hk_after_close_points_are_dropped() {
    let text = read_text("minute_hk_lunch.json");
    let raw = raw_points(&text, "hk00700").expect("裸点应解析成功");
    assert_eq!(raw.len(), 332, "实测港股原始 332 点（含盘后 1608）");

    let s = parse_minute(&text, "hk00700").expect("应解析成功");
    assert!(
        !s.points.iter().any(|p| p.hhmm > 1600),
        "1601-1608 的盘后点必须丢弃"
    );
    // 实测港股午盘只到 1559，1600 这一刻度不存在成交
    assert_eq!(s.points.last().map(|p| p.hhmm), Some(1559));
    assert_eq!(s.points.len(), 331, "332 原始点丢掉 1 个盘后点 1608");
}
