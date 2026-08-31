//! 用户直接输入的代码 → 候选身份键（**纯函数，不发网络**）。
//!
//! # 为什么产出的是「候选列表」而不是单个结果
//!
//! 裸 6 位码在中国市场**不唯一**。实测 2026-08-27：
//! `000001` 既是 `sz000001` 平安银行、又是 `sh000001` 上证指数 ——
//! 两个都真实存在。任何"按段位猜一个"的表都会有一半时候是错的，
//! 而且错得无声（用户以为加了平安银行，实际加了上证指数）。
//!
//! 所以这一层只负责**穷举可能**，由 [`crate::ipc::resolve_code`] 打报价接口
//! 逐个验存在性、取真实名称，再把活着的候选交给用户挑。
//! 这样也顺带解决了"段位表不全"的问题：未知段位同时给 sh/sz 两个候选，
//! 接口自然会淘汰不存在的那个（实测不存在的码返回 `v_pv_none_match="1"`）。
//!
//! # 段位 → 交易所（2026-08-27 逐个打 qt.gtimg.cn 实测，非查文档）
//!
//! | 段位 | 前缀 | 实测样本 |
//! |---|---|---|
//! | 600/601/603/605 | sh | sh600519 贵州茅台、sh601318 中国平安 |
//! | 688/689 科创板 | sh | sh688111 金山办公 |
//! | 900 沪B | sh | sh900901 云赛Ｂ股 |
//! | 000/001/002/003 | sz | sz000001 平安银行、sz002415 海康威视 |
//! | 300/301 创业板 | sz | sz300750 宁德时代、sz301269 华大九天 |
//! | 200 深B | sz | sz200011 深物业B |
//! | 399 | sz（指数）| sz399001 深证成指 |
//! | 000（沪）| sh（指数）| sh000001 上证指数、sh000300 沪深300 |
//! | 43/83/87/92 | bj | bj430047 诺思兰德、bj920819 颖泰生物 |
//!
//! 北交所有个**不对称**必须记住：腾讯前缀是 `bj`，但东财 secid 前缀是 `0.`
//! （实测 `0.430047` → rc=0，`1.430047` → rc=100）。两者不同源，别用一个规则推另一个。
//!
//! # 范围
//!
//! 只做 A 股（含指数/ETF/北交所），这是用户 2026-08-27 的明确决定
//! （"暂时不加美股，只加A股"）。港股/美股仍走名称搜索。

use crate::domain::market::{classify, Market};
use crate::model::WatchItem;

/// 一个待验证的候选。`label` 是给用户区分同码不同市场用的（"沪市/深市/北交所"）。
#[derive(Debug, Clone, PartialEq)]
pub struct CodeCandidate {
    pub tencent_code: String,
    pub em_secid: String,
    pub market: Market,
    pub is_index: bool,
    pub label: &'static str,
}

impl CodeCandidate {
    pub fn to_watch_item(&self) -> WatchItem {
        WatchItem {
            tencent_code: self.tencent_code.clone(),
            em_secid: Some(self.em_secid.clone()),
            market: self.market,
            is_index: self.is_index,
            // 新添加的标的默认可见 —— 用户刚加进来就藏起来毫无道理
            hidden: false,
        }
    }
}

/// 输入不像 A 股代码时的原因。做成枚举而不是空 `Vec`，
/// 是为了让前端能区分"你打的是中文，请用搜索"与"位数不对"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeInputError {
    /// 空输入
    Empty,
    /// 含非法字符（中文名、拼音等）——应该走名称搜索而不是直接添加
    NotNumeric,
    /// 位数不是 6（`60051` / `6005199`）
    BadLength(usize),
    /// 带了 hk/us 前缀。本期只做 A 股，明确告知而不是静默当成 A 股处理
    UnsupportedMarket(String),
}

impl std::fmt::Display for CodeInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodeInputError::Empty => write!(f, "请输入代码"),
            CodeInputError::NotNumeric => {
                write!(f, "不像股票代码。按名称/拼音查找请用上方搜索")
            }
            CodeInputError::BadLength(n) => {
                write!(f, "A 股代码是 6 位数字，当前 {n} 位")
            }
            CodeInputError::UnsupportedMarket(p) => {
                write!(f, "暂不支持直接输入 {p} 代码，请用上方名称搜索")
            }
        }
    }
}

/// 交易所归属。`bj` 的 secid 前缀是 `0.` 而非 `1.`（见模块文档的不对称说明）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exchange {
    Sh,
    Sz,
    Bj,
}

impl Exchange {
    fn prefix(self) -> &'static str {
        match self {
            Exchange::Sh => "sh",
            Exchange::Sz => "sz",
            Exchange::Bj => "bj",
        }
    }

    /// 东财 secid 的市场号。**北交所是 0**（与深市同号），实测确认。
    fn em_market(self) -> &'static str {
        match self {
            Exchange::Sh => "1",
            Exchange::Sz | Exchange::Bj => "0",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Exchange::Sh => "沪市",
            Exchange::Sz => "深市",
            Exchange::Bj => "北交所",
        }
    }
}

fn build(ex: Exchange, digits: &str) -> CodeCandidate {
    let tencent_code = format!("{}{}", ex.prefix(), digits);
    // 市场与指数标记一律走既有 classify()，不在这里另写一套判定 ——
    // 两处规则一旦分叉，指数会被当成个股（小数位与栅格都会错）
    let market = classify(&tencent_code);
    CodeCandidate {
        em_secid: format!("{}.{}", ex.em_market(), digits),
        market,
        is_index: market.is_index(),
        tencent_code,
        label: ex.label(),
    }
}

/// 按段位给出候选交易所。**返回多个 = 该段位本身有歧义**，交由接口淘汰。
///
/// 段位表见模块文档（全部实测）。未知段位返回 `[Sh, Sz]` 而不是报错 ——
/// 表不可能穷尽（新板块随时开），让接口来判存在性比让用户吃"不支持"更可用。
fn exchanges_for(digits: &str) -> Vec<Exchange> {
    debug_assert_eq!(digits.len(), 6, "调用方必须已校验 6 位");
    let p3 = &digits[..3];
    let p2 = &digits[..2];

    match p3 {
        // 沪市个股：主板 600/601/603/605，科创板 688/689
        "600" | "601" | "603" | "605" | "688" | "689" => vec![Exchange::Sh],
        // 沪市 B 股
        "900" => vec![Exchange::Sh],
        // 深市个股：主板 000/001（含原中小板 002/003）
        "001" | "002" | "003" => vec![Exchange::Sz],
        // 创业板
        "300" | "301" => vec![Exchange::Sz],
        // 深市 B 股
        "200" => vec![Exchange::Sz],
        // 深证指数段
        "399" => vec![Exchange::Sz],
        // ⚠️ 000xxx 是真歧义：sz000001 平安银行 与 sh000001 上证指数 都存在。
        // 指数放前面 —— 000001/000300 这类高频输入更常指指数。
        "000" => vec![Exchange::Sh, Exchange::Sz],
        _ => match p2 {
            // 北交所（含新三板转板段）
            "43" | "83" | "87" | "88" | "92" => vec![Exchange::Bj],
            // 沪市基金/ETF（51xxxx/56xxxx/58xxxx…）与可转债（11xxxx）
            "50" | "51" | "52" | "56" | "58" | "11" => vec![Exchange::Sh],
            // 深市基金/ETF（15xxxx/16xxxx/18xxxx）与可转债（12xxxx）
            "15" | "16" | "18" | "12" => vec![Exchange::Sz],
            // 未知段位：两个都试，接口会淘汰不存在的那个
            _ => vec![Exchange::Sh, Exchange::Sz],
        },
    }
}

/// 解析用户输入 → 候选列表（**不发网络，不判存在性**）。
///
/// 接受的写法（均实测过用户会这样打）：
/// - `sh600519` / `SH600519`（带前缀，大小写不敏感）
/// - `600519`（裸码）
/// - `600519.SH` / `600519.sz`（Wind/同花顺导出的常见格式）
/// - 中间带空格：`600 519` → 先剔空白
///
/// 返回的候选**未经存在性校验** —— `sh605888` 这种格式合法但不存在的码
/// 也会出现在结果里，必须由调用方打接口淘汰。
pub fn parse_code_input(raw: &str) -> Result<Vec<CodeCandidate>, CodeInputError> {
    // 空白一律剔除（含全角空格），用户从表格里复制常带尾随空白
    let s: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return Err(CodeInputError::Empty);
    }
    let lower = s.to_ascii_lowercase();

    // ── `600519.SH` 形式：先把后缀转成前缀，再走统一路径 ──
    let normalized = match lower.split_once('.') {
        Some((digits, suf)) => match suf {
            "sh" | "sz" | "bj" => format!("{suf}{digits}"),
            // `.hk` / `.us` 等：明确告知不支持，不静默当 A 股
            other if !other.is_empty() => {
                return Err(CodeInputError::UnsupportedMarket(other.to_uppercase()))
            }
            _ => lower.clone(),
        },
        None => lower.clone(),
    };

    // ── 带市场前缀 ──
    for p in ["sh", "sz", "bj", "hk", "us"] {
        if let Some(rest) = normalized.strip_prefix(p) {
            if p == "hk" || p == "us" {
                return Err(CodeInputError::UnsupportedMarket(p.to_uppercase()));
            }
            if !rest.bytes().all(|b| b.is_ascii_digit()) {
                return Err(CodeInputError::NotNumeric);
            }
            if rest.len() != 6 {
                return Err(CodeInputError::BadLength(rest.len()));
            }
            let ex = match p {
                "sh" => Exchange::Sh,
                "sz" => Exchange::Sz,
                _ => Exchange::Bj,
            };
            // 用户已明确指定交易所，就不再给别的候选 —— 尊重显式输入
            return Ok(vec![build(ex, rest)]);
        }
    }

    // ── 裸码 ──
    if !normalized.bytes().all(|b| b.is_ascii_digit()) {
        return Err(CodeInputError::NotNumeric);
    }
    if normalized.len() != 6 {
        return Err(CodeInputError::BadLength(normalized.len()));
    }
    Ok(exchanges_for(&normalized)
        .into_iter()
        .map(|ex| build(ex, &normalized))
        .collect())
}
