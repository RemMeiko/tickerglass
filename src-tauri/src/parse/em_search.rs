// 搜索结果解析。**纯函数层**：不 use reqwest / tauri（架构不变量 2）。
//
// 两个源结构完全不同，故**两个独立解析器**：
//
// | | 主源 `searchapi` | 兜底 `search-codetable` |
// |---|---|---|
// | 包装 | `{"QuotationCodeTable":{"Data":[...]}}` | `{"code":"0","result":[...]}` |
// | 空结果 | `Data: null`（**不是 `[]`**） | `result: []` |
// | 字段名 | 大驼峰 `Code`/`Name`/`MktNum` | 小驼峰 `code`/`shortName`/`market` |
// | 类型字段 | `SecurityType` 是**标量字符串** | `securityType` 是**整数数组** |
// | 顶层 `code` | 无 | **状态串 `'0'`，与条目内证券 `code` 撞名** |

use crate::domain::market::{classify, Market};
use serde_json::Value;

/// 一条搜索命中，已可直接落成 `WatchItem`。
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// 唯一身份键（`sh000001` 形式），由 `(市场号, 裸码)` **推导**而来
    pub tencent_code: String,
    /// 东财 secid。主源直接取 `QuoteID`（它就是 secid 原文），兜底源用 `{market}.{code}` 组合
    pub em_secid: Option<String>,
    pub market: Market,
    pub is_index: bool,
    pub name: String,
    /// 东财裸码（`000001`）。**不唯一**，仅用于界面展示，绝不作身份键
    pub raw_code: String,
    /// 市场中文名（`沪A`/`指数`/`港股`/`美股`），来自接口的 `securityTypeName`
    pub market_label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SearchError {
    /// 被限流：响应体缺 `QuotationCodeTable` 键。
    /// **必须与"无匹配结果"区分呈现**，否则用户会以为标的不存在
    RateLimited,
    /// 响应不是 JSON（HTML 错误页等）
    NotJson,
    /// 是 JSON 但结构不符
    Structure(String),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::RateLimited => write!(f, "搜索暂不可用，请稍后重试"),
            SearchError::NotJson => write!(f, "搜索响应不是 JSON"),
            SearchError::Structure(s) => write!(f, "搜索响应结构不符: {s}"),
        }
    }
}

/// 可加入自选股的 `securityTypeName` 白名单。
///
/// ⚠️ **`指数` 不可漏** —— 上证指数是 Sprint 1 默认四只之一，漏了它就加不回来（功能回归）。
///
/// 必须基于 `securityTypeName` 而**不是 market 号**做类型判定：
/// 600519 茅台与 510210 ETF 同为 `market=1`，market 号分不出股票与基金。
pub const ALLOWED_TYPE_NAMES: &[&str] = &["沪A", "深A", "港股", "美股", "指数"];

/// 已实测出现过的噪声类型名（仅用于文档与测试自证，判定走白名单的补集）。
pub const KNOWN_NOISE_TYPE_NAMES: &[&str] = &["基金", "行业", "英股", "韩股"];

/// `securityTypeName` 是否在白名单内。
pub fn is_allowed_type_name(name: &str) -> bool {
    ALLOWED_TYPE_NAMES.contains(&name)
}

/// 市场号 → 腾讯代码前缀。**这是遗留问题 3 的正解**：
/// 美股前缀不止 `105.`，权威来源是搜索接口的市场号，绝不硬编码猜测。
///
/// - `0` 深市、`1` 沪市
/// - `105` NASDAQ / `106` NYSE / `107` AMEX —— 三者都是美股
/// - `116` 港股
///
/// 返回 `None` = 该市场腾讯报价不支持（`2` 中证指数、`150` 场外基金、
/// `155` 英股、`177` 韩股）。这类条目**不能加入自选股**，因为它们没有腾讯身份键，
/// 加进来只会永远显示「—」。
pub fn tencent_prefix_for_market(market_num: i64) -> Option<&'static str> {
    match market_num {
        0 => Some("sz"),
        1 => Some("sh"),
        105 | 106 | 107 => Some("us"),
        116 => Some("hk"),
        _ => None,
    }
}

/// 由 `(市场号, 东财裸码)` 推导唯一身份键 `tencent_code`。
///
/// 这一步是防"删平安银行连带删上证指数"的根：
/// `(0, "000001")` → `sz000001`，`(1, "000001")` → `sh000001`，两者不同键。
pub fn tencent_code_for(market_num: i64, raw_code: &str) -> Option<String> {
    if raw_code.is_empty() {
        return None;
    }
    let prefix = tencent_prefix_for_market(market_num)?;
    Some(format!("{prefix}{raw_code}"))
}

/// 剥离可能存在的 JSONP 包裹。
///
/// 实测（2026-08-23，8 个 fixture）**两个源当前都返回裸 JSON**，无 JSONP 包裹 ——
/// 合同"JSONP 剥离"的前提与实测不符。但该接口历史上支持 `&cb=` 回调参数，
/// 保留这层防御成本为零，且不会误伤裸 JSON（首字符是 `{` 时直接原样返回）。
fn strip_jsonp(text: &str) -> &str {
    let t = text.trim();
    // 裸 JSON：直接返回，不做任何处理
    if t.starts_with('{') || t.starts_with('[') {
        return t;
    }
    // JSONP：`callback({...});` → 取第一个 '(' 与最后一个 ')' 之间
    match (t.find('('), t.rfind(')')) {
        (Some(a), Some(b)) if b > a + 1 => t[a + 1..b].trim(),
        _ => t,
    }
}

/// 主源 `searchapi` 解析。
///
/// **限流判据（单次请求即可判定，无需历史比对）**：有效响应恒含 `QuotationCodeTable` 键
/// —— 连 0 结果的响应也有（实测 `zzzzzzzz` → 106 字节仍带该键）；
/// 限流响应体完全没有这个键。
///
/// 空结果是 `Data: null` 而**不是** `[]`，按 `[]` 解析会得到 `Structure` 错误而非空列表。
pub fn parse_searchapi(text: &str) -> Result<Vec<SearchHit>, SearchError> {
    let body = strip_jsonp(text);
    let root: Value = serde_json::from_str(body).map_err(|_| SearchError::NotJson)?;

    // 限流判据：缺 QuotationCodeTable 键 → 限流（不是"无结果"）
    let table = root
        .get("QuotationCodeTable")
        .ok_or(SearchError::RateLimited)?;

    match table.get("Data") {
        // 空结果的真实形态：Data 是 null
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(rows)) => Ok(rows.iter().filter_map(searchapi_hit).collect()),
        Some(other) => Err(SearchError::Structure(format!(
            "Data 既非 null 也非数组: {}",
            kind_of(other)
        ))),
    }
}

/// searchapi 单条 → `SearchHit`，不合格的条目返回 `None`（被过滤掉）。
fn searchapi_hit(row: &Value) -> Option<SearchHit> {
    let type_name = row.get("SecurityTypeName")?.as_str()?;
    if !is_allowed_type_name(type_name) {
        return None;
    }
    // 债券/杠杆 ETF 混在「美股」类型名下（实测 `Apple Inc Notes 2022` 的
    // SecurityTypeName 就是「美股」），仅靠类型名白名单**挡不住**，
    // 必须再按 SecurityType 数值剔除。实测 8/8 债券均为 7。
    if str_or_num(row.get("SecurityType")).as_deref() == Some("7") {
        return None;
    }

    let raw_code = row.get("Code")?.as_str()?.trim().to_string();
    // MktNum 实测是字符串（"1"/"105"），但容错按数字也能读
    let market_num = str_or_num(row.get("MktNum"))?.parse::<i64>().ok()?;
    // 腾讯不支持的市场（中证指数/场外基金/英股/韩股）无身份键，直接排除
    let tencent_code = tencent_code_for(market_num, &raw_code)?;

    Some(SearchHit {
        market: classify(&tencent_code),
        is_index: type_name == "指数",
        // QuoteID 就是东财 secid 原文（`105.AAPL` / `1.000001`），
        // 直接取用即彻底消除"猜前缀"（遗留 3 的病根）
        em_secid: row
            .get("QuoteID")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| Some(format!("{market_num}.{raw_code}"))),
        name: row
            .get("Name")
            .and_then(|v| v.as_str())
            .unwrap_or(&raw_code)
            .trim()
            .to_string(),
        tencent_code,
        raw_code,
        market_label: type_name.to_string(),
    })
}

/// 兜底源 `search-codetable` 解析。**结构与主源完全不同**，故独立成函数。
///
/// 三个陷阱：
/// 1. 顶层 `code` 是状态串 `'0'`，**与条目内证券 `code` 撞名** —— 这里只读 `result`，不碰顶层 `code`
/// 2. `result` **直接是数组**，没有第二层包装
/// 3. `securityType` 是**整数数组**（`[401,101,5,501,301]`），按标量解析必崩
pub fn parse_codetable(text: &str) -> Result<Vec<SearchHit>, SearchError> {
    let body = strip_jsonp(text);
    let root: Value = serde_json::from_str(body).map_err(|_| SearchError::NotJson)?;

    // 注意：这里取的是 `result`，**不是**顶层 `code`（后者是状态串 '0'）
    match root.get("result") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(rows)) => Ok(rows.iter().filter_map(codetable_hit).collect()),
        Some(other) => Err(SearchError::Structure(format!(
            "result 既非 null 也非数组: {}",
            kind_of(other)
        ))),
    }
}

/// codetable 单条 → `SearchHit`。
fn codetable_hit(row: &Value) -> Option<SearchHit> {
    let type_name = row.get("securityTypeName")?.as_str()?;
    if !is_allowed_type_name(type_name) {
        return None;
    }
    // 债券剔除：codetable 侧**不能**用 `securityType` 含 7 判定 ——
    // 实测真身 AAPL 的数组同样含 7（`[401,501,102,7,201,301]`），会把正股一起误杀。
    // 实测可靠判据是 `status`：真实证券 31/31 为 10，债券 8/8 为 20。
    if row.get("status").and_then(|v| v.as_i64()) == Some(20) {
        return None;
    }

    let raw_code = row.get("code")?.as_str()?.trim().to_string();
    // market 实测是整数（不是字符串），与主源的 MktNum 相反
    let market_num = str_or_num(row.get("market"))?.parse::<i64>().ok()?;
    let tencent_code = tencent_code_for(market_num, &raw_code)?;

    Some(SearchHit {
        market: classify(&tencent_code),
        is_index: type_name == "指数",
        // codetable 不返回 QuoteID，secid 用 `{market}.{code}` 组合
        // （与主源 QuoteID 的形态一致，实测 `116.00700` / `105.AAPL`）
        em_secid: Some(format!("{market_num}.{raw_code}")),
        name: row
            .get("shortName")
            .and_then(|v| v.as_str())
            .unwrap_or(&raw_code)
            .trim()
            .to_string(),
        tencent_code,
        raw_code,
        market_label: type_name.to_string(),
    })
}

/// `securityType` 这类字段在两个源里一个是字符串一个是数字，统一读成字符串。
/// 数组（codetable 的 `securityType`）不参与此转换，返回 `None`。
fn str_or_num(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// 错误信息里描述实际拿到的 JSON 类型，便于定位结构变更。
fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// 去重：同一 `tencent_code` 只保留第一条。
///
/// 必要性：搜「000001」时主源返回平安银行(sz000001)与上证指数(sh000001) ——
/// 这两条**不是**重复项（键不同，都要保留）；而 pageSize 翻页边界上
/// 偶发同键重复才是要去掉的。按 `tencent_code` 判定天然区分这两种情况。
pub fn dedup_by_identity(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(hits.len());
    for h in hits {
        let k = h.tencent_code.to_ascii_lowercase();
        if !seen.contains(&k) {
            seen.push(k);
            out.push(h);
        }
    }
    out
}
