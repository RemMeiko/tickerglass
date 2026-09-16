//! 配置备份机制（v0.3.0 需求 2）。锁三条行为：
//!
//! 1. `save_at` 维护 `.bak` = **上一份完好配置**（不是损坏后的原文）
//! 2. 主文件损坏时 `load_at` 自动走 `.bak` 恢复（RecoveredFromBackup）+ 自愈主文件
//!    + 损坏原文另存 `.corrupt`
//! 3. 备份也不可用时才回默认配置（Corrupted），且不把垃圾写进 `.bak`
//!
//! 走 `load_at`/`save_at`（可测版本，指定目录），不碰真实 %APPDATA%。

use std::path::PathBuf;
use tickerglass_lib::config::schema::AppConfig;
use tickerglass_lib::config::store::{load_at, save_at};
use tickerglass_lib::config::LoadOutcome;

/// 每个用例独立临时目录，互不污染（cargo test 默认多线程跑）
fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("tickerglass-store-test-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn config_with_code(code: &str) -> AppConfig {
    let mut c = AppConfig::default();
    c.watchlist.truncate(1);
    c.watchlist[0].tencent_code = code.to_string();
    c
}

#[test]
fn save_maintains_last_good_backup() {
    let dir = tmp_dir("bak");
    // 第一次 save：主文件不存在，.bak 不产生（没有"上一份"可备份）
    save_at(&dir, &config_with_code("sh600519")).unwrap();
    assert!(!dir.join("config.json.bak").exists());

    // 第二次 save：上一份（600519）应转存进 .bak
    save_at(&dir, &config_with_code("sz002466")).unwrap();
    let bak = std::fs::read_to_string(dir.join("config.json.bak")).unwrap();
    let bak_cfg: AppConfig = serde_json::from_str(&bak).unwrap();
    assert_eq!(bak_cfg.watchlist[0].tencent_code, "sh600519");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupted_main_recovers_from_backup_and_heals() {
    let dir = tmp_dir("recover");
    save_at(&dir, &config_with_code("sh600519")).unwrap();
    save_at(&dir, &config_with_code("sz002466")).unwrap();

    // 异常关机：主文件写坏（半截 JSON），.bak 还是好的（600519 那份）
    std::fs::write(dir.join("config.json"), b"{\"schemaVersion\":6,\"watch").unwrap();

    let r = load_at(&dir);
    assert!(
        matches!(r.outcome, LoadOutcome::RecoveredFromBackup { .. }),
        "主文件损坏 + 备份可用 → 必须是 RecoveredFromBackup，实际 {:?}",
        r.outcome
    );
    // 用户数据从备份恢复（600519），不是默认四只
    assert_eq!(r.config.watchlist[0].tencent_code, "sh600519");

    // 自愈：主文件已被恢复结果覆写（再读一遍应是 Current）
    let r2 = load_at(&dir);
    assert_eq!(r2.outcome, LoadOutcome::Current);
    assert_eq!(r2.config.watchlist[0].tencent_code, "sh600519");

    // 损坏原文保留在 .corrupt
    let corrupt = std::fs::read_to_string(dir.join("config.json.corrupt")).unwrap();
    assert!(corrupt.contains("schemaVersion"), "原文另存内容不符: {corrupt}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupted_main_without_backup_falls_back_to_default() {
    let dir = tmp_dir("nobak");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.json"), "not json at all").unwrap();

    let r = load_at(&dir);
    assert!(matches!(r.outcome, LoadOutcome::Corrupted { .. }));
    assert_eq!(
        r.config.watchlist.len(),
        AppConfig::default().watchlist.len(),
        "无备份可救时回默认配置"
    );
    // 默认配置已写回（下次启动至少是完好的 v6 文件）
    assert!(dir.join("config.json").exists());
    assert!(!dir.join("config.json.bak").exists(), "无备份时不得伪造 .bak");
    assert!(dir.join("config.json.corrupt").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupted_main_with_corrupted_backup_also_falls_back_to_default() {
    let dir = tmp_dir("badbak");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.json"), "{ broken").unwrap();
    std::fs::write(dir.join("config.json.bak"), "{ also broken").unwrap();

    let r = load_at(&dir);
    assert!(matches!(r.outcome, LoadOutcome::Corrupted { .. }));
    assert_eq!(r.config.watchlist.len(), AppConfig::default().watchlist.len());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn first_run_writes_default_without_backup() {
    let dir = tmp_dir("first");
    let r = load_at(&dir);
    assert_eq!(r.outcome, LoadOutcome::Current, "首次启动不算损坏");
    assert!(dir.join("config.json").exists(), "首次启动立即落盘");
    assert!(!dir.join("config.json.bak").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn heal_after_corruption_does_not_poison_backup() {
    // 场景接力：损坏恢复后继续 save —— .bak 必须仍然是"上一份完好配置"，
    // 不能把损坏期那份东西写进去（save_at 只转存**可解析的**主文件）。
    // 注意必须 save 两次：第一次 save 不产生 .bak（没有"上一份"），
    // 少 save 一次的话损坏时备份还不存在，走的是"回默认"分支，测不到本条
    let dir = tmp_dir("poison");
    save_at(&dir, &config_with_code("sh600519")).unwrap();
    save_at(&dir, &config_with_code("sz002466")).unwrap(); // .bak ← 600519
    std::fs::write(dir.join("config.json"), b"garbage").unwrap();
    let r = load_at(&dir); // 自愈：从 .bak 恢复 600519 并写回主文件
    assert!(matches!(r.outcome, LoadOutcome::RecoveredFromBackup { .. }));

    save_at(&dir, &config_with_code("hk00700")).unwrap();
    let bak = std::fs::read_to_string(dir.join("config.json.bak")).unwrap();
    let bak_cfg: AppConfig = serde_json::from_str(&bak)
        .expect(".bak 必须始终是可解析的完好配置");
    // 恢复出来的 600519 是"上一份完好配置"，而不是 garbage 也不是默认四只
    assert_eq!(bak_cfg.watchlist[0].tencent_code, "sh600519");

    let _ = std::fs::remove_dir_all(&dir);
}
