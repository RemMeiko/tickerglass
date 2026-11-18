//! v0.4.0 需求 1（分时 501 限流根治）的调度策略与缓存测试。
//!
//! 背景：旧分时链路"每 tick 全量拉所有标的"（9 只 × 5s ≈ 108 请求/分钟，
//! 24 小时不间断）被上游以 HTTP 501 拒绝。新策略三层：按需（新鲜度目标）、
//! 预算（每 tick 上限）、冷却（失败退避）。这里锁的是纯函数层的行为，
//! 不发任何网络请求。

use std::time::Duration;
use tickerglass_lib::domain::market::Market;
use tickerglass_lib::minute_cache::{
    load_all_at, load_series_at, purge_old_days_at, save_series_at,
};
use tickerglass_lib::model::{MinutePoint, MinuteSeries, WatchItem};
use tickerglass_lib::scheduler::{apply_minute_cache, minute_cooldown_secs, plan_minute_batch};
use tickerglass_lib::state::AppState;

fn item(code: &str, market: Market) -> WatchItem {
    WatchItem {
        tencent_code: code.to_string(),
        em_secid: None,
        market,
        is_index: market.is_index(),
        hidden: false,
    }
}

fn series(code: &str, n_points: usize) -> MinuteSeries {
    MinuteSeries {
        code: code.to_string(),
        market: Market::Ashare,
        grid_len: 241,
        prev_close: Some(10.0),
        points: (0..n_points)
            .map(|i| MinutePoint {
                hhmm: 930 + (i as u16 / 60) * 100 + (i as u16 % 60),
                grid_index: i,
                price: 10.0 + i as f64 * 0.01,
                avg: Some(10.0 + i as f64 * 0.005),
                break_before: false,
            })
            .collect(),
    }
}

// ---------- 冷却表 ----------

#[test]
fn cooldown_table_scales_and_caps() {
    // 期望值由表达式直接算出（经验手册：期望值不心算）
    assert_eq!(minute_cooldown_secs(0), 0, "无失败不冷却");
    assert_eq!(minute_cooldown_secs(1), 60);
    assert_eq!(minute_cooldown_secs(2), 120);
    assert_eq!(minute_cooldown_secs(3), 240);
    assert_eq!(minute_cooldown_secs(4), 480);
    assert_eq!(minute_cooldown_secs(5), 600, "960 封顶到 600");
    assert_eq!(minute_cooldown_secs(50), 600, "持续失败也不超过 10 分钟");
}

// ---------- 批次选择 ----------

#[test]
fn never_fetched_items_are_due_and_batch_is_capped() {
    let items: Vec<WatchItem> = (0..5)
        .map(|i| item(&format!("sh60051{i}"), Market::Ashare))
        .collect();
    let ages = vec![None; 5];
    let in_session = vec![true; 5];
    let picked = plan_minute_batch(&items, &ages, &in_session, false);
    assert_eq!(picked, vec![0, 1, 2], "全部到期时按序取前 3（预算上限）");
}

#[test]
fn fresh_items_within_target_are_skipped() {
    let items = vec![item("sh600519", Market::Ashare)];
    // 盘中目标 60s：10s 前（不足）不取，60s 前（恰好到期）取
    assert_eq!(
        plan_minute_batch(&items, &[Some(Duration::from_secs(10))], &[true], false),
        Vec::<usize>::new()
    );
    assert_eq!(
        plan_minute_batch(&items, &[Some(Duration::from_secs(60))], &[true], false),
        vec![0]
    );
}

#[test]
fn us_target_is_30s_in_session() {
    let items = vec![item("usAAPL", Market::Us)];
    // 40s：美股（30s 目标）到期，但若按 A 股目标（60s）则不到 —— 用这条锁区分
    assert_eq!(
        plan_minute_batch(&items, &[Some(Duration::from_secs(40))], &[true], false),
        vec![0],
        "美股盘中 30s 目标，40s 已到期"
    );
}

#[test]
fn idle_market_target_is_30_minutes() {
    let items = vec![item("sh600519", Market::Ashare)];
    // 休市：100s 不到期，1800s 到期
    assert_eq!(
        plan_minute_batch(&items, &[Some(Duration::from_secs(100))], &[false], false),
        Vec::<usize>::new(),
        "休市时数据不变，100s 前取过就不该再打请求"
    );
    assert_eq!(
        plan_minute_batch(&items, &[Some(Duration::from_secs(1800))], &[false], false),
        vec![0]
    );
}

#[test]
fn tencent_cooldown_skips_non_us_but_keeps_us() {
    let items = vec![
        item("sh600519", Market::Ashare),
        item("hk00700", Market::Hk),
        item("usAAPL", Market::Us),
    ];
    let ages = vec![None; 3];
    let in_session = vec![true; 3];
    // 冷却中：A股/港股（web.ifzq.gtimg.cn）跳过，美股（push2delay）照取
    let picked = plan_minute_batch(&items, &ages, &in_session, true);
    assert_eq!(picked, vec![2], "冷却只封腾讯域，美股走 delay 不连坐");
}

#[test]
fn cooldown_does_not_count_us_as_tencent() {
    let items = vec![item("usAAPL", Market::Us)];
    let picked = plan_minute_batch(&items, &[None], &[true], true);
    assert_eq!(picked, vec![0]);
}

// ---------- 当日缓存读写 ----------

fn tmp_root(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tickerglass-mincache-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn cache_roundtrip_preserves_series() {
    let root = tmp_root("roundtrip");
    let s = series("sh600519", 100);
    save_series_at(&root, "2026-11-18", &s);

    let loaded = load_series_at(&root, "2026-11-18", "sh600519").expect("刚写的缓存必须能读回");
    assert_eq!(loaded.code, "sh600519");
    assert_eq!(loaded.points.len(), 100);
    assert!((loaded.points[42].price - s.points[42].price).abs() < 1e-9);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn empty_series_is_not_cached() {
    let root = tmp_root("empty");
    save_series_at(&root, "2026-11-18", &series("sh600519", 0));
    assert!(
        load_series_at(&root, "2026-11-18", "sh600519").is_none(),
        "空序列（盘前）没有缓存价值，不落盘"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cache_is_date_scoped() {
    let root = tmp_root("datescope");
    save_series_at(&root, "2026-11-17", &series("sh600519", 50));
    assert!(
        load_series_at(&root, "2026-11-18", "sh600519").is_none(),
        "昨天的缓存今天不可见（按日期分目录的意义）"
    );
    assert_eq!(
        load_all_at(&root, "2026-11-17").len(),
        1,
        "load_all 按指定日期读"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn purge_keeps_only_today() {
    let root = tmp_root("purge");
    save_series_at(&root, "2026-11-16", &series("sh600519", 10));
    save_series_at(&root, "2026-11-17", &series("sh600519", 10));
    save_series_at(&root, "2026-11-18", &series("sh600519", 10));

    purge_old_days_at(&root, "2026-11-18");
    assert!(root.join("2026-11-18").exists(), "今天的目录保留");
    assert!(!root.join("2026-11-17").exists(), "昨天的目录清掉");
    assert!(!root.join("2026-11-16").exists(), "前天的目录清掉");

    let _ = std::fs::remove_dir_all(&root);
}

// ---------- 启动预载 ----------

#[test]
fn preload_fills_only_rows_without_curve() {
    let state = std::sync::Arc::new(AppState::new());
    let snap = tickerglass_lib::scheduler::initial_snapshot(&[
        item("sh600519", Market::Ashare),
        item("sz002466", Market::Ashare),
    ]);
    state.set(snap);

    // 600519 有缓存 → 填入；002466 不在缓存里 → 保持 Pending
    apply_minute_cache(&state, &[series("SH600519", 80)]);

    let snap = state.get();
    assert_eq!(
        snap.rows[0].minute.as_ref().expect("缓存命中行必须有曲线").points.len(),
        80,
        "代码匹配必须大小写不敏感（缓存键规范化）"
    );
    assert!(snap.rows[0].minute_available);
    assert!(snap.rows[1].minute.is_none(), "无缓存的行保持原状");
}

#[test]
fn preload_never_overwrites_existing_curve() {
    let state = std::sync::Arc::new(AppState::new());
    let mut snap = tickerglass_lib::scheduler::initial_snapshot(&[item("sh600519", Market::Ashare)]);
    // 行上已有一条 5 点的曲线（本轮网络刚取的）
    snap.rows[0].minute = Some(series("sh600519", 5));
    snap.rows[0].minute_available = true;
    state.set(snap);

    // 缓存里是 80 点的旧数据 —— 不得覆盖新数据
    apply_minute_cache(&state, &[series("sh600519", 80)]);
    let snap = state.get();
    assert_eq!(
        snap.rows[0].minute.as_ref().unwrap().points.len(),
        5,
        "已有曲线（哪怕更短）不被缓存覆盖"
    );
}
