// 配置迁移。**纯函数层**：只做 文本 → 配置 的转换，不碰文件系统。
//
// 迁移链设计为逐版本递进（v1 → v2 → …），每步一个小函数，便于后续 Sprint 追加。

use super::schema::{AppConfig, SCHEMA_VERSION};
use serde_json::Value;

/// 加载配置的结果。**区分"迁移"与"损坏"** ——
/// 迁移不该备份 `.bak`（标准 20 明确要求不生成），损坏才备份（标准 19）。
#[derive(Debug, Clone, PartialEq)]
pub struct MigrationResult {
    pub config: AppConfig,
    pub outcome: LoadOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoadOutcome {
    /// 版本号正好是当前版本，未做任何改动
    Current,
    /// 从旧版本迁移而来（保留了可识别字段）
    Migrated { from: u32 },
    /// 版本号比当前程序新：**保守处理**（保留能识别的字段 + 补默认），
    /// 不当损坏清空 —— 用户装回旧版本不该丢自选股
    FutureVersion { found: u32 },
    /// 文件损坏（非法 JSON / 根节点不是对象 / 空文件）→ 用默认配置，调用方须备份原文件
    Corrupted { reason: String },
    /// 主文件损坏但备份救回来了（v0.3.0 备份机制）。
    ///
    /// `reason` 是主文件的损坏原因（供日志）。这个状态下配置**已经可用**，
    /// 调用方（store）负责把损坏原文另存 `.corrupt` 并把恢复结果写回主文件。
    /// 与 `Corrupted` 的本质区别：**用户数据没丢**，提示文案也因此不同。
    RecoveredFromBackup { reason: String },
}

impl LoadOutcome {
    /// 是否需要把用户原文件另存（`.corrupt`）。损坏与备份恢复都要留原文供排查。
    pub fn needs_backup(&self) -> bool {
        matches!(
            self,
            LoadOutcome::Corrupted { .. } | LoadOutcome::RecoveredFromBackup { .. }
        )
    }

    /// 是否需要把净化后的配置写回磁盘（迁移与损坏都要落盘固化）。
    pub fn needs_rewrite(&self) -> bool {
        !matches!(self, LoadOutcome::Current)
    }

    /// 给用户看的提示文案。`None` = 无需提示。
    pub fn user_message(&self) -> Option<String> {
        match self {
            LoadOutcome::Current => None,
            LoadOutcome::Migrated { from } => {
                Some(format!("配置已从旧版本（v{from}）升级，自选股与设置均已保留"))
            }
            LoadOutcome::FutureVersion { found } => Some(format!(
                "配置版本（v{found}）比当前程序新，已按兼容方式加载；\
                 部分新版设置可能不生效"
            )),
            LoadOutcome::Corrupted { .. } => Some(
                "配置文件损坏且备份不可用，已恢复默认设置（损坏原文保留为 config.json.corrupt）"
                    .to_string(),
            ),
            LoadOutcome::RecoveredFromBackup { .. } => Some(
                // v0.4.0 起不再提 .corrupt：主文件**丢失**（非损坏）的恢复路径
                // 没有原文可留，文案统一说"已从备份恢复"即可
                "配置文件异常，已自动从备份恢复，自选股与设置均已保留".to_string(),
            ),
        }
    }
}

/// 文本 → 配置。这是配置加载的**唯一**解析入口，`store` 只负责 I/O。
///
/// 四种损坏形态都走同一条降级路径（标准 19 的追加要求）：
/// 非法 JSON、空文件（0 字节）、根节点是数组 `[]`、根节点是标量。
pub fn migrate(text: &str) -> MigrationResult {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return corrupted("配置文件为空");
    }

    let root: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => return corrupted(&format!("非法 JSON：{e}")),
    };

    // 根节点必须是对象。数组 `[]` 与标量都算损坏
    let Some(obj) = root.as_object() else {
        return corrupted(&format!(
            "配置根节点应为对象，实际是 {}",
            json_kind(&root)
        ));
    };

    // 版本号缺失时按 v1 处理：Sprint 1 时代的文件没有 schemaVersion 字段
    let found = obj
        .get("schemaVersion")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(1);

    let mut value = root.clone();
    let outcome = if found > SCHEMA_VERSION {
        LoadOutcome::FutureVersion { found }
    } else if found < SCHEMA_VERSION {
        apply_chain(&mut value, found);
        LoadOutcome::Migrated { from: found }
    } else {
        LoadOutcome::Current
    };

    // 缺字段补默认由 serde 的 `#[serde(default)]` 承担；
    // 反序列化失败（字段类型完全错位）时降级为损坏，不 panic
    let mut config: AppConfig = match serde_json::from_value(value) {
        Ok(c) => c,
        Err(e) => return corrupted(&format!("字段类型不符：{e}")),
    };
    config.schema_version = SCHEMA_VERSION;
    config.sanitize();

    MigrationResult { config, outcome }
}

fn corrupted(reason: &str) -> MigrationResult {
    MigrationResult {
        config: AppConfig::default(),
        outcome: LoadOutcome::Corrupted {
            reason: reason.to_string(),
        },
    }
}

/// 逐版本递进地跑迁移链。后续 Sprint 追加新版本时在这里加一条 arm。
fn apply_chain(value: &mut Value, from: u32) {
    let mut v = from;
    while v < SCHEMA_VERSION {
        match v {
            1 => migrate_v1_to_v2(value),
            2 => migrate_v2_to_v3(value),
            3 => migrate_v3_to_v4(value),
            4 => migrate_v4_to_v5(value),
            5 => migrate_v5_to_v6(value),
            // 未知的中间版本不做结构改动，靠 serde default 补字段
            _ => {}
        }
        v += 1;
    }
}

/// v1 → v2：新增 `appearance` 与 `window` 两个字段。
///
/// v1 是 Sprint 1 时代的隐式结构（无外观/窗口配置）。迁移动作是
/// **补全新增字段的默认值并原样保留 `watchlist`** —— 自选股一条都不能丢。
fn migrate_v1_to_v2(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    // 只在缺失时补，已有值不覆盖（用户手改过 appearance 就该尊重）
    if !obj.contains_key("appearance") {
        obj.insert("appearance".to_string(), serde_json::json!({}));
    }
    if !obj.contains_key("window") {
        obj.insert("window".to_string(), serde_json::json!({}));
    }
    // 只能标 2，不能标 SCHEMA_VERSION —— 这一步只做完了 v2 的活。
    // 标成"当前版本"会让这个 Value 谎称已是最新结构；今天之所以没出事，
    // 是因为 apply_chain 用自己的计数器驱动循环、且末尾会统一覆盖该字段，
    // 但下一个加 v4 的人若改用 Value 里的版本号判断就会踩空。
    obj.insert("schemaVersion".to_string(), Value::Number(2.into()));
}

/// v5 → v6：图表配置（`charts.ma_periods`）+ 悬浮窗资金流开关
/// （`appearance.show_fund_flow`）。
///
/// 两个字段都有 serde default（`[5,20]` / `true`），结构上不补也能加载 ——
/// 这里显式补是为了让写回磁盘的文件**字段齐全**，用户手改配置时能看到
/// 这两个可调项的存在（与 v4→v5 补 `hidden` 同一理由）。
/// v5 用户升级后的观感不变：均线还是 MA5/20，资金流默认显示是 v0.3.0
/// 的新功能而非既有行为的改变。
fn migrate_v5_to_v6(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if !obj.contains_key("charts") {
        obj.insert(
            "charts".to_string(),
            serde_json::json!({ "ma_periods": [5, 20] }),
        );
    }
    let appearance = obj
        .entry("appearance".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if let Some(a) = appearance.as_object_mut() {
        if !a.contains_key("show_fund_flow") {
            a.insert("show_fund_flow".to_string(), Value::Bool(true));
        }
    }
}

fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// v2 → v3：新增 `alerts`（牛熊提醒）字段。
///
/// 只补一个空对象，其余交给 `AlertConfig` 的 serde default —— 关键是
/// **默认 `enabled: false`**：用户从 v2 升上来时不该突然开始弹提醒卡片。
/// 自选股、外观、窗口配置一律原样保留。
fn migrate_v2_to_v3(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    // 只在缺失时补；用户若已手写过 alerts 就尊重其内容
    if !obj.contains_key("alerts") {
        obj.insert("alerts".to_string(), serde_json::json!({}));
    }
}

/// v3 → v4：提示音从"固定音效开关"变成"可选音色 + 音量"。
///
/// 新增 `alerts.bull_tone` / `bear_tone` / `sound_volume`。
///
/// # 为什么这里必须显式补字段，不能只靠 serde default
///
/// 光靠 serde default 确实也能填上 —— 但**只在 `alerts` 对象存在时**。
/// 而这一步的真正职责是保证语义连续：v3 用户若把 `bull_sound` 关掉了，
/// 升级后不该因为多了音色字段就重新响起来。`bull_sound` 原样保留即可满足，
/// 故这里只补音色/音量，**绝不碰两个开关**。
///
/// 另外把 `alerts` 缺失的情况也补上：v3 配置理论上一定有 alerts
/// （v2→v3 补过），但用户手删过的文件也要能升上来。
/// v4 → v5：自选股新增 `hidden` 字段（可隐藏但不删除）。
///
/// # 为什么这一步几乎不做事
///
/// `WatchItem.hidden` 带 `#[serde(default)]`，v4 的条目读进来自然是 `false`
/// （= 可见），语义正确。所以**结构上无需改动**。
///
/// 那为什么还要升版本号 / 留这个函数：
/// 1. **降级可检测**：v5 的文件被旧版本打开时，`found > SCHEMA_VERSION` 分支
///    会走保守处理而不是默默丢掉 `hidden`。不升版本号就没有这个保护。
/// 2. **给后来人一个挂点**：下一个动 watchlist 结构的人会先看这条链，
///    看到 v5 存在就知道该在哪加自己的那一步。
///
/// 这里显式补 `hidden: false` 而不是纯靠 serde default —— 让写回磁盘的文件
/// 字段齐全，用户手动查看配置时能看到这个可调项的存在。
fn migrate_v4_to_v5(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let Some(list) = obj.get_mut("watchlist").and_then(|w| w.as_array_mut()) else {
        // 没有 watchlist 或它不是数组：交给后续的 serde 反序列化兜底
        return;
    };
    for item in list.iter_mut() {
        let Some(o) = item.as_object_mut() else { continue };
        // 只在缺失时补：用户手写过 hidden:true 就尊重它，不要重置成可见
        if !o.contains_key("hidden") {
            o.insert("hidden".to_string(), Value::Bool(false));
        }
    }
}

fn migrate_v3_to_v4(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let alerts = obj
        .entry("alerts".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let Some(a) = alerts.as_object_mut() else {
        // alerts 存在但不是对象（用户手改坏了）→ 整个替换成默认空对象，
        // 让 serde default 兜底，而不是让整份配置被判为损坏
        *alerts = serde_json::json!({});
        return;
    };
    // 只在缺失时补，已有值不覆盖
    if !a.contains_key("bull_tone") {
        a.insert("bull_tone".to_string(), Value::String("chime".into()));
    }
    if !a.contains_key("bear_tone") {
        a.insert("bear_tone".to_string(), Value::String("chime".into()));
    }
    if !a.contains_key("sound_volume") {
        a.insert(
            "sound_volume".to_string(),
            serde_json::json!(0.7),
        );
    }
}
