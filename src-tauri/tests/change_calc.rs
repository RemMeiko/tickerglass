//! 涨跌计算：报文优先、昨收兜底、除零保护、停牌、负值方向。表驱动，无 fixture。

use tickerglass_lib::domain::change::resolve_change;

#[test]
fn raw_fields_take_priority_over_calculation() {
    // 报文自带涨跌额/涨跌幅时直接用（哪怕与昨收算出来的略有差异 —— 交易所口径优先）
    let c = resolve_change(Some(1272.83), Some(1291.50), Some(-18.67), Some(-1.45));
    assert_eq!(c.amount, Some(-18.67));
    assert_eq!(c.percent, Some(-1.45));
}

#[test]
fn falls_back_to_prev_close_when_raw_missing() {
    let c = resolve_change(Some(110.0), Some(100.0), None, None);
    assert_eq!(c.amount, Some(10.0));
    assert_eq!(c.percent, Some(10.0));
}

#[test]
fn partial_fallback_computes_only_the_missing_one() {
    // 只缺涨跌幅时，涨跌额仍用报文的
    let c = resolve_change(Some(110.0), Some(100.0), Some(9.99), None);
    assert_eq!(c.amount, Some(9.99));
    assert_eq!(c.percent, Some(10.0));
}

#[test]
fn zero_prev_close_does_not_divide_by_zero() {
    let c = resolve_change(Some(10.0), Some(0.0), None, None);
    assert_eq!(c.percent, None, "昨收为 0 时不做除法，percent 必须为 None");
    // 涨跌额仍可给（10 - 0），但百分比无意义
    assert_eq!(c.amount, Some(10.0));
}

#[test]
fn negative_prev_close_is_also_guarded() {
    let c = resolve_change(Some(10.0), Some(-5.0), None, None);
    assert_eq!(c.percent, None, "昨收非正时不产出百分比");
}

#[test]
fn suspended_stock_with_zero_price_produces_nothing() {
    // 停牌无成交：现价 0 时不产出任何涨跌（不能显示 -100%）
    let c = resolve_change(Some(0.0), Some(100.0), Some(-100.0), Some(-100.0));
    assert_eq!(c.amount, None);
    assert_eq!(c.percent, None);
    // 现价完全缺失同理
    let c2 = resolve_change(None, Some(100.0), None, None);
    assert_eq!(c2.amount, None);
    assert_eq!(c2.percent, None);
}

#[test]
fn negative_direction_is_correct() {
    let c = resolve_change(Some(90.0), Some(100.0), None, None);
    assert_eq!(c.amount, Some(-10.0));
    assert_eq!(c.percent, Some(-10.0));
    assert!(c.percent.is_some_and(|p| p < 0.0), "下跌方向必须为负");
}

#[test]
fn missing_prev_close_yields_none_not_zero() {
    // 昨收拿不到时不能用 0 冒充（那会算出 +inf% 或把涨跌显示成等于现价）
    let c = resolve_change(Some(50.0), None, None, None);
    assert_eq!(c.amount, None);
    assert_eq!(c.percent, None);
}
