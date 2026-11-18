// 接口用法参考 StockPet (MIT)，实现为本项目原创
//
// 唯一的 I/O 层。parse / domain 两层不 use reqwest，因此 cargo test 可直接调用
// 且测试完全不发网络请求。

use std::time::Duration;

/// spec 指定的请求头，缺失可能被拒绝或返回异常内容。
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const REFERER: &str = "https://quote.eastmoney.com/";

/// 请求域名白名单（硬编码，无遥测）。
///
/// Sprint 2A 补入两个**搜索**域名 —— 不补的话 `get_bytes` 会直接 `Blocked`，
/// 标准 2 的搜索在运行时必挂（编译期看不出来）。
///
/// v0.4.0：`push2.eastmoney.com` 移出白名单 —— 大盘面板整体迁到 `push2delay`
///（2026-11-18 盘中实测：delay 的 `stock/get` 同样带 f113/f114/f115 家数与
/// f135~f149 资金流，沪 459/1836/60、恒等式成立；旧结论"只有 push2 带"是在
/// 别的路径上得出的）。push2 域对轮询的容忍度极差（host 级 TLS 掐断惩罚），
/// 不再保留任何请求路径走它。
const ALLOWED_HOSTS: &[&str] = &[
    "qt.gtimg.cn",
    "web.ifzq.gtimg.cn",
    "push2delay.eastmoney.com",
    // 日K 主源（**不同于 push2delay**，是独立域名，漏加会被 get_bytes 硬拒）
    "push2his.eastmoney.com",
    // 搜索主源
    "searchapi.eastmoney.com",
    // 搜索兜底源
    "search-codetable.eastmoney.com",
];

/// 东财 suggest 接口的**公开固定串**，全网文档与其前端 JS 都是同一个值。
/// 不是用户凭证、不含身份信息，属接口地址的一部分。
const EM_SUGGEST_TOKEN: &str = "D43BF722C8E33BDC906FB84D85E326E8";

/// 从 URL 取出主机名。独立成纯函数以便单测（不发请求即可验证白名单）。
pub fn host_of(url: &str) -> String {
    url.split("//")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .unwrap_or("")
        .to_string()
}

/// 主机是否在硬编码白名单内。
pub fn is_allowed_host(host: &str) -> bool {
    ALLOWED_HOSTS.contains(&host)
}

/// 白名单快照，供架构守卫测试断言"没有意外新增外联域名"。
pub fn allowed_hosts() -> &'static [&'static str] {
    ALLOWED_HOSTS
}

/// 最小 percent-encoding。
///
/// 只放行 RFC 3986 的 unreserved 字符，其余一律按 UTF-8 字节转 `%XX`。
/// 这既让中文查询词（「茅台」）能正确入 URL，也顺带挡住了用户输入里的
/// `&` / `?` / 空格 —— 否则 `a&type=99` 这类输入会篡改后面的查询参数。
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 把内部身份键换成腾讯日K要的请求代码。
///
/// A股/指数/港股原样返回；**美股必须补交易所后缀** —— 实测 `usAAPL` 只返 2 条、
/// `usAAPL.OQ` 返 121 条。这是**静默降级**（不报错、只是数据少），
/// 所以必须在这一层保证后缀正确，不能指望下游发现。
///
/// 后缀由 `em_secid` 前缀判定（实测 2026-08-27）：
/// - `105.` = 纳斯达克 → `.OQ`（AAPL、TSLA 已验）
/// - `106.` = 纽交所   → `.N`（JPM、KO 已验）
///
/// 二者不可互换：`usJPM.OQ` 只返 1 条，`usJPM.N` 正常。
/// `em_secid` 缺失或前缀未知时退回 `.OQ` —— 纳斯达克覆盖面更大，
/// 且错了是"数据少"而非"崩"，比直接放弃更可用。
pub fn tencent_kline_code(tencent_code: &str, em_secid: Option<&str>) -> String {
    if !tencent_code.starts_with("us") {
        return tencent_code.to_string();
    }
    let suffix = match em_secid.and_then(|s| s.split('.').next()) {
        Some("106") => ".N",
        Some("105") => ".OQ",
        _ => ".OQ",
    };
    format!("{tencent_code}{suffix}")
}

#[derive(Debug)]
pub enum NetError {
    Blocked(String),
    Http(String),
}

/// 把 reqwest 错误压成**带完整 source 链**的一行字符串。
///
/// # 为什么必须这么做
///
/// `reqwest::Error` 的 `Display` 只给最外层，实测永远是
/// `error sending request for url (...)` —— 这句话不含任何诊断信息。
/// 真正的原因在 `source()` 链里。2026-08-27 排查大盘面板时，
/// 日志里只有那句话，我不得不另写一个诊断文件（`tests/diag_push2.rs`）
/// 才拿到真因：
///
/// ```text
/// error sending request              ← 原先日志里只有这一层
///  └ client error (SendRequest)
///   └ connection error
///    └ peer closed connection without sending TLS close_notify   ← 真因
/// ```
///
/// 最后那层才说明"服务端掐断了 TLS 连接"（东财的限流表现），
/// 而它与"网络不通""DNS 失败""超时"在最外层是完全一样的一句话。
/// 用户报障时我只能看到日志，所以这条链必须留在日志里。
fn describe_reqwest(e: &reqwest::Error) -> String {
    use std::error::Error;
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(inner) = src {
        s.push_str(" ← ");
        s.push_str(&inner.to_string());
        src = inner.source();
    }
    // 补上分类位：区分超时/建连失败/发送失败，比纯文本更好判
    let mut tags: Vec<&str> = Vec::new();
    if e.is_timeout() {
        tags.push("timeout");
    }
    if e.is_connect() {
        tags.push("connect");
    }
    if e.is_request() {
        tags.push("request");
    }
    if e.is_body() {
        tags.push("body");
    }
    if e.is_decode() {
        tags.push("decode");
    }
    if !tags.is_empty() {
        s.push_str(&format!(" [{}]", tags.join(",")));
    }
    s
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Blocked(h) => write!(f, "域名不在白名单: {h}"),
            NetError::Http(e) => write!(f, "请求失败: {e}"),
        }
    }
}

#[derive(Clone)]
pub struct Fetcher {
    client: reqwest::Client,
}

impl Fetcher {
    pub fn new() -> Result<Self, NetError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .connect_timeout(Duration::from_secs(4))
            .user_agent(UA)
            .build()
            .map_err(|e| NetError::Http(describe_reqwest(&e)))?;
        Ok(Fetcher { client })
    }

    /// 只取字节流。**绝不调用 `.text()`** —— 它按 UTF-8 猜解码，
    /// GBK 报文里的中文名会全部变乱码（本项目头号坑）。解码在 `parse::gbk`。
    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>, NetError> {
        let host = host_of(url);
        if !is_allowed_host(&host) {
            return Err(NetError::Blocked(host));
        }

        let resp = self
            .client
            .get(url)
            .header("Referer", REFERER)
            // describe_reqwest 而不是 to_string()：后者只给"error sending request"，
            // 把真因（TLS 被掐 / 超时 / DNS）全丢了。详见该函数的文档。
            .send()
            .await
            .map_err(|e| NetError::Http(describe_reqwest(&e)))?;

        if !resp.status().is_success() {
            return Err(NetError::Http(format!("HTTP {}", resp.status())));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| NetError::Http(describe_reqwest(&e)))
    }

    /// 单批报价请求。**调用方必须先用 `watchlist::batch_codes` 切批** ——
    /// 腾讯一次最多 40 个代码，50 只上限下直接 `join` 会超限。
    pub async fn fetch_quotes(&self, codes: &[&str]) -> Result<Vec<u8>, NetError> {
        let url = format!("https://qt.gtimg.cn/q={}", codes.join(","));
        self.get_bytes(&url).await
    }

    /// 搜索主源。`input` 必须 URL 编码（中文查询词直接拼会得到非法 URL）。
    pub async fn fetch_em_search(&self, query: &str) -> Result<Vec<u8>, NetError> {
        let q = urlencode(query);
        let url = format!(
            "https://searchapi.eastmoney.com/api/suggest/get\
             ?input={q}&type=14&token={EM_SUGGEST_TOKEN}&count=10"
        );
        self.get_bytes(&url).await
    }

    /// 搜索兜底源。结构与主源不同，解析走 `parse::em_search::parse_codetable`。
    pub async fn fetch_codetable_search(&self, query: &str) -> Result<Vec<u8>, NetError> {
        let q = urlencode(query);
        let url = format!(
            "https://search-codetable.eastmoney.com/codetable/search/web\
             ?client=web&keyword={q}&pageSize=10&pageIndex=1"
        );
        self.get_bytes(&url).await
    }

    pub async fn fetch_tencent_minute(&self, code: &str) -> Result<Vec<u8>, NetError> {
        let url = format!("https://web.ifzq.gtimg.cn/appstock/app/minute/query?code={code}");
        self.get_bytes(&url).await
    }

    pub async fn fetch_em_trends(&self, secid: &str) -> Result<Vec<u8>, NetError> {
        let url = format!(
            "https://push2delay.eastmoney.com/api/qt/stock/trends2/get?secid={secid}\
             &ndays=1&fields1=f1,f2,f3,f4,f5&fields2=f51,f52,f53,f54,f55,f56,f57,f58"
        );
        self.get_bytes(&url).await
    }

    /// 日K（东财，主源）。`klt=101` 是日线，`fqt=1` 前复权。
    ///
    /// 用东财而非腾讯 `fqkline` 的两个理由见 `parse::em_kline` 模块文档：
    /// 腾讯美股须带 `.OQ` 后缀（而 `.OQ` 不能作身份键），且部分市场不带成交量。
    pub async fn fetch_em_kline(&self, secid: &str, limit: u32) -> Result<Vec<u8>, NetError> {
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid={secid}\
             &klt=101&fqt=1&end=20500101&lmt={limit}\
             &fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58"
        );
        self.get_bytes(&url).await
    }

    /// 日K（腾讯，**主源**）。`code` 必须是 [`tencent_kline_code`] 的产物 ——
    /// 美股不带交易所后缀只会返 2 条（而不是报错），是个静默降级的坑。
    ///
    /// 为什么不用东财：东财历史数据只在 `push2his`，而这台机器直连不通它
    /// （详见 `parse::tencent_kline` 模块文档）。
    pub async fn fetch_tencent_kline(&self, code: &str, limit: u32) -> Result<Vec<u8>, NetError> {
        let url = format!(
            "https://web.ifzq.gtimg.cn/appstock/app/fqkline/get?param={code},day,,,{limit},qfq"
        );
        self.get_bytes(&url).await
    }

    /// 5 日分时（腾讯）。**只支持 A股/指数/港股，不支持美股。**
    ///
    /// 实测 2026-08-26：`usAAPL` 返回 `{"code":-1,"msg":"param error"}`。
    /// 试过 `usAAPL` / `usAAPL.OQ` / `AAPL` / `us_AAPL` / `usaapl` / `gb_aapl` 六种写法全失败，
    /// 且 `AAPL`、`gb_aapl` 报的是 `code param error` 而其余报 `param error` ——
    /// 说明代码格式本身合法，腾讯这条通道就是不供美股数据。
    /// 美股走 [`Self::fetch_em_trends_5day`]。
    pub async fn fetch_tencent_5day(&self, code: &str) -> Result<Vec<u8>, NetError> {
        let url = format!("https://web.ifzq.gtimg.cn/appstock/app/day/query?code={code}");
        self.get_bytes(&url).await
    }

    /// 大盘面板：单个市场的涨跌家数 + 资金流向五档（**一个请求拿齐**）。
    ///
    /// `secid` 传 `1.000001`（沪）/ `0.399001`（深）/ `0.899050`（北）。
    ///
    /// # v0.4.0：host 从 `push2` 迁到 `push2delay`
    ///
    /// 2026-11-18 盘中实测推翻了"只有 push2 带 f113/f114/f115"的旧结论：
    /// delay 的 `stock/get` 同样带全部字段（沪 459/1836/60=2355 家、
    /// f135~f149 五档齐全、恒等式成立），代价是数据有分钟级延迟。
    /// 而 push2 域对轮询的容忍度极差（host 级 TLS 掐断惩罚，v0.3.0 实测），
    /// 大盘面板长期因它缺市场 —— 换域后三市场 3 分钟内必全。
    ///
    /// # ⚠️ 仍要把"空响应"当限流
    ///
    /// push2 曾实测几分钟内数百请求后整个 host 返回**空响应体**（无错误码）。
    /// delay 域宽容得多（美股分时 + 资金流已在上面跑了几个月），
    /// 但调用方仍须把"空响应"当**明确的限流状态**处理（parse 层的
    /// `MarketParseError::RateLimited`），绝不能当成"家数为 0"。
    pub async fn fetch_em_market_stats(&self, secid: &str) -> Result<Vec<u8>, NetError> {
        // 一次请求同时取家数与五档资金流向，省一半请求
        let fields = "f57,f58,f43,f170,f113,f114,f115,\
                      f135,f136,f137,f138,f139,f140,\
                      f141,f142,f143,f144,f145,f146,f147,f148,f149";
        let url = format!(
            "https://push2delay.eastmoney.com/api/qt/stock/get?secid={secid}&fields={fields}"
        );
        self.get_bytes(&url).await
    }

    /// 个股当日资金流（v0.3.0 需求 6）：**一个请求**带整批 secid。
    ///
    /// 字段：f12 裸码 / f13 市场号（拼回 secid）/ f14 名称 /
    /// f62 主力净 / f66 超大净 / f72 大单净 / f78 中单净 / f84 小单净 / f124 时间戳。
    /// 2026-09-16 实测：A 股/港股有值，美股字段为 `-`（解析层跳过 → 前端不显示）。
    ///
    /// # 为什么是 `push2delay` 而不是 `push2`（2026-09-16 下午实测定案）
    ///
    /// `push2` 当天对本机进入 TLS 掐断惩罚（生产路径冒烟复现：
    /// `peer closed connection without sending TLS close_notify`），
    /// 且探测会延长惩罚。而 `push2delay` 是**独立限流域** —— 本项目美股分时
    /// 已在它上面 ~30s 一次稳定轮询数月。资金流实测（含恒等式）在 delay 上
    /// 与 push2 完全一致，只是延迟 15 分钟 —— 对"日内累计主力/散户流向"无感。
    /// 故资金流固定走 delay，不占 push2 闸门额度，也不用担心互杀。
    pub async fn fetch_em_fund_flow(&self, secids: &[&str]) -> Result<Vec<u8>, NetError> {
        let fields = "f12,f13,f14,f62,f66,f72,f78,f84,f124";
        let url = format!(
            "https://push2delay.eastmoney.com/api/qt/ulist.np/get?secids={}&fields={fields}\
             &ut=b2884a393a59ad64002292a3e90d46a5",
            secids.join(",")
        );
        self.get_bytes(&url).await
    }

    /// 5 日分时（东财，**美股专用**）。腾讯不供美股 5 日，见 [`Self::fetch_tencent_5day`]。
    ///
    /// 两个实测要点（2026-08-26，`105.AAPL`）：
    ///
    /// 1. **host 必须是 `push2his`，不能用 `push2delay`。** `push2delay` 会**静默忽略**
    ///    `ndays` —— 传 1 和传 5 都只返 391 点（单个交易日）。`push2his` 传 5 返 1955 点
    ///    覆盖 5 个美股交易日。所以决定性的是 host 而不只是参数。
    /// 2. 与 [`Self::fetch_em_trends`] 只差 `ndays` 与 host，字段列表完全一致，
    ///    故响应结构相同（`data.trends` 字符串数组），可共用解析基元。
    pub async fn fetch_em_trends_5day(&self, secid: &str) -> Result<Vec<u8>, NetError> {
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/trends2/get?secid={secid}\
             &ndays=5&fields1=f1,f2,f3,f4,f5&fields2=f51,f52,f53,f54,f55,f56,f57,f58"
        );
        self.get_bytes(&url).await
    }
}
