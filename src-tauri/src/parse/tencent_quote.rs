// 接口用法参考 StockPet (MIT)，实现为本项目原创
//
// 报文形态：`v_{code}="字段1~字段2~...~";`，每个代码一行，GBK 编码。
// 字段位置（0-based，A 股为准）：[1] 名称、[2] 代码、[3] 现价、[4] 昨收、
// [5] 今开、[30] 时间戳、[31] 涨跌额、[32] 涨跌幅、[33] 最高、[34] 最低。
//
// 实测字段总数：A 股/指数 88、港股 78、美股 71 —— 故一律走 `get(i)` 不硬假定长度。

use crate::domain::change::resolve_change;
use crate::domain::market::classify;
use crate::model::{Quote, QuoteParseOutcome};
use crate::parse::timestamp::{format_hhmm, parse_quote_ts};

const IDX_NAME: usize = 1;
const IDX_SYMBOL: usize = 2;
const IDX_PRICE: usize = 3;
const IDX_PREV_CLOSE: usize = 4;
const IDX_OPEN: usize = 5;
const IDX_TIMESTAMP: usize = 30;
const IDX_CHG_AMOUNT: usize = 31;
const IDX_CHG_PERCENT: usize = 32;
const IDX_HIGH: usize = 33;
const IDX_LOW: usize = 34;
/// 成交额（万元）。2026-09-16 实测：上证指数与茅台的 `[38]` 均为万元
/// （茅台 219045万 ↔ 分时接口累计额 2,190,454,599 元），×10000 折成元。
const IDX_AMOUNT_WAN: usize = 38;

fn field(fields: &[&str], i: usize) -> Option<String> {
    fields.get(i).map(|s| s.trim().to_string())
}

/// 越界、空串、`-`、非数字一律 `None` —— 绝不 `unwrap_or(0.0)`。
fn num(fields: &[&str], i: usize) -> Option<f64> {
    let raw = fields.get(i)?.trim();
    if raw.is_empty() || raw == "-" {
        return None;
    }
    raw.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// 现价字符串的小数位数（港股实测 3 位 `457.000`，A 股 2 位）。
/// 按报文原样显示，避免把 `457.000` 显示成 `457.00` 被误判为解析 bug。
fn decimals(fields: &[&str], i: usize) -> Option<u8> {
    let raw = fields.get(i)?.trim();
    let frac = raw.split_once('.')?.1;
    if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(frac.len().min(8) as u8)
}

/// 解析单行。
///
/// 身份键的**唯一**来源是 `v_{code}=` 前缀里的 `{code}`。
/// 实测 `usAAPL` 的 `[2]` 是 `AAPL.OQ`、`hk00700` 的 `[2]` 是 `00700`，
/// 都与前缀键不同，所以 `[2]` 只作展示，**永不用于与自选股条目匹配**。
pub fn parse_line(line: &str) -> QuoteParseOutcome {
    let line = line.trim();
    if line.is_empty() {
        return QuoteParseOutcome::Malformed {
            raw_prefix: String::new(),
        };
    }

    let prefix_len = 40.min(line.len());
    let malformed = || QuoteParseOutcome::Malformed {
        raw_prefix: line[..prefix_len_boundary(line, prefix_len)].to_string(),
    };

    // 取 `v_{code}="`
    let Some(rest) = line.strip_prefix("v_") else {
        return malformed();
    };
    let Some((code, tail)) = rest.split_once("=\"") else {
        return malformed();
    };
    let code = code.trim();
    if code.is_empty() {
        return malformed();
    }

    // 无效代码的两种真实形态：整行 `v_{code}="";`，或单代码请求时的 `v_pv_none_match="1";`
    if code == "pv_none_match" {
        return QuoteParseOutcome::NoData {
            code: code.to_string(),
        };
    }

    // 行尾必须闭合 `";`，否则视为截断
    let Some(payload) = tail.strip_suffix("\";").or_else(|| tail.strip_suffix('"')) else {
        return malformed();
    };

    if payload.is_empty() {
        return QuoteParseOutcome::NoData {
            code: code.to_string(),
        };
    }

    let fields: Vec<&str> = payload.split('~').collect();
    if fields.len() < 5 {
        return QuoteParseOutcome::NoData {
            code: code.to_string(),
        };
    }

    let price = num(&fields, IDX_PRICE);
    if price.is_none() {
        // 现价不可得 = 无有效数据。不填 0。
        return QuoteParseOutcome::NoData {
            code: code.to_string(),
        };
    }

    let market = classify(code);
    let prev_close = num(&fields, IDX_PREV_CLOSE);
    let change = resolve_change(
        price,
        prev_close,
        num(&fields, IDX_CHG_AMOUNT),
        num(&fields, IDX_CHG_PERCENT),
    );

    let raw_timestamp = field(&fields, IDX_TIMESTAMP).unwrap_or_default();
    let quote_time = parse_quote_ts(&raw_timestamp, market).map(|dt| format_hhmm(&dt));

    let name = field(&fields, IDX_NAME).filter(|s| !s.is_empty());

    QuoteParseOutcome::Ok(Quote {
        code: code.to_string(),
        name: name.unwrap_or_else(|| code.to_string()),
        raw_symbol: field(&fields, IDX_SYMBOL).unwrap_or_default(),
        market,
        price,
        prev_close,
        open: num(&fields, IDX_OPEN),
        high: num(&fields, IDX_HIGH),
        low: num(&fields, IDX_LOW),
        change,
        price_decimals: decimals(&fields, IDX_PRICE),
        raw_timestamp,
        quote_time,
        amount: num(&fields, IDX_AMOUNT_WAN).map(|wan| wan * 10_000.0),
    })
}

/// 按 char 边界截断，避免中文名被切一半导致 panic。
fn prefix_len_boundary(s: &str, mut n: usize) -> usize {
    while n > 0 && !s.is_char_boundary(n) {
        n -= 1;
    }
    n
}

/// 逐行解析整批响应。单行损坏不影响其他行（每个代码一行，实测如此）。
pub fn parse_batch(text: &str) -> Vec<QuoteParseOutcome> {
    text.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_line)
        .collect()
}

/// 请求集 vs 返回行集对账。
///
/// 实测 `q=sh999999,sz000001` 的响应里 `sh999999` **整行不出现**（不是空串）。
/// 若只按"响应里出现的行"产出结果，失效标的会从快照里整行消失 —— 界面上那一行
/// 直接不见了，用户毫无感知，且走不到"保留旧值 + 过期标记"的分支。
///
/// 语义：以 `requested` 为准输出**等长**结果且顺序一致；缺失的补 `NoData`；
/// 响应里多回的行不静默丢弃，追加在尾部。
pub fn reconcile(requested: &[&str], outcomes: Vec<QuoteParseOutcome>) -> Vec<QuoteParseOutcome> {
    let mut extras: Vec<QuoteParseOutcome> = Vec::new();
    let mut matched: Vec<Option<QuoteParseOutcome>> = vec![None; requested.len()];

    for oc in outcomes {
        // 找第一个「代码相同且尚未占用」的槽位。大小写不敏感（美股代码大写，前缀小写）。
        let slot = oc.code().and_then(|code| {
            requested
                .iter()
                .enumerate()
                .find(|(i, r)| matched[*i].is_none() && r.eq_ignore_ascii_case(code))
                .map(|(i, _)| i)
        });
        match slot {
            Some(i) => matched[i] = Some(oc),
            // 没有对应请求项（含 Malformed —— 它无法归属到具体标的）：
            // 不静默丢弃，追加到尾部保留可见性
            None => extras.push(oc),
        }
    }

    let mut out: Vec<QuoteParseOutcome> = requested
        .iter()
        .zip(matched)
        .map(|(code, slot)| {
            slot.unwrap_or_else(|| QuoteParseOutcome::NoData {
                code: (*code).to_string(),
            })
        })
        .collect();
    out.extend(extras);
    out
}
