//! 当日分时磁盘缓存（v0.4.0 需求 1）。
//!
//! # 为什么需要它
//!
//! 分时是**逐标的一个请求**的数据源，长时间高频轮询会被上游以 HTTP 501
//! 拒绝（用户实测：悬浮窗挂一段时间后当日分时全挂）。缓存解决两件事：
//!
//! 1. **重启不重拉**：同一天内重启 app，曲线从盘上直接恢复、立即可见，
//!    不必等一轮网络（也不必在限流惩罚期里硬闯）；
//! 2. **限流期间有图可看**：501 冷却期间快照保留最后一次成功数据；
//!    若冷却跨过了重启，缓存仍是当日的。
//!
//! 布局：`%APPDATA%/StockPetPlus/cache/minute/{本地日期}/{tencent_code}.json`。
//! 按日期分目录，跨天即失效 —— 启动时清掉非今天的目录（[`purge_old_days`]）。
//!
//! 可测性：`*_at` 版本接受显式 root/date（测试用临时目录），
//! 无后缀版本是生产入口（包一层 `config_dir()` + 当天日期）。
//!
//! 写缓存**不 fsync**：丢最后一个分钟点无伤大雅（下一轮成功就补上），
//! 不值得为此在每轮轮询里拖一次强制刷盘。这与 config.json 的策略不同
//!（那个丢了用户自选股，见 `config::store` 的 v0.4.0 加固）。

use crate::config::store::config_dir;
use crate::model::MinuteSeries;
use std::path::{Path, PathBuf};

/// 本地日期字符串（`YYYY-MM-DD`）。交易日按本地时区算，与分时数据的口径一致。
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 生产缓存根目录（`%APPDATA%/StockPetPlus/cache/minute`）。
/// `None` = 拿不到配置目录（极端环境，调用方静默跳过）。
fn cache_root() -> Option<PathBuf> {
    config_dir().map(|d| d.join("cache").join("minute"))
}

/// 文件名安全化：小写 + 只留字母数字，其余换 `_`。
/// 正常的腾讯代码（`sh600519`/`hk00700`/`usAAPL`）本就只含字母数字，
/// 这层防御是给"配置被手改成怪代码"的场景的。
fn safe_name(code: &str) -> String {
    code.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

// ───────────── 生产入口（真实目录 + 当天日期） ─────────────

/// 写入一条当日缓存。失败只记 debug 日志 —— 缓存写失败不该影响行情链路。
pub fn save_series(series: &MinuteSeries) {
    if let Some(root) = cache_root() {
        save_series_at(&root, &today(), series);
    }
}

/// 读今天某标的的缓存。
pub fn load_series(code: &str) -> Option<MinuteSeries> {
    let root = cache_root()?;
    load_series_at(&root, &today(), code)
}

/// 读回今天全部缓存（启动预载用）。
pub fn load_all_today() -> Vec<MinuteSeries> {
    match cache_root() {
        Some(root) => load_all_at(&root, &today()),
        None => Vec::new(),
    }
}

/// 清掉非今天的日期目录（启动时调用一次）。
pub fn purge_old_days() {
    if let Some(root) = cache_root() {
        purge_old_days_at(&root, &today());
    }
}

// ───────────── 可测版本（显式 root + date） ─────────────

/// 空序列（盘前/栅格外）不写：它对"恢复曲线"没有价值，只占目录。
pub fn save_series_at(root: &Path, date: &str, series: &MinuteSeries) {
    if series.points.is_empty() {
        return;
    }
    let dir = root.join(date);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let name = safe_name(&series.code);
    let path = dir.join(format!("{name}.json"));
    match serde_json::to_string(series) {
        Ok(json) => {
            // tmp+rename：中途崩溃不留下半截 JSON
            let tmp = dir.join(format!("{name}.tmp"));
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        Err(e) => log::debug!("[分时缓存] {} 序列化失败: {e}", series.code),
    }
}

/// 损坏的缓存文件按"无缓存"处理（记 warn，不崩）。
pub fn load_series_at(root: &Path, date: &str, code: &str) -> Option<MinuteSeries> {
    let text =
        std::fs::read_to_string(root.join(date).join(format!("{}.json", safe_name(code)))).ok()?;
    match serde_json::from_str(&text) {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("[分时缓存] {code} 缓存损坏，按无缓存处理: {e}");
            None
        }
    }
}

/// 单个文件损坏不影响其余。
pub fn load_all_at(root: &Path, date: &str) -> Vec<MinuteSeries> {
    let Ok(rd) = std::fs::read_dir(root.join(date)) else {
        return Vec::new();
    };
    rd.filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x == "json")
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            match serde_json::from_str(&text) {
                Ok(s) => Some(s),
                Err(err) => {
                    log::warn!("[分时缓存] {} 损坏，跳过: {err}", e.path().display());
                    None
                }
            }
        })
        .collect()
}

/// 目录不存在不算错（首次运行/刚好跨天都会这样）。
pub fn purge_old_days_at(root: &Path, today: &str) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().to_string();
        if name != today {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}
