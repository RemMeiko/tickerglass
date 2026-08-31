//! 大盘面板报文解析：涨跌家数 + 资金流向五档 + 三大指数。
//!
//! 数据源 `push2.eastmoney.com`，全部字段含义均为 2026-08-27 盘中**实测认定**，
//! 不是查文档或凭记忆。认定依据见 `harness/survey-market-panel.md`，摘要如下。
//!
//! # 涨跌家数：`f113` 涨 / `f114` 跌 / `f115` 平
//!
//! 认定依据一（同一 secid 两次采样，分量变而和恒定）：
//!
//! | 时刻 | f113 | f114 | f115 | 和 |
//! |---|---|---|---|---|
//! | 10:33 | 1016 | 1272 | 67 | **2355** |
//! | 10:5x | 1098 | 1207 | 50 | **2355** |
//!
//! 和恒定而分量浮动 = 对固定总体的划分，这正是"涨/跌/平家数"的特征；
//! 且 2355 与沪市规模吻合（约 1700 主板 + 580 科创）。
//!
//! 认定依据二（口径是**整个市场**而非指数成分股）：
//! `0.899050` 北证50 返回 `136/190/12`，和为 **338**。
//! 北证50 的成分股固定 **50** 只 —— 若字段统计的是成分股，和必须是 50。
//! 是 338 而不是 50，故口径为该市场全体标的。
//!
//! ⚠️ 反例记录：`f104/f105/f106` **不是**家数。按记忆试过，
//! 在指数与板块 secid 上一律返回 `0/0/100`。别再去试它。
//!
//! # 资金流向：`f135`~`f149`，每档三个（流入/流出/净额）
//!
//! | 档位 | 流入 | 流出 | 净额 |
//! |---|---|---|---|
//! | 总 | f135 | f136 | f137 |
//! | 超大单 | f138 | f139 | f140 |
//! | 大单 | f141 | f142 | f143 |
//! | 中单 | f144 | f145 | f146 |
//! | 小单 | f147 | f148 | f149 |
//!
//! 三组独立恒等式在沪深两市**全部成立**（故字段含义不是猜的）：
//! 1. 每档 `流入 − 流出 = 净额`（5/5 档）
//! 2. `超大单净 + 大单净 = 总净额`（沪 +98.13−20.85=+77.27 ✓）
//! 3. 四档净额之和 ≈ 0（每笔买必有对应的卖；实测 −0.01）
//!
//! 恒等式 2 顺带定义了「主力」= 超大单 + 大单，与东财口径一致。
//!
//! # 缩放
//!
//! `f43` 现价 ×100、`f3` 涨跌幅 ×100、`f4` 涨跌额 ×100；
//! `f6` 成交额与 `f135`~`f149` 都是**元**（不缩放）；家数不缩放。

use serde::Serialize;

/// 解析失败的原因。**限流必须与其他错误分开** ——
/// 见 [`MarketParseError::RateLimited`] 的说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketParseError {
    /// 响应体为空。
    ///
    /// 这**不是**"没有数据"，而是 `push2` 的静默限流特征：
    /// 无 HTTP 错误码、无错误 JSON、无 `rc` 字段，恢复约 3 分钟。
    /// 单独成一种错误，是为了让 UI 能说"请求过于频繁，稍后重试"
    /// 而不是显示"涨 0 家 跌 0 家"——后者是彻底的假数据。
    RateLimited,
    /// 不是 JSON（HTML 错误页等）
    NotJson,
    /// 业务码非 0（`rc != 0`）
    Rejected(i64),
    /// 结构不符：缺 `data`、或家数字段全缺
    Structure(String),
}

impl std::fmt::Display for MarketParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarketParseError::RateLimited => write!(
                f,
                "东财返回空响应（请求过于频繁被限流，约 3 分钟后恢复）"
            ),
            MarketParseError::NotJson => write!(f, "响应不是 JSON"),
            MarketParseError::Rejected(rc) => write!(f, "东财拒绝请求 rc={rc}"),
            MarketParseError::Structure(s) => write!(f, "报文结构不符: {s}"),
        }
    }
}

/// 一个市场的涨跌家数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Breadth {
    pub up: u32,
    pub down: u32,
    pub flat: u32,
}

impl Breadth {
    pub fn total(&self) -> u32 {
        self.up + self.down + self.flat
    }
}

/// 一档资金流向（单位：元）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct FlowTier {
    pub inflow: f64,
    pub outflow: f64,
    pub net: f64,
}

/// 资金流向五档 + 主力口径。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct MoneyFlow {
    pub total: FlowTier,
    pub extra_large: FlowTier,
    pub large: FlowTier,
    pub medium: FlowTier,
    pub small: FlowTier,
}

impl MoneyFlow {
    /// 主力净流入 = 超大单净 + 大单净（东财口径，实测等于 `f137` 总净额）。
    ///
    /// 不直接返 `total.net` 而是重算：两者相等是实测结论，
    /// 显式相加能让"某天东财改了口径"这件事在数据上暴露出来，
    /// 而不是静默显示一个含义变了的数。
    pub fn main_force_net(&self) -> f64 {
        self.extra_large.net + self.large.net
    }
}

/// 单个市场的完整快照。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarketStats {
    /// 市场名（报文 `f58`，如「上证指数」）
    pub name: String,
    pub breadth: Breadth,
    pub flow: MoneyFlow,
}

// ───────────── 取值基元 ─────────────

/// 取整数字段。缺失/非数字返回 `None` —— **绝不 `unwrap_or(0)`**：
/// 家数为 0 与"没取到家数"在界面上必须是两种表现。
fn u32_of(data: &serde_json::Value, key: &str) -> Option<u32> {
    let v = data.get(key)?;
    // 东财家数字段是整数，但 serde_json 可能给成 f64
    if let Some(i) = v.as_i64() {
        return u32::try_from(i).ok();
    }
    let f = v.as_f64()?;
    if f.is_finite() && f >= 0.0 {
        Some(f as u32)
    } else {
        None
    }
}

/// 取浮点字段（资金流向，单位元）。
fn f64_of(data: &serde_json::Value, key: &str) -> Option<f64> {
    let v = data.get(key)?;
    if let Some(f) = v.as_f64() {
        return f.is_finite().then_some(f);
    }
    // 偶发字符串形态（如 "-"）一律视为缺失
    v.as_str()?.parse::<f64>().ok().filter(|x| x.is_finite())
}

fn tier(data: &serde_json::Value, fi: &str, fo: &str, fnet: &str) -> Option<FlowTier> {
    Some(FlowTier {
        inflow: f64_of(data, fi)?,
        outflow: f64_of(data, fo)?,
        net: f64_of(data, fnet)?,
    })
}

/// 解析响应的公共前处理：限流 → JSON → `rc` → 取出 `data`。
fn data_of(text: &str) -> Result<serde_json::Value, MarketParseError> {
    // 空响应是限流特征，必须最先判且单独成错 —— 见 MarketParseError::RateLimited
    if text.trim().is_empty() {
        return Err(MarketParseError::RateLimited);
    }
    let root: serde_json::Value =
        serde_json::from_str(text).map_err(|_| MarketParseError::NotJson)?;
    // rc 缺失时不当失败（部分接口不带），只在明确非 0 时拒绝
    if let Some(rc) = root.get("rc").and_then(|v| v.as_i64()) {
        if rc != 0 {
            return Err(MarketParseError::Rejected(rc));
        }
    }
    let data = root
        .get("data")
        .ok_or_else(|| MarketParseError::Structure("缺 data".into()))?;
    if data.is_null() {
        return Err(MarketParseError::Structure("data 为 null".into()));
    }
    Ok(data.clone())
}

/// 解析单市场快照（`stock/get?secid=1.000001&fields=...`）。
pub fn parse_market_stats(text: &str) -> Result<MarketStats, MarketParseError> {
    let data = data_of(text)?;

    let up = u32_of(&data, "f113");
    let down = u32_of(&data, "f114");
    let flat = u32_of(&data, "f115");
    let (Some(up), Some(down), Some(flat)) = (up, down, flat) else {
        return Err(MarketParseError::Structure(format!(
            "家数字段缺失 f113={up:?} f114={down:?} f115={flat:?}"
        )));
    };
    // 三者全 0 是"该 secid 不带家数"的表现（如 f104/f105/f106 那种占位），
    // 而不是真的全市场一只都没涨跌。盘中不可能出现，故按结构不符处理。
    if up == 0 && down == 0 && flat == 0 {
        return Err(MarketParseError::Structure(
            "家数三项全为 0（该 secid 不提供家数）".into(),
        ));
    }

    let flow = MoneyFlow {
        total: tier(&data, "f135", "f136", "f137").ok_or_else(|| {
            MarketParseError::Structure("资金流向总档字段缺失".into())
        })?,
        extra_large: tier(&data, "f138", "f139", "f140").ok_or_else(|| {
            MarketParseError::Structure("超大单档字段缺失".into())
        })?,
        large: tier(&data, "f141", "f142", "f143")
            .ok_or_else(|| MarketParseError::Structure("大单档字段缺失".into()))?,
        medium: tier(&data, "f144", "f145", "f146")
            .ok_or_else(|| MarketParseError::Structure("中单档字段缺失".into()))?,
        small: tier(&data, "f147", "f148", "f149")
            .ok_or_else(|| MarketParseError::Structure("小单档字段缺失".into()))?,
    };

    Ok(MarketStats {
        name: data
            .get("f58")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        breadth: Breadth { up, down, flat },
        flow,
    })
}

/// 一个指数的快照。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IndexQuote {
    pub code: String,
    pub name: String,
    /// 现价（已除 100）
    pub price: f64,
    /// 涨跌幅 %（已除 100）
    pub change_percent: f64,
    /// 涨跌额（已除 100）
    pub change_amount: f64,
    /// 成交额（元，不缩放）
    pub amount: Option<f64>,
}

/// 三个市场的合计家数。
///
/// 为什么要合计：用户要看的是"整个大盘涨跌多少"，而接口是按市场给的。
/// 合计在后端做 —— 前端不该关心 A 股由三个交易所构成。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BreadthTotal {
    pub up: u32,
    pub down: u32,
    pub flat: u32,
    /// 参与合计的市场数（1~3）。**少于 3 时前端必须标注"部分市场数据缺失"** ——
    /// 否则用户会把"沪市 + 深市"的家数当成全 A 股的，少算北交所约 300 只。
    pub markets_counted: u8,
}

impl BreadthTotal {
    pub fn total(&self) -> u32 {
        self.up + self.down + self.flat
    }

    /// 上涨占比（0~1）。总数为 0 时返回 `None`，不返 0.0 ——
    /// "没数据"和"一只都没涨"在界面上是两种表现。
    pub fn up_ratio(&self) -> Option<f64> {
        let t = self.total();
        (t > 0).then(|| f64::from(self.up) / f64::from(t))
    }
}

/// 把多个市场的家数相加。
pub fn sum_breadth(list: &[Breadth]) -> BreadthTotal {
    let mut out = BreadthTotal {
        up: 0,
        down: 0,
        flat: 0,
        markets_counted: 0,
    };
    for b in list {
        out.up += b.up;
        out.down += b.down;
        out.flat += b.flat;
        out.markets_counted = out.markets_counted.saturating_add(1);
    }
    out
}

/// 把多个市场的资金流向净额相加（单位：元）。
///
/// 只加净额不加流入流出：跨市场的"流入总额"没有解释意义，
/// 而净额可加（都是同一口径的净额）。
pub fn sum_main_force_net(list: &[MoneyFlow]) -> f64 {
    list.iter().map(|f| f.main_force_net()).sum()
}

/// 从 `ulist.np/get` 的多 secid 响应里解析各市场快照（按 `f12` 裸码索引）。
///
/// # 与 [`parse_market_stats`] 的关系
///
/// 两者取的是同一批字段，只是报文外层结构不同（`ulist.np` 多一层 `diff` 数组）。
/// 存在两条路的理由是**限流按路径独立计数**：`ulist.np` 一个请求能拿齐三个市场，
/// 而 `stock/get` 要打三次（详见 `net::fetch_em_markets_batch`）。
///
/// # 返回空 Vec 而不是报错的情形
///
/// `ulist.np` 是否携带 `f113/f114/f115` **尚未在真机验证**。若它不带，
/// 这里会返回空 `Vec`（而不是 `Err`），调用方据此**降级**到逐市场的
/// `stock/get`。做成"空"而不是"错"，是因为这不是故障 —— 是这条通道不提供该字段，
/// 属于预期内的分支。
pub fn parse_markets_batch(text: &str) -> Result<Vec<(String, MarketStats)>, MarketParseError> {
    let data = data_of(text)?;
    let diff = data
        .get("diff")
        .ok_or_else(|| MarketParseError::Structure("缺 data.diff".into()))?;

    let rows: Vec<&serde_json::Value> = match diff {
        serde_json::Value::Array(a) => a.iter().collect(),
        serde_json::Value::Object(m) => {
            let mut kv: Vec<(usize, &serde_json::Value)> = m
                .iter()
                .filter_map(|(k, v)| k.parse::<usize>().ok().map(|i| (i, v)))
                .collect();
            kv.sort_by_key(|(i, _)| *i);
            kv.into_iter().map(|(_, v)| v).collect()
        }
        _ => return Err(MarketParseError::Structure("diff 既非数组也非对象".into())),
    };

    let mut out = Vec::new();
    for r in rows {
        // 家数三项缺任一就跳过这行 —— 说明这条通道不带家数
        let (Some(up), Some(down), Some(flat)) =
            (u32_of(r, "f113"), u32_of(r, "f114"), u32_of(r, "f115"))
        else {
            continue;
        };
        // 全 0 同 parse_market_stats：是占位而非真实家数
        if up == 0 && down == 0 && flat == 0 {
            continue;
        }
        // 资金流向五档缺任一就跳过：半套数据会让"主力 = 超大 + 大"算出错的值
        let (Some(total), Some(extra_large), Some(large), Some(medium), Some(small)) = (
            tier(r, "f135", "f136", "f137"),
            tier(r, "f138", "f139", "f140"),
            tier(r, "f141", "f142", "f143"),
            tier(r, "f144", "f145", "f146"),
            tier(r, "f147", "f148", "f149"),
        ) else {
            continue;
        };
        let code = r
            .get("f12")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        out.push((
            code,
            MarketStats {
                name: r
                    .get("f14")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                breadth: Breadth { up, down, flat },
                flow: MoneyFlow {
                    total,
                    extra_large,
                    large,
                    medium,
                    small,
                },
            },
        ));
    }
    Ok(out)
}

/// 解析三大指数（`ulist.np/get`）。
///
/// `data.diff` 实测既可能是数组也可能是以 `"0"`/`"1"` 为键的对象
/// （同一接口不同参数下两种都出现过），故两种都要认。
pub fn parse_indices(text: &str) -> Result<Vec<IndexQuote>, MarketParseError> {
    let data = data_of(text)?;
    let diff = data
        .get("diff")
        .ok_or_else(|| MarketParseError::Structure("缺 data.diff".into()))?;

    let rows: Vec<&serde_json::Value> = match diff {
        serde_json::Value::Array(a) => a.iter().collect(),
        serde_json::Value::Object(m) => {
            // 键是 "0","1","2"… 必须按数值排序，否则 10 会排到 2 前面
            let mut kv: Vec<(usize, &serde_json::Value)> = m
                .iter()
                .filter_map(|(k, v)| k.parse::<usize>().ok().map(|i| (i, v)))
                .collect();
            kv.sort_by_key(|(i, _)| *i);
            kv.into_iter().map(|(_, v)| v).collect()
        }
        _ => return Err(MarketParseError::Structure("diff 既非数组也非对象".into())),
    };

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        // 现价拿不到就整条丢弃：宁可少一行，也不显示 0.00 的假指数
        let Some(price) = f64_of(r, "f2") else { continue };
        out.push(IndexQuote {
            code: r
                .get("f12")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            name: r
                .get("f14")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            price: price / 100.0,
            change_percent: f64_of(r, "f3").unwrap_or(0.0) / 100.0,
            change_amount: f64_of(r, "f4").unwrap_or(0.0) / 100.0,
            amount: f64_of(r, "f6"),
        });
    }
    if out.is_empty() {
        return Err(MarketParseError::Structure("无任何可用指数行".into()));
    }
    Ok(out)
}
