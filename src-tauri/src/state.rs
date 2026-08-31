use crate::config::AppConfig;
use crate::domain::alert::{ArmState, RuleKey};
use crate::model::Snapshot;
use std::collections::HashMap;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// 内存快照。整体覆盖写，故锁中毒后的数据仍是"上一次完整快照"或"即将被本次覆盖"，
/// 继续使用是安全的 —— 比让轮询线程 panic 退出（表现为界面永久冻结在旧值且无任何提示）
/// 好得多。因此下面两个方法都不用裸 `.unwrap()`。
///
/// Sprint 2A 新增配置快照（同样的 poison-safe 模式）。配置成为**唯一数据源**（D1）：
/// `scheduler` 不再读 `watchlist.rs` 常量，改读这里。
pub struct AppState {
    snapshot: RwLock<Snapshot>,
    config: RwLock<AppConfig>,
    /// 配置损坏/迁移后给用户的一次性提示（标准 19 的可见提示条）。
    /// `None` = 无需提示。
    config_notice: RwLock<Option<String>>,
    /// 提醒布防状态：`(tencent_code, RuleKey) -> ArmState`。
    ///
    /// **只存内存不落盘**：布防状态是"当前是否已越界过"的运行时判断，
    /// 重启后应当重新布防（否则用户重启程序反而漏掉提醒）。
    arm_states: RwLock<HashMap<(String, RuleKey), ArmState>>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            snapshot: RwLock::new(Snapshot::empty()),
            config: RwLock::new(AppConfig::default()),
            config_notice: RwLock::new(None),
            arm_states: RwLock::new(HashMap::new()),
        }
    }

    /// 取一条规则的布防状态；未记录过则视为「已布防」（启动后首次越界应当提醒）。
    pub fn arm_state(&self, code: &str, key: RuleKey) -> ArmState {
        match self.arm_states.read() {
            Ok(g) => g.get(&(code.to_string(), key)).copied().unwrap_or_default(),
            // 锁中毒时按默认（已布防）处理，宁可多提醒一次也不要让判定线程 panic
            Err(e) => e
                .into_inner()
                .get(&(code.to_string(), key))
                .copied()
                .unwrap_or_default(),
        }
    }

    pub fn set_arm_state(&self, code: &str, key: RuleKey, st: ArmState) {
        let mut g = match self.arm_states.write() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        g.insert((code.to_string(), key), st);
    }

    /// 规则被改动时清掉该标的的布防状态 → 立即重新布防。
    ///
    /// 必要性：用户把阈值从 +3% 调到 +5% 时，旧的「未布防」状态会吞掉下一次越界，
    /// 表现为"改了阈值反而不提醒了"。
    pub fn clear_arm_states_for(&self, code: &str) {
        let mut g = match self.arm_states.write() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        g.retain(|(c, _), _| c != code);
    }

    /// 自选股被删或规则被清空时，顺手清掉其布防状态，避免 HashMap 无限增长。
    pub fn retain_arm_states(&self, keep: &[String]) {
        let mut g = match self.arm_states.write() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        g.retain(|(code, _), _| keep.iter().any(|k| k == code));
    }

    pub fn read(&self) -> RwLockReadGuard<'_, Snapshot> {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, Snapshot> {
        self.snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn get(&self) -> Snapshot {
        self.read().clone()
    }

    pub fn set(&self, next: Snapshot) {
        *self.write() = next;
    }

    // ---------- 配置快照（同样 poison-safe，不用裸 unwrap）----------

    pub fn config(&self) -> AppConfig {
        self.config
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn set_config(&self, next: AppConfig) {
        *self.config.write().unwrap_or_else(|p| p.into_inner()) = next;
    }

    /// 在写锁内就地修改配置，避免"读出→改→写回"之间被另一个请求插队覆盖
    /// （标准 23 的快速连点 5 次场景就靠这个串行化）。
    pub fn update_config<R>(&self, f: impl FnOnce(&mut AppConfig) -> R) -> R {
        let mut guard = self.config.write().unwrap_or_else(|p| p.into_inner());
        f(&mut guard)
    }

    pub fn set_config_notice(&self, msg: Option<String>) {
        *self
            .config_notice
            .write()
            .unwrap_or_else(|p| p.into_inner()) = msg;
    }

    pub fn config_notice(&self) -> Option<String> {
        self.config_notice
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
