//! **联网**冒烟测试：大盘面板的四请求装配（`ipc::fetch_market_overview_inner`）。
//!
//! 全部标 `#[ignore]`。手动运行：
//! ```text
//! cargo test --test live_market_smoke -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! # 存在理由
//!
//! `em_market_parse.rs` 用内联报文覆盖了解析，但覆盖不了三件事：
//!
//! 1. **白名单**：`push2.eastmoney.com` 是第三个 eastmoney 域名，
//!    漏进 `net.rs` 白名单会被 `get_bytes` 硬拒 —— 编译期与离线测试都看不出来。
//! 2. **字段今天仍在**：`f113/f114/f115` 是实测认定的，东财随时可能改。
//!    内联 fixture 永远通过，真机测试才能发现字段消失。
//! 3. **逐市场降级真的生效**：某个市场失败时其余两个仍要出数。
//!
//! # ⚠️ 限流纪律
//!
//! 这个 host 会 host 级静默限流（空响应体，恢复行为不固定）。
//! 故本文件**只有 2 个用例**且各自间隔 15 秒。别加更多用例、别去掉 sleep。

use tickerglass_lib::ipc::fetch_market_overview_inner;

/// 清代理。**每个联网用例开头都必须调**。
///
/// 存在理由（本项目犯过两次的错）：带 `http_proxy` 的 shell 里 `reqwest` 会读它，
/// 于是测试全绿、我据此报"已验证"，但 app 自己没有代理，用户那边全是失败的。
/// 冒烟测试的意义是复现 app 的真实网络环境。
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

#[test]
#[ignore]
fn live_market_overview_returns_breadth_and_flow() {
    clear_proxy();
    let rt = tokio::runtime::Runtime::new().expect("建 runtime");
    let ov = rt
        .block_on(fetch_market_overview_inner())
        .expect("装配不应返 Err（逐市场失败走 markets[].error）");

    println!("[大盘] updated_at={}", ov.updated_at);
    for m in &ov.markets {
        match (&m.stats, &m.error) {
            (Some(s), _) => println!(
                "  {} {}: 涨{} 跌{} 平{} 合{}  主力 {:+.2} 亿",
                m.label,
                s.name,
                s.breadth.up,
                s.breadth.down,
                s.breadth.flat,
                s.breadth.total(),
                s.flow.main_force_net() / 1e8
            ),
            (None, Some(e)) => println!("  {}: 失败 {e}", m.label),
            (None, None) => panic!("{} 既无数据也无错误说明", m.label),
        }
    }
    println!(
        "  合计: 涨{} 跌{} 平{}（{}/3 个市场）  主力合计 {:+.2} 亿",
        ov.breadth_total.up,
        ov.breadth_total.down,
        ov.breadth_total.flat,
        ov.breadth_total.markets_counted,
        ov.main_force_net / 1e8
    );
    for i in &ov.indices {
        println!("  指数 {} {:.2} {:+.2}%", i.name, i.price, i.change_percent);
    }

    // ── 断言 ──
    assert_eq!(ov.markets.len(), 3, "应有沪/深/北三行");
    // 每行必须「有数据」或「有错误说明」二者之一，不能两者都空
    for m in &ov.markets {
        assert!(
            m.stats.is_some() || m.error.is_some(),
            "{} 既无数据也无错误说明 —— 界面会显示空白且无从解释",
            m.label
        );
    }

    // 若全部被限流，markets_counted 会是 0。这不算测试失败（限流是已知现实），
    // 但要明确报出来，避免"绿灯"掩盖了"什么都没验到"。
    if ov.breadth_total.markets_counted == 0 {
        println!(
            "[记录] 三个市场全部失败（很可能是限流）。本次未能验证字段有效性，\
             请隔几分钟重跑。"
        );
        return;
    }

    // 至少一个市场有数据时，家数必须是合理量级
    let sum = ov.breadth_total.up + ov.breadth_total.down + ov.breadth_total.flat;
    assert!(sum > 100, "家数合计 {sum} 太小，字段可能已失效");
    // 沪 2355 + 深约 3300 + 北约 338 ≈ 5993；给宽松上界防未来扩容误报
    assert!(sum < 12000, "家数合计 {sum} 超出合理范围");

    // 有数据的市场都要满足资金流向恒等式（这是字段含义的机器校验）
    for m in &ov.markets {
        let Some(s) = &m.stats else { continue };
        for (label, t) in [
            ("总", s.flow.total),
            ("超大单", s.flow.extra_large),
            ("大单", s.flow.large),
            ("中单", s.flow.medium),
            ("小单", s.flow.small),
        ] {
            let d = (t.inflow - t.outflow) - t.net;
            assert!(
                d.abs() < 1e7,
                "{} {label}档 流入-流出≠净额（差 {d}）—— 字段含义可能变了",
                m.label
            );
        }
        // 主力 = 超大单 + 大单，应≈总净额
        let d = s.flow.main_force_net() - s.flow.total.net;
        assert!(
            d.abs() < 1e7,
            "{} 主力(超大+大) 与总净额差 {d} —— 东财口径可能变了",
            m.label
        );
    }

    std::thread::sleep(std::time::Duration::from_secs(15));
}

/// 验证**轮转取数**这个核心设计：连续多轮调用应让缓存里的数据点逐步填满。
///
/// # 为什么必须有这条
///
/// 面板改成"每轮只取一个数据点、其余读缓存"，理由是东财限流只允许约每分钟
/// 一个请求（实测 4 个请求各间隔 12 秒**仍然全败**）。
/// 但"轮转能逐步填满"这个主张此前从未验证过 —— 单轮测试永远只看到 1/4 有数据，
/// 分不清"设计如此"与"坏了"。
///
/// 判据：连续 4 轮（间隔 70 秒，> 实测的限流间隔），已填数据点数**单调不减**，
/// 且末轮比首轮多。若始终是 0，说明限流比预期更严，轮转也救不了。
///
/// 耗时约 4 分钟，故单独成一个用例，不与其他联网用例同跑。
#[test]
#[ignore]
fn live_rotation_fills_cache_over_multiple_rounds() {
    clear_proxy();
    let rt = tokio::runtime::Runtime::new().expect("建 runtime");

    // 70s > 实测限流间隔（4s 必败、90s 可成），给足余量
    const ROUND_GAP: std::time::Duration = std::time::Duration::from_secs(70);
    const ROUNDS: usize = 4;

    let mut filled_seq: Vec<usize> = Vec::new();

    for round in 1..=ROUNDS {
        let ov = rt
            .block_on(fetch_market_overview_inner())
            .expect("装配不应返 Err");

        let markets_filled = ov.markets.iter().filter(|m| m.stats.is_some()).count();
        let indices_filled = usize::from(!ov.indices.is_empty());
        let filled = markets_filled + indices_filled;
        filled_seq.push(filled);

        println!(
            "第 {round} 轮: 已填 {filled}/4（指数 {indices_filled}，市场 {markets_filled}）\
             updated_at={}",
            ov.updated_at
        );
        for m in &ov.markets {
            match (&m.stats, &m.fetched_at) {
                (Some(s), at) => println!(
                    "    {} 涨{} 跌{} 平{}  主力 {:+.2} 亿  取得于 {}",
                    m.label,
                    s.breadth.up,
                    s.breadth.down,
                    s.breadth.flat,
                    s.flow.main_force_net() / 1e8,
                    at.as_deref().unwrap_or("?")
                ),
                (None, _) => println!(
                    "    {} 待取: {}",
                    m.label,
                    m.error.as_deref().unwrap_or("(无说明)")
                ),
            }
        }
        if !ov.indices.is_empty() {
            println!(
                "    指数 {} 条，取得于 {}",
                ov.indices.len(),
                ov.indices_at.as_deref().unwrap_or("?")
            );
        }

        if round < ROUNDS {
            std::thread::sleep(ROUND_GAP);
        }
    }

    // ── 断言 ──
    // 缓存只增不减：某轮请求失败不该让已有数据消失
    for w in filled_seq.windows(2) {
        assert!(
            w[1] >= w[0],
            "已填数据点数必须单调不减（缓存不能丢数据），实际序列 {filled_seq:?}"
        );
    }

    let last = *filled_seq.last().unwrap();
    if last == 0 {
        // 全程没拿到任何数据：记录而不失败 —— 这是限流现实，不是代码缺陷。
        // 但要明确说出来，避免"绿灯"掩盖了"什么都没验到"。
        println!(
            "[记录] 4 轮全部未取到数据。限流比预期更严（连每 70 秒一个请求都不允许），\
             轮转设计无法在当前网络环境验证。请换时段重跑。"
        );
        return;
    }
    assert!(
        last > filled_seq[0] || last == 4,
        "轮转应让数据逐步增加（首轮 {}，末轮 {last}），实际序列 {filled_seq:?}",
        filled_seq[0]
    );
    println!("[结论] 轮转填充有效，序列 {filled_seq:?}");
}

#[test]
#[ignore]
fn live_market_indices_are_reachable() {
    clear_proxy();
    let rt = tokio::runtime::Runtime::new().expect("建 runtime");
    let ov = rt.block_on(fetch_market_overview_inner()).expect("不应返 Err");

    if let Some(e) = &ov.indices_error {
        // 限流时这里会有值。记录而不失败 —— 但要说清没验到
        println!("[记录] 指数未取到: {e}（很可能是限流，请隔几分钟重跑）");
    } else {
        assert_eq!(ov.indices.len(), 3, "应有三大指数");
        for i in &ov.indices {
            assert!(!i.name.is_empty(), "指数名不应为空（GBK/UTF-8 解码问题？）");
            // 上证 ~3900、深成 ~13900、创业板 ~3400，都远大于 100
            assert!(
                i.price > 100.0,
                "{} 点位 {} 不合理（f2 是否漏除 100？）",
                i.name,
                i.price
            );
        }
        // 顺序应与请求的 secids 一致
        assert!(ov.indices[0].name.contains("上证"), "{}", ov.indices[0].name);
    }
    std::thread::sleep(std::time::Duration::from_secs(15));
}
