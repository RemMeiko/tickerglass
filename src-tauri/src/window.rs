// 窗口 I/O：置顶、锁定穿透、越界钳制、尺寸、设置窗创建、位置持久化。
//
// 越界钳制的**几何计算是纯函数**（`clamp_into_monitors`），可单测；
// 这一层只负责把 tauri 的显示器信息喂给它并把结果落到窗口上。

use crate::config::store;
use crate::state::AppState;
use std::sync::Arc;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewWindow};

pub const MAIN_LABEL: &str = "main";
pub const SETTINGS_LABEL: &str = "settings";
/// 大图窗（方案 A）。全应用只开一个，切标的走 `chart-target` 事件而非新建窗口。
pub const CHART_LABEL: &str = "chart";
/// 大盘面板窗。同样只开一个。
pub const MARKET_LABEL: &str = "market";
/// 屏幕中心提醒弹窗。与悬浮窗内的提醒卡片并存：
/// 卡片是常驻视野里的轻提示，这个是盖在所有窗口之上的强提示。
pub const POPUP_LABEL: &str = "popup";

/// 越界钳制的最小可见量：窗口至少要有这么大一块留在某个显示器内，
/// 否则用户抓不回来（Sprint 1 遗留：可拖出屏幕外 55px 无钳制）。
pub const MIN_VISIBLE_W: f64 = 60.0;
pub const MIN_VISIBLE_H: f64 = 30.0;

/// 一个矩形（逻辑像素）。纯几何，无 tauri 依赖。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }

    fn right(&self) -> f64 {
        self.x + self.w
    }

    fn bottom(&self) -> f64 {
        self.y + self.h
    }

    /// 与另一个矩形的交叠面积的宽/高（各自独立，用于判"可见量"）。
    fn overlap(&self, o: &Rect) -> (f64, f64) {
        let w = (self.right().min(o.right()) - self.x.max(o.x)).max(0.0);
        let h = (self.bottom().min(o.bottom()) - self.y.max(o.y)).max(0.0);
        (w, h)
    }
}

/// 判断窗口在任一显示器内是否已有足够可见量。
pub fn is_visible_enough(win: &Rect, monitors: &[Rect]) -> bool {
    monitors.iter().any(|m| {
        let (w, h) = win.overlap(m);
        w >= MIN_VISIBLE_W && h >= MIN_VISIBLE_H
    })
}

/// 把越界窗口钳回**最近**的合法位置。
///
/// 纯函数（这是标准 14「量化而非目测」的机器验证入口）。规则：
/// - 已有 ≥60×30 可见 → 原样返回（拖动中不干扰手感）
/// - 否则选交叠面积最大的显示器（完全无交叠时选中心距离最近的），
///   把坐标钳到"至少露出 60×30"的合法区间内
///
/// `monitors` 为空时原样返回（拿不到显示器信息就不乱动窗口）。
pub fn clamp_into_monitors(win: Rect, monitors: &[Rect]) -> Rect {
    if monitors.is_empty() || is_visible_enough(&win, monitors) {
        return win;
    }

    // 选目标显示器：先按交叠面积，再按中心距离
    let target = monitors
        .iter()
        .max_by(|a, b| {
            let sa = {
                let (w, h) = win.overlap(a);
                w * h
            };
            let sb = {
                let (w, h) = win.overlap(b);
                w * h
            };
            sa.partial_cmp(&sb)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    // 面积相同（常见于完全没交叠时都是 0）→ 比中心距离，近者优先
                    let da = center_dist2(&win, a);
                    let db = center_dist2(&win, b);
                    db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .copied()
        .unwrap_or(win);

    // 合法区间：窗口左边最多到 (显示器右边 - 最小可见宽)，最少到 (显示器左边 - (窗宽 - 最小可见宽))
    let min_x = target.x - (win.w - MIN_VISIBLE_W).max(0.0);
    let max_x = target.right() - MIN_VISIBLE_W;
    let min_y = target.y - (win.h - MIN_VISIBLE_H).max(0.0);
    let max_y = target.bottom() - MIN_VISIBLE_H;

    Rect {
        // min > max 只会在显示器比最小可见量还小时出现，clamp 会 panic，故取有序区间
        x: clamp_ordered(win.x, min_x, max_x),
        y: clamp_ordered(win.y, min_y, max_y),
        w: win.w,
        h: win.h,
    }
}

fn clamp_ordered(v: f64, lo: f64, hi: f64) -> f64 {
    if lo > hi {
        return lo;
    }
    v.clamp(lo, hi)
}

fn center_dist2(a: &Rect, b: &Rect) -> f64 {
    let (ax, ay) = (a.x + a.w / 2.0, a.y + a.h / 2.0);
    let (bx, by) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    (ax - bx).powi(2) + (ay - by).powi(2)
}

// ---------- 以下是 I/O 部分 ----------

/// 读取所有显示器的可见区域（逻辑像素）。
fn monitor_rects(win: &WebviewWindow) -> Vec<Rect> {
    let Ok(monitors) = win.available_monitors() else {
        return Vec::new();
    };
    monitors
        .iter()
        .map(|m| {
            let sf = m.scale_factor();
            let p = m.position().to_logical::<f64>(sf);
            let s = m.size().to_logical::<f64>(sf);
            Rect::new(p.x, p.y, s.width, s.height)
        })
        .collect()
}

/// 越界钳制。返回钳制后的位置（未移动则返回 `None`），并把量化数据写进日志 ——
/// 标准 14 要求"记录钳制后的坐标，确认 ≥60px 可见"，靠日志留证而非目测。
pub fn clamp_window_position(win: &WebviewWindow) -> Option<(f64, f64)> {
    let sf = win.scale_factor().unwrap_or(1.0);
    let pos = win.outer_position().ok()?.to_logical::<f64>(sf);
    let size = win.outer_size().ok()?.to_logical::<f64>(sf);
    let current = Rect::new(pos.x, pos.y, size.width, size.height);
    let monitors = monitor_rects(win);

    let clamped = clamp_into_monitors(current, &monitors);
    if (clamped.x - current.x).abs() < 0.5 && (clamped.y - current.y).abs() < 0.5 {
        return None;
    }
    log::info!(
        "[窗口] 越界钳制 ({:.0},{:.0}) → ({:.0},{:.0})，窗口 {:.0}x{:.0}，显示器 {:?}",
        current.x,
        current.y,
        clamped.x,
        clamped.y,
        current.w,
        current.h,
        monitors
            .iter()
            .map(|m| format!("{:.0},{:.0} {:.0}x{:.0}", m.x, m.y, m.w, m.h))
            .collect::<Vec<_>>()
    );
    if let Err(e) = win.set_position(LogicalPosition::new(clamped.x, clamped.y)) {
        log::warn!("[窗口] 钳制回弹失败: {e}");
        return None;
    }
    Some((clamped.x, clamped.y))
}

/// 持久化窗口位置。调用方需自行节流（`Moved` 事件是高频的）。
pub fn persist_position(state: &Arc<AppState>, x: i32, y: i32) {
    let next = state.update_config(|cfg| {
        cfg.window.x = Some(x);
        cfg.window.y = Some(y);
        cfg.clone()
    });
    if let Err(e) = store::save(&next) {
        log::warn!("[窗口] 位置写盘失败: {e}");
    }
}

/// 启动时应用配置里的窗口状态。
pub fn apply_window_config(app: &AppHandle) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    let cfg = state.config();
    let Some(win) = app.get_webview_window(MAIN_LABEL) else {
        log::error!("[窗口] 找不到主窗口 {MAIN_LABEL}");
        return;
    };

    if let (Some(x), Some(y)) = (cfg.window.x, cfg.window.y) {
        if let Err(e) = win.set_position(LogicalPosition::new(x as f64, y as f64)) {
            log::warn!("[窗口] 恢复位置失败: {e}");
        }
    }
    if let Err(e) = win.set_always_on_top(cfg.window.always_on_top) {
        log::warn!("[窗口] 恢复置顶失败: {e}");
    }
    if let Err(e) = win.set_ignore_cursor_events(cfg.window.locked) {
        log::warn!("[窗口] 恢复锁定失败: {e}");
    }
    // 恢复位置后立即钳制一次：上次退出时的坐标可能已因显示器变更而越界
    clamp_window_position(&win);
}

/// 打开（或聚焦）设置窗。已存在时**不重复创建**（标准 1 的连点 5 次判据）。
pub fn open_settings(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(SETTINGS_LABEL) {
        win.show().map_err(|e| e.to_string())?;
        win.unminimize().ok();
        win.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }
    let win = tauri::WebviewWindowBuilder::new(
        app,
        SETTINGS_LABEL,
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title("TickerGlass 设置")
    .inner_size(560.0, 620.0)
    .min_inner_size(460.0, 420.0)
    .resizable(true)
    .decorations(true)
    .transparent(false)
    .always_on_top(false)
    .skip_taskbar(false)
    .visible(true)
    .build()
    .map_err(|e| e.to_string())?;
    win.set_focus().ok();
    // 与 register_close_to_hide 保持一致：万一走到这条重建路径，
    // 也要把「关闭改隐藏」挂回去，否则关一次就又回到会 build() 的状态
    attach_close_to_hide(&win);
    Ok(())
}

/// 打开大图窗（方案 A：双击某行 → 弹独立可缩放窗口看分时/5日/日K）。
///
/// 复用同一个窗口而不是每只股票开一个：叠一堆图窗会失去"桌面悬浮窗"的意义。
/// 已存在时只切换标的（通过 `chart-target` 事件通知前端换股），不重建窗口。
pub fn open_chart(app: &AppHandle, code: &str, name: Option<&str>) -> Result<(), String> {
    let title = match name {
        Some(n) if !n.is_empty() => format!("{n} — TickerGlass"),
        _ => format!("{code} — TickerGlass"),
    };

    if let Some(win) = app.get_webview_window(CHART_LABEL) {
        win.show().map_err(|e| e.to_string())?;
        win.unminimize().ok();
        win.set_title(&title).ok();
        win.set_focus().map_err(|e| e.to_string())?;
        // 窗口已在，只让前端换标的。
        //
        // 必须用 `app.emit` 而不是 `win.emit`：
        // `WebviewWindow::emit` 把事件**限定投给该窗口**，而前端用的是
        // `@tauri-apps/api/event` 的裸 `listen()` —— 它只收**全局**事件，
        // 窗口定向的 emit 永远到不了它那里（要收得改用
        // `getCurrentWebviewWindow().listen()`）。
        //
        // 实测症状（2026-08-26 用户报）：大图窗开着时再双击别的标的，
        // 后端日志明明走了复用路径也 emit 了，但图不换股。
        // 项目里其他四处（config / snapshot ×2 / alert）用的都是 `app.emit`，
        // 四处都正常 —— 只有这一处是 `win.emit`，也只有这一处坏。
        // 广播到所有窗口无副作用：只有大图窗监听这个事件，且只有一个大图窗。
        use tauri::Emitter;
        app.emit("chart-target", code).map_err(|e| e.to_string())?;
        return Ok(());
    }

    let win = tauri::WebviewWindowBuilder::new(
        app,
        CHART_LABEL,
        // 首个标的通过 query 传，避免前端首帧不知道画谁而空转一轮
        tauri::WebviewUrl::App(format!("chart.html?code={code}").into()),
    )
    .title(&title)
    // 日K 要看清蜡烛与量柱，宽度给足；比设置窗更大
    .inner_size(880.0, 560.0)
    .min_inner_size(560.0, 380.0)
    .resizable(true)
    .decorations(true)
    .transparent(false)
    .always_on_top(false)
    .skip_taskbar(false)
    .visible(true)
    .build()
    .map_err(|e| e.to_string())?;
    // 大图窗是懒建的，启动时挂不上，建完立刻补挂「关闭改隐藏」——
    // 否则用户关掉它之后再双击又会走 build()，回到死锁那条路上
    attach_close_to_hide(&win);
    win.set_focus().ok();
    Ok(())
}

/// 打开（或聚焦）大盘面板窗。
///
/// 与 [`open_chart`] 同构，两个要点同样适用（都是踩过的坑）：
/// 1. 调用方必须是 **`async` 命令** —— 同步命令里跑 `build()` 会与主线程事件循环互等死锁
/// 2. `build()` 之后必须 `attach_close_to_hide`，否则用户关掉后再开又走 `build()`
///
/// 还有第三个坑不在这个函数里：`market` 必须写进
/// `capabilities/default.json` 的 `windows` 列表。漏了的话该窗口的
/// `listen()` 会被**静默拒绝**（无报错），这正是大图窗"双击不换股"的根因。
/// 本窗口当前不监听事件（数据靠 invoke 主动拉），但仍加进列表以免将来加事件时踩坑。
pub fn open_market(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(MARKET_LABEL) {
        win.show().map_err(|e| e.to_string())?;
        win.unminimize().ok();
        win.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }
    let win = tauri::WebviewWindowBuilder::new(
        app,
        MARKET_LABEL,
        tauri::WebviewUrl::App("market.html".into()),
    )
    .title("大盘 — TickerGlass")
    .inner_size(560.0, 640.0)
    .min_inner_size(420.0, 460.0)
    .resizable(true)
    .decorations(true)
    .transparent(false)
    .always_on_top(false)
    .skip_taskbar(false)
    .visible(true)
    .build()
    .map_err(|e| e.to_string())?;
    attach_close_to_hide(&win);
    win.set_focus().ok();
    Ok(())
}

/// 弹窗尺寸。内容是"名称 + 价格 + 一行原因"，比这个再大就只是空白。
const POPUP_W: f64 = 320.0;
const POPUP_H: f64 = 150.0;

/// 确保屏幕中心提醒弹窗存在并显示在正中。
///
/// 与 [`open_chart`]/[`open_market`] 的三个共同要点都适用（都是踩过的坑）：
/// 1. 调用方必须是 **`async` 命令** —— 同步命令里跑 `build()` 会与主线程事件循环互等死锁
/// 2. `build()` 之后必须 `attach_close_to_hide`
/// 3. `popup` 必须写进 `capabilities/default.json` 的 `windows` 列表，
///    否则该窗口的 `listen()` 会被**静默拒绝**（无报错）—— 这个窗口靠
///    监听 `alert` 事件工作，漏了就是"弹窗永远不出现"且查不到原因
///
/// # 为什么每次都重新居中
///
/// 弹窗是"隐藏而非关闭"复用的（关掉后下次提醒又要走一次 build，几百毫秒，
/// 提醒等不起）。但用户可能中途改了分辨率或换了主显示器，
/// 所以每次显示前都按当前显示器重算位置，而不是只在 build 时算一次。
///
/// # 为什么不用 center()
///
/// Tauri 的 `center()` 按**当前窗口所在**显示器居中。弹窗初次创建时位置是
/// 未定义的，可能落在副屏，于是"屏幕中心"会跑到用户没在看的那块屏上。
/// 这里显式取主显示器（`primary_monitor`）再算坐标。
fn place_popup_center(win: &tauri::WebviewWindow) -> Result<(), String> {
    let monitor = win
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .or(win.current_monitor().map_err(|e| e.to_string())?);
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let size = m.size().to_logical::<f64>(scale);
        let pos = m.position().to_logical::<f64>(scale);
        // 略高于几何中心：正中央会压住多数应用的内容区，
        // 抬到 42% 处更接近视线自然落点，也不至于挡住任务栏上方的东西。
        let x = pos.x + (size.width - POPUP_W) / 2.0;
        let y = pos.y + (size.height - POPUP_H) * 0.42;
        win.set_position(tauri::LogicalPosition::new(x, y))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 确保弹窗窗口**存在并摆到中心**，但**不负责显示**。
///
/// # 为什么 show 交给前端
///
/// 2026-08-31 用户报"第一次启动中间有一个弹窗不会消失，必须触发之后才会消失"。
/// 根因是这里原本建完就 `show()`：窗口首次创建要走 `build()`，
/// 而那一刻前端还没收到 `alert` 事件，`PopupApp` 的 `event` 是 null、
/// 渲染返回 null —— 于是窗口**显示了但内容是空的**。
/// 而 `setTimeout(dismiss, HOLD_MS)` 只在收到事件时才调度，
/// 所以没有任何东西会关掉它，空窗口就一直挂在屏幕中央。
/// （连带现象：空窗口没有 `.popup-card`，看到的是窗口本体，
/// 于是也报"四角不贴合"。两条是同一个 bug。）
///
/// 改由前端在真正拿到事件后调 `show()`：内容与可见性同一时刻发生，
/// 不存在"可见但空"的中间态。
pub fn ensure_popup(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(POPUP_LABEL) {
        // 只摆位置不显示。**不调 set_focus**：抢焦点会打断用户正在做的事
        // （打字会丢字符）。提醒只需要被看见，不需要被聚焦。
        place_popup_center(&win)?;
        return Ok(());
    }
    let win = tauri::WebviewWindowBuilder::new(
        app,
        POPUP_LABEL,
        tauri::WebviewUrl::App("popup.html".into()),
    )
    .title("提醒 — TickerGlass")
    .inner_size(POPUP_W, POPUP_H)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    // ⚠️ **必须显式关掉系统阴影。**
    //
    // Windows 上 shadow 默认 true，系统会沿**窗口矩形边界**画阴影与细边。
    // 而卡片是圆角的（border-radius: 18px），于是四角处那圈方形阴影
    // 从圆角外面透出来 —— 2026-08-31 用户报"四个角落会稍微多出一点，
    // 像是有个标签套着另一个标签"，看到的就是它。
    //
    // 这个坑改 CSS 永远修不掉：问题不在页面层，在窗口层。
    // 悬浮窗在 tauri.conf.json 里早就写了 `"shadow": false`（同样理由），
    // 这里是运行时建窗、没走配置文件，所以要自己补上。
    .shadow(false)
    .always_on_top(true)
    // 不进任务栏：它是瞬时提示，在任务栏留一格图标只是噪声
    .skip_taskbar(true)
    // 不抢焦点，理由同上
    .focused(false)
    // 先建后显：位置要等 place_popup_center 算完，
    // 否则会先在默认位置闪一下再跳到中间
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;
    attach_close_to_hide(&win);
    place_popup_center(&win)?;
    // **不 show**：等前端收到 alert 事件、真的有内容可画时它自己 show。
    // 见本函数顶部说明 —— 在这里 show 会留下一个永不消失的空窗口。
    Ok(())
}

/// 悬浮窗右键菜单的菜单项 id。与托盘菜单**共用**同一套 id，
/// 因为两者要触发完全相同的动作，分两套 id 只会让 handler 出现重复分支。
pub const CTX_ID_SETTINGS: &str = "ctx-settings";
pub const CTX_ID_MARKET: &str = "ctx-market";
pub const CTX_ID_TOP: &str = "ctx-top";
pub const CTX_ID_LOCK: &str = "ctx-lock";
pub const CTX_ID_QUIT: &str = "ctx-quit";
/// 球态 ↔ 展开列表。
///
/// 为什么右键菜单里也要有：球态下单击是"展开"，展开态下单击**不能**是"收起" ——
/// 展开后整个窗口是股票列表，单击某一行有自己的语义（双击开大图）。
/// 所以收起只能走菜单，这一项是展开态回到球态的唯一入口。
pub const CTX_ID_FORM: &str = "ctx-form";

/// 逐行「隐藏这只」的菜单项 id 前缀，后面直接拼 `tencent_code`
/// （如 `ctx-hide:sh600519`）。
///
/// 为什么把标的编进 id 而不是另存一份"当前右键的是哪行"的状态：
/// 菜单是**异步**的（popup 期间用户可以移动鼠标、也可能直接关掉菜单），
/// 存状态就要考虑"什么时候清"以及"两次右键之间是否残留"。
/// 编进 id 后每个菜单项自带上下文，handler 是纯函数，不存在竞态。
pub const CTX_ID_HIDE_PREFIX: &str = "ctx-hide:";

/// 在悬浮窗上弹出右键菜单。
///
/// # 为什么用系统原生菜单而不是 HTML 菜单
///
/// 悬浮窗的尺寸由内容量出来（D4），只有一只自选股时窗口才 ~60px 高。
/// HTML 菜单画在 webview 里，**画不出窗口边界** —— 一个 ~130px 高的菜单
/// 在 60px 的窗口里会被裁掉大半。要么把菜单计入测量（窗口会突然涨大一截，
/// 很怪），要么就用系统菜单。系统菜单是独立的 OS 窗口，没有这个限制，
/// 而且外观与托盘菜单一致。
///
/// 勾选项用 `CheckMenuItem` 直接反映当前配置，省掉"点开才知道现在是开还是关"。
pub fn popup_context_menu(
    app: &AppHandle,
    state: &Arc<AppState>,
    row_code: Option<&str>,
) -> Result<(), String> {
    use tauri::menu::{CheckMenuItem, ContextMenu, Menu, MenuItem, PredefinedMenuItem};

    let Some(win) = app.get_webview_window(MAIN_LABEL) else {
        return Err("找不到主窗口".to_string());
    };
    let cfg = state.config();

    // 在股票行上右键时，菜单顶部多一项「隐藏 名称」。
    //
    // 名称从快照取而不是配置：配置里只有 tencent_code，没有中文名（名称是行情
    // 报文带回来的）。快照拿不到名称时退回裸码显示 —— 不显示空白菜单项。
    //
    // 校验 row_code 真在自选股里：前端传什么后端都得当不可信输入。传了个
    // 不存在的码就当没传，只弹通用菜单，不给一个点了没反应的菜单项。
    let hide_target: Option<(String, String)> = row_code.and_then(|code| {
        let item = cfg
            .watchlist
            .iter()
            .find(|it| it.tencent_code.eq_ignore_ascii_case(code))?;
        let name = state
            .get()
            .rows
            .iter()
            .find(|r| r.code.eq_ignore_ascii_case(code))
            // 名称拿不到时用内部身份键兜底（如 `sh600519`）。
            // 不在 Rust 侧再实现一份剥前缀的 display_code —— 那是前端 format.ts
            // 的职责，为一个"数据还没到"的边缘分支把显示逻辑复制到两侧不值得。
            .and_then(|r| r.name.clone())
            .unwrap_or_else(|| item.tencent_code.clone());
        Some((item.tencent_code.clone(), name))
    });

    let settings = MenuItem::with_id(app, CTX_ID_SETTINGS, "设置…", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let market = MenuItem::with_id(app, CTX_ID_MARKET, "大盘…", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    // 形态切换。文字写"要变成什么"而不是当前状态 —— 菜单项是动作，
    // 写成"当前：球态"用户还得自己推断点下去会发生什么。
    let form = MenuItem::with_id(
        app,
        CTX_ID_FORM,
        if cfg.window.is_collapsed() {
            "展开为列表"
        } else {
            "收起为球"
        },
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let top = CheckMenuItem::with_id(
        app,
        CTX_ID_TOP,
        "窗口置顶",
        true,
        cfg.window.always_on_top,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    // 锁定后悬浮窗完全不接受鼠标事件，**右键菜单也弹不出来** ——
    // 所以这一项实际上是单向的：从这里能锁上，但解锁只能走托盘（标准 16）。
    // 菜单文字里明说，免得用户锁上后找不到出路。
    let lock = CheckMenuItem::with_id(
        app,
        CTX_ID_LOCK,
        "锁定（点击穿透，解锁走托盘）",
        true,
        cfg.window.locked,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let sep = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let quit = MenuItem::with_id(app, CTX_ID_QUIT, "退出", true, None::<&str>)
        .map_err(|e| e.to_string())?;

    // 逐行隐藏项放**最顶部**：它是"针对这一行"的操作，而下面几项是窗口级的。
    // 顶部离光标最近，且与下方窗口级项用分隔线隔开，语义层次一眼可辨。
    let hide_item = match &hide_target {
        Some((code, name)) => Some(
            MenuItem::with_id(
                app,
                format!("{CTX_ID_HIDE_PREFIX}{code}"),
                format!("隐藏 {name}"),
                true,
                None::<&str>,
            )
            .map_err(|e| e.to_string())?,
        ),
        None => None,
    };

    // 形态项放「设置…」之前：它是改变悬浮窗外观的高频操作，
    // 比设置窗更靠近"我现在就想换个样子"的意图。
    let menu = match &hide_item {
        Some(hide) => Menu::with_items(
            app,
            &[
                hide, &sep, &form, &settings, &market, &sep, &top, &lock, &sep, &quit,
            ],
        ),
        None => Menu::with_items(
            app,
            &[&form, &settings, &market, &sep, &top, &lock, &sep, &quit],
        ),
    }
    .map_err(|e| e.to_string())?;

    // popup 在鼠标位置弹出。不传坐标时 tauri 用当前光标位置，正是右键菜单要的行为。
    //
    // `popup` 要的是 `Window` 而不是 `WebviewWindow`（后者是"窗口 + webview"的组合），
    // 用 `as_ref().window()` 取出底层窗口。直接传 WebviewWindow 会编译失败。
    menu.popup(win.as_ref().window())
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 把「关闭窗口」改成「隐藏窗口」（设置窗、大图窗、大盘窗，不含主窗口）。
///
/// 这是死锁的**根因修复**，比把命令改 async 更靠前一层：
///
/// Tauri 默认「关闭即销毁」。设置窗虽然在 `tauri.conf.json` 里预建了
/// （`visible: false`），但一旦用户点了关闭按钮，窗口就被销毁 ——
/// 于是下次开设置时 `get_webview_window` 拿不到窗口，只能走 `build()`。
/// 从托盘菜单进来时 `build()` 跑在主线程上，与事件循环互等，直接冻死整个 UI。
///
/// 改成隐藏后，两个窗口在整个进程生命周期里只 `build()` 一次，
/// 之后永远走 `show()` 复用分支，从任何线程调用都安全。
/// 附带好处：再次打开是秒开（WebView 不用重建），且保留了上次的滚动位置与 Tab。
///
/// 主窗口不在此列：它是悬浮窗本体，`decorations: false` 没有关闭按钮，
/// 真要关闭是走托盘「退出」→ `app.exit(0)`。
pub fn register_close_to_hide(app: &AppHandle) {
    for label in [SETTINGS_LABEL, CHART_LABEL, MARKET_LABEL] {
        let Some(win) = app.get_webview_window(label) else {
            // 大图窗/大盘窗是懒建的，启动时不存在很正常 ——
            // 它们在 open_chart / open_market 里建完会各自补挂
            continue;
        };
        attach_close_to_hide(&win);
    }
}

/// 给单个窗口挂上「关闭改隐藏」。懒建的窗口在 `build()` 之后调用。
pub fn attach_close_to_hide(win: &WebviewWindow) {
    let handle = win.clone();
    let label = win.label().to_string();
    win.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Err(e) = handle.hide() {
                log::warn!("[窗口] {label} 隐藏失败: {e}");
            }
        }
    });
}

/// 调整主窗口尺寸（前端测量内容后上报，D4）。
pub fn resize_main(app: &AppHandle, w: f64, h: f64) -> Result<(), String> {
    let Some(win) = app.get_webview_window(MAIN_LABEL) else {
        return Err("找不到主窗口".to_string());
    };
    // 下限保护：前端测量异常（0 或负数）时不要把窗口缩成看不见
    let w = w.max(MIN_VISIBLE_W);
    let h = h.max(MIN_VISIBLE_H);
    win.set_size(LogicalSize::new(w, h))
        .map_err(|e| e.to_string())?;
    // 尺寸变化可能让原本合法的位置变越界
    clamp_window_position(&win);
    Ok(())
}

// ───────────────────────── 悬停资金流提示窗（v0.3.0） ─────────────────────────

pub const FLOW_TIP_LABEL: &str = "flowtip";
/// 提示窗逻辑尺寸（px）。内容是两个 chip，实测够放
/// 「主力 -12.34亿 · 散户 +5678万」。
const FLOW_TIP_W: f64 = 190.0;
const FLOW_TIP_H: f64 = 34.0;

/// 确保 flowtip 窗口存在（**只建不显**）。已存在则原样返回。
///
/// 与 popup 同一套注意点：无边框必须 `.shadow(false)`（系统沿矩形画阴影，
/// 圆角外会透出方形阴影）；`focused(false)` 不抢焦点；先建后显防"默认位置
/// 闪一下"。**额外一条本窗特有：`set_ignore_cursor_events(true)`** ——
/// 它贴着主窗右缘显示，若不穿透鼠标，掠过它去点别处会被这层看不见的
/// 窗口挡住。
///
/// # 调用方必须走 async 命令
///
/// 首次要走 `build()`，同步命令 + 主线程 = 死锁（见 ipc::open_chart_window
/// 的完整病历）。
pub fn ensure_flow_tip(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(win) = app.get_webview_window(FLOW_TIP_LABEL) {
        return Ok(win);
    }
    let win = tauri::WebviewWindowBuilder::new(
        app,
        FLOW_TIP_LABEL,
        tauri::WebviewUrl::App("flowtip.html".into()),
    )
    .title("TickerGlass")
    .inner_size(FLOW_TIP_W, FLOW_TIP_H)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;
    // 鼠标穿透：它永远不该成为鼠标事件的落点
    win.set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;
    Ok(win)
}

/// 把 flowtip 摆到主窗某行的右缘外侧（物理坐标，由前端按行矩形算好），
/// 然后显示。**钳制在显示器工作区内** —— 悬浮窗常贴屏幕右缘放着，
/// 不钳制的话提示会整体跑到屏幕外。
pub fn place_flow_tip(app: &AppHandle, x: i32, y: i32) -> Result<(), String> {
    let tip = ensure_flow_tip(app)?;
    let mut x = x;
    let mut y = y;

    // 主窗所在显示器的工作区做边界（拿不到显示器就不钳制，宁可冒一点
    // 越界也别因为钳制逻辑把提示弄丢）
    if let Some(main) = app.get_webview_window(MAIN_LABEL) {
        if let Ok(Some(mon)) = main.current_monitor() {
            let m_pos = mon.position();
            let m_size = mon.size();
            let (tw, th) = tip
                .outer_size()
                .map(|s| (s.width as i32, s.height as i32))
                .unwrap_or((FLOW_TIP_W as i32, FLOW_TIP_H as i32));
            let max_x = m_pos.x + m_size.width as i32 - tw;
            let max_y = m_pos.y + m_size.height as i32 - th;
            x = x.min(max_x).max(m_pos.x);
            y = y.min(max_y).max(m_pos.y);
        }
    }

    tip.set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    tip.show().map_err(|e| e.to_string())?;
    Ok(())
}

/// 隐藏 flowtip（窗口保留复用，销毁重建会有闪白）。
pub fn hide_flow_tip(app: &AppHandle) -> Result<(), String> {
    if let Some(tip) = app.get_webview_window(FLOW_TIP_LABEL) {
        tip.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}
