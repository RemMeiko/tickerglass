use crate::backoff::Backoff;
use crate::domain::market::Market;
use crate::minute_cache;
use crate::model::{
    DataState, MinuteSeries, Quote, QuoteParseOutcome, RowSnapshot, Snapshot, WatchItem,
};
use crate::net::Fetcher;
use crate::parse::{em_trends, gbk, tencent_minute, tencent_quote};
use crate::state::AppState;
use crate::watchlist::batch_codes;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

// 刷新间隔**不再是常量**（2026-08-31）。默认值与合法区间搬到了
// `config::RefreshConfig`，用户可在设置页调整，两条轮询链路每轮比对配置、
// 变了就重建 tokio interval。
//
// ① 报价走腾讯**批量**接口（一次拿全部自选股），所以间隔与自选股数量无关。
//    2s 是实测安全下限（QUOTE_SECS_MIN）。
//
// ② 分时是**每个标的一个请求**，间隔要乘以标的数量才是真实请求速率。
//    v0.4.0 起分时链路从"每 tick 全量拉所有标的"改为**按需调度**：
//    每标的有新鲜度目标（[`MINUTE_TARGET_SECS`] 等），到期才发请求，
//    每 tick 最多 [`MINUTE_MAX_PER_TICK`] 只，连续失败整链路冷却退避
//    （[`minute_cooldown_secs`]），当日数据另有磁盘缓存（`minute_cache`）。
//    旧形状在 9 只自选股 × 5s tick 下是 ~108 请求/分钟打
//    `web.ifzq.gtimg.cn` 且 24 小时不间断 —— 用户实测挂一段时间后
//    被上游以 HTTP 501 拒绝，失败后仍按原速率硬打，恶性循环。

/// 每 tick 最多取多少只标的的分时。预算上限 —— 自选股再多时把请求摊开，
/// 而不是一 tick 内打爆上游。
const MINUTE_MAX_PER_TICK: usize = 3;
/// 盘中分时新鲜度目标（秒）。分时数据本身是分钟粒度，60s 已是"实时"。
const MINUTE_TARGET_SECS: u64 = 60;
/// 美股分时目标（秒）。走 push2delay（与旧版 US_MINUTE_EVERY_N_TICKS=6
/// 在默认 5s tick 下的实际节奏一致：30s/只）。
const US_MINUTE_TARGET_SECS: u64 = 30;
/// 休市时的目标（秒）。数据不再变化，30 分钟确认一次足够 ——
/// 这同时消灭了旧版"夜里也每 5 秒全量轮询静态数据"的浪费（501 主因之一）。
const IDLE_MINUTE_TARGET_SECS: u64 = 1800;

/// 腾讯分时（A股/指数/港股共用 `web.ifzq.gtimg.cn`）失败冷却表：
/// 第 n 轮连续失败 → 冷却 60→120→240→480→600（封顶）秒。
///
/// 冷却期间**整条腾讯分时链路不发请求**（美股走 push2delay 不受影响），
/// 曲线用内存/磁盘缓存的旧值。任何一次成功即清零。
/// 这是 v0.4.0 对"HTTP 501 后仍按原速率硬打、惩罚被越打越长"的纠正 ——
/// 上游的惩罚靠静默等它过期，继续请求只会延长它。
pub fn minute_cooldown_secs(consecutive_failures: u32) -> u64 {
    if consecutive_failures == 0 {
        return 0;
    }
    let shift = (consecutive_failures - 1).min(4);
    (60u64).saturating_mul(1u64 << shift).min(600)
}

/// 本 tick 要取哪些标的的分时（**纯函数**，调度策略的可测核心）。
///
/// 返回选中的下标（按传入顺序），最多 [`MINUTE_MAX_PER_TICK`] 个：
/// - `ages[i] = None`（本会话从未成功）→ 立即到期；
/// - `ages[i] = Some(距上次成功时长)` ≥ 新鲜度目标 → 到期。
///   目标按市场与交易时段分档：美股盘中 30s、其余盘中 60s、休市一律 30 分钟。
/// - `tencent_cooldown`（腾讯分时域失败冷却中）时跳过所有非美股标的。
pub fn plan_minute_batch(
    items: &[WatchItem],
    ages: &[Option<Duration>],
    in_session: &[bool],
    tencent_cooldown: bool,
) -> Vec<usize> {
    let mut picked: Vec<usize> = Vec::with_capacity(MINUTE_MAX_PER_TICK);
    for (i, item) in items.iter().enumerate() {
        if picked.len() >= MINUTE_MAX_PER_TICK {
            break;
        }
        // 冷却只针对腾讯分时域（web.ifzq.gtimg.cn）；美股走 push2delay，独立限流域
        if tencent_cooldown && item.market != Market::Us {
            continue;
        }
        let target = if in_session.get(i).copied().unwrap_or(false) {
            match item.market {
                Market::Us => US_MINUTE_TARGET_SECS,
                _ => MINUTE_TARGET_SECS,
            }
        } else {
            IDLE_MINUTE_TARGET_SECS
        };
        let due = match ages.get(i).and_then(|a| *a) {
            None => true,
            Some(age) => age.as_secs() >= target,
        };
        if due {
            picked.push(i);
        }
    }
    picked
}

/// 本地时间 `HH:MM:SS`。大盘面板（`ipc::fetch_market_overview`）也用它，
/// 故为 `pub` —— 两处各写一份格式串会导致时间显示风格不一致。
pub fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// 毫秒精度时间戳，仅用于日志自证刷新周期（验收要求周期差落在 3s ± 0.5s）。
fn now_hms_milli() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

/// 首屏骨架：名称占位 + 价格「—」，不等网络。
pub fn initial_snapshot(items: &[WatchItem]) -> Snapshot {
    Snapshot {
        rows: items
            .iter()
            .map(|it| RowSnapshot {
                code: it.tencent_code.clone(),
                em_secid: it.em_secid.clone(),
                market: it.market,
                is_index: it.is_index,
                hidden: it.hidden,
                name: None,
                quote: None,
                minute: None,
                state: DataState::Pending,
                minute_available: false,
                fund_flow: None,
            })
            .collect(),
        updated_at: String::new(),
        quote_ok: false,
    }
}

/// 配置变更信号。`add`/`remove`/`reorder` 后由 ipc 层 `notify_waiters()`，
/// 让两条轮询链路**立即**重跑一轮（不等下一个 3s/30s tick），
/// 这是标准 3「添加后 ≤3s 出价格」的关键。
pub static CONFIG_CHANGED: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// 按当前配置重建首屏骨架，保留已有行的数据。
///
/// 用途：用户增删改序后，快照的行集必须立刻跟上配置（新行显示 `Pending` 占位「—」，
/// 删掉的行立即消失），而不是等下一个网络周期。
pub fn rebuild_snapshot(state: &Arc<AppState>, items: &[WatchItem]) {
    let mut snap = state.get();
    let old: HashMap<String, RowSnapshot> = snap
        .rows
        .drain(..)
        .map(|r| (r.code.to_ascii_lowercase(), r))
        .collect();

    snap.rows = items
        .iter()
        .map(|it| {
            let k = it.tencent_code.to_ascii_lowercase();
            match old.get(&k) {
                // 已有行：原样保留（含价格与曲线），只是顺序可能变了。
                //
                // ⚠️ `hidden` 必须从**配置**覆写，不能沿用克隆来的旧值 ——
                // 用户点"隐藏"走的正是这条路（配置变更 → rebuild_snapshot），
                // 沿用旧值会让隐藏看起来毫无反应，直到下一轮网络刷新才生效。
                Some(existing) => RowSnapshot {
                    hidden: it.hidden,
                    ..existing.clone()
                },
                // 新增行：Pending 占位，前端渲染「—」而不是 0
                None => RowSnapshot {
                    code: it.tencent_code.clone(),
                    em_secid: it.em_secid.clone(),
                    market: it.market,
                    is_index: it.is_index,
                    hidden: it.hidden,
                    name: None,
                    quote: None,
                    minute: None,
                    state: DataState::Pending,
                    minute_available: false,
                    fund_flow: None,
                },
            }
        })
        .collect();
    state.set(snap);
}

pub fn spawn(app: AppHandle) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    // 数据源改读配置快照（D1）：不再直读 watchlist.rs 常量
    let items = state.config().watchlist;
    state.set(initial_snapshot(&items));
    // v0.4.0：预载当日分时缓存 —— 曲线随首屏立即可见，零网络请求
    preload_minute_cache(&state);

    let fetcher = match Fetcher::new() {
        Ok(f) => f,
        Err(e) => {
            log::error!("HTTP 客户端构建失败: {e}");
            return;
        }
    };

    // 报价轮询：间隔由 config.refresh.quote_secs 决定（用户可在设置页改）
    {
        let app = app.clone();
        let state = state.clone();
        let fetcher = fetcher.clone();
        tauri::async_runtime::spawn(async move {
            // 当前生效的间隔。**必须记下来做比对** —— 配置变更时要用它判断
            // 间隔是否真的变了，从而决定要不要重建 interval（见循环内说明）。
            let mut cur_secs = state.config().refresh.quote_secs;
            let mut tick = tokio::time::interval(Duration::from_secs(cur_secs));
            // Burst（默认值）会在慢请求后连补几个即时 tick，制造「长间隔 + 极短间隔」的
            // 补偿对（实测 8 对，min 0.092s）。Delay 让下一个 tick 从本轮结束时刻重新起算，
            // 宁可整体略慢，也不出现挤在一起的两次刷新。
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // 首屏未拿到数据前用快速退避：失败立刻短间隔重试，不空等一整个 tick。
            let mut first_paint_done = false;
            loop {
                // tick 或"配置变更"任一先到就跑（配置变更走立即刷新路径）
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = CONFIG_CHANGED.notified() => {
                        log::debug!("[报价] 配置变更 → 立即刷新");
                    }
                }

                // **间隔改了要重建 interval。**
                // `tokio::time::interval` 的周期是创建时固定的，改配置不会自动生效 ——
                // 不重建的话用户在设置页调了间隔却毫无变化，要重启才生效。
                //
                // 只在值**真的变了**时重建：每轮无条件重建会重置计时起点，
                // 导致 CONFIG_CHANGED 频繁触发时（比如拖动某个滑块）刷新节奏被打乱。
                let want = state.config().refresh.quote_secs;
                if want != cur_secs {
                    log::info!("[报价] 刷新间隔 {cur_secs}s → {want}s");
                    cur_secs = want;
                    tick = tokio::time::interval(Duration::from_secs(cur_secs));
                    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                }
                let policy = if first_paint_done {
                    Backoff::none()
                } else {
                    Backoff::fast()
                };
                // 每轮重读配置：用户增删改序后立刻生效
                let items = state.config().watchlist;
                if refresh_quotes(&fetcher, &state, &items, policy).await {
                    first_paint_done = true;
                }
                // 提醒判定跟着报价周期跑（分时链路不需要，价格只来自报价）
                evaluate_alerts(&app, &state);
                emit_snapshot(&app, &state);
            }
        });
    }

    // 分时轮询（v0.4.0 按需调度：见文件头 ② 与 plan_minute_batch 的文档）
    {
        let app = app.clone();
        let state = state.clone();
        // 资金流链路还要用 fetcher，这里必须 clone 而不是 move（最后一个使用者）
        let fetcher = fetcher.clone();
        tauri::async_runtime::spawn(async move {
            // 同报价链路：记下当前间隔用于比对，改了才重建（见循环内说明）
            let mut cur_secs = state.config().refresh.minute_secs;
            let mut tick = tokio::time::interval(Duration::from_secs(cur_secs));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut first_paint_done = false;
            // ── v0.4.0 调度状态 ──
            // 每标的上次成功时刻（键 = 小写腾讯代码）。"到期"判定见 plan_minute_batch。
            let mut last_ok: HashMap<String, std::time::Instant> = HashMap::new();
            // 每标的上次写缓存的点数 —— 点数没变就不重写盘（盘中每分钟至多一次）
            let mut cached_points: HashMap<String, usize> = HashMap::new();
            // 腾讯分时域失败冷却（见 minute_cooldown_secs）
            let mut cooldown_until: Option<std::time::Instant> = None;
            let mut consecutive_failures: u32 = 0;

            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = CONFIG_CHANGED.notified() => {
                        log::debug!("[分时] 配置变更 → 立即评估");
                    }
                }

                // 间隔改了要重建 interval —— tokio 的 interval 周期在创建时固定，
                // 不重建的话用户改了设置得重启才生效。只在值真变了时重建，
                // 避免 CONFIG_CHANGED 频繁触发时把刷新节奏打乱。
                let want = state.config().refresh.minute_secs;
                if want != cur_secs {
                    log::info!("[分时] 刷新间隔 {cur_secs}s → {want}s");
                    cur_secs = want;
                    tick = tokio::time::interval(Duration::from_secs(cur_secs));
                    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                }

                let items = state.config().watchlist;
                let now_ms = chrono::Utc::now().timestamp_millis();
                let in_session: Vec<bool> = items
                    .iter()
                    .map(|it| crate::domain::trading_session::in_session(it.market, now_ms))
                    .collect();
                let cooling = cooldown_until
                    .map(|t| std::time::Instant::now() < t)
                    .unwrap_or(false);
                let ages: Vec<Option<Duration>> = items
                    .iter()
                    .map(|it| {
                        last_ok
                            .get(&it.tencent_code.to_ascii_lowercase())
                            .map(|t| t.elapsed())
                    })
                    .collect();
                let batch = plan_minute_batch(&items, &ages, &in_session, cooling);
                if batch.is_empty() {
                    // 没有到期标的：本轮零请求（全部新鲜 / 休市宽限 / 冷却中）
                    continue;
                }
                let selected: Vec<WatchItem> =
                    batch.iter().map(|&i| items[i].clone()).collect();
                if log::log_enabled!(log::Level::Debug) {
                    let codes: Vec<&str> =
                        selected.iter().map(|i| i.tencent_code.as_str()).collect();
                    log::debug!("[分时] 本轮取 {} 只: {}", codes.len(), codes.join(","));
                }
                let policy = if first_paint_done {
                    Backoff::none()
                } else {
                    Backoff::fast()
                };
                let outcome = refresh_minutes(&fetcher, &state, &selected, policy, |code, series| {
                    // 成功记账 + 点数变了才写当日缓存
                    let k = code.to_ascii_lowercase();
                    last_ok.insert(k.clone(), std::time::Instant::now());
                    if cached_points.get(&k) != Some(&series.points.len()) {
                        minute_cache::save_series(series);
                        cached_points.insert(k, series.points.len());
                    }
                })
                .await;
                if outcome.any_ok {
                    first_paint_done = true;
                }
                // 冷却记账：只统计腾讯域（美股走 push2delay，独立限流域，不连坐）
                if outcome.tencent_fail > 0 && outcome.tencent_ok == 0 {
                    consecutive_failures += 1;
                    let secs = minute_cooldown_secs(consecutive_failures);
                    cooldown_until =
                        Some(std::time::Instant::now() + Duration::from_secs(secs));
                    log::warn!(
                        "[分时] 腾讯域连续第 {consecutive_failures} 轮失败 → 冷却 {secs}s\
                         （期间不发请求，曲线用已缓存数据）"
                    );
                } else if outcome.tencent_ok > 0 {
                    consecutive_failures = 0;
                    cooldown_until = None;
                }
                emit_snapshot(&app, &state);
            }
        });
    }

    // 资金流轮询（v0.3.0 需求 6）：整批自选股一个 ulist.np 请求。
    //
    // 走 **push2delay**（见 net::fetch_em_fund_flow 的选型说明）：独立限流域，
    // 与 push2 大盘面板互不影响，60s 一轮即可（资金流是分钟级累计量）。
    // 首轮延迟 10s：错开启动时的报价/分时首屏高峰。
    {
        let app = app.clone();
        let state = state.clone();
        let fetcher = fetcher.clone();
        tauri::async_runtime::spawn(async move {
            // 首轮延迟 10s：错开启动时的报价/分时首屏高峰
            tokio::time::sleep(Duration::from_secs(10)).await;
            let mut tick = tokio::time::interval(Duration::from_secs(FLOW_SECS));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // 批次轮转游标：>40 只有 secid 时分多批，每轮只发一批
            let mut batch_cursor: usize = 0;
            loop {
                tick.tick().await;
                let items = state.config().watchlist;
                let secids: Vec<String> = items
                    .iter()
                    .filter_map(|it| it.em_secid.clone())
                    .collect();
                if secids.is_empty() {
                    continue;
                }
                let batches: Vec<Vec<String>> =
                    secids.chunks(FLOW_BATCH_SIZE).map(|c| c.to_vec()).collect();
                let idx = batch_cursor % batches.len();
                batch_cursor = batch_cursor.wrapping_add(1);
                let refs: Vec<&str> = batches[idx].iter().map(|s| s.as_str()).collect();
                if refresh_fund_flows(&fetcher, &state, &items, &refs).await.is_ok() {
                    emit_snapshot(&app, &state);
                } else {
                    // 限流/网络失败都是预期内的常态：保留旧值，下一轮再来。
                    // 不打 warn 刷屏 —— debug 足够
                    log::debug!("[资金流] 第 {}/{} 批本轮未更新", idx + 1, batches.len());
                }
            }
        });
    }
}

/// 资金流轮询间隔（秒）。push2delay 实测可承 ~30s 一次（美股分时链路在用），
/// 60s 对"看主力散户哪边在流入"绰绰有余。
const FLOW_SECS: u64 = 60;
/// 一批最多带多少 secid。与报价批 40 对齐（ulist.np 未实测更大的批量，
/// 超上限的风险由"失败即跳过、保留旧值"兜底）。
const FLOW_BATCH_SIZE: usize = 40;

/// 拉一批 secid 的当日资金流并写进快照。整批一个请求；失败时保留旧值。
///
/// 返回 `Err` 只用于调用方记日志；行级匹配不上的（响应缺行，如美股无数据）
/// 保持 `fund_flow: None`，不清空已有值 —— 与 quote/minute 的"失败不清空"同一纪律。
async fn refresh_fund_flows(
    fetcher: &Fetcher,
    state: &Arc<AppState>,
    items: &[WatchItem],
    secids: &[&str],
) -> Result<(), String> {
    let bytes = fetcher.fetch_em_fund_flow(secids).await.map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let flows = crate::parse::em_fundflow::parse_fund_flow(&text).map_err(|e| e.to_string())?;
    let by_secid: std::collections::HashMap<&str, _> =
        flows.iter().map(|(s, f)| (s.as_str(), f)).collect();

    let mut snap = state.get();
    for item in items {
        let Some(secid) = item.em_secid.as_deref() else {
            continue;
        };
        let Some(flow) = by_secid.get(secid) else {
            continue;
        };
        if let Some(row) = snap
            .rows
            .iter_mut()
            .find(|r| r.code.eq_ignore_ascii_case(&item.tencent_code))
        {
            row.fund_flow = Some(**flow);
        }
    }
    state.set(snap);
    Ok(())
}

fn emit_snapshot(app: &AppHandle, state: &Arc<AppState>) {
    let snap = state.get();
    if let Err(e) = app.emit("snapshot", &snap) {
        log::warn!("快照事件发送失败: {e}");
    }
}

/// 报价刷新。一次请求带全部代码；响应**必须**经 `reconcile` 才能进 state，
/// 否则响应里整行缺失的标的会从界面上消失（实测 `sh999999` 就是整行不出现）。
///
/// `policy` 决定失败后是否原地快速重试（首屏用 `Backoff::fast()`，稳态用 `Backoff::none()`）。
/// 返回值 = 本次是否拿到了可用响应，调用方用它判断首屏是否已完成。
async fn refresh_quotes(
    fetcher: &Fetcher,
    state: &Arc<AppState>,
    items: &[WatchItem],
    policy: Backoff,
) -> bool {
    let all_codes: Vec<String> = items.iter().map(|i| i.tencent_code.clone()).collect();
    // 0 只自选股：**不发请求**（而不是发一个 `q=` 空查询）
    let batches = batch_codes(&all_codes);
    if batches.is_empty() {
        return false;
    }

    // 每批独立请求、独立退避额度。一批失败不影响其他批。
    let mut results: Vec<(Vec<String>, Option<Vec<QuoteParseOutcome>>)> =
        Vec::with_capacity(batches.len());
    let mut any_ok = false;

    for (bi, batch) in batches.iter().enumerate() {
        let mut policy = policy.clone();
        let codes: Vec<&str> = batch.iter().map(|s| s.as_str()).collect();
        let started = std::time::Instant::now();
        let outcomes = loop {
            match fetcher.fetch_quotes(&codes).await {
                Ok(bytes) => {
                    // GBK 解码必须在这里，net 层只给字节
                    let text = gbk::decode_gbk(&bytes);
                    let parsed = tencent_quote::parse_batch(&text);
                    log::debug!(
                        "[报价] {} 第 {}/{} 批（{} 码）用时 {}ms 解析 {} 行（重试 {} 次）",
                        now_hms_milli(),
                        bi + 1,
                        batches.len(),
                        codes.len(),
                        started.elapsed().as_millis(),
                        parsed.len(),
                        policy.retries_used()
                    );
                    // 对账在**批内**做：请求了但响应整行缺失的代码进 NoData
                    break Some(tencent_quote::reconcile(&codes, parsed));
                }
                Err(e) => match policy.next_delay() {
                    Some(d) => {
                        log::warn!(
                            "[报价] {} 第 {}/{} 批失败: {e} — {}ms 后重试（第 {}/{} 次尝试）",
                            now_hms(),
                            bi + 1,
                            batches.len(),
                            d.as_millis(),
                            policy.retries_used() + 1,
                            policy.max_attempts()
                        );
                        tokio::time::sleep(d).await;
                    }
                    None => {
                        log::warn!(
                            "[报价] {} 第 {}/{} 批失败: {e}（该批 {} 个标的本轮进 NoData）",
                            now_hms(),
                            bi + 1,
                            batches.len(),
                            codes.len()
                        );
                        break None;
                    }
                },
            }
        };
        any_ok |= outcomes.is_some();
        results.push((batch.clone(), outcomes));
    }

    // 汇总所有批的结果 → 一次性落 state
    let (by_code, failed_codes) = merge_batch_results(results);

    let mut snap = state.get();
    snap.quote_ok = any_ok;
    for row in snap.rows.iter_mut() {
        let k = row.code.to_ascii_lowercase();
        if failed_codes.contains(&k) {
            // 所属批整批失败：保留上次值 + 标记，行不消失
            mark_stale(row, "网络请求失败");
            continue;
        }
        match by_code.get(&k) {
            Some(QuoteParseOutcome::Ok(q)) => {
                log::debug!(
                    "  {} {} 价 {:?} 涨跌幅 {:?} 报文时间 {}",
                    q.code,
                    q.name,
                    q.price,
                    q.change.percent,
                    q.raw_timestamp
                );
                apply_quote(row, q)
            }
            // NoData / 缺行：保留上次值，只改状态（绝不清成 0）
            Some(_) | None => mark_stale(row, "该标的本次无有效数据"),
        }
    }
    snap.updated_at = now_hms();
    state.set(snap);
    any_ok
}

/// 把各批结果汇总成 `(可用报价索引, 整批失败的代码集)`。
///
/// **纯函数**，独立出来是为了让「不变量 5 的分批延伸」可被机器验证：
/// 整批失败时该批所有代码都必须出现在 `failed_codes` 里，
/// 调用方据此给它们打 `Stale`/`NoData` 标记而**不是把行删掉**。
/// 只靠运行时观察证不了这一条（失败批本来就少见）。
///
/// 代码键统一小写：腾讯响应里的 `v_usAAPL=` 与请求里的大小写可能不一致。
#[allow(clippy::type_complexity)]
pub fn merge_batch_results(
    results: Vec<(Vec<String>, Option<Vec<QuoteParseOutcome>>)>,
) -> (HashMap<String, QuoteParseOutcome>, Vec<String>) {
    let mut by_code: HashMap<String, QuoteParseOutcome> = HashMap::new();
    let mut failed_codes: Vec<String> = Vec::new();
    for (batch, outcomes) in results {
        match outcomes {
            Some(list) => {
                for oc in list {
                    if let Some(c) = oc.code() {
                        by_code.insert(c.to_ascii_lowercase(), oc);
                    }
                }
            }
            // 整批失败：该批**全部**代码进失败集，一个都不能漏
            None => failed_codes.extend(batch.iter().map(|c| c.to_ascii_lowercase())),
        }
    }
    (by_code, failed_codes)
}

fn apply_quote(row: &mut RowSnapshot, q: &Quote) {
    row.name = Some(q.name.clone());
    // 昨收在报价里，分时基准线复用它（trends2 响应不含昨收）
    if let Some(series) = row.minute.as_mut() {
        if series.prev_close.is_none() {
            series.prev_close = q.prev_close;
        }
    }
    row.quote = Some(q.clone());
    row.state = DataState::Fresh;
}

/// 失败时只更新状态，价格与曲线保留上次值。
fn mark_stale(row: &mut RowSnapshot, message: &str) {
    match &row.quote {
        // 有历史值 → 过期标记。过期时间优先用报文时间戳（三格式已在 parse::timestamp 处理），
        // 报文时间不可得时降级为本地接收时间并标注来源
        Some(q) => {
            let (last_ok_at, from_report_time) = match &q.quote_time {
                Some(t) => (t.clone(), true),
                None => (now_hms(), false),
            };
            row.state = DataState::Stale {
                last_ok_at,
                from_report_time,
            };
        }
        // 从未成功过 → 错误态，价格渲染占位「—」而不是 0
        None => {
            row.state = DataState::Error {
                message: message.to_string(),
            }
        }
    }
}

/// 一轮分时批量刷新的结果汇总（调度方据此更新首屏状态与冷却计数）。
#[derive(Debug, Default)]
struct MinuteBatchOutcome {
    any_ok: bool,
    /// 腾讯域（web.ifzq.gtimg.cn：A股/指数/港股）成功数 —— 冷却只看它
    tencent_ok: usize,
    /// 腾讯域失败数（美股走 push2delay，不进冷却统计）
    tencent_fail: usize,
}

/// 分时刷新。按市场分派数据源：
/// A 股/指数/港股走腾讯 minute；**美股走东财 trends2**
/// （实测腾讯 `minute/query?code=usAAPL` 只返回 1 个点，画不出折线）。
///
/// v0.4.0：只刷**传入的这批**（由 [`plan_minute_batch`] 选出），
/// 每次成功经 `on_success` 回调报给调度方（记账 + 写当日缓存）。
/// 每个标的独立处理，单标的失败不影响其他标的与后续周期。
/// 拿不到序列时**不画曲线**（`minute_available = false`），
/// 绝不用单点/昨收线/插值补假曲线。
///
/// `policy` 每个标的各持一份副本（退避额度不跨标的共享）。
async fn refresh_minutes(
    fetcher: &Fetcher,
    state: &Arc<AppState>,
    items: &[WatchItem],
    policy: Backoff,
    mut on_success: impl FnMut(&str, &MinuteSeries),
) -> MinuteBatchOutcome {
    let mut out = MinuteBatchOutcome::default();
    for item in items {
        let mut policy = policy.clone();
        let result = loop {
            let attempt = match item.market {
                Market::Us => fetch_us_minute(fetcher, item).await,
                _ => fetch_tencent_minute(fetcher, item).await,
            };
            match attempt {
                Ok(series) => break Ok(series),
                Err(msg) => match policy.next_delay() {
                    Some(d) => {
                        log::warn!(
                            "[分时] {} 失败: {msg} — {}ms 后重试（第 {}/{} 次尝试）",
                            item.tencent_code,
                            d.as_millis(),
                            policy.retries_used() + 1,
                            policy.max_attempts()
                        );
                        tokio::time::sleep(d).await;
                    }
                    None => break Err(msg),
                },
            }
        };

        // 冷却记账（在动 state 之前完成判定，不依赖快照内容）
        match &result {
            Ok(_) => {
                out.any_ok = true;
                if item.market != Market::Us {
                    out.tencent_ok += 1;
                }
            }
            Err(_) => {
                if item.market != Market::Us {
                    out.tencent_fail += 1;
                }
            }
        }

        let mut snap = state.get();
        if let Some(row) = snap
            .rows
            .iter_mut()
            .find(|r| r.code.eq_ignore_ascii_case(&item.tencent_code))
        {
            match result {
                Ok(series) => {
                    if series.points.is_empty() {
                        // 空序列（盘前 / 全部点落在栅格外）：不画曲线，如实标记。
                        // 通道本身是通的（拿到合法响应），对冷却是"成功"
                        row.minute_available = false;
                        row.minute = Some(series);
                    } else {
                        let mut series = series;
                        if series.prev_close.is_none() {
                            series.prev_close =
                                row.quote.as_ref().and_then(|q| q.prev_close);
                        }
                        row.minute_available = true;
                        on_success(&item.tencent_code, &series);
                        row.minute = Some(series);
                    }
                }
                Err(msg) => {
                    log::warn!("[分时] {} 失败: {msg}", item.tencent_code);
                    // 保留上次曲线，不清空、不伪造
                    if row.minute.is_none() {
                        row.minute_available = false;
                    }
                }
            }
        }
        state.set(snap);
    }
    out
}

/// 启动时预载当日分时缓存（v0.4.0）：曲线随首屏出现，零网络请求。
fn preload_minute_cache(state: &Arc<AppState>) {
    minute_cache::purge_old_days();
    let cached = minute_cache::load_all_today();
    apply_minute_cache(state, &cached);
}

/// 把缓存序列填进快照里**还没有曲线**的行（不覆盖任何已有数据）。
/// 独立成 pub 供单测；缓存行没有 `last_ok` 记录，首轮自然到期、照常走网络刷新。
pub fn apply_minute_cache(state: &Arc<AppState>, cached: &[MinuteSeries]) {
    if cached.is_empty() {
        return;
    }
    let mut snap = state.get();
    let mut filled = 0usize;
    for series in cached {
        if let Some(row) = snap
            .rows
            .iter_mut()
            .find(|r| r.code.eq_ignore_ascii_case(&series.code))
        {
            if row.minute.is_none() {
                row.minute_available = !series.points.is_empty();
                row.minute = Some(series.clone());
                filled += 1;
            }
        }
    }
    if filled > 0 {
        state.set(snap);
        log::info!("[分时] 从当日缓存预载 {filled} 条曲线（零网络）");
    }
}

async fn fetch_tencent_minute(fetcher: &Fetcher, item: &WatchItem) -> Result<MinuteSeries, String> {
    let bytes = fetcher
        .fetch_tencent_minute(&item.tencent_code)
        .await
        .map_err(|e| e.to_string())?;
    // 分时接口是 UTF-8 JSON，但走同一个解码入口不会有害（ASCII 兼容）
    let text = gbk::decode_gbk(&bytes);
    let series = tencent_minute::parse_minute(&text, &item.tencent_code)
        .map_err(|e| e.to_string())?;
    log::debug!(
        "[分时] {} 落格后 {} 点（栅格 {}）",
        item.tencent_code,
        series.points.len(),
        series.grid_len
    );
    Ok(series)
}

async fn fetch_us_minute(fetcher: &Fetcher, item: &WatchItem) -> Result<MinuteSeries, String> {
    // secid 是 Option：拿不到就是拿不到，**不猜前缀**（105/106/107 只有搜索接口的
    // MktNum 是权威来源）。这里如实报错而不是编一个 `105.XXX` 去打一个必然 404 的地址。
    let secid = item
        .em_secid
        .as_deref()
        .ok_or_else(|| format!("{} 无已知东财 secid，东财分时通道不可用", item.tencent_code))?;
    let bytes = fetcher
        .fetch_em_trends(secid)
        .await
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes).to_string();

    // 标准 4 的诊断要求：原始点数与落格后点数都要能看到，
    // 用于区分「通道失败」（原始 0）与「栅格不匹配」（原始 >0 但落格 0）
    let raw_count = em_trends::raw_points(&text).map(|v| v.len()).unwrap_or(0);
    let series = em_trends::parse_trends(&text, &item.tencent_code).map_err(|e| e.to_string())?;
    log::info!(
        "[分时/东财] {} 原始 {} 点 → 落格后 {} 点（栅格 {}）",
        item.tencent_code,
        raw_count,
        series.points.len(),
        series.grid_len
    );
    Ok(series)
}

/// 每个报价周期后判定牛熊提醒，触发则 emit `alert` 事件给前端弹卡片。
///
/// 判定本身是 `domain::alert` 的纯函数；这里只负责组装输入、读写布防状态、发事件。
fn evaluate_alerts(app: &AppHandle, state: &Arc<AppState>) {
    use crate::domain::alert::{evaluate_symbol, AlertInput};
    use crate::domain::trading_session;

    let cfg = state.config();
    if !cfg.alerts.enabled {
        return;
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let now_u64 = now_ms.max(0) as u64;
    let cooldown_ms = cfg.alerts.cooldown_secs.saturating_mul(1000);

    let snap = state.get();
    let mut events = Vec::new();

    for row in &snap.rows {
        let rule = cfg.alerts.effective_rule(&row.code);
        if rule.is_empty() {
            continue;
        }
        // 只有 Fresh 参与判定：过期数据会让断网期间因旧值持续越界而反复提醒
        let fresh = matches!(row.state, crate::model::DataState::Fresh);
        let input = AlertInput {
            code: &row.code,
            price: row.quote.as_ref().and_then(|q| q.price),
            percent: row.quote.as_ref().and_then(|q| q.change.percent),
            fresh,
        };
        let in_session = trading_session::in_session(row.market, now_ms);

        let code = row.code.clone();
        let st_ref = state.clone();
        let code_get = code.clone();
        let mut get = move |k| st_ref.arm_state(&code_get, k);
        let st_ref2 = state.clone();
        let code_set = code.clone();
        let mut set = move |k, v| st_ref2.set_arm_state(&code_set, k, v);

        let fired = evaluate_symbol(
            input,
            &rule,
            &mut get,
            &mut set,
            now_u64,
            cooldown_ms,
            in_session,
        );
        events.extend(fired);
    }

    if events.is_empty() {
        return;
    }
    for e in &events {
        log::info!(
            "[提醒] {} {:?} {:?} 价 {} 阈值 {}",
            e.code,
            e.side,
            e.trigger,
            e.price,
            e.threshold
        );
    }
    // **先唤起弹窗再 emit**：弹窗窗口首次创建要走一次 build()，
    // 期间它还没挂上 listen("alert")。反过来先 emit 的话，
    // 第一次提醒必然丢失（窗口建好时事件已经发完了），
    // 表现为"第一次到价没弹窗、第二次才弹"这种偶发感的 bug。
    if let Err(err) = crate::window::ensure_popup(app) {
        // 弹窗失败不能影响声音与悬浮窗卡片 —— 那两条路是独立的提醒手段
        log::warn!("[提醒] 中心弹窗唤起失败: {err}");
    }
    if let Err(err) = app.emit("alert", &events) {
        log::warn!("提醒事件发送失败: {err}");
    }
}
