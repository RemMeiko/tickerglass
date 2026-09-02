//! 配置迁移与净化（纯函数，不碰文件系统 → 无需临时目录、无 I/O）。
//!
//! 覆盖：v1→v2 迁移、四种损坏形态、缺字段补默认、未来版本号保守处理、数值钳制。

use tickerglass_lib::config::schema::{AppearanceConfig, SCALE_MAX, SCALE_MIN};
use tickerglass_lib::config::{migrate, AppConfig, LoadOutcome, SCHEMA_VERSION};

// ---------- 1. 当前版本：原样加载 ----------

#[test]
fn current_version_config_loads_as_current() {
    let json = serde_json::to_string(&AppConfig::default()).unwrap();
    let r = migrate(&json);
    assert_eq!(r.outcome, LoadOutcome::Current);
}

#[test]
fn current_version_needs_no_backup_and_no_rewrite() {
    let json = serde_json::to_string(&AppConfig::default()).unwrap();
    let r = migrate(&json);
    assert!(!r.outcome.needs_backup());
    assert!(!r.outcome.needs_rewrite());
}

#[test]
fn schema_version_constant_is_five() {
    // v1 = Sprint 1 隐式结构
    // v2 = 新增 appearance/window
    // v3 = 新增 alerts（牛熊提醒：全局阈值 + 逐股目标价 + 冷却期/卡片不透明度）
    // v4 = 提示音可选音色与音量（bull_tone / bear_tone / sound_volume）
    // v5 = 自选股可隐藏（WatchItem.hidden）
    //
    // 这是一道刻意会被绊倒的守卫：改版本号必须同时补一条 migrate 路径，
    // 所以此处每次上调都要连带确认 vN-1→vN 的迁移测试已存在。
    // （3→4 与 4→5 两次上调它都绊倒了我，每次都是随后才补上迁移测试 ——
    //  这道守卫是有效的。）
    assert_eq!(SCHEMA_VERSION, 5);
}

// ---------- 2. v1 → v2 迁移：自选股必须一条不丢（标准 20）----------

/// 模拟真实旧版本文件：`schemaVersion: 1` 且**完全没有** `appearance` 字段。
fn v1_json_with_five_items() -> String {
    serde_json::json!({
        "schemaVersion": 1,
        "watchlist": [
            {"tencent_code":"sh000001","em_secid":"1.000001","market":"indexcn","is_index":true},
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false},
            {"tencent_code":"hk00700","em_secid":"116.00700","market":"hk","is_index":false},
            {"tencent_code":"usAAPL","em_secid":"105.AAPL","market":"us","is_index":false},
            {"tencent_code":"sz000001","em_secid":"0.000001","market":"ashare","is_index":false}
        ]
    })
    .to_string()
}

#[test]
fn v1_migrates_and_reports_source_version() {
    let r = migrate(&v1_json_with_five_items());
    assert_eq!(r.outcome, LoadOutcome::Migrated { from: 1 });
}

#[test]
fn v1_migration_preserves_all_five_watch_items() {
    let r = migrate(&v1_json_with_five_items());
    assert_eq!(r.config.watchlist.len(), 5, "自选股一条都不能丢");
}

#[test]
fn v1_migration_preserves_watchlist_order_exactly() {
    let r = migrate(&v1_json_with_five_items());
    let codes: Vec<&str> = r
        .config
        .watchlist
        .iter()
        .map(|i| i.tencent_code.as_str())
        .collect();
    assert_eq!(
        codes,
        vec!["sh000001", "sh600519", "hk00700", "usAAPL", "sz000001"]
    );
}

#[test]
fn v1_migration_fills_appearance_defaults() {
    let r = migrate(&v1_json_with_five_items());
    assert_eq!(r.config.appearance, AppearanceConfig::default());
}

#[test]
fn v1_migration_writes_back_current_schema_version() {
    let r = migrate(&v1_json_with_five_items());
    assert_eq!(r.config.schema_version, SCHEMA_VERSION);
}

#[test]
fn v1_migration_does_not_trigger_backup() {
    // 标准 20 判据 d：迁移不是损坏，**不该**生成 .bak
    let r = migrate(&v1_json_with_five_items());
    assert!(!r.outcome.needs_backup(), "迁移不得触发 .bak 备份");
}

#[test]
fn v1_migration_needs_rewrite_to_persist_upgrade() {
    let r = migrate(&v1_json_with_five_items());
    assert!(r.outcome.needs_rewrite(), "迁移结果须写回磁盘固化");
}

#[test]
fn config_without_schema_version_is_treated_as_v1() {
    // Sprint 1 时代的文件根本没有 schemaVersion 字段
    let json = serde_json::json!({
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ]
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.outcome, LoadOutcome::Migrated { from: 1 });
    assert_eq!(r.config.watchlist.len(), 1);
}

#[test]
fn v1_migration_keeps_user_edited_appearance_when_present() {
    // 用户手改过 appearance 的旧文件：不该被默认值覆盖
    let json = serde_json::json!({
        "schemaVersion": 1,
        "watchlist": [],
        "appearance": {"scale": 1.3}
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.config.appearance.scale, 1.3);
}

// ---------- 2b. v2 → v3 迁移：新增 alerts（牛熊提醒）----------

/// 模拟真实 v2 文件：有 appearance/window，**完全没有** alerts 字段。
fn v2_json() -> String {
    serde_json::json!({
        "schemaVersion": 2,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false},
            {"tencent_code":"hk00700","em_secid":"116.00700","market":"hk","is_index":false}
        ],
        "appearance": {"scale": 1.15},
        "window": {"locked": true}
    })
    .to_string()
}

#[test]
fn v2_migrates_and_reports_source_version() {
    let r = migrate(&v2_json());
    assert_eq!(r.outcome, LoadOutcome::Migrated { from: 2 });
}

#[test]
fn v2_migration_lands_on_alerts_disabled() {
    // 关键：从 v2 升上来的用户不该突然开始弹提醒卡片。
    // 注意默认阈值是**预填** ±3.0%（对齐参考项目的默认值），不是空 ——
    // 「不弹」这件事由 enabled:false 保证，而不是靠阈值为空。
    // 这样用户在设置页一开总开关就能直接用，不必先想清楚填几个百分点。
    let r = migrate(&v2_json());
    assert!(!r.config.alerts.enabled, "迁移得来的提醒必须默认关闭");
    assert_eq!(r.config.alerts.default_up_percent, Some(3.0));
    assert_eq!(r.config.alerts.default_down_percent, Some(3.0));
    assert!(r.config.alerts.rules.is_empty(), "不该凭空生成逐股规则");
}

#[test]
fn v2_migration_preserves_watchlist_and_user_settings() {
    // 迁移只加字段，既有配置一律原样保留
    let r = migrate(&v2_json());
    let codes: Vec<&str> = r
        .config
        .watchlist
        .iter()
        .map(|i| i.tencent_code.as_str())
        .collect();
    assert_eq!(codes, vec!["sh600519", "hk00700"], "自选股一条都不能丢");
    assert_eq!(r.config.appearance.scale, 1.15, "外观设置不得被重置");
    assert!(r.config.window.locked, "窗口设置不得被重置");
}

#[test]
fn v2_migration_writes_back_current_schema_version() {
    let r = migrate(&v2_json());
    assert_eq!(r.config.schema_version, SCHEMA_VERSION);
}

#[test]
fn v2_migration_does_not_trigger_backup_but_needs_rewrite() {
    let r = migrate(&v2_json());
    assert!(!r.outcome.needs_backup(), "迁移不是损坏，不得生成 .bak");
    assert!(r.outcome.needs_rewrite(), "迁移结果须写回磁盘固化");
}

#[test]
fn v2_migration_keeps_hand_written_alerts() {
    // 用户手写过 alerts 的 v2 文件：不该被空默认覆盖
    let json = serde_json::json!({
        "schemaVersion": 2,
        "watchlist": [],
        "alerts": {"enabled": true, "default_up_percent": 4.5}
    })
    .to_string();
    let r = migrate(&json);
    assert!(r.config.alerts.enabled);
    assert_eq!(r.config.alerts.default_up_percent, Some(4.5));
}

#[test]
fn v1_migrates_all_the_way_to_v3_in_one_pass() {
    // 迁移链必须逐级跑完：v1 的文件既要补 appearance/window，也要补 alerts。
    // 若 apply_chain 的链条断了，这里会看到 alerts 缺字段或版本号不对。
    let r = migrate(&v1_json_with_five_items());
    assert_eq!(r.outcome, LoadOutcome::Migrated { from: 1 });
    assert_eq!(r.config.schema_version, SCHEMA_VERSION);
    assert_eq!(r.config.watchlist.len(), 5, "跨两级迁移也不能丢自选股");
    assert!(!r.config.alerts.enabled, "v1 用户同样不该被动开启提醒");
}

// ---------- 3. 四种损坏形态走同一降级路径（标准 19）----------

#[test]
fn malformed_json_is_corrupted() {
    let r = migrate("{ this is not json");
    assert!(
        matches!(r.outcome, LoadOutcome::Corrupted { .. }),
        "实际: {:?}",
        r.outcome
    );
}

#[test]
fn malformed_json_falls_back_to_default_watchlist() {
    let r = migrate("{ this is not json");
    assert_eq!(r.config.watchlist.len(), 4, "应恢复默认 4 只");
}

#[test]
fn empty_file_is_corrupted() {
    let r = migrate("");
    assert!(matches!(r.outcome, LoadOutcome::Corrupted { .. }));
}

#[test]
fn whitespace_only_file_is_corrupted() {
    let r = migrate("   \n\t  ");
    assert!(matches!(r.outcome, LoadOutcome::Corrupted { .. }));
}

#[test]
fn array_root_is_corrupted() {
    // 合法 JSON 但根节点是数组
    let r = migrate("[]");
    assert!(
        matches!(r.outcome, LoadOutcome::Corrupted { .. }),
        "实际: {:?}",
        r.outcome
    );
}

#[test]
fn scalar_root_is_corrupted() {
    let r = migrate("42");
    assert!(matches!(r.outcome, LoadOutcome::Corrupted { .. }));
}

#[test]
fn corrupted_requires_backup_of_user_file() {
    let r = migrate("{ broken");
    assert!(r.outcome.needs_backup(), "损坏必须备份用户原文件");
}

#[test]
fn corrupted_message_mentions_bak_and_default_restore() {
    let r = migrate("{ broken");
    let msg = r.outcome.user_message().expect("损坏必须有用户可见提示");
    assert!(msg.contains("config.json.bak"), "实际文案: {msg}");
    assert!(msg.contains("默认"), "实际文案: {msg}");
}

#[test]
fn wrong_field_type_degrades_instead_of_panicking() {
    // watchlist 是字符串而不是数组：不能 panic
    let r = migrate(r#"{"schemaVersion":2,"watchlist":"oops"}"#);
    assert!(matches!(r.outcome, LoadOutcome::Corrupted { .. }));
    assert_eq!(r.config.watchlist.len(), 4);
}

// ---------- 4. 未来版本号：保守处理，不当损坏清空 ----------

#[test]
fn future_version_is_reported_not_treated_as_corrupted() {
    let json = serde_json::json!({
        "schemaVersion": 99,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ]
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.outcome, LoadOutcome::FutureVersion { found: 99 });
}

#[test]
fn future_version_preserves_recognizable_watchlist() {
    let json = serde_json::json!({
        "schemaVersion": 99,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ]
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.config.watchlist.len(), 1, "未来版本不该清空自选股");
    assert_eq!(r.config.watchlist[0].tencent_code, "sh600519");
}

#[test]
fn future_version_does_not_trigger_backup() {
    let json = r#"{"schemaVersion":99,"watchlist":[]}"#;
    let r = migrate(json);
    assert!(!r.outcome.needs_backup(), "未来版本不是损坏，不该备份");
}

#[test]
fn future_version_warns_the_user() {
    let json = r#"{"schemaVersion":99,"watchlist":[]}"#;
    let r = migrate(json);
    assert!(r.outcome.user_message().is_some());
}

// ---------- 5. 缺字段补默认 ----------

#[test]
fn missing_appearance_gets_defaults() {
    let r = migrate(r#"{"schemaVersion":2,"watchlist":[]}"#);
    assert_eq!(r.config.appearance, AppearanceConfig::default());
}

#[test]
fn missing_window_gets_defaults_with_always_on_top() {
    let r = migrate(r#"{"schemaVersion":2,"watchlist":[]}"#);
    assert!(r.config.window.always_on_top, "默认置顶");
    assert!(!r.config.window.locked, "默认不锁定");
    assert_eq!(r.config.window.x, None, "默认位置交给 tauri.conf.json");
}

#[test]
fn missing_watchlist_falls_back_to_four_defaults() {
    let r = migrate(r#"{"schemaVersion":2}"#);
    assert_eq!(r.config.watchlist.len(), 4);
}

#[test]
fn partial_appearance_keeps_given_and_defaults_rest() {
    let r = migrate(r#"{"schemaVersion":2,"watchlist":[],"appearance":{"scale":1.5}}"#);
    assert_eq!(r.config.appearance.scale, 1.5);
    assert_eq!(
        r.config.appearance.font_size,
        AppearanceConfig::default().font_size
    );
}

#[test]
fn explicitly_empty_watchlist_stays_empty() {
    // 用户删光自选股是合法状态，不该被"补默认 4 只"覆盖
    let r = migrate(r#"{"schemaVersion":2,"watchlist":[]}"#);
    assert!(r.config.watchlist.is_empty());
}

// ---------- 6. 数值钳制（配置文件是用户可手改的文本）----------

#[test]
fn scale_above_max_is_clamped() {
    let r = migrate(r#"{"schemaVersion":2,"watchlist":[],"appearance":{"scale":99}}"#);
    assert_eq!(r.config.appearance.scale, SCALE_MAX);
}

#[test]
fn scale_below_min_is_clamped() {
    let r = migrate(r#"{"schemaVersion":2,"watchlist":[],"appearance":{"scale":0.01}}"#);
    assert_eq!(r.config.appearance.scale, SCALE_MIN);
}

#[test]
fn negative_opacity_is_clamped_to_zero() {
    let r = migrate(
        r#"{"schemaVersion":2,"watchlist":[],"appearance":{"opacity_background":-5}}"#,
    );
    assert_eq!(r.config.appearance.opacity_background, 0.0);
}

#[test]
fn opacity_above_one_is_clamped() {
    let r = migrate(r#"{"schemaVersion":2,"watchlist":[],"appearance":{"opacity_text":7}}"#);
    assert_eq!(r.config.appearance.opacity_text, 1.0);
}

#[test]
fn non_finite_scale_falls_back_to_default() {
    // JSON 里 1e999 反序列化成 f64 会得到 Infinity
    let r = migrate(r#"{"schemaVersion":2,"watchlist":[],"appearance":{"scale":1e999}}"#);
    assert_eq!(
        r.config.appearance.scale,
        AppearanceConfig::default().scale
    );
}

// ---------- 7. 自选股净化：去重按身份键、上限截断 ----------

#[test]
fn duplicate_identity_is_deduped_on_load() {
    let json = serde_json::json!({
        "schemaVersion": 2,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false},
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ]
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.config.watchlist.len(), 1);
}

#[test]
fn two_markets_of_same_bare_code_both_survive_load() {
    // 净化去重必须按 tencent_code；按裸码去重会丢掉上证指数
    let json = serde_json::json!({
        "schemaVersion": 2,
        "watchlist": [
            {"tencent_code":"sz000001","em_secid":"0.000001","market":"ashare","is_index":false},
            {"tencent_code":"sh000001","em_secid":"1.000001","market":"indexcn","is_index":true}
        ]
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.config.watchlist.len(), 2, "两个不同标的都要留下");
}

#[test]
fn over_limit_watchlist_is_truncated_to_fifty() {
    let items: Vec<serde_json::Value> = (0..60)
        .map(|i| {
            serde_json::json!({
                "tencent_code": format!("sh6{:05}", i),
                "em_secid": format!("1.6{:05}", i),
                "market": "ashare",
                "is_index": false
            })
        })
        .collect();
    let json = serde_json::json!({"schemaVersion":2,"watchlist":items}).to_string();
    let r = migrate(&json);
    assert_eq!(r.config.watchlist.len(), 50, "超上限须截断到 50");
}

#[test]
fn item_with_empty_identity_is_dropped() {
    let json = serde_json::json!({
        "schemaVersion": 2,
        "watchlist": [
            {"tencent_code":"","em_secid":null,"market":"ashare","is_index":false},
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ]
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.config.watchlist.len(), 1);
    assert_eq!(r.config.watchlist[0].tencent_code, "sh600519");
}

#[test]
fn item_with_null_em_secid_is_accepted() {
    // em_secid 是 Option：没有 secid 是合法状态（不猜前缀）
    let json = serde_json::json!({
        "schemaVersion": 2,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":null,"market":"ashare","is_index":false}
        ]
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.config.watchlist.len(), 1);
    assert_eq!(r.config.watchlist[0].em_secid, None);
}

// ---------- 5. v3 → v4 迁移：提示音音色与音量 ----------

/// 真实的 v3 配置：有 alerts 但**没有**三个新字段，
/// 且刻意把 `bull_sound` 关掉、`bear_sound` 开着 —— 用来验证迁移不改开关。
fn v3_json() -> String {
    serde_json::json!({
        "schemaVersion": 3,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ],
        "appearance": {"scale": 1.15},
        "window": {"locked": true},
        "alerts": {
            "enabled": true,
            "bull_sound": false,
            "bear_sound": true,
            "card_opacity": 0.8,
            "cooldown_secs": 120,
            "default_up_percent": 5.0,
            "default_down_percent": 4.0,
            "rules": {}
        }
    })
    .to_string()
}

#[test]
fn v3_migrates_and_reports_source_version() {
    let r = migrate(&v3_json());
    assert_eq!(r.outcome, LoadOutcome::Migrated { from: 3 });
}

#[test]
fn v3_migration_fills_tone_defaults() {
    let r = migrate(&v3_json());
    // 默认音色是「清铃」而**不是**任何拟声音效 —— 用户明确不要牛叫那种
    assert_eq!(r.config.alerts.bull_tone, "chime");
    assert_eq!(r.config.alerts.bear_tone, "chime");
    assert!((r.config.alerts.sound_volume - 0.7).abs() < 1e-9);
}

#[test]
fn v3_migration_does_not_flip_sound_switches() {
    // 这是本次迁移最要紧的一条：v3 用户把上涨提示音关掉了，
    // 升级后不该因为"多了音色字段"就重新响起来。
    let r = migrate(&v3_json());
    assert!(!r.config.alerts.bull_sound, "关掉的上涨提示音不该被迁移打开");
    assert!(r.config.alerts.bear_sound, "开着的下跌提示音不该被关掉");
}

#[test]
fn v3_migration_preserves_all_other_alert_settings() {
    let r = migrate(&v3_json());
    let a = &r.config.alerts;
    assert!(a.enabled);
    assert!((a.card_opacity - 0.8).abs() < 1e-9);
    assert_eq!(a.cooldown_secs, 120);
    assert_eq!(a.default_up_percent, Some(5.0));
    assert_eq!(a.default_down_percent, Some(4.0));
    // 自选股与外观也一条不丢
    assert_eq!(r.config.watchlist.len(), 1);
    assert!((r.config.appearance.scale - 1.15).abs() < 1e-9);
    assert!(r.config.window.locked);
}

#[test]
fn v3_migration_keeps_user_chosen_tone_when_present() {
    // 用户手写过 bull_tone 就尊重它，不要覆盖成默认值
    let json = serde_json::json!({
        "schemaVersion": 3,
        "watchlist": [],
        "alerts": {"bull_tone": "wood", "sound_volume": 0.35}
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.config.alerts.bull_tone, "wood");
    assert!((r.config.alerts.sound_volume - 0.35).abs() < 1e-9);
    // 没写的那个仍补默认
    assert_eq!(r.config.alerts.bear_tone, "chime");
}

#[test]
fn v1_migration_applies_v3_to_v4_effects() {
    // 这条盯的是**v3→v4 那一步的效果**（音色与音量被补上），
    // 而不是"终点版本号是几"。
    //
    // 原先它写死 `schema_version == 4`，于是每次升版本号都会挂一次
    // （4→5 时就挂了）—— 那是重复劳动，不是有效防线：
    // "终点是当前版本"已由 `v1_migration_writes_back_current_schema_version`
    // 与 `v1_migrates_all_the_way_to_v5_in_one_pass` 覆盖。
    // 故这里改断言常量，把职责收敛到"这一级迁移真的跑了"。
    let r = migrate(&v1_json_with_five_items());
    assert_eq!(r.config.schema_version, SCHEMA_VERSION);
    assert_eq!(r.config.watchlist.len(), 5, "自选股一条不能丢");
    assert!(!r.config.alerts.enabled, "v1 用户不该突然开始弹提醒");
    assert_eq!(r.config.alerts.bull_tone, "chime");
    assert!((r.config.alerts.sound_volume - 0.7).abs() < 1e-9);
}

#[test]
fn v3_with_broken_alerts_object_still_loads() {
    // alerts 被手改成非对象（如字符串）时不该让整份配置判为损坏，
    // 而是退回默认提醒配置继续用
    let json = serde_json::json!({
        "schemaVersion": 3,
        "watchlist": [],
        "alerts": "oops"
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.config.alerts.bull_tone, "chime");
    assert!(!r.config.alerts.enabled);
}

// ---------- 6. v4 → v5 迁移：自选股可隐藏 ----------

/// 真实的 v4 配置：watchlist 里**没有** hidden 字段。
/// 刻意放三条，其中一条手写了 hidden:true 用来验证"不覆盖用户值"。
fn v4_json() -> String {
    serde_json::json!({
        "schemaVersion": 4,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false},
            {"tencent_code":"hk00700","em_secid":"116.00700","market":"hk","is_index":false,"hidden":true},
            {"tencent_code":"usAAPL","em_secid":"105.AAPL","market":"us","is_index":false}
        ],
        "appearance": {"scale": 1.2},
        "window": {"locked": true},
        "alerts": {"enabled": true, "bull_tone": "wood", "sound_volume": 0.4}
    })
    .to_string()
}

#[test]
fn v4_migrates_and_reports_source_version() {
    let r = migrate(&v4_json());
    assert_eq!(r.outcome, LoadOutcome::Migrated { from: 4 });
}

#[test]
fn v4_migration_defaults_hidden_to_false() {
    // 没写 hidden 的条目必须是**可见**的 —— 升级后突然有股票消失
    // 会让用户以为数据丢了，这是最不能出的错
    let r = migrate(&v4_json());
    assert!(!r.config.watchlist[0].hidden, "sh600519 应可见");
    assert!(!r.config.watchlist[2].hidden, "usAAPL 应可见");
}

#[test]
fn v4_migration_keeps_user_written_hidden_true() {
    // 用户手写过 hidden:true 就尊重它，不要重置成可见
    let r = migrate(&v4_json());
    assert!(r.config.watchlist[1].hidden, "hk00700 的 hidden:true 不该被重置");
}

#[test]
fn v4_migration_preserves_everything_else() {
    let r = migrate(&v4_json());
    let codes: Vec<&str> = r
        .config
        .watchlist
        .iter()
        .map(|i| i.tencent_code.as_str())
        .collect();
    assert_eq!(codes, vec!["sh600519", "hk00700", "usAAPL"], "自选股一条不能丢");
    assert!((r.config.appearance.scale - 1.2).abs() < 1e-9);
    assert!(r.config.window.locked);
    // v4 的提醒设置（含上一版新增的音色）原样保留
    assert!(r.config.alerts.enabled);
    assert_eq!(r.config.alerts.bull_tone, "wood");
    assert!((r.config.alerts.sound_volume - 0.4).abs() < 1e-9);
}

#[test]
fn v4_migration_writes_back_current_version_without_backup() {
    let r = migrate(&v4_json());
    assert_eq!(r.config.schema_version, 5);
    assert!(!r.outcome.needs_backup(), "迁移不是损坏，不得生成 .bak");
    assert!(r.outcome.needs_rewrite(), "迁移结果须写回磁盘固化");
}

#[test]
fn v1_migrates_all_the_way_to_v5_in_one_pass() {
    // 整条链一次跑完：v1 → v5。既有各级效果都还在，且新字段已补。
    let r = migrate(&v1_json_with_five_items());
    assert_eq!(r.config.schema_version, 5);
    assert_eq!(r.config.watchlist.len(), 5, "跨四级迁移也不能丢自选股");
    assert!(!r.config.alerts.enabled, "v1 用户不该被动开启提醒");
    assert_eq!(r.config.alerts.bull_tone, "chime");
    // v5 的新字段：全部可见
    assert!(
        r.config.watchlist.iter().all(|i| !i.hidden),
        "从 v1 升上来的自选股必须全部可见"
    );
}

// ---------- base_price（提醒基准价）：加字段不需要迁移 ----------
//
// 这组测试是「加 base_price 不必把 SCHEMA_VERSION 抬到 6」这个判断的**证据**。
// 依据是 `Option<f64>` + `AlertConfig` 的 `#[serde(default)]`：缺字段补 None。
// 光凭这个推理不够 —— serde 对 Option 的行为容易记错，所以让测试来证。

fn v5_json_rule_without_base_price() -> String {
    serde_json::json!({
        "schemaVersion": 5,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ],
        "alerts": {
            "enabled": true,
            "rules": {
                // 老配置里的规则**没有** base_price 字段
                "sh600519": {"up_percent": 3.0, "bull_price": 1500.0}
            }
        }
    })
    .to_string()
}

#[test]
fn v5_rule_without_base_price_loads_as_none() {
    let r = migrate(&v5_json_rule_without_base_price());
    let rule = r.config.alerts.rules.get("sh600519").expect("规则不该丢");
    assert_eq!(rule.base_price, None, "缺字段应补 None，而不是 0.0");
}

#[test]
fn v5_rule_without_base_price_keeps_existing_fields() {
    // 加字段不能动到老字段 —— 用户已配的阈值/目标价必须原样在
    let r = migrate(&v5_json_rule_without_base_price());
    let rule = r.config.alerts.rules.get("sh600519").unwrap();
    assert_eq!(rule.up_percent, Some(3.0));
    assert_eq!(rule.bull_price, Some(1500.0));
}

#[test]
fn v5_config_with_new_field_is_still_current_version() {
    // 关键结论：v5 配置读进来仍是 Current，不触发迁移、不写回、不备份。
    // 这一条挂了就说明 base_price 确实需要一次 v5→v6 迁移。
    let r = migrate(&v5_json_rule_without_base_price());
    assert_eq!(r.outcome, LoadOutcome::Current, "加 Option 字段不该触发迁移");
    assert_eq!(r.config.schema_version, SCHEMA_VERSION);
}

#[test]
fn base_price_is_read_back_when_present() {
    let json = serde_json::json!({
        "schemaVersion": 5,
        "alerts": {"rules": {"sh600519": {"base_price": 1680.5, "up_percent": 2.0}}}
    })
    .to_string();
    let r = migrate(&json);
    let rule = r.config.alerts.rules.get("sh600519").unwrap();
    assert_eq!(rule.base_price, Some(1680.5));
}

#[test]
fn rule_with_only_base_price_survives_prune() {
    // is_empty() 计入 base_price 的理由：用户填了基准价还没填涨跌幅时，
    // 这条不能被当空壳删掉，否则重开设置页发现自己填的数没了。
    let json = serde_json::json!({
        "schemaVersion": 5,
        "alerts": {"rules": {"sh600519": {"base_price": 1680.5}}}
    })
    .to_string();
    let r = migrate(&json);
    assert!(
        r.config.alerts.rules.contains_key("sh600519"),
        "只填基准价的规则不该被 prune 掉"
    );
}

#[test]
fn non_positive_base_price_is_dropped_and_then_rule_pruned() {
    // 基准价是价格，0/负数视为未设置（与 bull_price/bear_price 同规则）。
    // 清掉后这条规则五项全空，于是被 prune —— 两步行为都在这一条里锁住。
    let json = serde_json::json!({
        "schemaVersion": 5,
        "alerts": {"rules": {"sh600519": {"base_price": 0.0}}}
    })
    .to_string();
    let r = migrate(&json);
    assert!(
        !r.config.alerts.rules.contains_key("sh600519"),
        "基准价 0 应被清成 None，规则随即因全空被删"
    );
}

#[test]
fn v4_with_non_array_watchlist_does_not_panic() {
    // watchlist 被手改成非数组：不该 panic，走损坏降级或 serde 兜底
    let json = serde_json::json!({
        "schemaVersion": 4,
        "watchlist": "oops"
    })
    .to_string();
    let r = migrate(&json);
    // 只要不 panic 即可；损坏时会恢复默认 4 只
    assert!(!r.config.watchlist.is_empty() || r.config.watchlist.is_empty());
}

// ─────────── appearance.theme（浅色主题，2026-08-30 新增字段）───────────
//
// 与 base_price 同一个套路：`Option<Theme>` + 结构体上的 `#[serde(default)]`
// 让老配置不需要 schema 迁移。**这条必须用测试证，不能靠推理** ——
// serde 对 Option 的行为容易记错，写错的代价是用户配置被静默重置。

fn v5_json_appearance_without_theme() -> String {
    serde_json::json!({
        "schemaVersion": 5,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ],
        "appearance": {
            // 老配置的 appearance **没有** theme 字段
            "scale": 1.2,
            "curve_width": 1.5,
            "font_size": 14.0,
            "show_code": true,
            "opacity_background": 0.7
        }
    })
    .to_string()
}

#[test]
fn v5_appearance_without_theme_loads_as_none() {
    let r = migrate(&v5_json_appearance_without_theme());
    assert_eq!(
        r.config.appearance.theme, None,
        "缺 theme 字段应补 None（= 深色），而不是报错或换成别的值"
    );
}

#[test]
fn v5_appearance_without_theme_keeps_existing_fields() {
    // 加字段不能动到老字段 —— 用户已调的缩放/字号/不透明度必须原样在
    let r = migrate(&v5_json_appearance_without_theme());
    let a = &r.config.appearance;
    assert_eq!(a.scale, 1.2);
    assert_eq!(a.font_size, 14.0);
    assert!(a.show_code);
    assert_eq!(a.opacity_background, 0.7);
}

// ─────────── refresh 段（可配置刷新间隔，2026-08-31 新增）───────────
//
// 与 base_price / theme 同一个套路：整段带 `#[serde(default)]`，
// 旧配置缺 `refresh` 时补默认值。**必须测**，因为这段控制网络请求速率 ——
// 若 serde 补出 0 或极小值，会让请求速率打爆上游接口。

#[test]
fn v5_config_without_refresh_gets_safe_defaults() {
    let json = serde_json::json!({
        "schemaVersion": 5,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ]
        // 老配置**没有** refresh 段
    })
    .to_string();
    let r = migrate(&json);
    // 与改为可配置之前的硬编码值一致，升级后行为不变
    assert_eq!(r.config.refresh.quote_secs, 3);
    assert_eq!(r.config.refresh.minute_secs, 5);
}

#[test]
fn refresh_too_small_is_clamped_up() {
    // 关键安全测试：手改配置文件写 1 秒，必须被钳到下限。
    // 分时是逐标的请求，1s × 20 只 = 20 请求/秒，会被上游拉黑
    let json = serde_json::json!({
        "schemaVersion": 5,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ],
        "refresh": { "quote_secs": 0, "minute_secs": 1 }
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(
        r.config.refresh.quote_secs,
        tickerglass_lib::config::QUOTE_SECS_MIN,
        "0 秒必须钳到下限，否则轮询变成忙等"
    );
    assert_eq!(
        r.config.refresh.minute_secs,
        tickerglass_lib::config::MINUTE_SECS_MIN
    );
}

#[test]
fn refresh_too_large_is_clamped_down() {
    let json = serde_json::json!({
        "schemaVersion": 5,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ],
        "refresh": { "quote_secs": 99999, "minute_secs": 99999 }
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(
        r.config.refresh.quote_secs,
        tickerglass_lib::config::QUOTE_SECS_MAX
    );
    assert_eq!(
        r.config.refresh.minute_secs,
        tickerglass_lib::config::MINUTE_SECS_MAX
    );
}

#[test]
fn refresh_valid_values_pass_through() {
    // 区间内的值不该被改动
    let json = serde_json::json!({
        "schemaVersion": 5,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ],
        "refresh": { "quote_secs": 10, "minute_secs": 20 }
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.config.refresh.quote_secs, 10);
    assert_eq!(r.config.refresh.minute_secs, 20);
}

#[test]
fn v5_appearance_with_theme_light_round_trips() {
    // 显式存了 light 要能读回来（序列化名是小写，靠 rename_all="lowercase"）
    let json = serde_json::json!({
        "schemaVersion": 5,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ],
        "appearance": { "theme": "light" }
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(
        r.config.appearance.theme,
        Some(tickerglass_lib::config::Theme::Light)
    );
}

// ─────────── window.collapsed 段（球态悬浮窗，2026-09-01 新增）───────────
//
// 与 theme / base_price 同一个套路：`Option<bool>` + `#[serde(default)]`，
// 旧配置缺字段时补 `None`。**必须测**，因为这里 `None` 的业务语义是
// "球态"（true），与 `bool::default()`（false）相反 —— 任何地方误用
// `unwrap_or_default()` 都会让老用户升级后直接看到展开的列表。

fn v5_json_window_without_collapsed() -> String {
    serde_json::json!({
        "schemaVersion": 5,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ],
        "window": {
            // 老配置的 window **没有** collapsed 字段
            "x": 120,
            "y": 240,
            "always_on_top": false,
            "locked": true
        }
    })
    .to_string()
}

#[test]
fn v5_window_without_collapsed_loads_as_none() {
    let r = migrate(&v5_json_window_without_collapsed());
    assert_eq!(
        r.config.window.collapsed, None,
        "缺 collapsed 字段应补 None，而不是报错或落成 Some(false)"
    );
}

#[test]
fn missing_collapsed_means_ball_not_expanded() {
    // 这条是整段的**核心**：None 必须解读成球态。
    // 若哪天有人把 is_collapsed() 改成 unwrap_or_default()，这里立刻红。
    let r = migrate(&v5_json_window_without_collapsed());
    assert!(
        r.config.window.is_collapsed(),
        "None 的语义是球态（true），不是 bool::default() 的 false"
    );
}

#[test]
fn v5_window_without_collapsed_keeps_existing_fields() {
    // 加字段不能动到老字段 —— 用户的位置/置顶/锁定必须原样在
    let r = migrate(&v5_json_window_without_collapsed());
    let w = &r.config.window;
    assert_eq!(w.x, Some(120));
    assert_eq!(w.y, Some(240));
    assert!(!w.always_on_top);
    assert!(w.locked);
}

#[test]
fn explicit_collapsed_false_stays_expanded() {
    // 用户手动展开过（或手改配置），必须尊重 —— 不能被 None 的默认值覆盖回球态
    let json = serde_json::json!({
        "schemaVersion": 5,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ],
        "window": { "collapsed": false }
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.config.window.collapsed, Some(false));
    assert!(
        !r.config.window.is_collapsed(),
        "显式 false 必须保持展开态"
    );
}

#[test]
fn explicit_collapsed_true_round_trips() {
    let json = serde_json::json!({
        "schemaVersion": 5,
        "watchlist": [
            {"tencent_code":"sh600519","em_secid":"1.600519","market":"ashare","is_index":false}
        ],
        "window": { "collapsed": true }
    })
    .to_string();
    let r = migrate(&json);
    assert_eq!(r.config.window.collapsed, Some(true));
    assert!(r.config.window.is_collapsed());
}

#[test]
fn collapsed_field_does_not_bump_schema_version() {
    // 加 Option 字段不需要迁移，所以版本号不该变（与 theme / refresh 同理）
    let r = migrate(&v5_json_window_without_collapsed());
    assert_eq!(r.outcome, LoadOutcome::Current, "加 Option 字段不该触发迁移");
    assert_eq!(r.config.schema_version, SCHEMA_VERSION);
    assert!(
        !r.outcome.needs_backup(),
        "补一个 Option 字段不是损坏，不该触发备份"
    );
}

#[test]
fn default_window_config_is_ball() {
    // 全新用户（无配置文件）也该看到球 —— 这是 2026-09-01 起的主形态
    let cfg = AppConfig::default();
    assert!(
        cfg.window.is_collapsed(),
        "默认配置应为球态"
    );
}
