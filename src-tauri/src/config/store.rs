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
//
// # v0.4.0 断电加固（需求 3）—— 用户实际遭遇过异常关机丢配置
//
// v0.3.0 的备份机制有两处窗口会在断电时失效（用户配置目录里真的出现过
// `config.json.corrupt`）：
//
// 1. **写入没有 fsync**。`fs::write` 只到 OS 缓存；断电时 rename（元数据）
//    可能已落盘而数据没有，留下 0 字节或半截文件 —— 损坏就是这么来的。
//    现在 [`write_atomic`] 在 rename 前 `sync_all`，字节真正到达磁盘才换名。
// 2. **"文件不存在"不尝试 .bak**。旧逻辑把 NotFound 当"首次启动"回默认配置，
//    但断电也可能造成文件丢失（旧实现先 remove 后 rename 的中间窗口、
//    或文件系统元数据回滚）。现在 NotFound 同样先试 .bak，能救就救+自愈。

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
            // v0.4.0：NotFound 不再直接当"首次启动"。断电也可能让主文件整个丢失
            // （旧 write_atomic 的 remove→rename 中间窗口、NTFS 元数据回滚），
            // 此时 .bak 里往往还有上一份完好配置 —— 不看它就回默认，
            // 用户的自选股就全没了（用户实际遭遇过的场景）。
            if let Some(healed) =
                recover_from_backup(dir, "主文件丢失（疑似异常关机落在写入窗口）")
            {
                return healed;
            }
            log::info!("[配置] 首次启动（无主文件亦无可用备份），使用默认配置：{}", path.display());
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

        if let Some(healed) = recover_from_backup(dir, reason) {
            return healed;
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

/// 尝试从 `.bak` 恢复（v0.4.0 抽出的共用路径，主文件损坏与丢失都走这里）。
///
/// 返回 `Some(healed)` = 备份可用，已构造 RecoveredFromBackup 结果并自愈写回主文件；
/// `None` = 备份不存在/同样损坏，调用方回落默认配置。
fn recover_from_backup(dir: &Path, reason: &str) -> Option<MigrationResult> {
    let bak_raw = std::fs::read_to_string(dir.join(BACKUP_FILE)).ok()?;
    let bak = migrate(&bak_raw);
    if matches!(bak.outcome, LoadOutcome::Corrupted { .. }) {
        log::error!("[配置] 备份文件同样损坏，恢复默认配置");
        return None;
    }
    log::warn!(
        "[配置] 已从 {} 恢复（备份版本态 {:?}，原因：{reason}），自选股与设置保留",
        BACKUP_FILE,
        bak.outcome
    );
    let healed = MigrationResult {
        config: bak.config,
        outcome: LoadOutcome::RecoveredFromBackup {
            reason: reason.to_string(),
        },
    };
    // 自愈：把恢复结果写回主文件。save_at 只会备份"可解析的"主文件，
    // 而此刻主文件是损坏/丢失的 → 不会把垃圾写进 .bak，安全。
    if let Err(e) = save_at(dir, &healed.config) {
        log::error!("[配置] 自愈写回失败: {e}");
    }
    Some(healed)
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

/// 原子写。v0.4.0 两处断电加固（见模块文档的 v0.4.0 一节）：
///
/// 1. **`sync_all` 后才 rename** —— `fs::write` 只到 OS 缓存，断电时 rename
///    可能已落盘而数据没有，留下 0 字节/半截主文件。sync 把字节真正推到磁盘。
/// 2. **不再"先删旧文件再改名"** —— `std::fs::rename` 在 Windows 上对已存在的
///    目标文件走 `MoveFileEx(REPLACE_EXISTING)` 直接覆盖（本函数被
///    `save_maintains_last_good_backup` 等测试在覆盖场景下反复验证）。
///    旧实现的"先 remove 后 rename"中间若断电，主文件直接消失。
///    现在即使真出现文件丢失，加载侧还有 .bak 恢复兜底（`load_at` 的 NotFound 分支）。
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("写临时文件失败: {e}"))?;
        f.write_all(bytes).map_err(|e| format!("写临时文件失败: {e}"))?;
        f.sync_all().map_err(|e| format!("刷盘失败: {e}"))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("重命名临时文件失败: {e}"))
}
