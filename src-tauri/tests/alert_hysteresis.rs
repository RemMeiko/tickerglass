//! 牛熊提醒的滞回状态机测试。纯离线。
//!
//! 核心不变量（对齐参考项目 StockPet 的文档行为）：
//! **每次越界只提醒一次；价格回到阈值内侧后才重新布防。**
//! 参考项目原文示例：上涨阈值 `+3.0%` → 首次达到时提醒 → 回落到 `+2.85%` 以下后重新布防
//! → 再次达到 `+3.0%` 才再提醒。

use tickerglass_lib::config::AlertConfig;
use tickerglass_lib::domain::alert::{
    evaluate_one, evaluate_symbol, AlertInput, AlertRule, ArmState, RuleKey, Side, Trigger,
    REARM_RATIO,
};
use std::collections::HashMap;

const NO_COOLDOWN: u64 = 0;

fn armed() -> ArmState {
    ArmState::default()
}

// ───────── 参考项目文档示例的逐步复现 ─────────

#[test]
fn reference_example_3_percent_fires_once_then_rearms_at_2_85() {
    // 这是参考项目 README 明文给的例子，逐步走一遍
    let thr = 3.0;
    let mut st = armed();

    // ① 首次达到 +3.0% → 提醒
    let (fired, next) = evaluate_one(3.0, thr, Side::Bull, st, 1000, NO_COOLDOWN);
    assert!(fired, "首次达到阈值必须提醒");
    st = next;
    assert!(!st.armed, "提醒后转为未布防");

    // ② 仍在 +3.2%（继续在阈值外）→ 不重复提醒
    let (fired, next) = evaluate_one(3.2, thr, Side::Bull, st, 2000, NO_COOLDOWN);
    assert!(!fired, "持续越界不得重复提醒");
    st = next;

    // ③ 回落到 +2.9%（还没跌破布防线 2.85）→ 仍不重新布防
    let (fired, next) = evaluate_one(2.9, thr, Side::Bull, st, 3000, NO_COOLDOWN);
    assert!(!fired);
    st = next;
    assert!(!st.armed, "2.9% 还在布防线外侧，不该重新布防");

    // ④ 回落到 +2.84%（跌破布防线 2.85）→ 重新布防，但不提醒
    let (fired, next) = evaluate_one(2.84, thr, Side::Bull, st, 4000, NO_COOLDOWN);
    assert!(!fired, "重新布防本身不是提醒事件");
    st = next;
    assert!(st.armed, "跌破 2.85 后必须重新布防");

    // ⑤ 再次达到 +3.0% → 再提醒
    let (fired, _) = evaluate_one(3.0, thr, Side::Bull, st, 5000, NO_COOLDOWN);
    assert!(fired, "重新布防后再次越界必须再提醒");
}

#[test]
fn rearm_line_is_five_percent_inside_threshold() {
    // 布防线 = 阈值 × (1 − 5%)。阈值 3.0 → 2.85，正是参考项目的数字
    assert_eq!(REARM_RATIO, 0.05);
    let rearm = 3.0 * (1.0 - REARM_RATIO);
    assert!((rearm - 2.85).abs() < 1e-9, "实际 {rearm}");
}

// ───────── 小熊方向（阈值为负） ─────────

#[test]
fn bear_side_fires_when_falling_below_negative_threshold() {
    let thr = -3.0; // 下跌 3%
    let (fired, st) = evaluate_one(-3.0, thr, Side::Bear, armed(), 1000, NO_COOLDOWN);
    assert!(fired, "跌到 −3.0% 必须提醒");
    assert!(!st.armed);
}

#[test]
fn bear_side_does_not_fire_above_threshold() {
    let (fired, st) = evaluate_one(-2.5, -3.0, Side::Bear, armed(), 1000, NO_COOLDOWN);
    assert!(!fired, "只跌 2.5% 未到 −3% 阈值，不该提醒");
    assert!(st.armed, "未越界应保持布防");
}

#[test]
fn bear_rearm_moves_toward_zero_not_further_negative() {
    // 阈值 −3.0 的布防线是 −2.85（朝零方向收缩），不是 −3.15
    let thr = -3.0;
    let (_, st) = evaluate_one(-3.5, thr, Side::Bear, armed(), 1000, NO_COOLDOWN);
    assert!(!st.armed);

    // −2.9 还在布防线外侧（比 −2.85 更负）→ 不重新布防
    let (_, st) = evaluate_one(-2.9, thr, Side::Bear, st, 2000, NO_COOLDOWN);
    assert!(!st.armed, "−2.9 尚未回到 −2.85 内侧");

    // −2.8 已回到内侧 → 重新布防
    let (_, st) = evaluate_one(-2.8, thr, Side::Bear, st, 3000, NO_COOLDOWN);
    assert!(st.armed, "−2.8 已越过 −2.85 回到内侧，应重新布防");
}

// ───────── 冷却期 ─────────

#[test]
fn cooldown_suppresses_second_fire_within_window() {
    let cooldown = 300_000; // 5 分钟
    let (fired, st) = evaluate_one(3.0, 3.0, Side::Bull, armed(), 1_000_000, cooldown);
    assert!(fired);

    // 重新布防（价格回内侧）
    let (_, st) = evaluate_one(2.0, 3.0, Side::Bull, st, 1_010_000, cooldown);
    assert!(st.armed);

    // 冷却期内再次越界 → 不响
    let (fired, st) = evaluate_one(3.1, 3.0, Side::Bull, st, 1_020_000, cooldown);
    assert!(!fired, "距上次触发仅 20s，在 5 分钟冷却期内");
    assert!(!st.armed, "冷却期内被抑制也要解除布防，否则冷却期一过会立刻补发");
}

#[test]
fn cooldown_expires_and_allows_fire_again() {
    let cooldown = 300_000;
    let (_, st) = evaluate_one(3.0, 3.0, Side::Bull, armed(), 1_000_000, cooldown);
    let (_, st) = evaluate_one(2.0, 3.0, Side::Bull, st, 1_010_000, cooldown);
    // 冷却期已过（+301s）
    let (fired, _) = evaluate_one(3.0, 3.0, Side::Bull, st, 1_301_001, cooldown);
    assert!(fired, "冷却期结束后应恢复提醒");
}

#[test]
fn cooldown_suppression_does_not_leave_pending_burst() {
    // 关键设计点：冷却期内被抑制时也要解除布防。
    // 若不解除，冷却期一过就会立刻补发一次（用户体验是"延迟 5 分钟的莫名提醒"）
    let cooldown = 100_000;
    let (_, st) = evaluate_one(3.0, 3.0, Side::Bull, armed(), 0, cooldown);
    let (_, st) = evaluate_one(2.0, 3.0, Side::Bull, st, 10_000, cooldown); // 重新布防
    let (fired, st) = evaluate_one(3.5, 3.0, Side::Bull, st, 20_000, cooldown);
    assert!(!fired);
    assert!(!st.armed, "必须解除布防");
    // 冷却期过后，价格仍在阈值外但已未布防 → 不补发
    let (fired, _) = evaluate_one(3.5, 3.0, Side::Bull, st, 200_000, cooldown);
    assert!(!fired, "冷却期结束不该补发被抑制的那一次");
}

// ───────── 初始状态 ─────────

#[test]
fn initial_state_is_armed_so_startup_crossing_fires() {
    // 启动时价格若已在阈值外，应当立刻提醒一次（而不是等价格先回落）
    assert!(ArmState::default().armed);
    let (fired, _) = evaluate_one(5.0, 3.0, Side::Bull, ArmState::default(), 1000, NO_COOLDOWN);
    assert!(fired);
}

// ───────── 整只标的的判定 ─────────

struct Bag(HashMap<RuleKey, ArmState>);

impl Bag {
    fn new() -> Self {
        Bag(HashMap::new())
    }
}

fn run(
    input: AlertInput<'_>,
    rule: &AlertRule,
    bag: &mut Bag,
    now: u64,
    cooldown: u64,
    in_session: bool,
) -> Vec<tickerglass_lib::domain::alert::AlertEvent> {
    let snapshot = bag.0.clone();
    let mut get = |k: RuleKey| snapshot.get(&k).copied().unwrap_or_default();
    let mut pending: Vec<(RuleKey, ArmState)> = Vec::new();
    let mut set = |k: RuleKey, v: ArmState| pending.push((k, v));
    let out = evaluate_symbol(input, rule, &mut get, &mut set, now, cooldown, in_session);
    for (k, v) in pending {
        bag.0.insert(k, v);
    }
    out
}

fn input(price: f64, percent: f64) -> AlertInput<'static> {
    AlertInput {
        code: "sh600519",
        price: Some(price),
        percent: Some(percent),
        fresh: true,
    }
}

#[test]
fn percent_and_target_price_are_independent() {
    let rule = AlertRule {
        up_percent: Some(3.0),
        bull_price: Some(1300.0),
        ..Default::default()
    };
    let mut bag = Bag::new();

    // 只越过百分比阈值（价格未到 1300）→ 只有百分比那条触发
    let ev = run(input(1290.0, 3.5), &rule, &mut bag, 1000, NO_COOLDOWN, true);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].trigger, Trigger::Percent);

    // 再越过目标价 → 目标价那条独立触发（百分比那条已未布防）
    let ev = run(input(1305.0, 3.6), &rule, &mut bag, 2000, NO_COOLDOWN, true);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].trigger, Trigger::TargetPrice);
    assert_eq!(ev[0].threshold, 1300.0);
}

#[test]
fn stale_data_does_not_participate() {
    // 断网期间旧值持续在阈值外，不该反复提醒
    let rule = AlertRule {
        up_percent: Some(3.0),
        ..Default::default()
    };
    let mut bag = Bag::new();
    let stale = AlertInput {
        code: "sh600519",
        price: Some(1300.0),
        percent: Some(9.9),
        fresh: false,
    };
    let ev = run(stale, &rule, &mut bag, 1000, NO_COOLDOWN, true);
    assert!(ev.is_empty(), "过期数据不参与判定");
    assert!(bag.0.is_empty(), "过期数据也不该改动布防状态");
}

#[test]
fn missing_price_skips_target_rule_but_percent_still_works() {
    let rule = AlertRule {
        up_percent: Some(3.0),
        bull_price: Some(1300.0),
        ..Default::default()
    };
    let mut bag = Bag::new();
    let no_price = AlertInput {
        code: "sh600519",
        price: None,
        percent: Some(5.0),
        fresh: true,
    };
    let ev = run(no_price, &rule, &mut bag, 1000, NO_COOLDOWN, true);
    // 百分比规则会改状态但因缺价格而不产出事件（卡片要显示价格）
    assert!(ev.is_empty());
    // 目标价规则完全不参与
    assert!(!bag.0.contains_key(&RuleKey {
        side: Side::Bull,
        trigger: Trigger::TargetPrice
    }));
}

#[test]
fn out_of_session_updates_state_but_emits_nothing() {
    // 非交易时段不提醒，但仍要更新布防状态 ——
    // 否则收盘后价格回落无法重新布防，次日开盘会漏掉第一次越界
    let rule = AlertRule {
        up_percent: Some(3.0),
        ..Default::default()
    };
    let mut bag = Bag::new();
    let ev = run(input(1300.0, 5.0), &rule, &mut bag, 1000, NO_COOLDOWN, false);
    assert!(ev.is_empty(), "非交易时段不得产生提醒");
    let st = bag.0[&RuleKey {
        side: Side::Bull,
        trigger: Trigger::Percent,
    }];
    assert!(!st.armed, "状态仍须推进，否则次日会漏第一次越界");
}

#[test]
fn empty_rule_does_nothing() {
    let mut bag = Bag::new();
    let ev = run(
        input(1300.0, 9.0),
        &AlertRule::default(),
        &mut bag,
        1000,
        NO_COOLDOWN,
        true,
    );
    assert!(ev.is_empty());
    assert!(bag.0.is_empty());
}

#[test]
fn down_percent_is_stored_positive_but_evaluated_negative() {
    // 配置里用户填 3.0 表示"跌 3%"，判定时必须当 −3.0 用
    let rule = AlertRule {
        down_percent: Some(3.0),
        ..Default::default()
    };
    let mut bag = Bag::new();
    let ev = run(input(1250.0, -3.1), &rule, &mut bag, 1000, NO_COOLDOWN, true);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].side, Side::Bear);
    assert_eq!(ev[0].threshold, -3.0, "阈值应以负数形式出现在事件里");
}

// ───────── 配置层 ─────────

#[test]
fn alerts_default_to_disabled() {
    // 用户没设过任何阈值时不该突然开始弹东西
    assert!(!AlertConfig::default().enabled);
}

#[test]
fn effective_rule_falls_back_to_global_percent() {
    let mut cfg = AlertConfig::default();
    cfg.default_up_percent = Some(4.0);
    cfg.default_down_percent = Some(6.0);
    // 该股只设了目标价，没设百分比 → 百分比回落到全局默认
    cfg.rules.insert(
        "sh600519".into(),
        AlertRule {
            bull_price: Some(1300.0),
            ..Default::default()
        },
    );
    let r = cfg.effective_rule("sh600519");
    assert_eq!(r.bull_price, Some(1300.0));
    assert_eq!(r.up_percent, Some(4.0), "未设的百分比应回落全局默认");
    assert_eq!(r.down_percent, Some(6.0));
}

#[test]
fn effective_rule_prefers_per_stock_over_global() {
    let mut cfg = AlertConfig::default();
    cfg.default_up_percent = Some(3.0);
    cfg.rules.insert(
        "sh600519".into(),
        AlertRule {
            up_percent: Some(8.0),
            ..Default::default()
        },
    );
    assert_eq!(cfg.effective_rule("sh600519").up_percent, Some(8.0));
}

#[test]
fn sanitize_clamps_cooldown_and_opacity() {
    let mut cfg = AlertConfig {
        cooldown_secs: 0,
        card_opacity: 5.0,
        ..Default::default()
    };
    cfg.sanitize();
    assert!(cfg.cooldown_secs >= 5, "冷却期过短会在阈值边缘每 3s 响一次");
    assert!(cfg.card_opacity <= 1.0);
}

#[test]
fn sanitize_rejects_nonsense_thresholds() {
    let mut cfg = AlertConfig {
        default_up_percent: Some(-5.0),
        default_down_percent: Some(f64::INFINITY),
        ..Default::default()
    };
    cfg.rules.insert(
        "sh600519".into(),
        AlertRule {
            bull_price: Some(-100.0),
            bear_price: Some(f64::NAN),
            up_percent: Some(999.0),
            ..Default::default()
        },
    );
    cfg.sanitize();
    assert_eq!(cfg.default_up_percent, None, "负阈值无意义");
    assert_eq!(cfg.default_down_percent, None, "Infinity 必须剔除");
    // 该股四项全被剔除 → 整条规则被清掉，不在配置里留空壳
    assert!(!cfg.rules.contains_key("sh600519"));
}

#[test]
fn sanitize_keeps_valid_rule() {
    let mut cfg = AlertConfig::default();
    cfg.rules.insert(
        "sh600519".into(),
        AlertRule {
            bull_price: Some(1300.0),
            ..Default::default()
        },
    );
    cfg.sanitize();
    assert!(cfg.rules.contains_key("sh600519"));
}
