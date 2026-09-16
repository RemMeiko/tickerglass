use crate::domain::change::Change;
use crate::domain::market::Market;
use serde::{Deserialize, Serialize};

/// 自选股条目。带双标识（腾讯代码 + 东财 secid）。
///
/// # 身份键（Sprint 2A 明确规定）
///
/// **唯一键是 `tencent_code`**（`sh000001` / `sz000001` / `hk00700` / `usAAPL`），
/// 市场前缀已内嵌，故它与 `(market, 裸码)` 等价且唯一。
///
/// ⚠️ 东财裸码 `000001` **不唯一**：`market=0` 是平安银行(深A)、`market=1` 是上证指数。
/// 因此去重 / `remove` / `reorder` 一律按 `tencent_code`，
/// 误按裸码删会同时删掉 `sz000001` 与 `sh000001`。
///
/// # Sprint 2A 的两处破坏性改动
///
/// 1. `&'static str` → `String`：用户可管理列表要求运行时构造，`String` 无法 const
/// 2. `em_secid` → `Option<String>`：**不猜前缀**。美股 secid 前缀有 `105/106/107` 三种，
///    只有搜索接口的 `MktNum` 是权威来源；拿不到就是 `None`，绝不硬编码猜一个
///    （这是遗留问题 3 的正解）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchItem {
    pub tencent_code: String,
    /// 东财 secid（`1.000001` / `116.00700` / `105.AAPL`）。
    /// `None` = 该标的没有已知 secid，东财通道不可用（不猜、不编）
    pub em_secid: Option<String>,
    pub market: Market,
    pub is_index: bool,
    /// 是否在悬浮窗中隐藏。**隐藏 ≠ 删除**。
    ///
    /// 语义（用户 2026-08-28 明确选定）：只影响悬浮窗**显示**，
    /// 后台照常取数、照常判定提醒。所以隐藏的标的：
    /// - 不出现在悬浮窗里
    /// - 仍然出现在设置页（带「已隐藏」标记，可再取消隐藏）
    /// - 仍然会触发提醒卡片（用户可能就是想"不占地方但别漏掉提醒"）
    /// - 取消隐藏后立刻有数据，不需要等下一轮
    ///
    /// `#[serde(default)]` 在结构体级别已声明，故 v4 及更早的配置读进来是 `false`。
    #[serde(default)]
    pub hidden: bool,
}

/// 一条报价。`price` 是 `Option` —— 停牌/无效代码时是 `None` 而不是 0。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub code: String,
    pub name: String,
    /// 报文 `[2]` 字段，仅作展示与一致性校验，**永不用于与自选股匹配**
    pub raw_symbol: String,
    pub market: Market,
    pub price: Option<f64>,
    pub prev_close: Option<f64>,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub change: Change,
    /// 现价字符串的小数位数，用于按报文原样显示（港股 3 位不能被截成 2 位）
    pub price_decimals: Option<u8>,
    /// 报文时间戳原文（`[30]`），三种格式之一
    pub raw_timestamp: String,
    /// 报文时间戳解析结果（`HH:MM`），解析失败为 `None`
    pub quote_time: Option<String>,
    /// 当日累计成交额（元）。腾讯报文 `[38]` 成交额（万元）×10000。
    ///
    /// `None` = 该报文不带此字段（美股/部分港股形态）。大盘面板的指数卡
    /// 用它显示成交额；此前指数卡一直缺这一行（v0.3.0 需求 5 的一部分）。
    #[serde(default)]
    pub amount: Option<f64>,
}

/// 解析结果一等类型：解析不出来是一种明确状态，不靠 `unwrap_or(0.0)` 掩盖。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuoteParseOutcome {
    Ok(Quote),
    NoData { code: String },
    Malformed { raw_prefix: String },
}

impl QuoteParseOutcome {
    /// 该结果对应的身份键。`Malformed` 无法归属到具体标的，故为 `None`。
    pub fn code(&self) -> Option<&str> {
        match self {
            QuoteParseOutcome::Ok(q) => Some(&q.code),
            QuoteParseOutcome::NoData { code } => Some(code),
            QuoteParseOutcome::Malformed { .. } => None,
        }
    }
}

/// 解析前的裸点，尚未落栅格。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawPoint {
    pub hhmm: u16,
    pub price: f64,
    pub avg: Option<f64>,
}

/// 已落栅格的分时点。`grid_index` 是前端 X 轴映射的唯一依据；
/// `break_before` 让前端不需要理解任何市场的午休规则。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MinutePoint {
    pub hhmm: u16,
    pub grid_index: usize,
    pub price: f64,
    pub avg: Option<f64>,
    pub break_before: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MinuteSeries {
    pub code: String,
    pub market: Market,
    /// 该市场栅格长度，随序列一起下发；前端不自行推算
    pub grid_len: usize,
    pub prev_close: Option<f64>,
    pub points: Vec<MinutePoint>,
}

impl MinuteSeries {
    /// Y 轴极值只在已裁到栅格内的 `points` 上算，盘后重复点因已被丢弃而不参与。
    pub fn price_range(&self) -> Option<(f64, f64)> {
        let mut it = self.points.iter().map(|p| p.price);
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v))))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    /// 响应不是 JSON（HTML 错误页等）
    NotJson,
    /// 是 JSON 但结构不符（缺 key、`data` 为 null 等）
    Structure(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::NotJson => write!(f, "响应不是 JSON"),
            ParseError::Structure(s) => write!(f, "JSON 结构不符: {s}"),
        }
    }
}

/// 单个标的的数据新鲜度。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataState {
    /// 还没拿到过任何数据（首屏）
    Pending,
    Fresh,
    /// 本周期失败，展示的是上次成功的值
    Stale { last_ok_at: String, from_report_time: bool },
    /// 从未成功过且本周期也失败
    Error { message: String },
}

/// 单只标的的当日资金流（单位：元，净流入口径）。
///
/// 字段含义 2026-09-16 盘中实测认定（东财 `ulist.np`，恒等式全部成立）：
/// - `main_net` = f62 主力净流入 = 超大单净 + 大单净
/// - `retail_net` = f84 小单净流入（散户口径）
/// - `medium_net` = f78 中单净流入
/// - 恒等式：main + medium + retail ≈ 0（每笔买必有对应的卖）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FundFlow {
    /// 主力净流入（超大单 + 大单，东财口径）
    pub main_net: f64,
    /// 小单净流入（散户）
    pub retail_net: f64,
    /// 中单净流入
    pub medium_net: f64,
    /// 超大单净流入
    pub extra_large_net: f64,
    /// 大单净流入
    pub large_net: f64,
}

/// 一行的完整渲染数据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowSnapshot {
    pub code: String,
    /// 与 `WatchItem::em_secid` 同源，`None` = 东财通道不可用（不猜前缀）
    pub em_secid: Option<String>,
    pub market: Market,
    pub is_index: bool,
    /// 是否在悬浮窗中隐藏（来自 `WatchItem::hidden`）。
    ///
    /// **快照里保留隐藏行而不是过滤掉**，理由有三：
    /// 1. 设置页要显示隐藏标的的名称与价格（名称只能从快照拿）
    /// 2. 提醒判定要照常跑（隐藏语义是"不占地方"而非"停止关注"）
    /// 3. 取消隐藏后立刻有数据，不必等下一轮网络
    ///
    /// 过滤只发生在**悬浮窗的渲染层**。
    pub hidden: bool,
    /// 名称拿不到时前端显示代码，不显示空白
    pub name: Option<String>,
    pub quote: Option<Quote>,
    pub minute: Option<MinuteSeries>,
    pub state: DataState,
    /// 分时序列本周期是否可用。false 时前端不画曲线（严禁补点伪造）
    pub minute_available: bool,
    /// 当日资金流（v0.3.0 需求 6）。`None` = 无 secid / 该标的不供资金流 /
    /// 本轮尚未取到。资金流是独立的慢速轮询链路，与 quote/minute 互不阻塞。
    #[serde(default)]
    pub fund_flow: Option<FundFlow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub rows: Vec<RowSnapshot>,
    /// 快照生成的本地时间 `HH:MM:SS`
    pub updated_at: String,
    /// 本周期报价请求是否成功（用于整体错误提示）
    pub quote_ok: bool,
}

impl Snapshot {
    pub fn empty() -> Self {
        Snapshot {
            rows: Vec::new(),
            updated_at: String::new(),
            quote_ok: false,
        }
    }
}
