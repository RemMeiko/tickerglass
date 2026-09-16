//! 资金流链路的联网冒烟（--ignored，手动跑）。
//!
//! 走与生产完全一致的路径：`Fetcher::fetch_em_fund_flow`（含 push2 闸门）+
//! `parse_fund_flow`，用当前真实配置里的自选股 secid。用于回答
//! "行内资金流没显示"到底是数据侧（限流/接口）还是 UI 侧。
//!
//! 跑法：`cargo test --test live_fund_flow_smoke -- --ignored --nocapture`

use tickerglass_lib::config::store::load;
use tickerglass_lib::net::Fetcher;
use tickerglass_lib::parse::em_fundflow::parse_fund_flow;

#[tokio::test]
#[ignore = "联网测试，手动跑"]
async fn live_fund_flow_end_to_end() {
    let cfg = load().config;
    let secids: Vec<String> = cfg
        .watchlist
        .iter()
        .filter_map(|w| w.em_secid.clone())
        .collect();
    assert!(!secids.is_empty(), "配置里没有可用 secid");
    println!("[冒烟] {} 个 secid: {:?}", secids.len(), secids);

    let fetcher = Fetcher::new().expect("client");
    let refs: Vec<&str> = secids.iter().map(|s| s.as_str()).collect();
    match fetcher.fetch_em_fund_flow(&refs).await {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes).to_string();
            println!("[冒烟] HTTP OK，{} 字节", text.len());
            match parse_fund_flow(&text) {
                Ok(flows) => {
                    println!("[冒烟] 解析成功 {} 行", flows.len());
                    for (k, f) in flows.iter().take(6) {
                        println!("  {k} 主力 {:.0} 散户 {:.0}", f.main_net, f.retail_net);
                    }
                    // 恒等式：主力 = 超大 + 大（沪深港实测都成立）
                    for (k, f) in &flows {
                        assert!(
                            ((f.extra_large_net + f.large_net) - f.main_net).abs() < 1.0,
                            "{k} 恒等式不成立"
                        );
                    }
                }
                Err(e) => println!("[冒烟] 解析失败: {e}（空响应=限流特征）"),
            }
        }
        Err(e) => println!("[冒烟] 请求失败: {e}"),
    }
}
