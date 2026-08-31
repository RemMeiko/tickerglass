import { Component, type ErrorInfo, type ReactNode, useEffect, useState } from "react";
import type { AppConfig } from "./types";
import { useConfig } from "./hooks/useConfig";
import { useTheme } from "./hooks/useTheme";
import ThemeToggle from "./components/settings/ThemeToggle";
import WatchlistPanel from "./components/settings/WatchlistPanel";
import AppearancePanel from "./components/settings/AppearancePanel";
import WindowPanel from "./components/settings/WindowPanel";
import AlertPanel from "./components/settings/AlertPanel";
import AboutPanel from "./components/settings/AboutPanel";

/** 兜底，保证任何渲染异常都不会让设置窗变成空白 */
class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("设置窗渲染异常:", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return <div className="fatal">界面异常：{this.state.error.message}</div>;
    }
    return this.props.children;
  }
}

type TabId = "watchlist" | "alerts" | "appearance" | "window" | "about";

const TABS: { id: TabId; label: string }[] = [
  { id: "watchlist", label: "自选股" },
  { id: "alerts", label: "提醒" },
  { id: "appearance", label: "外观" },
  { id: "window", label: "窗口" },
  { id: "about", label: "关于" },
];

export default function SettingsApp() {
  const [tab, setTab] = useState<TabId>("watchlist");
  const hookConfig = useConfig();
  // 提醒面板的写操作会返回新配置；本地覆盖一层，避免等下一次事件推送才回显
  const [override, setOverride] = useState<AppConfig | null>(null);

  /**
   * 广播一到就丢掉本地覆盖，让后端数据重新接管。
   *
   * **不加这段的话 override 会永久遮蔽 hookConfig。** 2026-08-30 用户报了四个
   * 看起来无关的现象，根因都是这一处：外观页勾选框点了不变、窗口页置顶/锁定
   * 勾了不变、自选股排序在设置里不动（悬浮窗却动了）、外观滑块拖不动。
   *
   * 链路是：进过一次提醒页 → AlertPanel 调 setOverride → override 非 null →
   * 此后 `override ?? hookConfig` 永远取 override → 后端每次 commit 的
   * `emit("config")` 设置窗全都收不到。写盘和悬浮窗都正常，只有设置窗自己冻住，
   * 所以现象是"功能生效但显示没变化"。滑块尤其明显：它是受控的 value={a.scale}，
   * config 冻住后每次拖动都被旧值弹回，看起来就是"完全没法用"。
   *
   * 依赖 hookConfig 的引用：useConfig 每收到一次广播就 setConfig 一个新对象，
   * 引用变化即代表"有新数据到了"，这时本地覆盖已无价值。
   */
  useEffect(() => {
    if (hookConfig) setOverride(null);
  }, [hookConfig]);

  const config = override ?? hookConfig;
  useTheme(config);

  return (
    <div className="settings">
      <nav className="tabs" role="tablist" aria-label="设置分类">
        {TABS.map((t) => (
          <button
            key={t.id}
            role="tab"
            id={`tab-${t.id}`}
            aria-selected={tab === t.id}
            aria-controls={`panel-${t.id}`}
            className={`tab ${tab === t.id ? "tab-active" : ""}`}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
        {/* 主题切换放在 Tab 栏右端而不是外观页里：它是全局开关，
            不该藏在某一页下面（用户要切换主题得先找到那一页）。
            role="switch" 而非 tab —— 它不参与 tablist 的选中语义。 */}
        <ThemeToggle config={config} />
      </nav>

      <div
        // key 随 tab 变化 → React 重建这个节点，入场动画才会**每次切换都重放**。
        // 不加 key 的话同一个 DOM 节点被复用，CSS animation 只在首次挂载时跑一次，
        // 之后切 Tab 就是硬切换（用户反馈的"僵硬"有一半来自这里）。
        key={tab}
        className="tab-panel"
        role="tabpanel"
        id={`panel-${tab}`}
        aria-labelledby={`tab-${tab}`}
      >
        <ErrorBoundary>
          {config === null ? (
            <div className="loading">加载配置…</div>
          ) : (
            <>
              {tab === "watchlist" && <WatchlistPanel config={config} />}
              {tab === "alerts" && (
                <AlertPanel config={config} onChange={setOverride} />
              )}
              {tab === "appearance" && <AppearancePanel config={config} />}
              {tab === "window" && <WindowPanel config={config} />}
              {tab === "about" && <AboutPanel />}
            </>
          )}
        </ErrorBoundary>
      </div>
    </div>
  );
}
