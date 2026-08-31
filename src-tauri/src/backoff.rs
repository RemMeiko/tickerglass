//! 短退避重试调度。
//!
//! 存在的原因：轮询循环的失败恢复原本完全依赖 `interval` 的下一个 tick，
//! 于是**首个周期失败 = 首屏至少罚满一整个 tick**（实测首屏 5.486s，其中约 3s 是纯空等）。
//! 这里把「失败后按递增间隔快速重试若干次」抽成与 I/O 无关的纯策略，
//! 供 scheduler 的报价/分时两条链路复用，Sprint 2 的主备源切换也用同一套节律。
//!
//! 设计约束：
//! - 调度表是纯数据（`&'static [Duration]`），可在单测中直接断言，不需要跑异步或造网络失败；
//! - 总退避时长必须留出余量，不能自己把首屏预算吃满（见 `FAST_RETRY` 注释）。

use std::time::Duration;

/// 首个周期用的快速退避表：300ms → 600ms → 1.2s，累计等待 2.1s，最多 4 次尝试。
///
/// 为什么是这三个数：首屏预算 3s，单次请求 connect_timeout 4s / timeout 8s 是上限而非常态
/// （实测正常一轮 ≈1.0s）。2.1s 的累计等待 + 常态请求耗时仍在预算内；
/// 而如果是真断网，`Fetcher` 会快速返回连接错误，退避表跑完也就 2.1s 左右，
/// 不会拖到下一个 tick 之后。
pub const FAST_RETRY: &[Duration] = &[
    Duration::from_millis(300),
    Duration::from_millis(600),
    Duration::from_millis(1200),
];

/// 稳态周期用的退避表：空表 = 不重试，失败就等下一个 tick。
///
/// 首屏之后已有数据在界面上（失败只会让行进入 Stale/Error 态，不会退回占位），
/// 3s 后自然重试足够；此时再叠加重试只会放大请求量并加剧节律抖动。
pub const NO_RETRY: &[Duration] = &[];

/// 一张退避表驱动的重试计数器。
///
/// 用法：每次尝试失败后调用 [`Backoff::next_delay`]，
/// 返回 `Some(d)` 表示「睡 d 之后再试一次」，返回 `None` 表示重试额度用尽。
#[derive(Debug, Clone)]
pub struct Backoff {
    schedule: &'static [Duration],
    attempt: usize,
}

impl Backoff {
    pub fn new(schedule: &'static [Duration]) -> Self {
        Backoff {
            schedule,
            attempt: 0,
        }
    }

    /// 首个周期：快速退避。
    pub fn fast() -> Self {
        Backoff::new(FAST_RETRY)
    }

    /// 稳态周期：不重试。
    pub fn none() -> Self {
        Backoff::new(NO_RETRY)
    }

    /// 已经消耗掉的重试次数（不含第一次尝试）。
    pub fn retries_used(&self) -> usize {
        self.attempt
    }

    /// 允许的最大尝试次数 = 1 次首发 + 退避表长度。
    pub fn max_attempts(&self) -> usize {
        self.schedule.len() + 1
    }

    /// 取下一次重试前的等待时长；额度用尽返回 `None`。
    pub fn next_delay(&mut self) -> Option<Duration> {
        let d = self.schedule.get(self.attempt).copied();
        if d.is_some() {
            self.attempt += 1;
        }
        d
    }

    /// 退避表跑完的累计等待上限，用于自证「不会吃满首屏预算」。
    pub fn total_backoff(&self) -> Duration {
        self.schedule.iter().sum()
    }
}
