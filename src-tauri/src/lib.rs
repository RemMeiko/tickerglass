pub mod backoff;
pub mod config;
pub mod domain;
pub mod ipc;
pub mod minute_cache;
pub mod model;
pub mod net;
pub mod parse;
pub mod scheduler;
pub mod state;
pub mod tray;
pub mod watchlist;
pub mod window;

use state::AppState;
use std::sync::Arc;
use std::time::{Duration, Instant};
// Emitter 是 app.emit 的 trait；Manager 提供 get_webview_window / state
use tauri::{Emitter, Manager};

/// 窗口位置写盘节流间隔。`Moved` 事件在拖动过程中每帧都来，
/// 不节流会造成拖一次窗口写几百次文件。
const MOVE_PERSIST_THROTTLE: Duration = Duration::from_millis(500);

pub fn run() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("tickerglass_lib=debug,info"),
    )
    .init();

    let app_state = Arc::new(AppState::new());

    // 启动顺序：先读配置（scheduler 要用它当数据源），再建窗口
    let loaded = config::store::load();
    if let Some(msg) = loaded.outcome.user_message() {
        log::warn!("[配置] {msg}");
        app_state.set_config_notice(Some(msg));
    }
    app_state.set_config(loaded.config);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            ipc::get_snapshot,
            ipc::get_config,
            ipc::get_config_notice,
            ipc::dismiss_config_notice,
            ipc::search_symbols,
            ipc::resolve_code,
            ipc::update_appearance,
            ipc::add_watch_item,
            ipc::remove_watch_item,
            ipc::reorder_watchlist,
            ipc::set_watch_hidden,
            ipc::open_settings_window,
            ipc::set_always_on_top,
            ipc::set_locked,
            ipc::set_collapsed,
            ipc::resize_main_window,
            ipc::fetch_chart,
            ipc::open_chart_window,
            ipc::fetch_market_overview,
            ipc::open_market_window,
            ipc::show_context_menu,
            ipc::update_alert_settings,
            ipc::set_alert_rule,
            ipc::import_alert_sound,
            ipc::read_alert_sound,
            ipc::list_alert_sounds,
            ipc::delete_alert_sound,
            ipc::demo_alert_popup,
            ipc::update_refresh,
            ipc::update_ma_periods,
            ipc::show_flow_tip,
            ipc::hide_flow_tip,
            ipc::flow_tip_current,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            // 托盘先建：锁定态的逃生通道应尽早可用
            if let Err(e) = tray::build(&handle) {
                log::error!("托盘构建失败: {e}");
            }
            window::apply_window_config(&handle);
            register_move_handler(&handle);
            window::register_close_to_hide(&handle);
            register_context_menu_handler(&handle);
            scheduler::spawn(handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}

/// 悬浮窗右键菜单的点击处理。
///
/// 托盘菜单的 handler 挂在 `TrayIconBuilder::on_menu_event` 上，只收托盘那个菜单的事件；
/// `menu.popup()` 弹出的菜单走的是**应用级** handler，故必须在这里单独注册一份。
/// 两者 id 前缀不同（`ctx-*` vs 无前缀），不会互相误命中。
fn register_context_menu_handler(app: &tauri::AppHandle) {
    app.on_menu_event(move |app, event| {
        let state = app.state::<Arc<AppState>>().inner().clone();
        let id = event.id().as_ref();

        // 逐行隐藏的 id 是**动态**的（`ctx-hide:sh600519`），match 的字面量分支
        // 表达不了，故在 match 之前先剥前缀。
        //
        // 直接同步调用而不 spawn：`apply_watch_hidden` 只做「写内存 + 落盘 + 发事件 +
        // notify_waiters」，没有网络请求也不建窗，不存在 open_settings 那类主线程死锁风险。
        // 与下方 CTX_ID_LOCK 直调 `ipc::apply_locked` 是同一种处理。
        if let Some(code) = id.strip_prefix(window::CTX_ID_HIDE_PREFIX) {
            ipc::apply_watch_hidden(app, &state, code, true);
            log::info!("[右键菜单] 隐藏 {code}");
            return;
        }

        match id {
            window::CTX_ID_SETTINGS => {
                // 投到别的线程：设置窗虽然预建了，但万一它被销毁过就会走 build()，
                // 而这个闭包在主线程上 —— 同建窗死锁那条坑（见 ipc::open_chart_window）
                let h = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = window::open_settings(&h) {
                        log::error!("[右键菜单] 打开设置窗失败: {e}");
                    }
                });
            }
            window::CTX_ID_MARKET => {
                let h = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = window::open_market(&h) {
                        log::error!("[右键菜单] 打开大盘窗失败: {e}");
                    }
                });
            }
            window::CTX_ID_TOP => {
                // 勾选项点一下就是取反当前值
                let next = !state.config().window.always_on_top;
                if let Some(win) = app.get_webview_window(window::MAIN_LABEL) {
                    if let Err(e) = win.set_always_on_top(next) {
                        log::error!("[右键菜单] 置顶设置失败: {e}");
                        return;
                    }
                }
                let cfg = state.update_config(|c| {
                    c.window.always_on_top = next;
                    c.clone()
                });
                if let Err(e) = config::store::save(&cfg) {
                    log::error!("[右键菜单] 置顶写盘失败: {e}");
                }
                if let Err(e) = app.emit("config", &cfg) {
                    log::warn!("[右键菜单] 配置事件发送失败: {e}");
                }
                log::info!("[右键菜单] 置顶 → {next}");
            }
            window::CTX_ID_LOCK => {
                let next = !state.config().window.locked;
                // 复用 ipc::apply_locked：它同时管 set_ignore_cursor_events、
                // 落盘与事件广播。这里另写一份必然与它分叉。
                if let Err(e) = ipc::apply_locked(app, &state, next) {
                    log::error!("[右键菜单] 锁定设置失败: {e}");
                }
            }
            window::CTX_ID_FORM => {
                // 取反当前形态。用 is_collapsed() 而不是直接读 collapsed ——
                // 老配置里那个字段是 None，直读取反会得到错误的目标态。
                let next = !state.config().window.is_collapsed();
                if let Err(e) = ipc::apply_collapsed(app, &state, next) {
                    log::error!("[右键菜单] 形态切换失败: {e}");
                }
            }
            window::CTX_ID_QUIT => {
                log::info!("[右键菜单] 用户请求退出");
                app.exit(0);
            }
            // 托盘菜单的 id 也会流到这里（应用级 handler 收所有菜单事件），
            // 那些由 tray.rs 自己的 handler 处理，这里静默忽略即可 —— 不要 warn，
            // 否则每次点托盘菜单都会刷一条无意义的告警
            _ => {}
        }
    });
}

/// 监听主窗口 `Moved`：越界钳制 + 节流持久化位置。
fn register_move_handler(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window(window::MAIN_LABEL) else {
        log::error!("找不到主窗口，跳过位置监听");
        return;
    };
    let state = app.state::<Arc<AppState>>().inner().clone();
    let handle = win.clone();
    // `on_window_event` 要求 `Fn`（不是 `FnMut`），故节流时间戳必须放在
    // 内部可变的容器里，不能直接用捕获的局部变量。
    let last_persist = std::sync::Mutex::new(Instant::now() - MOVE_PERSIST_THROTTLE);

    win.on_window_event(move |event| {
        if let tauri::WindowEvent::Moved(pos) = event {
            // 钳制在 Moved 里做：拖动结束时回弹，不在拖动中干扰手感
            let clamped = window::clamp_window_position(&handle);
            {
                let mut last = last_persist
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                if last.elapsed() < MOVE_PERSIST_THROTTLE {
                    return;
                }
                *last = Instant::now();
            }
            let (x, y) = match clamped {
                Some((cx, cy)) => (cx as i32, cy as i32),
                None => {
                    let sf = handle.scale_factor().unwrap_or(1.0);
                    let lp = pos.to_logical::<f64>(sf);
                    (lp.x as i32, lp.y as i32)
                }
            };
            window::persist_position(&state, x, y);
        }
    });
}
