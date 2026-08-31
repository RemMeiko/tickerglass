//! **联网**冒烟测试：5 日分时两条源（腾讯 A股/港股 + 东财美股）。
//!
//! 全部标 `#[ignore]`，`cargo test` 默认不跑，常规套件保持纯离线。
//!
//! 手动运行：
//! ```text
//! cargo test --test live_5day_smoke -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! # 存在理由
//!
//! 离线 fixture 测试能覆盖解析，但覆盖不了两件只在真机上炸的事：
//!
//! 1. **白名单**：`push2his` 若漏进 `net.rs` 白名单，`get_bytes` 会 `Blocked` ——
//!    编译期与离线测试都发现不了，只在用户点「5 日」时炸。
//! 2. **host 与参数的组合**：`push2delay` 会**静默忽略** `ndays`（传 5 也只返 1 天）。
//!    这不是错误响应，是"看起来正常但只有 1 天数据"，唯有实际请求才能验出来。
//!
//! 用户报的「5 日经常获取失败」根因就是腾讯不供美股，故这里对两条源分别断言。

use tickerglass_lib::net::Fetcher;
use tickerglass_lib::parse::em_trends::parse_trends_5day;
use tickerglass_lib::parse::tencent_5day::parse_5day;

const GAP: std::time::Duration = std::time::Duration::from_secs(5);

fn sleep_gap() {
    std::thread::sleep(GAP);
}

/// 清掉代理环境变量，**每个联网用例开头都必须调**。
///
/// 存在理由（2026-08-27 血的教训）：我在带 `export http_proxy=127.0.0.1:7897` 的 shell 里
/// 跑这组测试，`reqwest` 默认会读这些变量，于是 3/3 通过、我据此报了"功能已验证"。
/// 但 **app 自己没有代理** —— 用户点开 5 日/日K 时全是失败的。
/// 我证明的是"经 Clash 时端点可用"，却当成"功能可用"报了出去。同一个错在本项目犯过两次。
///
/// 冒烟测试的意义正是"复现 app 的真实网络环境"，所以这里必须主动把代理摘掉，
/// 让测试环境与 app 一致。宁可测试失败，也不要一个会骗人的绿灯。
fn clear_proxy() {
    for k in [
        "http_proxy",
        "https_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
    ] {
        std::env::remove_var(k);
    }
}

fn ashare_session(hhmm: u16) -> bool {
    (930..=1130).contains(&hhmm) || (1300..=1500).contains(&hhmm)
}

fn us_session(hhmm: u16) -> bool {
    (930..=1600).contains(&hhmm)
}

#[test]
#[ignore]
fn live_tencent_5day_ashare_returns_multiple_days() {
    clear_proxy();
    let rt = tokio::runtime::Runtime::new().expect("建 runtime");
    let bytes = rt
        .block_on(async {
            let f = Fetcher::new().expect("建 Fetcher");
            f.fetch_tencent_5day("sh600519").await
        })
        .expect("腾讯 5 日请求应成功（若为 Blocked 说明白名单漏了域名）");

    let text = tickerglass_lib::parse::gbk::decode_gbk(&bytes);
    let days = parse_5day(&text, "sh600519", ashare_session).expect("A股 5 日应解析成功");
    assert!(days.len() >= 2, "应至少返回 2 个交易日，实际 {}", days.len());
    // 升序断言：真机数据同样必须最早在前，否则前端画反
    let dates: Vec<&str> = days.iter().map(|d| d.date.as_str()).collect();
    let mut sorted = dates.clone();
    sorted.sort_unstable();
    assert_eq!(dates, sorted, "5 日必须升序");
    sleep_gap();
}

#[test]
#[ignore]
fn live_tencent_5day_does_not_serve_us() {
    // 这条是**负向**断言：腾讯这条通道不供美股。
    // 若某天腾讯开始支持了，这条会失败 —— 那时可以考虑把美股切回腾讯（少一个源）。
    clear_proxy();
    let rt = tokio::runtime::Runtime::new().expect("建 runtime");
    let bytes = rt
        .block_on(async {
            let f = Fetcher::new().expect("建 Fetcher");
            f.fetch_tencent_5day("usAAPL").await
        })
        .expect("请求本身应成功（HTTP 200，只是业务 code 为 -1）");

    let text = tickerglass_lib::parse::gbk::decode_gbk(&bytes);
    let r = parse_5day(&text, "usAAPL", us_session);
    assert!(
        r.is_err(),
        "腾讯若开始支持美股 5 日，请重新评估 ipc::fetch_chart 的分流"
    );
    sleep_gap();
}

/// 记录**本机封堵事实**：美股 5 日的唯一多日源 `push2his` 直连不通。
///
/// 这条原先断言"美股 5 日应返 5 个交易日"，它当时之所以通过，是因为我在带
/// `http_proxy` 的 shell 里跑的 —— reqwest 读了代理变量。摘掉代理后必然失败。
///
/// 现在改成**记录现状**而不是断言期望：只要 `push2his` 仍不可达就通过。
/// 若某天它通了（换网络/运营商解封），这条会失败并提醒把美股 5 日恢复成正常路径。
/// 这是有意的"提醒式断言"，不是坏测试。
///
/// 参数化决定（用户 2026-08-27）：暂不做美股，故这里只记录、不追求让它可用。
#[test]
#[ignore]
fn live_em_5day_us_source_is_unreachable_directly() {
    clear_proxy();
    let rt = tokio::runtime::Runtime::new().expect("建 runtime");
    let r = rt.block_on(async {
        let f = Fetcher::new().expect("建 Fetcher");
        f.fetch_em_trends_5day("105.AAPL").await
    });

    match r {
        Err(e) => {
            // 预期路径：连接被重置。记录下来便于日后比对。
            eprintln!("[记录] push2his 仍不可直连: {e}");
        }
        Ok(bytes) => {
            // 通了 —— 那就顺手确认它真能给多日，然后提醒恢复正常路径
            let text = String::from_utf8_lossy(&bytes).to_string();
            let days = parse_trends_5day(&text, us_session).expect("既然通了就应能解析");
            panic!(
                "push2his 现在可直连了（返回 {} 天）—— 请去掉 ipc::fetch_chart 里\
                 「美股 5 日不可用」的兜底提示，恢复正常路径",
                days.len()
            );
        }
    }
    sleep_gap();
}
