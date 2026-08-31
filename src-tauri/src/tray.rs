// 系统托盘。**锁定态下这是用户唯一的逃生通道** —— 锁定后悬浮窗
// 完全不接受鼠标事件（`set_ignore_cursor_events(true)`），
// 没有托盘菜单用户就被永久锁死，只能去任务管理器杀进程。

use crate::ipc;
use crate::state::AppState;
use crate::window;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

const ID_UNLOCK: &str = "unlock";
const ID_SETTINGS: &str = "settings";
const ID_MARKET: &str = "market";
const ID_QUIT: &str = "quit";

/// 构建托盘图标与菜单。在 `setup` 里调用一次。
pub fn build(app: &AppHandle) -> Result<(), String> {
    let unlock = MenuItem::with_id(app, ID_UNLOCK, "解锁悬浮窗", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let settings = MenuItem::with_id(app, ID_SETTINGS, "设置…", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let market = MenuItem::with_id(app, ID_MARKET, "大盘…", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let quit =
        MenuItem::with_id(app, ID_QUIT, "退出", true, None::<&str>).map_err(|e| e.to_string())?;
    let menu = Menu::with_items(app, &[&unlock, &market, &settings, &quit])
        .map_err(|e| e.to_string())?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| "缺少默认窗口图标".to_string())?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("TickerGlass")
        .menu(&menu)
        // 左键点托盘不弹菜单（Windows 习惯是右键弹），左键用来打开设置
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let state = app.state::<Arc<AppState>>().inner().clone();
            match event.id().as_ref() {
                ID_UNLOCK => {
                    // 锁定态的唯一解锁入口
                    if let Err(e) = ipc::apply_locked(app, &state, false) {
                        log::error!("[托盘] 解锁失败: {e}");
                    }
                }
                ID_SETTINGS => {
                    if let Err(e) = window::open_settings(app) {
                        log::error!("[托盘] 打开设置窗失败: {e}");
                    }
                }
                ID_MARKET => {
                    // ⚠️ **必须投到别的线程**，不能直接在这里调 open_market。
                    //
                    // 这个闭包跑在主线程上，而大盘窗是懒建的 —— 首次点击必然走
                    // `WebviewWindowBuilder::build()`。build() 要把建窗任务交给主线程的
                    // 事件循环再等回执，而主线程此刻正卡在这个闭包里等 build() 返回，
                    // 两边互等 → 整个 UI 冻死，托盘也点不动，只能任务管理器杀进程。
                    // 这正是 2026-08-25 用户报的"点击就卡死"的同一个坑
                    // （详见 ipc::open_chart_window 的文档）。
                    //
                    // 为什么 ID_SETTINGS 那条不用这么写：设置窗在 tauri.conf.json 里
                    // 预建了，且有「关闭改隐藏」保着，open_settings 永远走 show() 复用分支，
                    // 碰不到 build()。大盘窗没有预建，所以必须异步。
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = window::open_market(&handle) {
                            log::error!("[托盘] 打开大盘窗失败: {e}");
                        }
                    });
                }
                ID_QUIT => {
                    log::info!("[托盘] 用户请求退出");
                    app.exit(0);
                }
                other => log::warn!("[托盘] 未处理的菜单项: {other}"),
            }
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击托盘图标 → 打开设置窗（锁定态下悬浮窗双击已失效，
            // 这给用户第二条不依赖右键菜单的通路）
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                if let Err(e) = window::open_settings(tray.app_handle()) {
                    log::error!("[托盘] 打开设置窗失败: {e}");
                }
            }
        })
        .build(app)
        .map_err(|e| e.to_string())?;
    Ok(())
}
