//! 窗口越界钳制的几何计算（纯函数，无 tauri 窗口依赖）。
//!
//! 这组测试是标准 14「记录钳制后的坐标，确认 ≥60px 可见 —— 量化而非目测」的
//! 机器验证入口，也是 Sprint 1 遗留「可拖出屏幕外 55px 无钳制」的回归防线。

use tickerglass_lib::window::{
    clamp_into_monitors, is_visible_enough, Rect, MIN_VISIBLE_H, MIN_VISIBLE_W,
};

/// 单显示器 1920x1080，原点在 (0,0)。
fn single() -> Vec<Rect> {
    vec![Rect::new(0.0, 0.0, 1920.0, 1080.0)]
}

/// 双显示器：主屏 (0,0) 1920x1080 + 副屏在右侧 (1920,0) 1280x1024。
fn dual() -> Vec<Rect> {
    vec![
        Rect::new(0.0, 0.0, 1920.0, 1080.0),
        Rect::new(1920.0, 0.0, 1280.0, 1024.0),
    ]
}

fn win_at(x: f64, y: f64) -> Rect {
    Rect::new(x, y, 300.0, 208.0)
}

/// 钳制后必须满足的最终判据：在某个显示器内至少露出 60x30。
fn assert_visible(r: Rect, monitors: &[Rect]) {
    assert!(
        is_visible_enough(&r, monitors),
        "钳制后仍不满足 {MIN_VISIBLE_W}x{MIN_VISIBLE_H} 可见: {r:?}"
    );
}

// ---------- 1. 常量与合法位置不动 ----------

#[test]
fn min_visible_constants_match_contract() {
    assert_eq!(MIN_VISIBLE_W, 60.0);
    assert_eq!(MIN_VISIBLE_H, 30.0);
}

#[test]
fn fully_visible_window_is_not_moved() {
    let w = win_at(100.0, 100.0);
    assert_eq!(clamp_into_monitors(w, &single()), w);
}

#[test]
fn window_touching_edge_but_visible_enough_is_not_moved() {
    // 右边只露出 100px（> 60）→ 合法，不该动它（拖动中不干扰手感）
    let w = win_at(1820.0, 100.0);
    assert_eq!(clamp_into_monitors(w, &single()), w);
}

#[test]
fn window_with_exactly_min_visible_width_is_not_moved() {
    // 边界：正好露出 60px
    let w = win_at(1860.0, 100.0);
    assert_eq!(clamp_into_monitors(w, &single()), w);
}

// ---------- 2. 四个方向拖出屏幕外都能找回（标准 14）----------

#[test]
fn dragged_off_right_edge_is_clamped_back() {
    let w = win_at(2400.0, 300.0);
    let c = clamp_into_monitors(w, &single());
    assert_ne!(c, w, "越界必须回弹");
    assert_visible(c, &single());
}

#[test]
fn dragged_off_right_edge_keeps_exactly_min_visible() {
    // 量化断言：钳到最右合法位 = 1920 - 60 = 1860
    let c = clamp_into_monitors(win_at(2400.0, 300.0), &single());
    assert_eq!(c.x, 1860.0);
}

#[test]
fn dragged_off_left_edge_is_clamped_back() {
    let w = win_at(-500.0, 300.0);
    let c = clamp_into_monitors(w, &single());
    assert_ne!(c, w);
    assert_visible(c, &single());
}

#[test]
fn dragged_off_left_edge_keeps_exactly_min_visible() {
    // 最左合法位 = 0 - (300 - 60) = -240
    let c = clamp_into_monitors(win_at(-500.0, 300.0), &single());
    assert_eq!(c.x, -240.0);
}

#[test]
fn dragged_off_bottom_edge_is_clamped_back() {
    let w = win_at(300.0, 1500.0);
    let c = clamp_into_monitors(w, &single());
    assert_ne!(c, w);
    assert_visible(c, &single());
}

#[test]
fn dragged_off_bottom_edge_keeps_exactly_min_visible() {
    // 最下合法位 = 1080 - 30 = 1050
    let c = clamp_into_monitors(win_at(300.0, 1500.0), &single());
    assert_eq!(c.y, 1050.0);
}

#[test]
fn dragged_off_top_edge_is_clamped_back() {
    let w = win_at(300.0, -400.0);
    let c = clamp_into_monitors(w, &single());
    assert_ne!(c, w);
    assert_visible(c, &single());
}

#[test]
fn dragged_off_top_edge_keeps_exactly_min_visible() {
    // 最上合法位 = 0 - (208 - 30) = -178
    let c = clamp_into_monitors(win_at(300.0, -400.0), &single());
    assert_eq!(c.y, -178.0);
}

#[test]
fn dragged_off_corner_clamps_both_axes() {
    let c = clamp_into_monitors(win_at(-800.0, -700.0), &single());
    assert_eq!(c.x, -240.0);
    assert_eq!(c.y, -178.0);
    assert_visible(c, &single());
}

// ---------- 3. Sprint 1 遗留的 55px 越界场景 ----------

#[test]
fn sprint1_regression_only_55px_visible_gets_clamped() {
    // 只剩 55px 可见 —— 小于 60px 门槛，必须被钳回
    let w = win_at(1865.0, 300.0);
    let (before_w, _) = (1920.0 - 1865.0, 0);
    assert!(before_w < MIN_VISIBLE_W, "前提：55px < 60px 门槛");
    let c = clamp_into_monitors(w, &single());
    assert_eq!(c.x, 1860.0, "应钳到正好 60px 可见");
    assert_visible(c, &single());
}

#[test]
fn sprint1_regression_vertical_25px_visible_gets_clamped() {
    let w = win_at(300.0, 1055.0);
    assert!(1080.0 - 1055.0 < MIN_VISIBLE_H);
    let c = clamp_into_monitors(w, &single());
    assert_eq!(c.y, 1050.0);
}

// ---------- 4. 多显示器：并集边界，副屏上的窗口不该被拽回主屏 ----------

#[test]
fn window_on_second_monitor_is_left_alone() {
    // 副屏内的合法位置不该因"不在主屏里"而被钳回主屏
    let w = win_at(2100.0, 200.0);
    assert_eq!(clamp_into_monitors(w, &dual()), w);
}

#[test]
fn window_off_right_of_second_monitor_clamps_to_second_monitor() {
    // 副屏右边界 = 1920 + 1280 = 3200 → 最右合法位 3140
    let c = clamp_into_monitors(win_at(3600.0, 200.0), &dual());
    assert_eq!(c.x, 3140.0);
    assert_visible(c, &dual());
}

#[test]
fn window_in_gap_between_monitors_goes_to_nearest() {
    // 副屏比主屏矮（1024 vs 1080）：y=1050 处只有主屏覆盖，
    // x=1900 时主屏仅露 20px、副屏 y 方向不覆盖 → 必须被钳到有效位置
    let c = clamp_into_monitors(win_at(1900.0, 1050.0), &dual());
    assert_visible(c, &dual());
}

#[test]
fn dual_monitor_left_of_primary_is_clamped() {
    let c = clamp_into_monitors(win_at(-900.0, 100.0), &dual());
    assert_eq!(c.x, -240.0);
    assert_visible(c, &dual());
}

// ---------- 5. 退化输入不崩 ----------

#[test]
fn empty_monitor_list_leaves_window_untouched() {
    // 拿不到显示器信息时不乱动窗口
    let w = win_at(-5000.0, -5000.0);
    assert_eq!(clamp_into_monitors(w, &[]), w);
}

#[test]
fn window_larger_than_monitor_does_not_panic() {
    let big = Rect::new(-100.0, -100.0, 4000.0, 3000.0);
    let c = clamp_into_monitors(big, &single());
    assert_eq!(c.w, 4000.0, "尺寸不该被钳制，只钳位置");
}

#[test]
fn tiny_monitor_smaller_than_min_visible_does_not_panic() {
    // 显示器比最小可见量还小：clamp 的 lo > hi 会 panic，实现须取有序区间
    let tiny = vec![Rect::new(0.0, 0.0, 40.0, 20.0)];
    let c = clamp_into_monitors(win_at(500.0, 500.0), &tiny);
    assert!(c.x.is_finite() && c.y.is_finite());
}

#[test]
fn zero_size_window_does_not_panic() {
    let z = Rect::new(5000.0, 5000.0, 0.0, 0.0);
    let c = clamp_into_monitors(z, &single());
    assert!(c.x.is_finite() && c.y.is_finite());
}

// ---------- 6. is_visible_enough 自身 ----------

#[test]
fn visibility_check_rejects_fully_offscreen_window() {
    assert!(!is_visible_enough(&win_at(5000.0, 5000.0), &single()));
}

#[test]
fn visibility_check_accepts_centered_window() {
    assert!(is_visible_enough(&win_at(800.0, 400.0), &single()));
}

#[test]
fn visibility_check_needs_both_axes_not_just_one() {
    // x 方向露很多但 y 方向完全在屏幕外 → 不算可见
    // （只按面积>0 判定的实现会漏掉这种情况）
    let w = Rect::new(800.0, 1080.0, 300.0, 208.0);
    assert!(!is_visible_enough(&w, &single()));
}
