//! 大盘面板报文解析。不发网络请求。
//!
//! # fixture 来源说明（重要）
//!
//! 这里的 JSON 是**内联构造**的，但里面的数值全部来自 2026-08-27 盘中对
//! `push2.eastmoney.com` 的真实请求（记录在 `harness/survey-market-panel.md`）。
//!
//! 之所以内联而不是存盘 fixture：抓取过程中把该 host 打进了限流惩罚状态，
//! 反复重试只会延长封禁。而这些数值的**关键性质**（家数和恒定、
//! 资金流向三组恒等式）已经在真机上验证过，内联下来足以锁住解析逻辑。
//!
//! 内联的诚实边界：它验证的是"解析器对这种形状的报文处理正确"，
//! **不验证**"东财今天仍返回这种形状"。后者要靠 `live_*_smoke` 那类联网测试，
//! 待限流解除后补。

mod common;

use common::read_text;
use tickerglass_lib::parse::em_market::{
    parse_indices, parse_market_stats, parse_markets_batch, sum_breadth, sum_main_force_net,
    Breadth, MarketParseError,
};

/// 沪市真实响应（2026-08-27 10:33 一次采样的**完整快照**）。
///
/// 家数 1016/1272/67 和为 2355；`f135`~`f149` 为同一次请求的原始值。
///
/// ⚠️ 必须是**同一时刻**的完整快照，不能把不同分钟的字段拼在一起。
/// 我第一版就是这么错的：`f135/f136/f137` 取自 10:33，`f138`~`f149` 取自 10:5x，
/// 结果 `main_force_equals_extra_large_plus_large` 失败（77.27 亿 vs 74.20 亿，差 3 亿）——
/// 资金流向的恒等式只在同一瞬间成立，跨时刻拼接必然破坏它。
/// 那次失败是测试在正确工作，不是代码有问题。
const SH_REAL: &str = r#"{"rc":0,"rt":4,"svr":183124739,"lt":1,"full":1,"data":{
  "f57":"000001","f58":"上证指数","f43":391886,"f170":17,
  "f113":1016,"f114":1272,"f115":67,
  "f135":208907272192.0,"f136":201487433728.0,"f137":7419838464.0,
  "f138":80250146816.0,"f139":70690471936.0,"f140":9559674880.0,
  "f141":128657125376.0,"f142":130796961792.0,"f143":-2139836416.0,
  "f144":156054958080.0,"f145":163934298112.0,"f146":-7879340032.0,
  "f147":127376384000.0,"f148":126914240512.0,"f149":462143488.0
}}"#;

// ───────── 涨跌家数 ─────────

#[test]
fn parses_breadth_from_real_shanghai_response() {
    let s = parse_market_stats(SH_REAL).expect("真实沪市报文应解析成功");
    assert_eq!(s.name, "上证指数");
    assert_eq!(s.breadth.up, 1016);
    assert_eq!(s.breadth.down, 1272);
    assert_eq!(s.breadth.flat, 67);
    // 和恒定 2355 是认定这三个字段为"家数"的核心依据（见模块文档）
    assert_eq!(s.breadth.total(), 2355, "沪市标的总数应为 2355");
}

#[test]
fn all_zero_breadth_is_rejected_not_reported_as_zero() {
    // f104/f105/f106 那种占位字段返回 0/0/100 的形态。
    // 盘中不可能"一只都没涨跌"，故必须报错而不是显示「涨 0 家」。
    let json = r#"{"rc":0,"data":{"f58":"某指数","f113":0,"f114":0,"f115":0,
      "f135":1.0,"f136":1.0,"f137":0.0,"f138":1.0,"f139":1.0,"f140":0.0,
      "f141":1.0,"f142":1.0,"f143":0.0,"f144":1.0,"f145":1.0,"f146":0.0,
      "f147":1.0,"f148":1.0,"f149":0.0}}"#;
    let err = parse_market_stats(json).expect_err("三项全 0 应报错");
    assert!(matches!(err, MarketParseError::Structure(_)), "{err:?}");
}

#[test]
fn missing_breadth_fields_are_rejected() {
    let json = r#"{"rc":0,"data":{"f58":"某指数","f43":100}}"#;
    assert!(matches!(
        parse_market_stats(json),
        Err(MarketParseError::Structure(_))
    ));
}

// ───────── 资金流向：三组恒等式 ─────────

#[test]
fn each_flow_tier_satisfies_inflow_minus_outflow_equals_net() {
    // 这是认定 f135~f149 含义的依据一，在真机上 5/5 档成立。
    // 若字段映射写错（如把 f141 当成超大单），这条会立刻失败。
    let s = parse_market_stats(SH_REAL).unwrap();
    let f = s.flow;
    for (label, t) in [
        ("总", f.total),
        ("超大单", f.extra_large),
        ("大单", f.large),
        ("中单", f.medium),
        ("小单", f.small),
    ] {
        let diff = (t.inflow - t.outflow) - t.net;
        assert!(
            diff.abs() < 1e6, // 亿元级数值，容差 100 万足够严格
            "{label}档: 流入{} - 流出{} = {} ≠ 净额{}",
            t.inflow,
            t.outflow,
            t.inflow - t.outflow,
            t.net
        );
    }
}

#[test]
fn main_force_equals_extra_large_plus_large() {
    // 依据二：主力 = 超大单 + 大单，且实测等于总净额 f137。
    let s = parse_market_stats(SH_REAL).unwrap();
    let main = s.flow.main_force_net();
    assert!(
        (main - s.flow.total.net).abs() < 1e6,
        "主力(超大{}+大{}={}) 应≈总净额 {}",
        s.flow.extra_large.net,
        s.flow.large.net,
        main,
        s.flow.total.net
    );
}

#[test]
fn four_tier_nets_sum_to_roughly_zero() {
    // 依据三：每笔买必有对应的卖，四档净额之和 ≈ 0（真机实测 -0.01 亿）。
    // 这条能抓出"把某档的流入错当净额"这类映射错误。
    let s = parse_market_stats(SH_REAL).unwrap();
    let f = s.flow;
    let sum = f.extra_large.net + f.large.net + f.medium.net + f.small.net;
    assert!(
        sum.abs() < 5e7,
        "四档净额之和应≈0，实际 {sum}（{:.2} 亿）",
        sum / 1e8
    );
}

#[test]
fn flow_values_are_in_yuan_not_scaled() {
    // f135~f149 是元、不缩放。若误当成 ×100 会把 74 亿显示成 0.74 亿。
    let s = parse_market_stats(SH_REAL).unwrap();
    // 沪市单日主力净流入量级应在亿~百亿之间
    let yi = s.flow.total.net / 1e8;
    assert!(
        (0.1..1000.0).contains(&yi.abs()),
        "主力净流入 {yi:.2} 亿，量级不合理（检查是否误缩放）"
    );
}

// ───────── 限流：必须单独成一种状态 ─────────

#[test]
fn empty_body_is_rate_limited_not_a_parse_error() {
    // push2 限流的特征就是空响应体（无错误码、无 JSON）。
    // 必须能与"解析失败"区分，否则 UI 无法给出"稍后重试"的正确提示。
    for empty in ["", "   ", "\n", "\r\n\t "] {
        assert_eq!(
            parse_market_stats(empty),
            Err(MarketParseError::RateLimited),
            "空响应 {empty:?} 应判为限流"
        );
        assert_eq!(parse_indices(empty), Err(MarketParseError::RateLimited));
    }
}

#[test]
fn rate_limited_message_tells_user_to_wait() {
    let msg = MarketParseError::RateLimited.to_string();
    assert!(msg.contains("限流") || msg.contains("频繁"), "{msg}");
    // 不能把限流说成"没有数据"
    assert!(!msg.contains("无数据"), "限流提示不应暗示数据不存在: {msg}");
}

#[test]
fn nonzero_rc_is_rejected() {
    assert_eq!(
        parse_market_stats(r#"{"rc":100,"data":null}"#),
        Err(MarketParseError::Rejected(100))
    );
}

#[test]
fn html_error_page_is_not_json() {
    assert_eq!(
        parse_market_stats("<html><body>502</body></html>"),
        Err(MarketParseError::NotJson)
    );
}

#[test]
fn null_data_is_structure_error() {
    assert!(matches!(
        parse_market_stats(r#"{"rc":0,"data":null}"#),
        Err(MarketParseError::Structure(_))
    ));
}

// ───────── 跨市场合计 ─────────

#[test]
fn sums_breadth_across_markets() {
    // 实测量级：沪 2355、深约 3300、北约 338，合计约 5993
    let sh = Breadth { up: 1016, down: 1272, flat: 67 };
    let sz = Breadth { up: 1450, down: 1600, flat: 250 };
    let bj = Breadth { up: 136, down: 190, flat: 12 };
    let t = sum_breadth(&[sh, sz, bj]);
    assert_eq!(t.up, 2602);
    assert_eq!(t.down, 3062);
    assert_eq!(t.flat, 329);
    assert_eq!(t.total(), 5993);
    assert_eq!(t.markets_counted, 3);
}

#[test]
fn markets_counted_reveals_partial_data() {
    // 只有 2 个市场成功时 markets_counted=2，前端据此标注"数据不全"。
    // 不标的话用户会把"沪+深"当成全 A 股，少算北交所约 338 只。
    let t = sum_breadth(&[
        Breadth { up: 1016, down: 1272, flat: 67 },
        Breadth { up: 1450, down: 1600, flat: 250 },
    ]);
    assert_eq!(t.markets_counted, 2, "必须能看出只统计了 2 个市场");
    assert!(t.markets_counted < 3);
}

#[test]
fn empty_breadth_list_yields_zero_markets_not_fake_zeros() {
    // 三个市场全失败时：家数是 0 但 markets_counted 也是 0。
    // 前端必须看 markets_counted 而不是看 total()==0 —— 后者分不清
    // "全都没涨跌"（不可能）与"一个市场都没拉到"（限流时的常态）。
    let t = sum_breadth(&[]);
    assert_eq!(t.markets_counted, 0);
    assert_eq!(t.total(), 0);
    assert_eq!(t.up_ratio(), None, "总数为 0 时占比应是 None 而不是 0.0");
}

#[test]
fn up_ratio_is_fraction_not_percent() {
    let t = sum_breadth(&[Breadth { up: 3000, down: 2000, flat: 1000 }]);
    let r = t.up_ratio().expect("有数据时应有占比");
    assert!((r - 0.5).abs() < 1e-9, "3000/6000 应为 0.5，实际 {r}");
}

#[test]
fn sums_main_force_net_across_markets() {
    // 净额可加（同口径）；流入/流出跨市场相加无意义，故 sum 只处理净额
    let sh = parse_market_stats(SH_REAL).unwrap().flow;
    let total = sum_main_force_net(&[sh, sh]);
    assert!(
        (total - sh.main_force_net() * 2.0).abs() < 1.0,
        "两份相同数据的合计应是单份的两倍"
    );
}

#[test]
fn sum_main_force_net_of_empty_is_zero() {
    assert_eq!(sum_main_force_net(&[]), 0.0);
}

// ───────── 批量通道（ulist.np 多 secid）─────────

/// 批量响应：三个市场各带家数与五档资金流向。
/// 数值沿用沪市实测那组（同一时刻的完整快照），深/北按量级构造。
const BATCH_WITH_BREADTH: &str = r#"{"rc":0,"data":{"total":3,"diff":[
  {"f12":"000001","f14":"上证指数","f113":1016,"f114":1272,"f115":67,
   "f135":208907272192.0,"f136":201487433728.0,"f137":7419838464.0,
   "f138":80250146816.0,"f139":70690471936.0,"f140":9559674880.0,
   "f141":128657125376.0,"f142":130796961792.0,"f143":-2139836416.0,
   "f144":156054958080.0,"f145":163934298112.0,"f146":-7879340032.0,
   "f147":127376384000.0,"f148":126914240512.0,"f149":462143488.0},
  {"f12":"399001","f14":"深证成指","f113":1450,"f114":1600,"f115":250,
   "f135":207302000000.0,"f136":197786000000.0,"f137":9516000000.0,
   "f138":73889000000.0,"f139":64197000000.0,"f140":9692000000.0,
   "f141":133413000000.0,"f142":133589000000.0,"f143":-176000000.0,
   "f144":173218000000.0,"f145":180958000000.0,"f146":-7740000000.0,
   "f147":163578000000.0,"f148":165354000000.0,"f149":-1776000000.0},
  {"f12":"899050","f14":"北证50","f113":136,"f114":190,"f115":12,
   "f135":8000000000.0,"f136":7900000000.0,"f137":100000000.0,
   "f138":3000000000.0,"f139":2900000000.0,"f140":100000000.0,
   "f141":2000000000.0,"f142":2000000000.0,"f143":0.0,
   "f144":1600000000.0,"f145":1620000000.0,"f146":-20000000.0,
   "f147":1400000000.0,"f148":1480000000.0,"f149":-80000000.0}
]}}"#;
// ⚠️ 构造这类 fixture 时**每一档都必须自己先算一遍** `流入 − 流出 = 净额`。
// 我第一版把深市中单写成 -7739000000（真值 -7740000000）、小单写成 -1777000000
// （真值 -1776000000）—— 差 0.01 亿，来自把"-77.39 亿"这种已四舍五入的打印值
// 反推回元。测试立刻抓出来了。
// 这与之前那次"混用两个时刻的字段"是同一类错误：**恒等式对数据的要求比肉眼严**。
// 容差 1e6（0.01 亿）不要为了迁就手算误差而放宽 —— 它正是靠这个精度抓出字段错位的。
//
// 另外北证50 那组我同时调了小单，让四档净额之和为 0
// （超大 +1 亿、大 0、中 -0.2 亿、小 -0.8 亿），与真实报文的守恒性一致。

#[test]
fn batch_parses_three_markets_keyed_by_bare_code() {
    let got = parse_markets_batch(BATCH_WITH_BREADTH).expect("应解析成功");
    assert_eq!(got.len(), 3);
    // 键是 f12 的**裸码**，不带市场前缀 —— ipc 侧靠它与 secid 的点号后半段匹配。
    // 若这里返回 "1.000001"，匹配会永远失败而静默退回逐市场（性能白丢且难发现）
    assert_eq!(got[0].0, "000001");
    assert_eq!(got[1].0, "399001");
    assert_eq!(got[2].0, "899050");
    assert_eq!(got[0].1.name, "上证指数");
    assert_eq!(got[0].1.breadth.total(), 2355, "沪市和应为 2355");
    assert_eq!(got[2].1.breadth.total(), 338, "北交所和应为 338");
}

#[test]
fn batch_flow_satisfies_same_identities_as_single() {
    // 批量与单市场取的是同一批字段，恒等式必须同样成立 ——
    // 这条能抓出"批量解析器把档位映射抄错"
    let got = parse_markets_batch(BATCH_WITH_BREADTH).unwrap();
    for (code, s) in &got {
        for (label, t) in [
            ("总", s.flow.total),
            ("超大单", s.flow.extra_large),
            ("大单", s.flow.large),
            ("中单", s.flow.medium),
            ("小单", s.flow.small),
        ] {
            let d = (t.inflow - t.outflow) - t.net;
            assert!(d.abs() < 1e6, "{code} {label}档 流入-流出≠净额（差 {d}）");
        }
        let d = s.flow.main_force_net() - s.flow.total.net;
        assert!(d.abs() < 1e6, "{code} 主力(超大+大) 与总净额差 {d}");
    }
}

#[test]
fn real_ulist_response_carries_no_breadth_so_batch_yields_nothing() {
    // **这条锁住一个已被真机否证的假设。**
    //
    // 我曾推测 `ulist.np/get` 也带 f113/f114/f115 与 f135~f149，若成立就能把面板的
    // 4 个请求压成 1 个（限流是按路径计数的，见 survey-market-panel.md）。
    // 2026-08-27 15:40 实测（fixture 即当次原始响应）证明**不成立**：
    //
    //   f113/f114/f115 全为 0；f140/f141/f146/f147 整个不存在；
    //   f144=1404888 对应深证成指点位 14048.88×100 —— 那是**行情字段**不是资金流向。
    //
    // 即**同一个字段号在 ulist.np 与 stock/get 上含义完全不同**，不可跨路径套用。
    // 所以家数/资金流向只能走 stock/get 逐市场请求。
    //
    // 这条测试的价值：将来若有人（包括我）又想"合并请求省限流"，
    // 它会立刻说明此路不通，省掉一次重复的真机排查。
    let got = parse_markets_batch(&read_text("em_ulist_no_breadth.json"))
        .expect("响应本身是合法 JSON，不该报错");
    assert!(
        got.is_empty(),
        "ulist.np 不提供家数，批量解析应返回空（触发降级），实际 {got:?}"
    );
}

#[test]
fn batch_without_breadth_fields_returns_empty_not_error() {
    // **降级契约**：若 ulist.np 不带 f113/f114/f115，必须返回空 Vec 而不是 Err。
    // ipc 侧据此静默退回逐市场 stock/get。
    // 做成 Err 的话调用方会把它当故障记 error 日志，而这其实是预期分支。
    let json = r#"{"rc":0,"data":{"diff":[
      {"f12":"000001","f14":"上证指数","f2":391911,"f3":17}
    ]}}"#;
    let got = parse_markets_batch(json).expect("不该是 Err");
    assert!(got.is_empty(), "缺家数字段时应返回空，实际 {got:?}");
}

#[test]
fn batch_skips_rows_with_partial_flow_fields() {
    // 只有家数、资金流向缺一半的行必须跳过。
    // 半套数据会让"主力 = 超大 + 大"算出一个看似正常实则错误的值 ——
    // 界面上没有任何迹象表明它是错的，比缺数据危险得多。
    let json = r#"{"rc":0,"data":{"diff":[
      {"f12":"000001","f14":"上证指数","f113":1016,"f114":1272,"f115":67,
       "f135":1.0,"f136":1.0,"f137":0.0,"f138":1.0,"f139":1.0,"f140":0.0}
    ]}}"#;
    let got = parse_markets_batch(json).expect("不该是 Err");
    assert!(got.is_empty(), "资金流向不全的行应跳过，实际 {got:?}");
}

#[test]
fn batch_skips_all_zero_breadth_rows() {
    // 同单市场路径：三项全 0 是占位字段的表现，不是"一只都没涨跌"
    let json = r#"{"rc":0,"data":{"diff":[
      {"f12":"000001","f14":"某指数","f113":0,"f114":0,"f115":0,
       "f135":1.0,"f136":1.0,"f137":0.0,"f138":1.0,"f139":1.0,"f140":0.0,
       "f141":1.0,"f142":1.0,"f143":0.0,"f144":1.0,"f145":1.0,"f146":0.0,
       "f147":1.0,"f148":1.0,"f149":0.0}
    ]}}"#;
    assert!(parse_markets_batch(json).unwrap().is_empty());
}

#[test]
fn batch_keeps_good_rows_when_one_is_bad() {
    // 部分市场有数据时不能因为坏行丢掉好行 —— 好行照样要出来，
    // ipc 侧只对缺失的那个市场退回逐市场请求
    let json = r#"{"rc":0,"data":{"diff":[
      {"f12":"000001","f14":"坏行缺资金","f113":10,"f114":20,"f115":1},
      {"f12":"399001","f14":"好行","f113":1450,"f114":1600,"f115":250,
       "f135":207302000000.0,"f136":197786000000.0,"f137":9516000000.0,
       "f138":73889000000.0,"f139":64197000000.0,"f140":9692000000.0,
       "f141":133413000000.0,"f142":133589000000.0,"f143":-176000000.0,
       "f144":173218000000.0,"f145":180958000000.0,"f146":-7739000000.0,
       "f147":163578000000.0,"f148":165354000000.0,"f149":-1777000000.0}
    ]}}"#;
    let got = parse_markets_batch(json).unwrap();
    assert_eq!(got.len(), 1, "应只保留好行");
    assert_eq!(got[0].0, "399001");
}

#[test]
fn batch_empty_body_is_rate_limited() {
    // 批量通道同样要能区分限流与解析失败
    assert_eq!(
        parse_markets_batch(""),
        Err(MarketParseError::RateLimited)
    );
}

#[test]
fn batch_accepts_object_form_diff() {
    // ulist.np 的 diff 实测两种形态都出现过（数组 / "0","1" 为键的对象）
    let json = r#"{"rc":0,"data":{"diff":{
      "0":{"f12":"000001","f14":"上证指数","f113":1016,"f114":1272,"f115":67,
       "f135":208907272192.0,"f136":201487433728.0,"f137":7419838464.0,
       "f138":80250146816.0,"f139":70690471936.0,"f140":9559674880.0,
       "f141":128657125376.0,"f142":130796961792.0,"f143":-2139836416.0,
       "f144":156054958080.0,"f145":163934298112.0,"f146":-7879340032.0,
       "f147":127376384000.0,"f148":126914240512.0,"f149":462143488.0}
    }}}"#;
    let got = parse_markets_batch(json).expect("对象形态应被接受");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1.breadth.total(), 2355);
}

// ───────── 三大指数 ─────────

/// 三大指数真实响应（2026-08-27 10:33 实测值，diff 为对象形态）。
const INDICES_REAL: &str = r#"{"rc":0,"data":{"total":3,"diff":{
  "0":{"f1":2,"f2":391911,"f3":17,"f4":659,"f6":484798249023.0,"f12":"000001","f14":"上证指数"},
  "1":{"f1":2,"f2":1392052,"f3":57,"f4":7891,"f6":531600000000.0,"f12":"399001","f14":"深证成指"},
  "2":{"f1":2,"f2":344385,"f3":85,"f4":2903,"f6":261600000000.0,"f12":"399006","f14":"创业板指"}
}}}"#;

#[test]
fn parses_three_indices_with_correct_scaling() {
    let idx = parse_indices(INDICES_REAL).expect("应解析成功");
    assert_eq!(idx.len(), 3);
    // f2 现价 ×100：391911 → 3919.11（实测当时上证点位）
    assert!((idx[0].price - 3919.11).abs() < 0.01, "{}", idx[0].price);
    assert_eq!(idx[0].name, "上证指数");
    // f3 涨跌幅 ×100：17 → 0.17%
    assert!((idx[0].change_percent - 0.17).abs() < 0.001, "{}", idx[0].change_percent);
    assert!((idx[1].price - 13920.52).abs() < 0.01, "{}", idx[1].price);
    assert!((idx[2].price - 3443.85).abs() < 0.01, "{}", idx[2].price);
}

#[test]
fn index_order_follows_numeric_key_not_string_order() {
    // diff 键是 "0","1","2"… 按字符串排会让 "10" 跑到 "2" 前面。
    // 这里造 11 条验证按数值排序。
    let mut rows = Vec::new();
    for i in 0..11 {
        rows.push(format!(
            r#""{i}":{{"f2":{},"f3":0,"f4":0,"f12":"c{i}","f14":"idx{i}"}}"#,
            (i + 1) * 10000
        ));
    }
    let json = format!(r#"{{"rc":0,"data":{{"diff":{{{}}}}}}}"#, rows.join(","));
    let idx = parse_indices(&json).expect("应解析成功");
    assert_eq!(idx.len(), 11);
    let names: Vec<&str> = idx.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names[1], "idx1", "第 2 条应是 idx1 而不是 idx10");
    assert_eq!(names[10], "idx10", "idx10 应排在最后");
}

#[test]
fn indices_accept_array_form_of_diff() {
    // 同一接口在不同参数下 diff 既出现过对象也出现过数组，两种都要认
    let json = r#"{"rc":0,"data":{"diff":[
      {"f2":391911,"f3":17,"f4":659,"f12":"000001","f14":"上证指数"}
    ]}}"#;
    let idx = parse_indices(json).expect("数组形态应被接受");
    assert_eq!(idx.len(), 1);
    assert!((idx[0].price - 3919.11).abs() < 0.01);
}

#[test]
fn index_rows_without_price_are_dropped_not_zeroed() {
    // 现价缺失时整条丢弃 —— 显示 0.00 的假指数比少一行更糟
    let json = r#"{"rc":0,"data":{"diff":[
      {"f2":"-","f3":17,"f12":"000001","f14":"坏行"},
      {"f2":391911,"f3":17,"f12":"000002","f14":"好行"}
    ]}}"#;
    let idx = parse_indices(json).expect("应保留可用行");
    assert_eq!(idx.len(), 1);
    assert_eq!(idx[0].name, "好行");
}

#[test]
fn all_index_rows_unusable_reports_error() {
    let json = r#"{"rc":0,"data":{"diff":[{"f2":"-","f12":"x","f14":"坏行"}]}}"#;
    assert!(matches!(
        parse_indices(json),
        Err(MarketParseError::Structure(_))
    ));
}
