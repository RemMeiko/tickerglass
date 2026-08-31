//! **联网**冒烟测试：验证 `net` 层白名单 + 真实响应能被解析器吃下。
//!
//! 全部标 `#[ignore]`，故 `cargo test` 默认**不跑**它们 ——
//! 这保证常规测试套件（262 个）仍是纯离线的。
//!
//! 手动运行：
//! ```text
//! cargo test --test live_search_smoke -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! `--test-threads=1` 是必需的：并发打搜索接口会触发限流，
//! 让本该通过的用例假失败（Evaluator N-5 的教训）。
//!
//! 存在理由：搜索域名若漏进 `net.rs` 白名单，`get_bytes` 会 `Blocked`，
//! 而这**编译期与离线测试都发现不了**，只在用户点搜索时炸。

use tickerglass_lib::net::Fetcher;
use tickerglass_lib::parse::em_search::{parse_codetable, parse_searchapi};

/// 请求间隔。低频访问避免把自己打进限流窗口。
const GAP: std::time::Duration = std::time::Duration::from_secs(8);

fn sleep_gap() {
    std::thread::sleep(GAP);
}

/// 带重试地取字节。
///
/// 本机实测存在**首次连接偶发失败**（`error sending request`，非白名单拒绝）——
/// 应用日志里同样可见「第 1 次失败 → 300ms 后重试 → 成功」。
/// 生产代码有 `Backoff` 兜这一层，联网冒烟测试也必须有，否则测的是网络抖动而不是代码。
async fn fetch_with_retry<F, Fut>(what: &str, mut call: F) -> Vec<u8>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>, tickerglass_lib::net::NetError>>,
{
    let mut last = String::new();
    for attempt in 1..=4 {
        match call().await {
            Ok(b) => return b,
            Err(e) => {
                // 白名单拒绝是**代码缺陷**，重试无意义，立即失败并给出明确指向
                if matches!(e, tickerglass_lib::net::NetError::Blocked(_)) {
                    panic!("{what} 被白名单拒绝（net.rs 漏了该域名）: {e}");
                }
                last = e.to_string();
                eprintln!("  {what} 第 {attempt}/4 次尝试失败: {last}");
                tokio::time::sleep(std::time::Duration::from_millis(500 * attempt)).await;
            }
        }
    }
    panic!("{what} 重试 4 次仍失败: {last}");
}

async fn search_primary(q: &str) -> Vec<tickerglass_lib::parse::em_search::SearchHit> {
    let f = Fetcher::new().expect("构建 HTTP 客户端");
    let bytes = fetch_with_retry("主源", || f.fetch_em_search(q)).await;
    let text = String::from_utf8_lossy(&bytes);
    parse_searchapi(&text).unwrap_or_else(|e| panic!("主源解析失败: {e}（响应 {} 字节）", bytes.len()))
}

#[tokio::test]
#[ignore = "需联网"]
async fn live_primary_source_is_reachable_and_finds_maotai_by_name() {
    let hits = search_primary("茅台").await;
    assert!(
        hits.iter().any(|h| h.tencent_code == "sh600519"),
        "「茅台」应搜到贵州茅台，实际: {:?}",
        hits.iter().map(|h| &h.name).collect::<Vec<_>>()
    );
}

#[tokio::test]
#[ignore = "需联网"]
async fn live_primary_finds_maotai_by_code() {
    sleep_gap();
    let hits = search_primary("600519").await;
    assert!(hits.iter().any(|h| h.tencent_code == "sh600519"));
}

#[tokio::test]
#[ignore = "需联网"]
async fn live_primary_finds_maotai_by_pinyin_gzmt() {
    sleep_gap();
    let hits = search_primary("GZMT").await;
    assert!(hits.iter().any(|h| h.tencent_code == "sh600519"));
}

/// 拼音首字母：**主源的支持不完整，兜底源才是可靠通道。**
///
/// 2026-08-31 实测差异（同一时刻、同一 token）：
/// - `GZMT`（茅台）→ 主源正常返回
/// - `ysly`（永杉锂业）→ 主源响应**整个缺 QuotationCodeTable 键**，
///   按解析器约定这会判成 `RateLimited` 从而触发降级
///
/// 所以拼音搜索不能只依赖主源。这个用例锁的是"兜底源必须能用拼音查到"，
/// 它是 `ipc::search_symbols` 降级链的最后一环 ——
/// 如果哪天兜底源也不支持拼音了，这里会先炸，而不是等用户报"搜不到"。
#[tokio::test]
#[ignore = "需联网"]
async fn live_fallback_finds_yongshan_by_pinyin_ysly() {
    sleep_gap();
    let bytes = fetch_with_retry("兜底源 ysly", || {
        let f = Fetcher::new().expect("Fetcher");
        async move { f.fetch_codetable_search("ysly").await }
    })
    .await;
    let text = String::from_utf8_lossy(&bytes);
    let hits = parse_codetable(&text).expect("兜底源解析应成功");
    println!(
        "ysly 兜底命中: {:?}",
        hits.iter()
            .map(|h| format!("{} {}", h.tencent_code, h.name))
            .collect::<Vec<_>>()
    );
    assert!(
        hits.iter().any(|h| h.tencent_code == "sh603399"),
        "兜底源应能用拼音首字母 ysly 查到永杉锂业(sh603399)，实际: {hits:?}"
    );
}

/// 大小写不敏感：`YSLY` 与 `ysly` 应得到同一结果。
#[tokio::test]
#[ignore = "需联网"]
async fn live_fallback_pinyin_is_case_insensitive() {
    sleep_gap();
    let bytes = fetch_with_retry("兜底源 YSLY", || {
        let f = Fetcher::new().expect("Fetcher");
        async move { f.fetch_codetable_search("YSLY").await }
    })
    .await;
    let text = String::from_utf8_lossy(&bytes);
    let hits = parse_codetable(&text).expect("兜底源解析应成功");
    assert!(
        hits.iter().any(|h| h.tencent_code == "sh603399"),
        "大写 YSLY 也应查到 sh603399，实际: {hits:?}"
    );
}

#[tokio::test]
#[ignore = "需联网"]
async fn live_primary_finds_shanghai_index_and_flags_it() {
    // 「指数」白名单的联网版断言（S1 的功能回归防线）
    sleep_gap();
    let hits = search_primary("上证指数").await;
    let h = hits
        .iter()
        .find(|h| h.tencent_code == "sh000001")
        .expect("上证指数必须能搜到");
    assert!(h.is_index);
    assert_eq!(h.em_secid.as_deref(), Some("1.000001"));
}

#[tokio::test]
#[ignore = "需联网"]
async fn live_primary_returns_empty_for_garbage_not_an_error() {
    sleep_gap();
    let hits = search_primary("zzzzzzzz").await;
    assert!(hits.is_empty(), "乱码应返 0 条而不是报错");
}

#[tokio::test]
#[ignore = "需联网"]
async fn live_primary_tolerates_200_char_input() {
    // 标准 2 的异常输入要求：超长字符串不崩
    sleep_gap();
    let long = "茅".repeat(200);
    let f = Fetcher::new().unwrap();
    let r = f.fetch_em_search(&long).await;
    assert!(r.is_ok(), "超长输入不该让请求构造失败: {r:?}");
}

#[tokio::test]
#[ignore = "需联网"]
async fn live_fallback_source_is_reachable_and_parses() {
    sleep_gap();
    let f = Fetcher::new().expect("构建 HTTP 客户端");
    let bytes = fetch_with_retry("兜底源", || f.fetch_codetable_search("上证指数")).await;
    let text = String::from_utf8_lossy(&bytes);
    let hits = parse_codetable(&text).expect("兜底源解析失败");
    assert!(
        hits.iter().any(|h| h.tencent_code == "sh000001"),
        "兜底源也应搜到上证指数"
    );
}

/// **必须最后执行**（Evaluator N-5）：本用例故意打到限流，
/// 若排在上面那些用例之前，会把它们一起拖进限流窗口而假失败。
///
/// 命名以 `zz_` 开头 + `--test-threads=1` 下 cargo 按名称字母序执行，
/// 从而保证它排在最后。
#[tokio::test]
#[ignore = "需联网；会主动触发限流，必须最后跑"]
async fn zz_live_rate_limit_is_reported_as_unavailable_not_no_result() {
    let f = Fetcher::new().expect("构建 HTTP 客户端");
    let queries = [
        "茅台", "腾讯", "苹果", "平安", "招商", "宁德", "比亚迪", "中国", "工商", "建设",
        "农业", "交通", "浦发", "兴业", "民生", "光大", "华夏", "南方", "嘉实", "易方达",
    ];

    let mut rate_limited_at: Option<usize> = None;
    let mut last_err = String::new();
    for (i, q) in queries.iter().enumerate() {
        // 无间隔猛打，制造限流
        match f.fetch_em_search(q).await {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                match parse_searchapi(&text) {
                    Ok(_) => {}
                    Err(e) => {
                        last_err = e.to_string();
                        rate_limited_at = Some(i + 1);
                        break;
                    }
                }
            }
            Err(e) => {
                last_err = e.to_string();
                rate_limited_at = Some(i + 1);
                break;
            }
        }
    }

    match rate_limited_at {
        Some(n) => {
            println!("第 {n} 次请求后被限流/失败，判定文案: {last_err}");
            // 核心断言：文案必须是"暂不可用"，**不得**是"无匹配结果"
            assert!(
                last_err.contains("暂不可用") || last_err.contains("请求失败"),
                "限流文案不合格: {last_err}"
            );
            assert!(
                !last_err.contains("无匹配"),
                "限流被误报成无匹配结果: {last_err}"
            );
        }
        None => {
            // 没触发到限流不算失败：接口配额随时段变化，不能让它成为脆弱测试。
            // 「缺 QuotationCodeTable 键 → 判限流」这条判定分支已由
            // tests/em_search_parse.rs 的固化 fixture 单测锁死。
            println!("20 次连打未触发限流（配额随时段变化）。判定逻辑由离线 fixture 单测覆盖。");
        }
    }
}
