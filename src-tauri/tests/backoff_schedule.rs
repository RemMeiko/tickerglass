//! 退避调度表：额度、递增性、总时长预算、稳态不重试。纯策略断言，不发网络请求、不睡眠。

use std::time::Duration;
use tickerglass_lib::backoff::{Backoff, FAST_RETRY, NO_RETRY};

#[test]
fn fast_schedule_is_300_600_1200() {
    // 评审给的修复指引就是这三档，写死断言防止后人无意调大把首屏预算吃回去
    assert_eq!(
        FAST_RETRY,
        &[
            Duration::from_millis(300),
            Duration::from_millis(600),
            Duration::from_millis(1200)
        ]
    );
}

#[test]
fn fast_backoff_yields_each_delay_then_stops() {
    let mut b = Backoff::fast();
    assert_eq!(b.retries_used(), 0);
    assert_eq!(b.next_delay(), Some(Duration::from_millis(300)));
    assert_eq!(b.retries_used(), 1);
    assert_eq!(b.next_delay(), Some(Duration::from_millis(600)));
    assert_eq!(b.next_delay(), Some(Duration::from_millis(1200)));
    assert_eq!(b.retries_used(), 3);
    // 额度用尽后必须持续返回 None，不能绕回去无限重试
    assert_eq!(b.next_delay(), None);
    assert_eq!(b.next_delay(), None);
    assert_eq!(b.retries_used(), 3, "额度用尽后计数不再增长");
}

#[test]
fn delays_are_monotonically_increasing() {
    let mut b = Backoff::fast();
    let mut prev = Duration::ZERO;
    while let Some(d) = b.next_delay() {
        assert!(d > prev, "退避间隔必须递增：{d:?} 应大于 {prev:?}");
        prev = d;
    }
}

#[test]
fn fast_backoff_total_wait_fits_first_paint_budget() {
    let b = Backoff::fast();
    // 首屏预算 3s。退避累计等待必须明显小于它，否则光等待就把标准 12 判负
    assert_eq!(b.total_backoff(), Duration::from_millis(2100));
    assert!(
        b.total_backoff() < Duration::from_secs(3),
        "累计退避 {:?} 不得吃满 3s 首屏预算",
        b.total_backoff()
    );
}

#[test]
fn fast_backoff_allows_four_attempts_total() {
    // 1 次首发 + 3 次重试。这是单个周期内的请求上限，用于界定断网时的请求放大倍数
    assert_eq!(Backoff::fast().max_attempts(), 4);
}

#[test]
fn steady_state_backoff_never_retries() {
    assert_eq!(NO_RETRY, &[] as &[Duration]);
    let mut b = Backoff::none();
    assert_eq!(b.next_delay(), None, "稳态周期失败应直接等下一个 tick");
    assert_eq!(b.max_attempts(), 1);
    assert_eq!(b.total_backoff(), Duration::ZERO);
}

#[test]
fn clone_does_not_share_quota() {
    // refresh_minutes 给每个标的克隆一份 policy，额度必须互相独立
    let mut a = Backoff::fast();
    assert_eq!(a.next_delay(), Some(Duration::from_millis(300)));
    let mut b = a.clone();
    assert_eq!(
        b.next_delay(),
        Some(Duration::from_millis(600)),
        "克隆体应继承已用额度"
    );
    assert_eq!(
        a.next_delay(),
        Some(Duration::from_millis(600)),
        "克隆后原对象的额度不受克隆体消耗影响"
    );
}
