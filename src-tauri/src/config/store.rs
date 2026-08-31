// 配置的**唯一 I/O 出入口**。解析逻辑全在纯函数 `migrate` 里，
// 这里只做：定位路径、读文件、备份、原子写。

use super::migrate::{migrate, LoadOutcome, MigrationResult};
use super::schema::AppConfig;
use std::path::{Path, PathBuf};

/// 配置目录名（`%APPDATA%/StockPetPlus/`）。
const APP_DIR: &str = "StockPetPlus";
const CONFIG_FILE: &str = "config.json";
const BACKUP_FILE: &str = "config.json.bak";

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

/// 加载配置。文件不存在 = 首次启动，用默认配置且**不算损坏**（不备份、不提示）。
///
/// 损坏时把**用户原文件**备份为 `config.json.bak`（不是备份默认配置 ——
/// 标准 19 的判据 c 明确要求 `.bak` 内容是那段非法 JSON）。
pub fn load() -> MigrationResult {
    let Some(path) = config_path() else {
        log::warn!("[配置] 无法定位 %APPDATA%，本次运行使用默认配置且不持久化");
        return MigrationResult {
            config: AppConfig::default(),
            outcome: LoadOutcome::Current,
        };
    };

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!("[配置] 首次启动，使用默认配置：{}", path.display());
            let config = AppConfig::default();
            // 首次启动立即落盘，让用户能看到并手改这个文件
            if let Err(e) = save(&config) {
                log::warn!("[配置] 首次写盘失败: {e}");
            }
            return MigrationResult {
                config,
                outcome: LoadOutcome::Current,
            };
        }
        Err(e) => {
            // 读失败（权限等）不备份：原文件还在，不该拿一个读不出的文件去覆盖 .bak
            log::error!("[配置] 读取失败 {}: {e}", path.display());
            return MigrationResult {
                config: AppConfig::default(),
                outcome: LoadOutcome::Current,
            };
        }
    };

    let result = migrate(&raw);

    if result.outcome.needs_backup() {
        match backup_path() {
            Some(bak) => {
                // 备份的是用户原文件内容（`raw`），已存在则覆盖
                match std::fs::write(&bak, raw.as_bytes()) {
                    Ok(_) => log::warn!(
                        "[配置] 文件损坏，已备份原文件到 {}（原因见下）",
                        bak.display()
                    ),
                    Err(e) => log::error!("[配置] 备份失败: {e}"),
                }
            }
            None => log::error!("[配置] 无法定位备份路径"),
        }
        if let LoadOutcome::Corrupted { reason } = &result.outcome {
            log::warn!("[配置] 损坏原因: {reason}");
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
        if let Err(e) = save(&result.config) {
            log::error!("[配置] 迁移后写回失败: {e}");
        }
    }

    result
}

/// 原子写：先写同目录临时文件再 `rename`，避免写入过程中崩溃留下半截文件。
///
/// 同目录是必须的 —— 跨盘 `rename` 在 Windows 上会失败。
pub fn save(config: &AppConfig) -> Result<(), String> {
    let Some(path) = config_path() else {
        return Err("无法定位 %APPDATA%".to_string());
    };
    let dir = path
        .parent()
        .ok_or_else(|| "配置路径没有父目录".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败: {e}"))?;

    let json = serde_json::to_string_pretty(config).map_err(|e| format!("序列化失败: {e}"))?;
    write_atomic(&path, json.as_bytes())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("写临时文件失败: {e}"))?;
    // Windows 上 rename 到已存在的文件会失败，故先删旧文件。
    // 这中间的窗口里若崩溃，.tmp 仍在，下次启动会走"文件不存在→默认配置"，
    // 比留下半截 config.json 好（后者会被判损坏并覆盖 .bak）。
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| format!("删除旧配置失败: {e}"))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("重命名临时文件失败: {e}"))
}
