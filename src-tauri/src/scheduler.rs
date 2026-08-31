use crate::backoff::Backoff;
use crate::domain::market::Market;
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
// 关于间隔的两条约束仍然成立，改动时必读：
//
// ① 报价走腾讯**批量**接口（一次拿全部自选股），所以间隔与自选股数量无关。
//    2s 是实测安全下限（QUOTE_SECS_MIN）。
//
// ② 分时是**每个标的一个请求**，间隔要乘以标的数量才是真实请求速率。
//    所以它的下限（MINUTE_SECS_MIN=3）比报价高，且美股另有降频 ——
//    见下方 US_MINUTE_EVERY_N_TICKS。

/// 美股分时的降频倍数：每 N 个分时 tick 才取一次美股。
///
/// **这条是限流保护，不是性能优化，不要跟着 minute_secs 一起放开。**
/// 美股分时走东财 `trends2`（腾讯对美股只返 1 个点，画不出线），
/// 而东财是**间隔型限流且按路径独立计数** —— 约每分钟只容一个请求，
/// 连发必败，且探测行为本身会刷新它的计时器。
///
/// 它是**倍数**而不是绝对秒数，所以用户把 minute_secs 调小时美股同步变慢：
/// minute_secs=5 → 美股 30s，minute_secs=3 → 美股 18s。
/// 即使用户设到下限 3s，美股仍是 18s 一次（约 3.3 请求/分钟）——
/// 仍高于东财的容忍线，但远好过不降频的 20 请求/分钟。
/// 若哪天报"美股曲线没了"，先查这里是不是被放开了。
const US_MINUTE_EVERY_N_TICKS: u64 = 6;

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

    // 分时轮询：间隔由 config.refresh.minute_secs 决定（用户可在设置页改），首轮立即执行
    {
        let app = app.clone();
        let state = state.clone();
        tauri::async_runtime::spawn(async move {
            // 同报价链路：记下当前间隔用于比对，改了才重建（见循环内说明）
            let mut cur_secs = state.config().refresh.minute_secs;
            let mut tick = tokio::time::interval(Duration::from_secs(cur_secs));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // 首轮失败的空等代价比报价更大，同样走快速退避。
            // 这条链路在独立任务里，退避耗时不阻塞报价链路的首屏。
            let mut first_paint_done = false;
            // 分时 tick 计数，用于给美股降频（见 US_MINUTE_EVERY_N_TICKS）
            let mut ticks: u64 = 0;
            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = CONFIG_CHANGED.notified() => {
                        log::debug!("[分时] 配置变更 → 立即刷新");
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
                let policy = if first_paint_done {
                    Backoff::none()
                } else {
                    Backoff::fast()
                };
                // 首轮必须带上美股，否则美股要等 30s 才出现第一条曲线。
                // 之后每 N 轮带一次。
                let include_us = !first_paint_done || ticks % US_MINUTE_EVERY_N_TICKS == 0;
                let items = state.config().watchlist;
                if refresh_minutes(&fetcher, &state, &items, policy, include_us).await {
                    first_paint_done = true;
                }
                emit_snapshot(&app, &state);
                ticks = ticks.wrapping_add(1);
            }
        });
    }
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

/// 分时刷新。按市场分派数据源：
/// A 股/指数/港股走腾讯 minute；**美股走东财 trends2**
/// （实测腾讯 `minute/query?code=usAAPL` 只返回 1 个点，画不出折线）。
///
/// 每个标的独立处理，单标的失败不影响其他标的与后续周期。
/// 拿不到序列时**不画曲线**（`minute_available = false`），
/// 绝不用单点/昨收线/插值补假曲线。
///
/// `policy` 每个标的各持一份副本（退避额度不跨标的共享）。
/// 返回值 = 是否至少一个标的拿到了序列。
async fn refresh_minutes(
    fetcher: &Fetcher,
    state: &Arc<AppState>,
    items: &[WatchItem],
    policy: Backoff,
    // 本轮是否包含美股。false 时跳过美股标的（保留它们已有的曲线）。
    // 详见 `US_MINUTE_EVERY_N_TICKS`：东财按路径限流，美股不能跟着 5s。
    include_us: bool,
) -> bool {
    let mut any_ok = false;
    for item in items {
        // 跳过本轮不取的美股。**只是不刷新，不清空** ——
        // 清掉会让美股曲线每 6 轮闪一下（minute_available 转 false 就不画线）
        if item.market == Market::Us && !include_us {
            continue;
        }
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
        any_ok |= result.is_ok();

        let mut snap = state.get();
        if let Some(row) = snap
            .rows
            .iter_mut()
            .find(|r| r.code.eq_ignore_ascii_case(&item.tencent_code))
        {
            match result {
                Ok(series) => {
                    if series.points.is_empty() {
                        // 空序列（盘前 / 全部点落在栅格外）：不画曲线，如实标记
                        log::debug!("[分时] {} 无可用点位，不绘制曲线", item.tencent_code);
                        row.minute_available = false;
                        row.minute = Some(series);
                    } else {
                        let mut series = series;
                        if series.prev_close.is_none() {
                            series.prev_close =
                                row.quote.as_ref().and_then(|q| q.prev_close);
                        }
                        row.minute_available = true;
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
    any_ok
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
