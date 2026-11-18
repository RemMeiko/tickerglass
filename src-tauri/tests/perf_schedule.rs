//! v0.4.1 性能优化的纯函数测试：快照语义签名 + 报价休市降频。
//!
//! 背景：旧版每 3s 无条件克隆+序列化+广播全量快照（含全部自选股的当日
//! 分时，~200KB JSON）给所有窗口（含隐藏的），夜间周末也不停。
//! 签名门保证"数据没变就零 IPC"；降频保证"休市时少轮询"。

use tickerglass_lib::domain::change::Change;
use tickerglass_lib::domain::market::Market;
use tickerglass_lib::model::{
    DataState, FundFlow, MinutePoint, MinuteSeries, Quote, RowSnapshot, Snapshot,
};
use tickerglass_lib::scheduler::{effective_quote_secs, snapshot_signature};

fn quote(price: f64) -> Quote {
    Quote {
        code: "sh600519".into(),
        name: "贵州茅台".into(),
        raw_symbol: "600519".into(),
        market: Market::Ashare,
        price: Some(price),
        prev_close: Some(1700.0),
        open: Some(1698.0),
        high: Some(1710.0),
        low: Some(1695.0),
        change: Change {
            amount: Some(price - 1700.0),
            percent: Some((price - 1700.0) / 1700.0 * 100.0),
        },
        price_decimals: Some(2),
        raw_timestamp: "20261118150000".into(),
        quote_time: Some("15:00".into()),
        amount: Some(1.234e10),
    }
}

fn minute(n: usize, last_price: f64) -> MinuteSeries {
    MinuteSeries {
        code: "sh600519".into(),
        market: Market::Ashare,
        grid_len: 241,
        prev_close: Some(1700.0),
        points: (0..n)
            .map(|i| MinutePoint {
                hhmm: 930 + (i as u16 / 60) * 100 + (i as u16 % 60),
                grid_index: i,
                price: if i + 1 == n { last_price } else { 1700.0 + i as f64 },
                avg: Some(1700.0),
                break_before: false,
            })
            .collect(),
    }
}

fn row() -> RowSnapshot {
    RowSnapshot {
        code: "sh600519".into(),
        em_secid: Some("1.600519".into()),
        market: Market::Ashare,
        is_index: false,
        hidden: false,
        name: Some("贵州茅台".into()),
        quote: Some(quote(1705.0)),
        minute: Some(minute(120, 1705.0)),
        state: DataState::Fresh,
        minute_available: true,
        fund_flow: Some(FundFlow {
            main_net: 1.0e8,
            retail_net: -0.8e8,
            medium_net: -0.2e8,
            extra_large_net: 0.6e8,
            large_net: 0.4e8,
        }),
    }
}

fn snap(rows: Vec<RowSnapshot>, updated_at: &str) -> Snapshot {
    Snapshot {
        rows,
        updated_at: updated_at.to_string(),
        quote_ok: true,
    }
}

// ---------- 签名：未变的数据必须得到相同签名 ----------

#[test]
fn sig_identical_content_matches_even_with_different_updated_at() {
    let a = snap(vec![row()], "10:00:00");
    let b = snap(vec![row()], "15:30:45");
    assert_eq!(
        snapshot_signature(&a),
        snapshot_signature(&b),
        "updated_at 只是'数据变了'的结果，不是原因 —— 只有它变了不得触发广播"
    );
}

#[test]
fn sig_equal_semantic_content_matches() {
    // 换入内容完全相同的新对象（克隆），签名必须一致
    let a = snap(vec![row()], "10:00:00");
    let b = snap(vec![row()], "10:00:01");
    assert_eq!(snapshot_signature(&a), snapshot_signature(&b));
}

// ---------- 签名：前端可见的每个字段变了都必须换签名 ----------

fn assert_sig_changes(mutate: impl FnOnce(&mut RowSnapshot), what: &str) {
    let base = snap(vec![row()], "10:00:00");
    let mut r = row();
    mutate(&mut r);
    let changed = snap(vec![r], "10:00:00");
    assert_ne!(
        snapshot_signature(&base),
        snapshot_signature(&changed),
        "{what} 变了签名必须变（否则前端显示旧数据）"
    );
}

#[test]
fn sig_changes_on_visible_fields() {
    assert_sig_changes(|r| r.quote = Some(quote(1706.0)), "现价");
    assert_sig_changes(|r| r.hidden = true, "隐藏标志");
    assert_sig_changes(|r| r.name = Some("改名".into()), "名称");
    assert_sig_changes(
        |r| r.state = DataState::Stale { last_ok_at: "14:59".into(), from_report_time: true },
        "状态 Fresh→Stale",
    );
    assert_sig_changes(
        |r| r.minute = Some(minute(121, 1705.0)),
        "分时新增一点（长度变）",
    );
    assert_sig_changes(
        |r| r.minute = Some(minute(120, 1706.0)),
        "分时末点价变（长度不变）",
    );
    assert_sig_changes(|r| r.minute_available = false, "曲线可用标志");
    assert_sig_changes(
        |r| r.fund_flow = None,
        "资金流清空",
    );
    assert_sig_changes(
        |r| {
            r.fund_flow = Some(FundFlow {
                main_net: 2.0e8,
                ..r.fund_flow.unwrap()
            })
        },
        "主力净流入",
    );
}

// ---------- 报价休市降频 ----------

#[test]
fn quote_secs_idle_floor() {
    assert_eq!(effective_quote_secs(3, true), 3, "盘中用配置值");
    assert_eq!(effective_quote_secs(3, false), 30, "休市钳到 ≥30s");
    assert_eq!(effective_quote_secs(10, false), 30, "休市钳到 ≥30s");
    assert_eq!(effective_quote_secs(60, false), 60, "配置已高于下限则不变");
    assert_eq!(effective_quote_secs(60, true), 60, "盘中用配置值");
}
