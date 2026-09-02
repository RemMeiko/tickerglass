import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppConfig,
  AppearanceConfig,
  MarketOverview,
  RefreshConfig,
  ResolveResponse,
  SearchResponse,
  Snapshot,
  WatchItem,
} from "../types";

/** 首屏拉一次，避免错过启动瞬间的首个 snapshot 事件（竞态） */
export async function fetchSnapshot(): Promise<Snapshot> {
  return invoke<Snapshot>("get_snapshot");
}

/** 订阅后端推送的快照 */
export async function subscribeSnapshot(
  onSnapshot: (snap: Snapshot) => void
): Promise<UnlistenFn> {
  return listen<Snapshot>("snapshot", (event) => onSnapshot(event.payload));
}

// ---------- Sprint 2A：配置 ----------

export async function fetchConfig(): Promise<AppConfig> {
  return invoke<AppConfig>("get_config");
}

/** 订阅配置变更（跨窗口广播：设置窗改动 → 悬浮窗实时预览） */
export async function subscribeConfig(
  onConfig: (cfg: AppConfig) => void
): Promise<UnlistenFn> {
  return listen<AppConfig>("config", (event) => onConfig(event.payload));
}

export async function fetchConfigNotice(): Promise<string | null> {
  return invoke<string | null>("get_config_notice");
}

export async function dismissConfigNotice(): Promise<void> {
  return invoke("dismiss_config_notice");
}

export async function updateAppearance(
  appearance: AppearanceConfig
): Promise<AppConfig> {
  return invoke<AppConfig>("update_appearance", { appearance });
}

export async function searchSymbols(query: string): Promise<SearchResponse> {
  return invoke<SearchResponse>("search_symbols", { query });
}

/**
 * 按代码直接解析（A 股）。与 {@link searchSymbols} 的区别：
 * 走报价接口而非东财搜索接口，故搜索限流时它仍然可用。
 *
 * 返回的候选**已经过存在性校验**并带真实名称；歧义码（`000001` 同时是
 * 沪指与平安银行）会返回多条，由用户选。
 */
export async function resolveCode(input: string): Promise<ResolveResponse> {
  return invoke<ResolveResponse>("resolve_code", { input });
}

/** 失败时后端返回 Err（已达上限 / 已在自选股中），调用方须 catch */
export async function addWatchItem(item: WatchItem): Promise<AppConfig> {
  return invoke<AppConfig>("add_watch_item", { item });
}

/** 参数是 `tencent_code`（唯一键）。传裸码会同时命中两个市场的标的 */
export async function removeWatchItem(tencentCode: string): Promise<AppConfig> {
  return invoke<AppConfig>("remove_watch_item", { tencentCode });
}

export async function reorderWatchlist(order: string[]): Promise<AppConfig> {
  return invoke<AppConfig>("reorder_watchlist", { order });
}

/**
 * 设置隐藏状态。**隐藏 ≠ 删除** ——
 * 只影响悬浮窗显示，后台照常取数、照常提醒，取消后立刻有数据。
 */
export async function setWatchHidden(
  tencentCode: string,
  hidden: boolean
): Promise<AppConfig> {
  return invoke<AppConfig>("set_watch_hidden", { tencentCode, hidden });
}

// ---------- Sprint 2A：窗口交互 ----------

export async function openSettingsWindow(): Promise<void> {
  return invoke("open_settings_window");
}

export async function setAlwaysOnTop(enabled: boolean): Promise<AppConfig> {
  return invoke<AppConfig>("set_always_on_top", { enabled });
}

/**
 * 切换悬浮窗形态：球态 ↔ 展开列表。
 *
 * 后端**不改窗口尺寸** —— 尺寸仍由 App 的 ResizeObserver 测内容后
 * 走 resizeMainWindow 上报（D4 单一真源）。
 */
export async function setCollapsed(collapsed: boolean): Promise<AppConfig> {
  return invoke<AppConfig>("set_collapsed", { collapsed });
}

export async function setLocked(locked: boolean): Promise<AppConfig> {
  return invoke<AppConfig>("set_locked", { locked });
}

export async function resizeMainWindow(
  width: number,
  height: number
): Promise<void> {
  return invoke("resize_main_window", { width, height });
}

/**
 * 拉大盘面板数据（A 股）。**一次调用会打 4 个请求**（1 指数 + 3 市场）。
 *
 * ⚠️ 别做快速轮询：数据源 `push2.eastmoney.com` 会静默限流
 * （整个 host 返回空响应体，且不是等固定时长就恢复）。
 * 刷新间隔见 `MarketApp` 里的 `REFRESH_MS`。
 */
export async function fetchMarketOverview(): Promise<MarketOverview> {
  return invoke<MarketOverview>("fetch_market_overview");
}

/** 打开大盘面板窗。已开时只聚焦，不叠新窗。 */
export async function openMarketWindow(): Promise<void> {
  await invoke("open_market_window");
}

/**
 * 演示中心弹窗效果。
 *
 * 后端走的是与真实提醒完全相同的两步（唤起窗口 + emit "alert"），
 * 所以这里看到的就是真到价时会看到的 —— 不存在"演示版"和"真实版"的差异。
 */
export async function demoAlertPopup(): Promise<void> {
  await invoke("demo_alert_popup");
}

/**
 * 更新刷新间隔。返回落盘后的完整配置（后端可能钳制过值）。
 *
 * 改完即生效、不需要重启：后端 commit 会广播 config，
 * 两条轮询链路每轮比对间隔值，变了就重建 tokio interval。
 */
export async function updateRefresh(
  refresh: RefreshConfig
): Promise<AppConfig> {
  return invoke<AppConfig>("update_refresh", { refresh });
}

// ---------- 自定义提示音 ----------

/**
 * 导入一个自定义提示音。返回**存盘后的文件名**（可能被规范化过，
 * 非 ASCII 字符会换成 `_`），调用方拼成 `custom:文件名` 存进配置。
 *
 * 后端会校验扩展名与大小（≤5MB），不合规直接 Err。
 */
export async function importAlertSound(
  fileName: string,
  bytes: Uint8Array
): Promise<string> {
  // Tauri 的 IPC 序列化不认 Uint8Array，要转成普通数组
  return invoke<string>("import_alert_sound", {
    fileName,
    bytes: Array.from(bytes),
  });
}

/** 读回自定义提示音的字节，用于播放/试听。 */
export async function readAlertSound(fileName: string): Promise<Uint8Array> {
  const arr = await invoke<number[]>("read_alert_sound", { fileName });
  return new Uint8Array(arr);
}

/** 列出已导入的自定义提示音文件名。 */
export async function listAlertSounds(): Promise<string[]> {
  return invoke<string[]>("list_alert_sounds");
}

export async function deleteAlertSound(fileName: string): Promise<void> {
  await invoke("delete_alert_sound", { fileName });
}

/**
 * 悬浮窗右键 → 弹**系统原生**菜单（设置/大盘/置顶/锁定/退出）。
 *
 * 为什么不用 HTML 菜单：悬浮窗高度由内容量出来（D4），只有一只自选股时才 ~60px，
 * HTML 菜单画不出窗口边界会被裁掉大半。系统菜单是独立 OS 窗口，没这个限制。
 */
export async function showContextMenu(rowCode?: string | null): Promise<void> {
  // 传 rowCode 时菜单顶部多一项「隐藏 名称」。后端会校验它真在自选股里，
  // 传了不存在的码就退化成通用菜单（不会出现点了没反应的菜单项）。
  await invoke("show_context_menu", { rowCode: rowCode ?? null });
}

/** 打开大图窗看指定标的（方案 A）。已开时后端只切标的，不叠新窗。 */
export async function openChartWindow(
  code: string,
  name?: string | null
): Promise<void> {
  await invoke("open_chart_window", { code, name: name ?? null });
}
