//! 交易分钟栅格：三市场长度与首末值、午休相邻、栅格外输入、空输入。
//!
//! 栅格长度是 X 轴映射的分母（`X = grid_index / (len - 1)`），差 1 会让整条曲线
//! 横向系统性偏移、末点被挤出画布。故三个数字由断言钉死，不靠人算。

use tickerglass_lib::domain::market::Market;
use tickerglass_lib::domain::session::{attach_grid, grid, grid_len, index_of};
use tickerglass_lib::model::RawPoint;

#[test]
fn ashare_grid_is_242_with_correct_ends() {
    let g = grid(Market::Ashare);
    // 闭区间 09:30..11:30 = 121 + 13:00..15:00 = 121 → 242
    assert_eq!(g.len(), 242);
    assert_eq!(g[0], 930);
    assert_eq!(g[241], 1500);
    // 指数与 A 股共用同一栅格
    assert_eq!(grid_len(Market::IndexCn), 242);
}

#[test]
fn hk_grid_is_332_with_correct_ends() {
    let g = grid(Market::Hk);
    // 09:30..12:00 = 151 + 13:00..16:00 = 181 → 332（交易时段闭区间刻度数）
    assert_eq!(g.len(), 332);
    assert_eq!(g[0], 930);
    assert_eq!(g[331], 1600);
}

#[test]
fn us_grid_is_391_with_correct_ends() {
    let g = grid(Market::Us);
    // 美东 09:30..16:00 连续无午休 → 391
    assert_eq!(g.len(), 391);
    assert_eq!(g[0], 930);
    assert_eq!(g[390], 1600);
}

#[test]
fn ashare_lunch_ticks_are_adjacent() {
    // 午休不占格：11:30 与 13:00 在栅格上相邻
    let a = index_of(Market::Ashare, 1130).expect("1130 应在 A 股栅格内");
    let b = index_of(Market::Ashare, 1300).expect("1300 应在 A 股栅格内");
    assert_eq!(a + 1, b);
    assert_eq!(a, 120);
}

#[test]
fn hk_lunch_ticks_are_adjacent_at_1200() {
    // 港股午休在 12:00→13:00（不是 A 股的 11:30）
    let a = index_of(Market::Hk, 1200).expect("1200 应在港股栅格内");
    let b = index_of(Market::Hk, 1300).expect("1300 应在港股栅格内");
    assert_eq!(a + 1, b);
    assert_eq!(a, 150);
    // 港股 11:30 仍在连续交易时段内
    assert!(index_of(Market::Hk, 1130).is_some(), "港股 11:30 应在栅格内");
}

#[test]
fn pre_open_tick_is_outside_grid() {
    // 09:25 集合竞价不在分时栅格内
    assert_eq!(index_of(Market::Ashare, 925), None);
    assert_eq!(index_of(Market::Hk, 925), None);
    assert_eq!(index_of(Market::Us, 925), None);
}

#[test]
fn ashare_after_close_tick_is_outside_grid() {
    // 实测 A 股分时延伸到 1530，15:06 之后的点必须被识别为栅格外
    assert_eq!(index_of(Market::Ashare, 1506), None);
    assert_eq!(index_of(Market::Ashare, 1530), None);
    // 午休中间的时刻同样在栅格外
    assert_eq!(index_of(Market::Ashare, 1200), None);
}

#[test]
fn hk_after_close_tick_is_outside_grid() {
    // 实测港股分时延伸到 1608
    assert_eq!(index_of(Market::Hk, 1608), None);
    assert_eq!(index_of(Market::Hk, 1601), None);
}

#[test]
fn attach_grid_on_empty_input_returns_empty_without_panic() {
    let out = attach_grid(Market::Ashare, &[]);
    assert!(out.is_empty());
}

#[test]
fn attach_grid_drops_out_of_grid_points_and_marks_breaks() {
    let raw = vec![
        RawPoint { hhmm: 925, price: 10.0, avg: None },  // 盘前 → 丢弃
        RawPoint { hhmm: 930, price: 10.0, avg: None },
        RawPoint { hhmm: 931, price: 10.5, avg: None },
        RawPoint { hhmm: 1130, price: 11.0, avg: None },
        RawPoint { hhmm: 1300, price: 11.5, avg: None }, // 午休后首点 → break_before
        RawPoint { hhmm: 1506, price: 11.5, avg: None }, // 盘后 → 丢弃
    ];
    let out = attach_grid(Market::Ashare, &raw);
    assert_eq!(out.len(), 4, "6 个点中 2 个在栅格外应被丢弃");
    assert_eq!(out.iter().map(|p| p.hhmm).collect::<Vec<_>>(), vec![930, 931, 1130, 1300]);
    assert!(!out[0].break_before, "序列首点恒为 false");
    assert!(!out[1].break_before, "931 紧接 930，连续");
    // 931 → 1130 中间缺了 118 个刻度，是真实断口，应断线而不是拉一条长直线
    assert!(out[2].break_before, "本段内 grid_index 跳跃也应断线");
    assert!(out[3].break_before, "午休跨段应标 break_before");
}

#[test]
fn attach_grid_drops_invalid_prices() {
    let raw = vec![
        RawPoint { hhmm: 930, price: 0.0, avg: None },
        RawPoint { hhmm: 931, price: -1.0, avg: None },
        RawPoint { hhmm: 932, price: f64::NAN, avg: None },
        RawPoint { hhmm: 933, price: 10.0, avg: None },
    ];
    let out = attach_grid(Market::Ashare, &raw);
    assert_eq!(out.len(), 1, "0 价 / 负价 / NaN 都不应进曲线");
    assert_eq!(out[0].hhmm, 933);
    assert!(!out[0].break_before, "第一个有效点即序列首点，恒为 false");
}

#[test]
fn attach_grid_indexes_match_grid_positions() {
    let raw = vec![
        RawPoint { hhmm: 930, price: 10.0, avg: None },
        RawPoint { hhmm: 1500, price: 11.0, avg: None },
    ];
    let out = attach_grid(Market::Ashare, &raw);
    assert_eq!(out[0].grid_index, 0);
    assert_eq!(out[1].grid_index, 241, "末点应落在第 242 格（索引 241）");
    assert!(out[1].break_before, "跳跃的点应标断点");
}
