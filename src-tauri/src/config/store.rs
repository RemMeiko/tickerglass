// 配置的**唯一 I/O 出入口**。解析逻辑全在纯函数 `migrate` 里，
// 这里只做：定位路径、读文件、备份、原子写。
//
// # v0.3.0 备份机制（需求 2）—— .bak 的语义变了
//
// 旧语义（Sprint 2 时代）：.bak 只在主文件损坏**之后**写入，内容是**损坏的原文**，
// 用户的自选股照样丢（回默认配置）。它只是取证用。
//
// 新语义：
// - `.bak` = **上一份完好配置**。每次 save 落盘前，若当前主文件内容是合法
//   JSON 对象，就先把它转存进 .bak —— 于是 .bak 永远是"最近一次可加载的配置"。
// - 主文件损坏（异常关机截断等）→ 加载时自动改走 .bak，把恢复结果写回主文件
//   （自愈），损坏原文另存 `.corrupt` 留排查。用户除了看到一条可关掉的提示，
//   自选股与设置原样都在 —— 这就是"感受不到"。
// - .bak 也坏/不存在 → 仍旧回默认配置（与旧行为一致，这是能做的全部）。

use super::migrate::{migrate, LoadOutcome, MigrationResult};
use super::schema::AppConfig;
use std::path::{Path, PathBuf};

/// 配置目录名（`%APPDATA%/StockPetPlus/`）。
const APP_DIR: &str = "StockPetPlus";
const CONFIG_FILE: &str = "config.json";
/// 上一份完好配置（见模块文档的新语义）。
const BACKUP_FILE: &str = "config.json.bak";
/// 主文件损坏时的原文另存（覆盖式，只留最近一次）。
const CORRUPT_FILE: &str = "config.json.corrupt";

/// 配置文件所在目录。`None` = 拿不到 `%APPDATA%`（极端环境）。
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR))
}

pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join(CONFIG_FILE))
}

pub fn backup_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join(BACKUP_FILE))
}

/// 自定义提示音的存放目录（`%APPDATA%/StockPetPlus/sounds/`）。
///
/// 为什么把用户选的音频**复制进来**而不是在配置里记原路径：
/// 原文件可能被删、被移动、或在移动硬盘上 —— 那样提示音会某天突然失效
/// 且用户完全不知道为什么。复制一份进配置目录后，配置里只存文件名，
/// 与 `AlertConfig::sanitize` 里禁止文件名含路径分隔符的约束配套。
pub fn sounds_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("sounds"))
}

/// 加载配置（生产入口，`%APPDATA%`）。行为见 [`load_at`]。
pub fn load() -> MigrationResult {
    match config_dir() {
        Some(dir) => load_at(&dir),
        None => {
            log::warn!("[配置] 无法定位 %APPDATA%，本次运行使用默认配置且不持久化");
            MigrationResult {
                config: AppConfig::default(),
                outcome: LoadOutcome::Current,
            }
        }
    }
}

/// [`load`] 的可测版本：在指定目录上执行完整的 加载/回退/自愈 流程。
///
/// 文件不存在 = 首次启动，用默认配置且**不算损坏**（不备份、不提示）。
/// 主文件损坏 → 存 `.corrupt` → 尝试 `.bak`；`.bak` 可加载则用它并自愈主文件。
pub fn load_at(dir: &Path) -> MigrationResult {
    let path = dir.join(CONFIG_FILE);
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!("[配置] 首次启动，使用默认配置：{}", path.display());
            let config = AppConfig::default();
            // 首次启动立即落盘，让用户能看到并手改这个文件
            if let Err(e) = save_at(dir, &config) {
                log::warn!("[配置] 首次写盘失败: {e}");
            }
            return MigrationResult {
                config,
                outcome: LoadOutcome::Current,
            };
        }
        Err(e) => {
            // 读失败（权限等）不另存 .corrupt：原文件还在，
            // 拿一个读不出的文件去覆盖 .corrupt 没有意义
            log::error!("[配置] 读取失败 {}: {e}", path.display());
            return MigrationResult {
                config: AppConfig::default(),
                outcome: LoadOutcome::Current,
            };
        }
    };

    let result = migrate(&raw);

    if let LoadOutcome::Corrupted { reason } = &result.outcome {
        // ── 主文件损坏：另存原文 → 尝试备份 ──
        let corrupt = dir.join(CORRUPT_FILE);
        match std::fs::write(&corrupt, raw.as_bytes()) {
            Ok(_) => log::warn!(
                "[配置] 主文件损坏（{reason}），原文已存 {}，尝试备份恢复",
                corrupt.display()
            ),
            Err(e) => log::error!("[配置] 损坏原文另存失败: {e}"),
        }

        match std::fs::read_to_string(dir.join(BACKUP_FILE)) {
            Ok(bak_raw) => {
                let bak = migrate(&bak_raw);
                if matches!(bak.outcome, LoadOutcome::Corrupted { .. }) {
                    log::error!("[配置] 备份文件同样损坏，恢复默认配置");
                    // 走原有的损坏路径：默认配置 + 写回固化
                } else {
                    log::warn!(
                        "[配置] 已从 {} 恢复（备份版本态 {:?}），自选股与设置保留",
                        BACKUP_FILE,
                        bak.outcome
                    );
                    let healed = MigrationResult {
                        config: bak.config,
                        outcome: LoadOutcome::RecoveredFromBackup {
                            reason: reason.clone(),
                        },
                    };
                    // 自愈：把恢复结果写回主文件。save_at 只会备份"可解析的"主文件，
                    // 而此刻主文件是损坏的 → 不会把垃圾写进 .bak，安全。
                    if let Err(e) = save_at(dir, &healed.config) {
                        log::error!("[配置] 自愈写回失败: {e}");
                    }
                    return healed;
                }
            }
            Err(e) => {
                log::error!("[配置] 备份不可读，恢复默认配置: {e}");
            }
        }
    }

    if let LoadOutcome::FutureVersion { found } = &result.outcome {
        log::warn!(
            "[配置] 文件版本 v{found} 比当前程序（v{}）新，已按兼容方式加载；\
             未识别的字段不会被保留",
            super::schema::SCHEMA_VERSION
        );
    }

    // 迁移与损坏都把净化结果写回，固化到当前版本
    if result.outcome.needs_rewrite() {
        if let Err(e) = save_at(dir, &result.config) {
            log::error!("[配置] 迁移后写回失败: {e}");
        }
    }

    result
}

/// 原子写：先写同目录临时文件再 `rename`，避免写入过程中崩溃留下半截文件。
///
/// 同目录是必须的 —— 跨盘 `rename` 在 Windows 上会失败。
pub fn save(config: &AppConfig) -> Result<(), String> {
    match config_dir() {
        Some(dir) => save_at(&dir, config),
        None => Err("无法定位 %APPDATA%".to_string()),
    }
}

/// [`save`] 的可测版本。
///
/// 落盘前先做**滚动备份**：若当前主文件内容是合法 JSON 对象（即上一份完好
/// 配置），把它转存进 `.bak`。只在"可解析"时备份 —— 主文件此刻若已是
/// 损坏状态（例如 load_at 自愈前的窗口），把垃圾转存进 .bak 会把最后一份
/// 好备份也毁掉。
pub fn save_at(dir: &Path, config: &AppConfig) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
    let path = dir.join(CONFIG_FILE);

    if let Ok(old) = std::fs::read_to_string(&path) {
        let parses = serde_json::from_str::<serde_json::Value>(&old)
            .ok()
            .and_then(|v| v.as_object().map(|_| ()))
            .is_some();
        if parses {
            let bak = dir.join(BACKUP_FILE);
            write_atomic(&bak, old.as_bytes())?;
        }
    }

    let json = serde_json::to_string_pretty(config).map_err(|e| format!("序列化失败: {e}"))?;
    write_atomic(&path, json.as_bytes())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("写临时文件失败: {e}"))?;
    // Windows 上 rename 到已存在的文件会失败，故先删旧文件。
    // 这中间的窗口里若崩溃，.tmp 仍在，下次启动会走"文件不存在→默认配置"，
    // 比留下半截 config.json 好（后者会被判损坏并触发备份恢复）。
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| format!("删除旧配置失败: {e}"))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("重命名临时文件失败: {e}"))
}
