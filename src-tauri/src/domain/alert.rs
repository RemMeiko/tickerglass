//! 到价提醒的滞回状态机。纯函数：不读时钟、不发网络、不碰 tauri，可离线单测。
//!
//! # 判定规则
//!
//! - 两种触发源：**涨跌幅阈值**（相对昨收的百分比）与**逐股目标价**
//! - 越过上涨阈值或止盈价 → 上涨提醒；跌破下跌阈值或止跌价 → 下跌提醒
//! - **每次越界只提醒一次**；价格回到阈值内侧后才重新布防
//!   （例：上涨阈值 `+3.0%` → 首次达到时提醒 → 回落到 `+2.85%`
//!   以下后重新布防 → 再次达到 `+3.0%` 才再提醒。即内侧 5% 缓冲）
//! - 冷却期：同一规则在冷却期内不重复提醒，防止在阈值边缘反复触发
//! - **非交易时段不提醒**（否则半夜会被叫醒）
//! - 数据缺失/过期的标的不参与判定（拿不到价格不等于没越界）

use serde::{Deserialize, Serialize};

/// 滞回缓冲比例：重新布防线 = 阈值 × (1 − 该值)。
///
/// 取 5% 是为对齐参考项目的示例（阈值 3.0% → 布防线 2.85%）。
/// 太小会在阈值边缘反复触发，太大会让第二次真实越界被吞掉。
pub const REARM_RATIO: f64 = 0.05;

/// 提醒方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// 小牛：涨到阈值/目标价之上
    Bull,
    /// 小熊：跌到阈值/目标价之下
    Bear,
}

/// 触发依据，用于提醒卡片说明"为什么弹出来"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Trigger {
    /// 涨跌幅越过阈值
    Percent,
    /// 最新价越过逐股目标价
    TargetPrice,
}

/// 单只标的的提醒规则。两种模式可同时配置，各自独立判定。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlertRule {
    /// 上涨阈值（百分比，如 `3.0` 表示 +3.0%）。`None` = 不启用
    pub up_percent: Option<f64>,
    /// 下跌阈值（**正数**表示跌幅，如 `3.0` 表示 −3.0%）。`None` = 不启用
    pub down_percent: Option<f64>,
    /// 小牛目标价。`None` = 不启用
    pub bull_price: Option<f64>,
    /// 小熊目标价。`None` = 不启用
    pub bear_price: Option<f64>,
    /// 基准价（v6 新增）。**不参与任何判定**，只是前端算目标价的输入：
    /// 填基准价 + 涨跌幅 → 自动算出 `bull_price`/`bear_price`。
    ///
    /// 为什么存进配置而不是只留在前端内存：不存的话用户下次打开设置页
    /// 只看到一个目标价数字，看不出它是从哪个基准算出来的，想微调只能重算。
    ///
    /// 为什么不参与判定：`up_percent` 的既有语义是「当日涨跌幅（相对昨收）」，
    /// 是一个**独立触发条件**。若让它改成相对基准价，同一个字段就背了两种语义，
    /// 且会静默改变所有已配置规则的行为。真正的触发仍只看目标价与涨跌幅。
    pub base_price: Option<f64>,
}

impl AlertRule {
    /// 五项全空 = 这条规则没有任何信息，可以从配置里删掉。
    ///
    /// **`base_price` 必须计入**：它虽不参与判定，但 `is_empty()` 同时是
    /// `sanitize()` 的 prune 判据和 `set_alert_rule` 的删除判据（见那两处）。
    /// 不计入的话，用户只填基准价还没来得及填涨跌幅，这条就会被当空壳删掉 ——
    /// 下次打开设置页发现自己填的数没了。
    ///
    /// 代价是「只有基准价」的规则会进 `evaluate_symbol`，但那里两个分支都是
    /// `let Some(thr) = thr else { continue }`，没阈值就不会触发，只是空跑一趟。
    pub fn is_empty(&self) -> bool {
        self.up_percent.is_none()
            && self.down_percent.is_none()
            && self.bull_price.is_none()
            && self.bear_price.is_none()
            && self.base_price.is_none()
    }
}

/// 一条规则的布防状态。`armed=true` 表示"已布防，可以触发"。
///
/// 每个 (标的, 方向, 触发源) 组合各有一份状态 —— 涨跌幅与目标价互不干扰。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ArmState {
    pub armed: bool,
    /// 上次触发的时刻（毫秒时间戳）。`None` = 从未触发
    pub last_fired_ms: Option<u64>,
}

impl Default for ArmState {
    fn default() -> Self {
        // 初始已布防：启动后价格若已在阈值外，应当立刻提醒一次
        ArmState {
            armed: true,
            last_fired_ms: None,
        }
    }
}

/// 判定所需的输入。由调用方从快照与配置组装，本模块不自行读取任何全局状态。
#[derive(Debug, Clone, Copy)]
pub struct AlertInput<'a> {
    pub code: &'a str,
    /// 最新价。`None` = 数据未到达或已失效 → 不参与判定
    pub price: Option<f64>,
    /// 相对昨收的涨跌幅（百分比）。`None` = 无法计算 → 百分比规则不参与判定
    pub percent: Option<f64>,
    /// 该标的数据是否新鲜。过期数据不参与判定（否则断网期间会因旧值持续越界而反复提醒）
    pub fresh: bool,
}

/// 一次触发。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlertEvent {
    pub code: String,
    pub side: Side,
    pub trigger: Trigger,
    /// 触发时的价格
    pub price: f64,
    /// 触发时的涨跌幅（目标价触发时也带上，卡片要显示）
    pub percent: Option<f64>,
    /// 被越过的阈值（百分比模式是百分比，目标价模式是价格）
    pub threshold: f64,
}

/// 一条规则的标识：用于在状态表里定位 `ArmState`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuleKey {
    pub side: Side,
    pub trigger: Trigger,
}

/// 判定单条规则：给定当前值、阈值、方向与旧状态，返回 (是否触发, 新状态)。
///
/// 这是整个提醒机制的核心不变量所在：
/// - `armed` 且越界 → 触发，转为 `!armed`
/// - `!armed` 且回到布防线内侧 → 重新布防（不触发）
/// - 冷却期内即使 `armed` 且越界也不触发（但仍会转为 `!armed`，避免冷却期结束后立刻补一发）
pub fn evaluate_one(
    value: f64,
    threshold: f64,
    side: Side,
    state: ArmState,
    now_ms: u64,
    cooldown_ms: u64,
) -> (bool, ArmState) {
    let crossed = match side {
        Side::Bull => value >= threshold,
        Side::Bear => value <= threshold,
    };

    // 布防线：阈值的内侧 REARM_RATIO。
    // 注意阈值可能为负（如下跌阈值 −3.0% 或做空目标价），故用"朝零方向收缩"而非固定加减。
    let rearm = threshold * (1.0 - REARM_RATIO);
    let back_inside = match side {
        Side::Bull => value < rearm,
        Side::Bear => value > rearm,
    };

    if state.armed {
        if !crossed {
            // 未越界，保持布防
            return (false, state);
        }
        // 越界了：冷却期内不响，但仍解除布防（否则冷却期一过会立刻补发一次）
        let in_cooldown = state
            .last_fired_ms
            .is_some_and(|t| now_ms.saturating_sub(t) < cooldown_ms);
        if in_cooldown {
            return (
                false,
                ArmState {
                    armed: false,
                    last_fired_ms: state.last_fired_ms,
                },
            );
        }
        (
            true,
            ArmState {
                armed: false,
                last_fired_ms: Some(now_ms),
            },
        )
    } else {
        // 未布防：只有回到布防线内侧才重新布防
        if back_inside {
            (
                false,
                ArmState {
                    armed: true,
                    last_fired_ms: state.last_fired_ms,
                },
            )
        } else {
            (false, state)
        }
    }
}

/// 判定一只标的的全部规则。
///
/// `states` 是该标的四条规则（涨幅/跌幅/牛价/熊价）的旧状态，按 `RuleKey` 取。
/// 返回触发的事件与更新后的状态；未配置的规则不产生状态变化。
///
/// `in_session` 由调用方传入（交易时段判定属 domain 层其他模块的职责）。
/// 非交易时段**只更新布防状态、不产生事件** —— 这样收盘后价格回落能正常重新布防，
/// 次日开盘不会因为"昨天遗留的未布防状态"而漏掉第一次越界。
#[allow(clippy::too_many_arguments)]
pub fn evaluate_symbol(
    input: AlertInput<'_>,
    rule: &AlertRule,
    states: &mut dyn FnMut(RuleKey) -> ArmState,
    set_state: &mut dyn FnMut(RuleKey, ArmState),
    now_ms: u64,
    cooldown_ms: u64,
    in_session: bool,
) -> Vec<AlertEvent> {
    let mut out = Vec::new();
    if rule.is_empty() || !input.fresh {
        return out;
    }

    // 百分比规则
    if let Some(pct) = input.percent {
        for (side, thr) in [
            (Side::Bull, rule.up_percent),
            // 配置里下跌阈值存正数（用户填 3.0 表示 −3.0%），判定时取负
            (Side::Bear, rule.down_percent.map(|v| -v.abs())),
        ] {
            let Some(thr) = thr else { continue };
            let key = RuleKey {
                side,
                trigger: Trigger::Percent,
            };
            let (fired, next) = evaluate_one(pct, thr, side, states(key), now_ms, cooldown_ms);
            set_state(key, next);
            if fired && in_session {
                if let Some(price) = input.price {
                    out.push(AlertEvent {
                        code: input.code.to_string(),
                        side,
                        trigger: Trigger::Percent,
                        price,
                        percent: Some(pct),
                        threshold: thr,
                    });
                }
            }
        }
    }

    // 目标价规则
    if let Some(price) = input.price {
        for (side, thr) in [
            (Side::Bull, rule.bull_price),
            (Side::Bear, rule.bear_price),
        ] {
            let Some(thr) = thr else { continue };
            if thr <= 0.0 {
                continue;
            }
            let key = RuleKey {
                side,
                trigger: Trigger::TargetPrice,
            };
            let (fired, next) = evaluate_one(price, thr, side, states(key), now_ms, cooldown_ms);
            set_state(key, next);
            if fired && in_session {
                out.push(AlertEvent {
                    code: input.code.to_string(),
                    side,
                    trigger: Trigger::TargetPrice,
                    price,
                    percent: input.percent,
                    threshold: thr,
                });
            }
        }
    }

    out
}
