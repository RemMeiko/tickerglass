//! 东财 trends2 分时解析（美股分时源）+ 北京→美东刻度换算。
//!
//! 这组测试的核心是**落格断言必须穿过 `attach_grid`** —— 只断言"解析出 N 点、hhmm 正确"
//! 会恰好停在 `attach_grid` 之前，抓不住"点位全落在栅格外被丢弃"这个 bug。
//! 全部读固化 fixture，不发网络请求。

mod common;

use common::read_text;
use tickerglass_lib::domain::market::Market;
use tickerglass_lib::domain::session::attach_grid;
use tickerglass_lib::model::ParseError;
use tickerglass_lib::parse::em_trends::{bj_to_us_east_hhmm, parse_trends, raw_points};

// ---------- 1. 纯换算断言（3 条，逐条独立命名）----------

#[test]
fn bj_to_us_east_summer_open() {
    // 夏令时 EDT，时差 12h：北京 21:30 = 美东 09:30 开盘
    assert_eq!(bj_to_us_east_hhmm("2026-08-21 21:30"), Some(930));
}

#[test]
fn bj_to_us_east_summer_close_crosses_midnight() {
    // 夏令时 EDT，跨零点：北京 8/22 04:00 = 美东 8/21 16:00 收盘（日期回退一天）
    assert_eq!(bj_to_us_east_hhmm("2026-08-22 04:00"), Some(1600));
}

#[test]
fn bj_to_us_east_winter_open_uses_13h_offset() {
    // 冬令时 EST，时差 13h：北京 22:30 = 美东 09:30 开盘
    // 这一条专门防固定 -12h 硬编码（写死 12h 会得到 1030，整体偏移 60 格）
    assert_eq!(bj_to_us_east_hhmm("2026-12-15 22:30"), Some(930));
}

#[test]
fn bj_to_us_east_rejects_bad_input() {
    assert_eq!(bj_to_us_east_hhmm(""), None);
    assert_eq!(bj_to_us_east_hhmm("2026-08-21"), None);
    assert_eq!(bj_to_us_east_hhmm("not a time"), None);
}

// ---------- 2. em_trends_us.json（夏令时真实响应）----------

#[test]
fn us_trends_parses_at_least_50_points() {
    let raw = raw_points(&read_text("em_trends_us.json")).expect("真实响应应解析成功");
    assert!(
        raw.len() >= 50,
        "东财 trends2 应返回 ≥50 点（腾讯只给 1 点才需要走这条），实际 {}",
        raw.len()
    );
}

#[test]
fn us_trends_first_point_converts_2130_to_930() {
    // fixture 首点是北京 2026-08-21 21:30，换算后必须是美东 930
    let raw = raw_points(&read_text("em_trends_us.json")).expect("应解析成功");
    assert_eq!(
        raw.first().map(|p| p.hhmm),
        Some(930),
        "首点北京 21:30 应换算为美东 930"
    );
}

#[test]
fn us_trends_price_takes_close_field() {
    // 每项 `时间,开,收,高,低,成交量,成交额,均价`，price 取「收」（索引 2）
    let text = read_text("em_trends_us.json");
    let raw = raw_points(&text).expect("应解析成功");
    let root: serde_json::Value = serde_json::from_str(&text).unwrap();
    let first_item = root["data"]["trends"][0].as_str().unwrap();
    let parts: Vec<&str> = first_item.split(',').collect();
    let close: f64 = parts[2].parse().unwrap();
    let open: f64 = parts[1].parse().unwrap();
    assert_eq!(raw[0].price, close, "price 应取「收」字段");
    assert_ne!(open, close, "该 fixture 首点开≠收，故这条断言有区分力");
}

#[test]
fn us_trends_output_is_structurally_same_as_tencent_path() {
    let s = parse_trends(&read_text("em_trends_us.json"), "usAAPL").expect("应解析成功");
    // 输出同一个 MinuteSeries 类型，下游 session / 前端完全复用
    assert_eq!(s.code, "usAAPL");
    assert_eq!(s.market, Market::Us);
    assert_eq!(s.grid_len, 391);
    assert!(s.points.iter().all(|p| p.price > 0.0));
    assert!(!s.points[0].break_before, "序列首点恒为 false");
}

#[test]
fn us_trends_points_actually_land_inside_the_grid() {
    // 【核心断言】必须穿过 attach_grid —— 证明美股点位真的落在 391 格美东栅格内。
    // 若时区换算缺失或算错，点位会是 2130..0400，全部落在 930..1600 之外被丢弃，
    // 这里就会是 0，而只断言解析后点数的写法抓不住。
    let raw = raw_points(&read_text("em_trends_us.json")).expect("应解析成功");
    let landed = attach_grid(Market::Us, &raw);
    assert!(
        landed.len() >= 50,
        "attach_grid 后应剩 ≥50 点（原始 {} 点），实际 {} —— 为 0 说明时区换算没生效",
        raw.len(),
        landed.len()
    );
    assert!(
        landed.iter().all(|p| p.grid_index < 391),
        "所有点的 grid_index 必须在 391 格内"
    );
}

#[test]
fn us_trends_full_pipeline_yields_curve_points() {
    // 从 parse_trends（内部已调用 attach_grid）这条真实路径再验一次
    let s = parse_trends(&read_text("em_trends_us.json"), "usAAPL").expect("应解析成功");
    assert!(
        s.points.len() >= 50,
        "完整管线后应剩 ≥50 点，实际 {} —— 少于 50 苹果那行就画不出曲线",
        s.points.len()
    );
    assert_eq!(s.points.first().map(|p| p.grid_index), Some(0), "美东 930 = 第 0 格");
}

// ---------- 3. em_trends_winter.json（冬令时，防硬编码时差）----------

#[test]
fn winter_trends_first_point_is_2230_beijing() {
    // fixture 首点必须是北京 22:30（冬令时美股开盘对应 22:30，不是夏令时的 21:30）
    let text = read_text("em_trends_winter.json");
    let root: serde_json::Value = serde_json::from_str(&text).unwrap();
    let first = root["data"]["trends"][0].as_str().unwrap();
    assert!(
        first.starts_with("2026-12-15 22:30"),
        "冬令时 fixture 首点应为 2026-12-15 22:30，实际 {first}"
    );
}

#[test]
fn winter_trends_converts_2230_to_930() {
    let raw = raw_points(&read_text("em_trends_winter.json")).expect("应解析成功");
    assert_eq!(
        raw.first().map(|p| p.hhmm),
        Some(930),
        "冬令时北京 22:30 应换算为美东 930（时差 13h）"
    );
}

#[test]
fn winter_trends_points_also_land_inside_the_grid() {
    // 冬令时同样要跑落格断言。若时差被硬编码为 12h，这里首点会是 1030，
    // 整条曲线横向偏移 60 格（虽然仍在栅格内，但位置错），
    // 故配合上一条 hhmm == 930 的断言一起把偏移钉死。
    let raw = raw_points(&read_text("em_trends_winter.json")).expect("应解析成功");
    let landed = attach_grid(Market::Us, &raw);
    assert!(
        landed.len() >= 50,
        "冬令时 attach_grid 后应剩 ≥50 点（原始 {} 点），实际 {}",
        raw.len(),
        landed.len()
    );
    assert_eq!(landed.first().map(|p| p.grid_index), Some(0));
}

// ---------- 4. 结构错误不退化为造点 ----------

#[test]
fn null_data_returns_structure_error_and_never_fabricates_points() {
    let err = raw_points(&read_text("em_trends_null.json"))
        .expect_err("data 为 null 应返回 Structure");
    assert!(matches!(err, ParseError::Structure(_)), "实际 {err:?}");
    // 关键：走 Err 而不是返回一个"补出来"的序列
    assert!(parse_trends(&read_text("em_trends_null.json"), "usAAPL").is_err());
}

#[test]
fn non_json_returns_not_json() {
    let err = raw_points(&read_text("minute_html_error.html")).expect_err("HTML 应返回 NotJson");
    assert_eq!(err, ParseError::NotJson);
}
