use crate::config::store;
use crate::config::{AppConfig, AppearanceConfig};
use crate::model::{Snapshot, WatchItem};
use crate::net::Fetcher;
use crate::parse::em_search::{
    dedup_by_identity, parse_codetable, parse_searchapi, SearchError, SearchHit,
};
use crate::scheduler::{rebuild_snapshot, CONFIG_CHANGED};
use crate::state::AppState;
use crate::watchlist::{self, AddError};
use crate::window;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// 前端首屏调用一次，避免错过启动瞬间的首个 `snapshot` 事件（竞态）。
#[tauri::command]
pub fn get_snapshot(state: tauri::State<'_, Arc<AppState>>) -> Snapshot {
    state.get()
}

#[tauri::command]
pub fn get_config(state: tauri::State<'_, Arc<AppState>>) -> AppConfig {
    state.config()
}

/// 配置损坏/迁移后的一次性提示（标准 19 的可见提示条）。
#[tauri::command]
pub fn get_config_notice(state: tauri::State<'_, Arc<AppState>>) -> Option<String> {
    state.config_notice()
}

#[tauri::command]
pub fn dismiss_config_notice(state: tauri::State<'_, Arc<AppState>>) {
    state.set_config_notice(None);
}

/// 搜索结果条目（前端渲染用）。
#[derive(Debug, Clone, Serialize)]
pub struct SearchItem {
    pub tencent_code: String,
    pub raw_code: String,
    pub name: String,
    pub market_label: String,
    pub em_secid: Option<String>,
    pub is_index: bool,
    /// 是否已在自选股中（前端据此把该项标灰并提示"已添加"）
    pub already_added: bool,
}

/// 搜索结果。**限流与"无匹配结果"必须分开呈现** ——
/// 把限流当成无结果会让用户以为标的不存在。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchResponse {
    Ok { items: Vec<SearchItem> },
    /// 两个源都限流/失败。`message` 是给用户看的"搜索暂不可用"
    Unavailable { message: String },
}

fn to_items(hits: Vec<SearchHit>, current: &[WatchItem]) -> Vec<SearchItem> {
    dedup_by_identity(hits)
        .into_iter()
        .map(|h| SearchItem {
            already_added: watchlist::contains(current, &h.tencent_code),
            tencent_code: h.tencent_code,
            raw_code: h.raw_code,
            name: h.name,
            market_label: h.market_label,
            em_secid: h.em_secid,
            is_index: h.is_index,
        })
        .collect()
}

/// 搜索标的。主源限流/失败时自动切兜底源（两者结构不同，各有独立解析器）。
///
/// 空查询直接返空，不打网络。
#[tauri::command]
pub async fn search_symbols(
    query: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<SearchResponse, String> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(SearchResponse::Ok { items: Vec::new() });
    }
    let current = state.config().watchlist;
    let fetcher = Fetcher::new().map_err(|e| e.to_string())?;

    // 主源
    let primary_err = match fetcher.fetch_em_search(&q).await {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            match parse_searchapi(&text) {
                // ⚠️ **命中为空时不能提前返回，必须继续走兜底源。**
                //
                // 主源（suggest 接口）**不支持拼音首字母**：搜 `ysly` 它返回
                // 合法 JSON + 零条行情（只带一个无关的用户资料），
                // 于是 parse 成功、这里 Ok(vec![]) 直接 return，用户看到"无结果"。
                // 而兜底源（codetable）原生支持拼音缩写且大小写不敏感 ——
                // 实测 `ysly` / `YSLY` 都能查到 603399 永杉锂业。
                //
                // 2026-08-31 修：改成"主源为空 → 视同失败，继续兜底"。
                // 所以拼音搜索**不需要自己实现拼音库**，只需要别拦住降级路径。
                Ok(hits) if !hits.is_empty() => {
                    return Ok(SearchResponse::Ok {
                        items: to_items(hits, &current),
                    })
                }
                Ok(_) => {
                    log::info!("[搜索] 主源零命中(q={q})，切兜底源试拼音/简称");
                    "主源零命中".to_string()
                }
                Err(e) => {
                    // 限流是预期内的降级触发条件，不是 bug
                    log::warn!("[搜索] 主源解析失败({e})，切兜底源");
                    e.to_string()
                }
            }
        }
        Err(e) => {
            log::warn!("[搜索] 主源请求失败({e})，切兜底源");
            e.to_string()
        }
    };

    // 兜底源
    match fetcher.fetch_codetable_search(&q).await {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            match parse_codetable(&text) {
                Ok(hits) => Ok(SearchResponse::Ok {
                    items: to_items(hits, &current),
                }),
                Err(e) => {
                    log::error!("[搜索] 兜底源也失败: {e}（主源: {primary_err}）");
                    Ok(SearchResponse::Unavailable {
                        message: SearchError::RateLimited.to_string(),
                    })
                }
            }
        }
        Err(e) => {
            log::error!("[搜索] 兜底源请求失败: {e}（主源: {primary_err}）");
            Ok(SearchResponse::Unavailable {
                message: SearchError::RateLimited.to_string(),
            })
        }
    }
}

/// 直接输代码的解析结果。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolveResponse {
    /// 至少一个候选真实存在。可能多条（`000001` 同时命中沪指与平安银行）
    Ok { items: Vec<SearchItem> },
    /// 格式就不对（中文、位数不对、hk/us 前缀）——不打网络
    Invalid { message: String },
    /// 格式合法但接口说这些码都不存在
    NotFound { message: String },
    /// 网络/接口故障。**必须与 NotFound 分开** ——
    /// 断网时说"代码不存在"会让用户以为自己记错了代码
    Unavailable { message: String },
}

/// 把用户直接输入的代码解析成可添加的标的（A 股，含存在性校验与真实名称）。
///
/// 与 [`search_symbols`] 的分工：搜索按**名称/拼音**查、要东财搜索接口；
/// 这个按**代码**查、只要报价接口。用户报的"想直接输代码添加"走这条 ——
/// 它不依赖搜索接口，故搜索限流时仍然可用。
///
/// # 为什么要打接口而不是纯本地推导
///
/// 段位表推不出两件事：
/// 1. **该码是否存在**：`sh605888` 段位合法但无此股（实测返 `v_pv_none_match="1"`）。
///    纯本地会让用户加进一只永远显示「—」的死行。
/// 2. **真实名称**：自选股列表要显示名字，本地无从得知。
///
/// 歧义码（`000001`）两个候选一起请求，接口返回哪个就留哪个；
/// 都返回就都给用户，让他自己选沪指还是平安银行。
#[tauri::command]
pub async fn resolve_code(
    input: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<ResolveResponse, String> {
    let current = state.config().watchlist;
    resolve_code_inner(&input, &current).await
}

/// [`resolve_code`] 的真实逻辑。抽出来是为了让联网冒烟测试能直接调 ——
/// `tauri::command` 需要 `State`，测试里造不出来。
///
/// 与 `fetch_chart_inner` 同一个理由：测试必须走用户实际走的那条路，
/// 抄一份副本只能证明副本可用。
pub async fn resolve_code_inner(
    input: &str,
    current: &[WatchItem],
) -> Result<ResolveResponse, String> {
    use crate::domain::code_input::parse_code_input;
    use crate::model::QuoteParseOutcome;

    let candidates = match parse_code_input(input) {
        Ok(c) => c,
        Err(e) => {
            return Ok(ResolveResponse::Invalid {
                message: e.to_string(),
            })
        }
    };

    let fetcher = Fetcher::new().map_err(|e| e.to_string())?;
    let codes: Vec<&str> = candidates.iter().map(|c| c.tencent_code.as_str()).collect();
    // 候选最多 2 个，远低于 40 的批上限，一次请求足够
    let bytes = match fetcher.fetch_quotes(&codes).await {
        Ok(b) => b,
        Err(e) => {
            log::warn!("[直接输码] 报价请求失败: {e}");
            return Ok(ResolveResponse::Unavailable {
                message: format!("网络不可用，无法确认代码是否存在：{e}"),
            });
        }
    };

    let text = crate::parse::gbk::decode_gbk(&bytes);
    let outcomes = crate::parse::tencent_quote::parse_batch(&text);

    let mut items: Vec<SearchItem> = Vec::new();
    for cand in &candidates {
        // 只认解析出 Ok 的那条：NoData（含 pv_none_match）= 此码在该市场不存在
        let hit = outcomes.iter().find_map(|oc| match oc {
            QuoteParseOutcome::Ok(q) if q.code.eq_ignore_ascii_case(&cand.tencent_code) => Some(q),
            _ => None,
        });
        let Some(q) = hit else { continue };
        items.push(SearchItem {
            already_added: watchlist::contains(current, &cand.tencent_code),
            tencent_code: cand.tencent_code.clone(),
            raw_code: q.raw_symbol.clone(),
            name: q.name.clone(),
            market_label: cand.label.to_string(),
            em_secid: Some(cand.em_secid.clone()),
            is_index: cand.is_index,
        });
    }

    if items.is_empty() {
        let tried: Vec<&str> = candidates.iter().map(|c| c.tencent_code.as_str()).collect();
        return Ok(ResolveResponse::NotFound {
            message: format!("查无此代码（已试 {}）", tried.join(" / ")),
        });
    }
    Ok(ResolveResponse::Ok { items })
}

/// 配置落盘 + 推事件 + 触发立即刷新。所有写配置的 command 走这一条出口。
fn commit(app: &AppHandle, state: &Arc<AppState>, config: AppConfig, refresh: bool) {
    state.set_config(config.clone());
    if let Err(e) = store::save(&config) {
        log::error!("[配置] 写盘失败: {e}");
    }
    if let Err(e) = app.emit("config", &config) {
        log::warn!("[配置] 变更事件发送失败: {e}");
    }
    if refresh {
        // 快照的行集立刻跟上配置（新行占位「—」、删掉的行立即消失）
        rebuild_snapshot(state, &config.watchlist);
        let snap = state.get();
        if let Err(e) = app.emit("snapshot", &snap) {
            log::warn!("[配置] 快照事件发送失败: {e}");
        }
        // 不等下一个 3s tick，立即拉一轮（标准 3 的 ≤3s 靠这个）
        CONFIG_CHANGED.notify_waiters();
    }
}

#[tauri::command]
pub fn update_appearance(
    appearance: AppearanceConfig,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> AppConfig {
    let state = state.inner().clone();
    let mut next = state.config();
    next.appearance = appearance;
    // 钳制到合法区间：配置进 state 前永远合法
    next.appearance.sanitize();
    commit(&app, &state, next.clone(), false);
    next
}

/// 更新刷新间隔。
///
/// `commit` 会 `emit("config")` 并通知 `CONFIG_CHANGED`，两条轮询链路
/// 每轮都比对配置里的间隔值，发现变化就重建 tokio interval ——
/// 所以这里不需要额外做"重启调度器"之类的动作，改配置即生效。
#[tauri::command]
pub fn update_refresh(
    refresh: crate::config::RefreshConfig,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> AppConfig {
    let state = state.inner().clone();
    let mut next = state.config();
    next.refresh = refresh;
    // 钳到安全区间：前端只提供合法选项，但 IPC 是可被绕过的入口，
    // 不能信调用方。手写 minute_secs=1 会让请求速率乘以自选股数量打爆上游
    next.refresh.sanitize();
    commit(&app, &state, next.clone(), false);
    next
}

/// 更新日K 均线周期列表（v0.3.0 需求 4）。
///
/// 传什么周期由图表窗口的均线编辑器决定；这里负责钳制（升序去重、
/// 区间 2~250、≤8 条）—— IPC 是可被绕过的入口，不能信调用方。
/// 下次 `fetch_chart("dayk")` 自然按新配置算均线，无需通知图表窗。
#[tauri::command]
pub fn update_ma_periods(
    periods: Vec<u32>,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> AppConfig {
    let state = state.inner().clone();
    let mut next = state.config();
    next.charts.ma_periods = periods;
    next.charts.sanitize();
    commit(&app, &state, next.clone(), false);
    next
}

/// 添加自选股。上限与去重判定在纯函数 `watchlist::add` 里。
///
/// # `market` / `is_index` 一律服务端重算，不信前端传的值
///
/// 前端拿不到可靠的市场信息 —— `WatchlistPanel.handlePick` 只能按 `is_index`
/// 猜 `"indexcn" | "ashare"`，于是**经搜索框添加的港股/美股会被存成 `ashare`**。
/// 这不是显示问题：`market` 决定 `scheduler` 用哪个分时源（美股走
/// `fetch_us_minute`、其余走腾讯）以及提醒按哪个市场的交易时段门控，
/// 存错会让美股行拉不到分时、且在美股盘中被 A 股时段判定为休市。
///
/// 唯一权威来源是 `tencent_code`（市场前缀已内嵌），故这里用 `classify()` 覆写。
#[tauri::command]
pub fn add_watch_item(
    item: WatchItem,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<AppConfig, String> {
    let state = state.inner().clone();
    let item = {
        let market = crate::domain::market::classify(&item.tencent_code);
        WatchItem {
            market,
            is_index: market.is_index(),
            ..item
        }
    };
    // 在写锁内完成"读→判→改"，防止快速连点时两个请求都读到旧列表（标准 23）
    let outcome = state.update_config(|cfg| match watchlist::add(&cfg.watchlist, item.clone()) {
        Ok(next) => {
            cfg.watchlist = next;
            Ok(cfg.clone())
        }
        Err(e) => Err(e),
    });

    match outcome {
        Ok(next) => {
            commit(&app, &state, next.clone(), true);
            Ok(next)
        }
        Err(AddError::LimitReached) => Err(AddError::LimitReached.to_string()),
        Err(AddError::Duplicate) => Err(AddError::Duplicate.to_string()),
    }
}

/// 删除自选股。**参数是 `tencent_code`（唯一键）** ——
/// 传东财裸码会同时命中 `sz000001` 与 `sh000001`，这是明确要防的 bug。
#[tauri::command]
pub fn remove_watch_item(
    tencent_code: String,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> AppConfig {
    let state = state.inner().clone();
    let next = state.update_config(|cfg| {
        let (list, _removed) = watchlist::remove(&cfg.watchlist, &tencent_code);
        cfg.watchlist = list;
        cfg.clone()
    });
    commit(&app, &state, next.clone(), true);
    next
}

/// 设置某只标的的隐藏状态。**隐藏 ≠ 删除。**
///
/// 语义（用户 2026-08-28 选定）：只影响悬浮窗显示，后台照常取数、照常判定提醒。
/// 所以这里只翻一个 bool，不动 watchlist 的成员，也不清任何缓存。
///
/// `refresh: true` 是必需的：它会走 `rebuild_snapshot`，把新的 `hidden`
/// 覆写进快照的对应行（见 `scheduler::rebuild_snapshot` 里那处 `..existing.clone()`），
/// 否则悬浮窗要等下一轮网络刷新才反应，用户会以为点了没用。
#[tauri::command]
pub fn set_watch_hidden(
    tencent_code: String,
    hidden: bool,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> AppConfig {
    let state = state.inner().clone();
    apply_watch_hidden(&app, &state, &tencent_code, hidden)
}

/// 隐藏/取消隐藏的**实际写入路径**。command 与右键菜单 handler 共用这一份。
///
/// 抽出来的理由：`set_watch_hidden` 是 `#[tauri::command]`，参数里有
/// `tauri::State<'_, _>`，只能由 tauri 的 invoke 机制注入 —— 菜单 handler
/// 拿到的是裸 `AppHandle` + `Arc<AppState>`，调不了那个签名。
/// 不抽的话就得把这段写入逻辑（含 `commit(refresh=true)` 那个易漏的参数）复制一份。
pub fn apply_watch_hidden(
    app: &AppHandle,
    state: &Arc<AppState>,
    tencent_code: &str,
    hidden: bool,
) -> AppConfig {
    let next = state.update_config(|cfg| {
        for it in cfg.watchlist.iter_mut() {
            // 大小写不敏感：与 watchlist::key 的规范化一致（usAAPL / USAAPL 同一标的）
            if it.tencent_code.eq_ignore_ascii_case(tencent_code) {
                it.hidden = hidden;
            }
        }
        cfg.clone()
    });
    log::info!("[自选股] {tencent_code} 隐藏 → {hidden}");
    commit(app, state, next.clone(), true);
    next
}

/// 按给定的 `tencent_code` 顺序重排。
#[tauri::command]
pub fn reorder_watchlist(
    order: Vec<String>,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> AppConfig {
    let state = state.inner().clone();
    let next = state.update_config(|cfg| {
        cfg.watchlist = watchlist::reorder(&cfg.watchlist, &order);
        cfg.clone()
    });
    // 顺序变化不需要重新拉数据，但快照行序要跟上
    commit(&app, &state, next.clone(), true);
    next
}

// ---------- 窗口交互 ----------

/// 打开设置窗。
///
/// **必须是 `async fn`**，理由同 [`open_chart_window`]：同步命令跑在主线程上，
/// 一旦走到 `build()`（设置窗被关掉销毁后再开就会走到）就会与事件循环互等死锁。
#[tauri::command]
pub async fn open_settings_window(app: AppHandle) -> Result<(), String> {
    window::open_settings(&app)
}

/// 打开大盘面板窗。
///
/// **必须是 `async fn`**，理由同 [`open_chart_window`]（同步命令 + `build()` = 死锁）。
#[tauri::command]
pub async fn open_market_window(app: AppHandle) -> Result<(), String> {
    window::open_market(&app)
}

/// 悬浮窗右键 → 弹系统菜单。
///
/// 用户反馈「双击空白处很难，空白处很小」——悬浮窗尺寸由内容量出来，
/// 一只自选股时几乎没有空白可点。右键在**任何位置**都能用（含股票行上）。
///
/// 为什么是 `async`：`menu.popup()` 会同步阻塞直到菜单关闭（Windows 上
/// 弹出菜单跑的是模态消息循环）。同步命令占着主线程做这件事，
/// 与建窗死锁是同一类问题 —— 期间整个 UI 无法响应。
/// `row_code` = 右键落在哪只标的的行上（`None` = 落在空白处/整窗）。
/// 传了就在菜单顶部多一项「隐藏 名称」。后端会校验它真在自选股里，前端传错当没传。
#[tauri::command]
pub async fn show_context_menu(
    row_code: Option<String>,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let state = state.inner().clone();
    window::popup_context_menu(&app, &state, row_code.as_deref())
}

#[tauri::command]
pub fn set_always_on_top(
    enabled: bool,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<AppConfig, String> {
    let state = state.inner().clone();
    let win = app
        .get_webview_window(window::MAIN_LABEL)
        .ok_or_else(|| "找不到主窗口".to_string())?;
    win.set_always_on_top(enabled).map_err(|e| e.to_string())?;
    let next = state.update_config(|cfg| {
        cfg.window.always_on_top = enabled;
        cfg.clone()
    });
    commit(&app, &state, next.clone(), false);
    Ok(next)
}

/// 锁定 = 点击穿透 + 不可拖动。
///
/// ⚠️ 锁定态下悬浮窗完全不接受鼠标事件，**解锁的唯一通道是托盘菜单** ——
/// 这是标准 16 的明文要求（缺它会把用户锁死）。
#[tauri::command]
pub fn set_locked(
    locked: bool,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<AppConfig, String> {
    let state = state.inner().clone();
    apply_locked(&app, &state, locked)
}

/// 供 ipc 与托盘菜单共用的落地逻辑。
pub fn apply_locked(
    app: &AppHandle,
    state: &Arc<AppState>,
    locked: bool,
) -> Result<AppConfig, String> {
    let win = app
        .get_webview_window(window::MAIN_LABEL)
        .ok_or_else(|| "找不到主窗口".to_string())?;
    win.set_ignore_cursor_events(locked)
        .map_err(|e| e.to_string())?;
    // 锁定瞬间鼠标事件开始被系统穿透，悬停中的行**永远收不到 mouseleave**
    // —— 资金流提示卡会悬在屏幕上。锁定路径必须顺手收掉（2026-09-16 bug）。
    if locked {
        if let Err(e) = window::hide_flow_tip(app) {
            log::warn!("[窗口] 锁定时收起资金流提示失败: {e}");
        }
    }
    let next = state.update_config(|cfg| {
        cfg.window.locked = locked;
        cfg.clone()
    });
    commit(app, state, next.clone(), false);
    log::info!("[窗口] 锁定状态 → {locked}（解锁通道：托盘菜单）");
    Ok(next)
}

/// 悬浮窗形态切换：球态（单只轮播）↔ 展开列表。
///
/// 这里**不动窗口尺寸** —— 尺寸仍由前端 `ResizeObserver` 测内容后走
/// `resize_main_window` 上报（D4 的单一真源）。后端若也改一次 set_size，
/// 会和前端的上报打架，出现"先跳到球大小、又被列表尺寸顶回去"的抽动。
#[tauri::command]
pub fn set_collapsed(
    collapsed: bool,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<AppConfig, String> {
    let state = state.inner().clone();
    apply_collapsed(&app, &state, collapsed)
}

/// 供 ipc 与右键菜单共用的落地逻辑。
pub fn apply_collapsed(
    app: &AppHandle,
    state: &Arc<AppState>,
    collapsed: bool,
) -> Result<AppConfig, String> {
    let next = state.update_config(|cfg| {
        cfg.window.collapsed = Some(collapsed);
        cfg.clone()
    });
    commit(app, state, next.clone(), false);
    log::info!(
        "[窗口] 形态 → {}",
        if collapsed { "球态" } else { "展开列表" }
    );
    Ok(next)
}

/// 前端测得内容尺寸后上报（D4）。前端已做 ≥100ms 防抖，这里不再节流。
#[tauri::command]
pub fn resize_main_window(width: f64, height: f64, app: AppHandle) -> Result<(), String> {
    window::resize_main(&app, width, height)
}

// ───────────────────────── 大图窗（方案 A） ─────────────────────────

/// 大图窗的一屏数据。三种周期共用一个返回类型，前端按 `kind` 分派渲染。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChartPayload {
    /// 当日分时：复用 Sprint 1 已验证的 `MinuteSeries`（含栅格与午休断点）
    Intraday {
        series: crate::model::MinuteSeries,
    },
    /// 5 日分时：每天一段，各自带昨收基准
    FiveDay {
        days: Vec<crate::parse::tencent_5day::DaySeries>,
        grid_len: usize,
    },
    /// 日K：蜡烛 + 按用户配置的均线组（均线在后端算，前端只画）。
    /// v0.3.0 起周期可配置（`charts.ma_periods`），不再是硬编码的 MA5/20。
    DailyK {
        bars: Vec<crate::parse::em_kline::KlineBar>,
        mas: Vec<MaLine>,
    },
}

/// 一条均线：周期 + 逐根值（前 period-1 根为 `None`）。
#[derive(Debug, Clone, Serialize)]
pub struct MaLine {
    pub period: u32,
    pub values: Vec<Option<f64>>,
}

/// 大图窗拉数据。`period` 取 `"intraday"` / `"5day"` / `"dayk"`。
///
/// 不进 `AppState`：大图是**按需查看**的瞬时数据，塞进全局快照会让每 3s 的
/// 悬浮窗刷新携带大量无关字节。大图窗自己持有返回值即可。
#[tauri::command]
pub async fn fetch_chart(
    code: String,
    period: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<ChartPayload, String> {
    // 真实逻辑在 fetch_chart_inner：命令与排查探针共用同一条路径，
    // 避免"探针能跑但用户点 Tab 仍失败"这种验了等于没验的局面
    fetch_chart_inner(code, period, state.inner()).await
}

async fn fetch_chart_inner(
    code: String,
    period: String,
    state: &Arc<AppState>,
) -> Result<ChartPayload, String> {
    use crate::domain::session;
    use crate::parse::{em_kline, tencent_5day, tencent_minute};

    // 从配置里找这只标的，拿到 market 与 em_secid
    let item: WatchItem = {
        let cfg = state.config();
        cfg.watchlist
            .iter()
            .find(|w| w.tencent_code == code)
            .cloned()
            .ok_or_else(|| format!("{code} 不在自选股中"))?
    };

    let fetcher = Fetcher::new().map_err(|e| format!("HTTP 客户端构建失败: {e}"))?;

    match period.as_str() {
        "intraday" => {
            let bytes = fetcher
                .fetch_tencent_minute(&code)
                .await
                .map_err(|e| format!("当日分时请求失败: {e}"))?;
            let text = crate::parse::gbk::decode_gbk(&bytes);
            let series = tencent_minute::parse_minute(&text, &code)
                .map_err(|e| format!("当日分时解析失败: {e:?}"))?;
            Ok(ChartPayload::Intraday { series })
        }
        "5day" => {
            let market = item.market;
            // 交易时段判定交给 domain 层：落不进栅格的（如 A股盘后 1506~1530）一律丢弃
            let in_session = |hhmm: u16| session::index_of(market, hhmm).is_some();

            // 美股必须走东财：腾讯 5 日通道不供美股数据（实测 param error，
            // 六种代码写法全失败）—— 这正是用户报的「5日经常获取失败」。
            // 详见 net::fetch_tencent_5day / fetch_em_trends_5day 的文档。
            let days = if matches!(market, crate::domain::market::Market::Us) {
                let secid = item.em_secid.as_deref().ok_or_else(|| {
                    format!("{code} 无已知东财 secid，美股 5 日通道不可用")
                })?;
                // 优先 push2his（唯一认 ndays=5 的 host）。它在部分网络环境下直连不通，
                // 那就退到 push2delay —— 后者可直连但**静默忽略 ndays**，只返当日。
                // 与其抛一句看不懂的连接错误，不如把"只有 1 天"这件事说清楚。
                let text = match fetcher.fetch_em_trends_5day(secid).await {
                    Ok(b) => String::from_utf8_lossy(&b).to_string(),
                    Err(primary_err) => {
                        log::warn!(
                            "[5日] push2his 不可达（{primary_err}），退到 push2delay（只有当日）"
                        );
                        let b = fetcher
                            .fetch_em_trends(secid)
                            .await
                            .map_err(|e| format!("5 日分时请求失败: {e}"))?;
                        String::from_utf8_lossy(&b).to_string()
                    }
                };
                let parsed = crate::parse::em_trends::parse_trends_5day(&text, in_session)
                    .map_err(|e| format!("5 日分时解析失败: {e:?}"))?;
                if parsed.len() <= 1 {
                    return Err(format!(
                        "美股 5 日数据当前不可用：本机直连不通 push2his.eastmoney.com（\
                         它是唯一提供多日分时的源），退用的 push2delay 只返当日。\
                         开代理后可恢复。当日分时与日K 不受影响。"
                    ));
                }
                parsed
            } else {
                let bytes = fetcher
                    .fetch_tencent_5day(&code)
                    .await
                    .map_err(|e| format!("5 日分时请求失败: {e}"))?;
                let text = crate::parse::gbk::decode_gbk(&bytes);
                tencent_5day::parse_5day(&text, &code, in_session)
                    .map_err(|e| format!("5 日分时解析失败: {e:?}"))?
            };

            Ok(ChartPayload::FiveDay {
                days,
                grid_len: session::grid_len(market),
            })
        }
        "dayk" => {
            // 日K 走腾讯而非东财：东财历史数据只在 push2his，本机直连不通
            // （详见 parse::tencent_kline 模块文档）。
            // 美股要补交易所后缀，后缀由 em_secid 前缀推导。
            let req_code = crate::net::tencent_kline_code(&code, item.em_secid.as_deref());
            let bytes = fetcher
                .fetch_tencent_kline(&req_code, 120)
                .await
                .map_err(|e| format!("日K 请求失败: {e}"))?;
            let text = crate::parse::gbk::decode_gbk(&bytes);
            let bars = crate::parse::tencent_kline::parse_tencent_kline(&text, &req_code)
                .map_err(|e| format!("日K 解析失败: {e:?}"))?;
            // 均线周期读用户配置（charts.ma_periods，进 state 前已 sanitize：
            // 升序去重、条数≤8），旧配置缺字段时是默认 [5, 20]，观感与 v0.2.x 一致
            let periods = state.config().charts.ma_periods.clone();
            let mas = periods
                .iter()
                .map(|p| MaLine {
                    period: *p,
                    values: em_kline::moving_average(&bars, *p as usize),
                })
                .collect();
            Ok(ChartPayload::DailyK { bars, mas })
        }
        other => Err(format!("未知周期: {other}")),
    }
}

/// 打开大图窗并定位到指定标的。已存在则复用同一个窗口（不叠窗）。
///
/// **必须是 `async fn`。** 写成同步 `fn` 会死锁：同步命令在主线程上执行，而
/// `WebviewWindowBuilder::build()` 需要把建窗任务投递给主线程的事件循环再等回执 ——
/// 主线程正卡在命令里等 build，事件循环收不到派发，build 永远等不到回执。
///
/// 实测症状（2026-08-25）：双击行 → 整个 UI 冻住，托盘菜单点了没反应、退不出，
/// 只能任务管理器杀进程。日志停在「build 开始」没有「build 返回」，
/// 大图窗留下一个 160x28 的空壳停在 (-32000,-32000)，
/// 主窗口的 resize 通道同时死掉（提醒卡片到点不收，窗口卡在 236x240）。
///
/// 为什么设置窗没这个毛病：`settings` 在 `tauri.conf.json` 里声明了
/// （`visible: false` 预建好），`open_settings` 永远走复用分支只调 `show()`，
/// 碰不到 `build()`。而 `chart` 没有声明，首次双击必然走 `build()`。
/// —— 所以这是同步命令 + 运行时建窗的组合才会触发，不是 chart 独有的问题。
#[tauri::command]
pub async fn open_chart_window(
    code: String,
    name: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    window::open_chart(&app, &code, name.as_deref())
}

/// 演示中心弹窗效果（设置页「预览弹窗效果」按钮）。
///
/// **必须是 `async fn`**，理由同 [`open_chart_window`]（同步命令 + `build()` = 死锁）。
///
/// 走的是与真实提醒**完全相同**的两步：`ensure_popup` 唤起窗口 + `emit("alert")`。
/// 分两套实现会出现"演示好看、真提醒时不对"，那种问题只有用户碰到才知道。
///
/// 用第一只自选股的真实名称与现价构造事件；自选股为空时用占位数据 ——
/// 演示按钮不该因为"还没添加自选股"就点了没反应。
#[tauri::command]
pub async fn demo_alert_popup(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    use crate::domain::alert::{AlertEvent, Side, Trigger};

    let snap = state.get();
    // `Quote::price` 是 Option<f64>，用 and_then 展平；
    // 没有任何标的拿到价格时用上证指数占位，保证按钮总有反馈
    let (code, price) = snap
        .rows
        .iter()
        .find_map(|r| {
            r.quote
                .as_ref()
                .and_then(|q| q.price)
                .map(|p| (r.code.clone(), p))
        })
        .unwrap_or_else(|| ("sh000001".to_string(), 3200.00));

    let events = vec![AlertEvent {
        code,
        side: Side::Bull,
        trigger: Trigger::TargetPrice,
        price,
        percent: Some(3.00),
        // 演示用：目标价取现价，读起来就是"刚好到价"
        threshold: price,
    }];

    window::ensure_popup(&app)?;
    app.emit("alert", &events).map_err(|e| e.to_string())?;
    Ok(())
}

// ───────────────────────── 自定义提示音 ─────────────────────────

/// 自定义音频允许的扩展名。
///
/// 只放行 WebView2（Chromium）确定能解码的格式。放行 `.flac`/`.ogg` 这类
/// 会让用户选进来却播不出声，而且**没有任何错误提示** —— 宁可在导入时就拒绝。
const ALLOWED_SOUND_EXT: &[&str] = &["mp3", "wav", "m4a", "aac", "ogg"];

/// 单个音频文件大小上限 5 MB。
///
/// 提示音是几百毫秒的短音，正常都在几十 KB。设上限是防止用户误选一首歌
/// （几 MB 的文件每次提醒都要读盘 + 解码，且配置目录会被撑大）。
const MAX_SOUND_BYTES: usize = 5 * 1024 * 1024;

/// 导入一个自定义提示音。前端读出文件字节后调这个。
///
/// 返回**存盘后的文件名**，前端把它拼成 `custom:文件名` 存进配置。
///
/// # 为什么由前端读字节、后端只管落盘
///
/// 项目没装 `tauri-plugin-dialog`，而 HTML `<input type="file">` 在 webview 里
/// 就能选文件并读出 `ArrayBuffer` —— 零新依赖。代价是文件内容要过一次 IPC，
/// 对几十 KB 的提示音完全可接受。
#[tauri::command]
pub fn import_alert_sound(file_name: String, bytes: Vec<u8>) -> Result<String, String> {
    if bytes.is_empty() {
        return Err("文件为空".to_string());
    }
    if bytes.len() > MAX_SOUND_BYTES {
        return Err(format!(
            "文件过大（{:.1} MB），上限 5 MB。提示音应是短音频",
            bytes.len() as f64 / 1024.0 / 1024.0
        ));
    }

    // 只取文件名部分，丢掉任何路径成分 —— 浏览器一般只给纯文件名，
    // 但不能依赖这一点：拼进路径前必须自己清理（防路径穿越）
    let raw = std::path::Path::new(&file_name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if raw.is_empty() {
        return Err("文件名非法".to_string());
    }

    let ext = std::path::Path::new(&raw)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !ALLOWED_SOUND_EXT.contains(&ext.as_str()) {
        return Err(format!(
            "不支持的格式 .{ext}。可用：{}",
            ALLOWED_SOUND_EXT.join(" / ")
        ));
    }

    // 文件名规范化：只保留 ASCII 字母数字、点、横线、下划线，其余换成 `_`。
    //
    // 中文文件名本身能存，但它要经过 IPC → 配置 JSON → 前端 URL 拼接多层，
    // 任一层编码处理不一致就会变成"文件在但读不到"的怪问题。
    // 统一成安全字符集，代价只是显示名变了（前端会另存原名给用户看）。
    let safe: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    let dir = store::sounds_dir().ok_or_else(|| "拿不到配置目录".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 sounds 目录失败: {e}"))?;
    let path = dir.join(&safe);
    std::fs::write(&path, &bytes).map_err(|e| format!("写入失败: {e}"))?;
    log::info!("[提示音] 已导入 {safe}（{} 字节）", bytes.len());
    Ok(safe)
}

/// 读回一个自定义提示音的字节，供前端播放。
///
/// 前端拿到后转成 Blob URL 交给 `<audio>`。不直接用 `asset://` 协议是因为
/// 那要额外配 CSP 与 `assetProtocol` 作用域，而配置目录在作用域外。
#[tauri::command]
pub fn read_alert_sound(file_name: String) -> Result<Vec<u8>, String> {
    // 再校验一次：这个参数来自配置文件，用户可手改成 `../../something`
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
        return Err("文件名非法".to_string());
    }
    let dir = store::sounds_dir().ok_or_else(|| "拿不到配置目录".to_string())?;
    let path = dir.join(&file_name);
    std::fs::read(&path).map_err(|e| format!("读取 {file_name} 失败: {e}"))
}

/// 列出已导入的自定义提示音（文件名）。设置页用它渲染下拉项。
#[tauri::command]
pub fn list_alert_sounds() -> Vec<String> {
    let Some(dir) = store::sounds_dir() else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        // 目录不存在 = 还没导入过任何音频，不是错误
        return Vec::new();
    };
    let mut out: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .filter(|n| {
            let ext = std::path::Path::new(n)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            ALLOWED_SOUND_EXT.contains(&ext.as_str())
        })
        .collect();
    out.sort();
    out
}

/// 删除一个已导入的提示音。
#[tauri::command]
pub fn delete_alert_sound(file_name: String) -> Result<(), String> {
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
        return Err("文件名非法".to_string());
    }
    let dir = store::sounds_dir().ok_or_else(|| "拿不到配置目录".to_string())?;
    std::fs::remove_file(dir.join(&file_name)).map_err(|e| format!("删除失败: {e}"))
}

// ───────────────────────── 大盘面板 ─────────────────────────

/// 单个市场在面板里的一行。失败也要有一行 —— 见 `error` 字段。
#[derive(Debug, Clone, Serialize)]
pub struct MarketRow {
    /// 展示名（"沪市" / "深市" / "北交所"）
    pub label: String,
    pub secid: String,
    /// 成功时有值
    pub stats: Option<crate::parse::em_market::MarketStats>,
    /// 失败原因。**与 `stats: None` 一起出现，前端逐行显示** ——
    /// 某个市场挂了不该让整个面板空白，也不该假装它的家数是 0
    pub error: Option<String>,
    /// 该市场数据的取得时刻 `HH:MM:SS`。
    ///
    /// **必须逐行显示。** 面板是轮转取数的（每轮只取一个数据点），
    /// 各市场的新鲜度天然不同 —— 不标出来用户会以为所有数字都是同一时刻的，
    /// 那是比数据旧更严重的误导。
    pub fetched_at: Option<String>,
}

/// 大盘面板一屏数据。
#[derive(Debug, Clone, Serialize)]
pub struct MarketOverview {
    /// 三大指数。整体失败时为空 `Vec` + `indices_error`
    pub indices: Vec<crate::parse::em_market::IndexQuote>,
    pub indices_error: Option<String>,
    /// 指数数据的取得时刻。同 `MarketRow::fetched_at` 的理由：轮转下新鲜度不同
    pub indices_at: Option<String>,
    pub markets: Vec<MarketRow>,
    /// 三市场家数合计（只累加成功的市场）
    pub breadth_total: crate::parse::em_market::BreadthTotal,
    /// 主力资金净流入合计（元，只累加成功的市场）
    pub main_force_net: f64,
    /// 快照生成时刻 `HH:MM:SS`
    pub updated_at: String,
}

/// 面板要拉的三个市场。北交所用北证50 的 secid ——
/// 实测它带的家数是**北交所全体**（338 只）而非该指数的 50 只成分股。
const MARKET_SECIDS: [(&str, &str); 3] = [
    ("沪市", "1.000001"),
    ("深市", "0.399001"),
    ("北交所", "0.899050"),
];

/// 三大指数的**腾讯**代码。
///
/// # 为什么指数走腾讯而不是东财
///
/// 用户要的两件事里，「大盘形势（涨跌多少）」不依赖东财：
/// `qt.gtimg.cn` 一个请求就能给齐三大指数，且它是 app 的报价主源 ——
/// 全天实测稳定，从未被限流（东财 `push2` 同期已被封到 TCP connect 超时）。
///
/// 把可靠的部分放到可靠的源上，东财只用来取它独有的家数与资金流向。
/// 这样即使东财完全不可用，面板仍能显示指数（用户至少知道大盘涨跌）。
///
/// 复用 `parse::tencent_quote::parse_batch`（已有 49 项测试覆盖，含 GBK 解码），
/// 不另写解析器。
/// 大盘面板的指数清单（**腾讯**代码）。v0.3.0 需求 5：补科创50 ——
/// 此前只有上证/深成/创业板三条，缺了 2023 年后看科技盘最常用的那条；
/// 腾讯批量接口一次带四条，不增加请求数。
const INDEX_TENCENT_CODES: [&str; 4] = ["sh000001", "sz399001", "sz399006", "sh000688"];

/// 大盘数据的**最后一次成功值**缓存（按 secid）。
///
/// # 为什么面板必须有缓存
///
/// 东财对 `push2` 是**间隔型限流**：请求之间要有足够间隔，连发必败。
/// 实测（详见 `harness/survey-market-panel.md`）：
/// - 静默 90s 后打一个 → ✅；4s 后再打 → ❌；之后连打 5 次全 ❌
/// - 改成 4 个请求间隔 12s → **仍然全败**（连指数那个都失败）
///
/// 即允许频率大约是**每分钟一个请求**量级。面板需要 4 个数据点
/// （指数 + 沪深北三市场），一轮里全取回是不可能的。
///
/// 所以改成：**每轮只取一个数据点（轮转），其余用缓存**。
/// 60s 刷新 → 4 分钟内所有数据点各更新一次。对"看大盘形势"这个用途足够，
/// 而且从第二轮起界面就一直有内容，不会像原来那样整块空白。
///
/// 用 `Mutex` 而不是放进 `AppState`：这是纯展示缓存，与配置/快照无关，
/// 塞进 `AppState` 会让每 3 秒的悬浮窗刷新都带上这些无关字节。
static MARKET_CACHE: std::sync::Mutex<Option<MarketCache>> = std::sync::Mutex::new(None);

/// 缓存内容。每项独立记录取得时刻，前端据此显示"数据时间"。
#[derive(Debug, Clone, Default)]
struct MarketCache {
    indices: Vec<crate::parse::em_market::IndexQuote>,
    indices_at: String,
    /// secid → (数据, 取得时刻)
    markets: std::collections::HashMap<String, (crate::parse::em_market::MarketStats, String)>,
    /// 下一轮该取第几个数据点（0=指数，1..=3 对应三个市场）
    cursor: usize,
}

/// 拉大盘面板数据（**A 股，用户 2026-08-27 决定暂不含美股**）。
///
/// # 请求预算：4 个（1 指数 + 3 市场），**彼此间隔 12 秒**
///
/// 这个数字是硬约束不是随手定的。`push2.eastmoney.com` 会**静默限流**：
/// 几分钟内数百请求后整个 host 返回**空响应体**（无状态码、无错误 JSON），
/// 且恢复行为不是固定窗口 —— 实测 75s 时放行一个请求、随即连发又被立刻掐死
/// （详见 `harness/survey-market-panel.md`）。
///
/// 曾评估过的替代方案与它们的代价：
/// - `clist` 全量枚举统计家数：**60 个请求**（`pz` 被静默截到 100）→ 几次刷新就把用户打进限流
/// - 按 `f3` 降序二分找零点：13 个请求，但跨页排序在页边界处不严格单调（实测两处逆序）
/// - 现方案 `f113/f114/f115`：**1 个请求/市场**，且是服务端同一时刻算出的，无跨请求误差
///
/// 故调用方（前端）必须把刷新间隔设得足够大，别自作聪明去做快速轮询。
#[tauri::command]
pub async fn fetch_market_overview() -> Result<MarketOverview, String> {
    fetch_market_overview_inner().await
}

/// [`fetch_market_overview`] 的真实逻辑。抽出来供联网冒烟测试直接调用 ——
/// `#[tauri::command]` 在集成测试里没法调。同 `fetch_chart_inner` 的理由：
/// 测试必须走用户实际走的那条路，抄一份副本只能证明副本可用。
pub async fn fetch_market_overview_inner() -> Result<MarketOverview, String> {
    use crate::parse::em_market::{
        // parse_indices 已不再用于本函数：指数改走腾讯 parse_batch。
        // 它仍是 pub 且有测试覆盖（东财 ulist 那条通道若将来恢复可直接用）。
        parse_market_stats, sum_breadth, sum_main_force_net, Breadth, MoneyFlow,
    };

    let fetcher = Fetcher::new().map_err(|e| format!("HTTP 客户端构建失败: {e}"))?;
    let now = crate::scheduler::now_hms();

    // ── 本轮只取一个数据点 ──
    //
    // 为什么不一次取全（这是本函数最重要的设计决定）：
    // 东财对 push2 是间隔型限流，允许频率约**每分钟一个请求**。
    // 曾试过"4 个请求各间隔 12 秒"，实测**仍然全败**（连指数那个都失败）。
    // 所以一轮里取回 4 个数据点是不可能的，硬试只会让面板永远空白。
    //
    // 改成轮转：每轮取一个，其余读缓存。60s 刷新 → 4 分钟内全部更新一次。
    // 对"看大盘形势"这个用途足够，且第二轮起界面就一直有内容。
    //
    // ⚠️ 曾试过并已否证的另一条路：用 `ulist.np` 多 secid 一次拿齐三市场。
    // 实测 `ulist.np` 上 `f113/f114/f115` 全为 0、`f140/f141/f146/f147` 不存在，
    // 而 `f144=1404888` 恰是深证成指点位 14048.88×100 —— 那些槽位在这条路径上是
    // **行情字段**，不是家数与资金流向。同一字段号在两条路径含义不同，不可套用。
    // 证据固化在 `tests/fixtures/em_ulist_no_breadth.json`。
    let mut fetch_error: Option<String> = None;

    // ── 指数：每轮都取，走腾讯（不占东财额度）──
    //
    // 指数不参与轮转 —— 它来自 qt.gtimg.cn，与东财是两套限流，互不影响。
    // 这样即使东财完全不可用（今天下午就是这样），用户仍能看到大盘涨跌。
    match fetcher.fetch_quotes(&INDEX_TENCENT_CODES).await {
        Ok(bytes) => {
            // 腾讯报价是 **GBK**，必须走 decode_gbk；用 from_utf8_lossy 会得到乱码指数名
            let text = crate::parse::gbk::decode_gbk(&bytes);
            let quotes = crate::parse::tencent_quote::parse_batch(&text);
            let mut list: Vec<crate::parse::em_market::IndexQuote> = Vec::new();
            for oc in &quotes {
                if let crate::model::QuoteParseOutcome::Ok(q) = oc {
                    let Some(price) = q.price else { continue };
                    list.push(crate::parse::em_market::IndexQuote {
                        code: q.code.clone(),
                        name: q.name.clone(),
                        // 腾讯给的已是**真实点位**，不像东财那样 ×100，故不除
                        price,
                        change_percent: q.change.percent.unwrap_or(0.0),
                        change_amount: q.change.amount.unwrap_or(0.0),
                        // 成交额来自报价报文 [38]（万元→元）。v0.3.0 需求 5：
                        // 此前恒为 None，指数卡一直缺"成交"这一行
                        amount: q.amount,
                    });
                }
            }
            if list.is_empty() {
                log::warn!("[大盘] 腾讯指数解析后为空");
                fetch_error = Some("指数解析后为空".to_string());
            } else {
                let mut guard = MARKET_CACHE.lock().unwrap_or_else(|p| p.into_inner());
                let c = guard.get_or_insert_with(MarketCache::default);
                c.indices = list;
                c.indices_at = now.clone();
            }
        }
        Err(e) => {
            log::warn!("[大盘] 腾讯指数请求失败: {e}");
            fetch_error = Some(format!("指数请求失败: {e}"));
        }
    }

    // ── 家数与资金流向：每轮只取一个市场（轮转），走东财 ──
    let cursor = {
        let guard = MARKET_CACHE.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().map(|c| c.cursor).unwrap_or(0)
    };
    let (label, secid) = MARKET_SECIDS[cursor % MARKET_SECIDS.len()];
    match fetcher.fetch_em_market_stats(secid).await {
        Ok(bytes) => {
            // push2 返回 UTF-8 JSON（不是 GBK）—— 与上面腾讯那条链路不同
            let text = String::from_utf8_lossy(&bytes);
            match parse_market_stats(&text) {
                Ok(stats) => {
                    let mut guard = MARKET_CACHE.lock().unwrap_or_else(|p| p.into_inner());
                    let c = guard.get_or_insert_with(MarketCache::default);
                    c.markets.insert(secid.to_string(), (stats, now.clone()));
                }
                Err(e) => {
                    log::warn!("[大盘] {label} 解析失败: {e}");
                    fetch_error = Some(format!("{label}: {e}"));
                }
            }
        }
        Err(e) => {
            log::warn!("[大盘] {label} 请求失败: {e}");
            fetch_error = Some(format!("{label}请求失败: {e}"));
        }
    }

    // 游标推进：**无论成功失败都推进**。卡在失败的那一格上会让其余市场
    // 永远不更新 —— 一个暂时取不到的市场不该阻塞另外两个。
    {
        let mut guard = MARKET_CACHE.lock().unwrap_or_else(|p| p.into_inner());
        let c = guard.get_or_insert_with(MarketCache::default);
        c.cursor = (cursor + 1) % MARKET_SECIDS.len();
    }

    // ── 用缓存装配返回值 ──
    let guard = MARKET_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let cache = guard.clone().unwrap_or_default();
    drop(guard);

    let mut rows: Vec<MarketRow> = Vec::with_capacity(MARKET_SECIDS.len());
    let mut breadths: Vec<Breadth> = Vec::new();
    let mut flows: Vec<MoneyFlow> = Vec::new();

    for (label, secid) in MARKET_SECIDS {
        match cache.markets.get(secid) {
            Some((stats, at)) => {
                breadths.push(stats.breadth);
                flows.push(stats.flow);
                rows.push(MarketRow {
                    label: label.to_string(),
                    secid: secid.to_string(),
                    stats: Some(stats.clone()),
                    // 带上数据时刻：轮转下各市场的新鲜度不同，
                    // 不标出来用户会以为所有数字都是同一时刻的
                    error: None,
                    fetched_at: Some(at.clone()),
                });
            }
            None => rows.push(MarketRow {
                label: label.to_string(),
                secid: secid.to_string(),
                stats: None,
                // 尚未轮到 / 上次失败。说清"排队中"而不是让用户以为坏了
                error: Some(match &fetch_error {
                    Some(e) if e.starts_with(label) => e.clone(),
                    _ => "等待取数（每轮只取一个，约 4 分钟轮完一遍）".to_string(),
                }),
                fetched_at: None,
            }),
        }
    }

    Ok(MarketOverview {
        indices: cache.indices.clone(),
        // 指数每轮都取（走腾讯），所以空就是真的失败 ——
        // 不能像市场那样说"等待轮到"，那会让用户白等一个永远不来的轮次
        indices_error: if cache.indices.is_empty() {
            Some(
                fetch_error
                    .clone()
                    .unwrap_or_else(|| "指数暂不可用".to_string()),
            )
        } else {
            None
        },
        indices_at: if cache.indices_at.is_empty() {
            None
        } else {
            Some(cache.indices_at.clone())
        },
        markets: rows,
        // 只累加有数据的市场；markets_counted < 3 时前端标注数据不全
        breadth_total: sum_breadth(&breadths),
        main_force_net: sum_main_force_net(&flows),
        updated_at: now,
    })
}

// ───────────────────────── 牛熊提醒 ─────────────────────────

/// 更新提醒的全局设置（总开关、音效、卡片不透明度、冷却期、默认阈值）。
#[tauri::command]
pub fn update_alert_settings(
    settings: crate::config::AlertConfig,
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<AppConfig, String> {
    let next = state.update_config(|cfg| {
        // 逐股规则不在本命令的职责内，原样保留（前端只传全局项）
        let rules = cfg.alerts.rules.clone();
        cfg.alerts = settings.clone();
        cfg.alerts.rules = rules;
        cfg.alerts.sanitize();
        cfg.clone()
    });
    commit(&app, &state, next.clone(), false);
    Ok(next)
}

/// 设置某只标的的逐股提醒规则。四项全空时删除该条（不在配置里留空壳）。
#[tauri::command]
pub fn set_alert_rule(
    code: String,
    rule: crate::domain::alert::AlertRule,
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<AppConfig, String> {
    let next = state.update_config(|cfg| {
        if rule.is_empty() {
            cfg.alerts.rules.remove(&code);
        } else {
            cfg.alerts.rules.insert(code.clone(), rule.clone());
        }
        cfg.alerts.sanitize();
        cfg.clone()
    });
    // 规则改了就重新布防：否则用户把阈值调宽后，旧的「未布防」状态会吞掉下一次越界
    state.clear_arm_states_for(&code);
    commit(&app, &state, next.clone(), false);
    Ok(next)
}

// ───────────────────────── 悬停资金流提示窗（v0.3.0） ─────────────────────────

/// flowtip 当前内容。**pull 模型**：flowtip 页面挂载时 invoke
/// `flow_tip_current` 拉一次，防"窗口刚建好、页面还没挂上 listen、
/// 第一个事件已发完"的首帧空白（大图窗换股栽过同一个坑）。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FlowTipPayload {
    pub main_net: f64,
    pub retail_net: f64,
    pub market: crate::domain::market::Market,
}

static FLOW_TIP: std::sync::Mutex<Option<FlowTipPayload>> = std::sync::Mutex::new(None);

/// 悬停某行 → 前端算好该行右缘的**物理屏幕坐标**后调用。
/// 数据从快照取（不信前端传的数值），找不到该行/无资金流时如实 Err
/// —— 提示窗不显示比显示错误数字好。
///
/// **必须 async**：首次要走 `build()`，同步命令 + 主线程 = 死锁
/// （见 open_chart_window 的病历）。
#[tauri::command]
pub async fn show_flow_tip(
    x: i32,
    y: i32,
    code: String,
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let payload = {
        let snap = state.get();
        let row = snap
            .rows
            .iter()
            .find(|r| r.code.eq_ignore_ascii_case(&code))
            .ok_or_else(|| format!("{code} 不在快照中"))?;
        let flow = row
            .fund_flow
            .ok_or_else(|| format!("{code} 暂无资金流数据"))?;
        FlowTipPayload {
            main_net: flow.main_net,
            retail_net: flow.retail_net,
            market: row.market,
        }
    };

    {
        let mut g = FLOW_TIP.lock().unwrap_or_else(|p| p.into_inner());
        *g = Some(payload);
    }
    window::place_flow_tip(&app, x, y)?;
    // emit 在 show 之后：内容与可见性同一帧到达（页面还有 pull 兜底）
    use tauri::Emitter;
    app.emit_to(window::FLOW_TIP_LABEL, "flow-tip", payload)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 鼠标离开该行 → 隐藏提示窗并清掉载荷。
#[tauri::command]
pub fn hide_flow_tip(app: AppHandle) -> Result<(), String> {
    {
        let mut g = FLOW_TIP.lock().unwrap_or_else(|p| p.into_inner());
        *g = None;
    }
    window::hide_flow_tip(&app)
}

/// flowtip 页面挂载时拉当前载荷（pull 兜底，见 FLOW_TIP 注释）。
#[tauri::command]
pub fn flow_tip_current() -> Option<FlowTipPayload> {
    *FLOW_TIP.lock().unwrap_or_else(|p| p.into_inner())
}
