import { useCallback, useEffect, useRef, useState } from "react";
import type { MarketOverview } from "./types";
import { fetchMarketOverview } from "./lib/ipc";
import IndexCards from "./components/market/IndexCards";
import BreadthBar from "./components/market/BreadthBar";
import FlowPanel from "./components/market/FlowPanel";
import { useConfig } from "./hooks/useConfig";
import { useTheme } from "./hooks/useTheme";

/**
 * 自动刷新间隔。**不要调小。**
 *
 * v0.4.0 起大盘走 `push2delay.eastmoney.com`（分钟级延迟、全量三市场），
 * 稳态每轮 2 个请求：腾讯指数（实时）+ 轮转一个市场的家数/资金流
 * （60s/轮 → 3 分钟全周期；进程内首次打开会一次取齐三市场）。
 *
 * 为什么 60s 够用：涨跌家数实测漂移约 60 家/5 分钟，资金流向是分钟级累计量，
 * 60s 刷一次完全跟得上"看大盘形势"这个用途。想立刻看最新的有手动刷新按钮。
 * delay 域宽容不等于无限，轮转节奏是按 60s 设计的，调小只会白打请求。
 */
const REFRESH_MS = 60_000;

type State =
  | { kind: "loading" }
  | { kind: "ok"; data: MarketOverview }
  /** 整体失败（invoke 层面）。逐市场失败不走这里，走 data.markets[].error */
  | { kind: "error"; message: string };

export default function MarketApp() {
  // 主题：独立 WebView 各自挂一次（理由见 useTheme 注释）
  useTheme(useConfig());

  const [state, setState] = useState<State>({ kind: "loading" });
  /** 手动刷新进行中，禁用按钮防连点（连点会直接把自己打进限流） */
  const [busy, setBusy] = useState(false);
  /** 竞态保护：只接受最新一次请求的响应 */
  const seqRef = useRef(0);

  const load = useCallback(async () => {
    const seq = ++seqRef.current;
    setBusy(true);
    try {
      const data = await fetchMarketOverview();
      if (seq !== seqRef.current) return;
      setState({ kind: "ok", data });
    } catch (e) {
      if (seq !== seqRef.current) return;
      setState({ kind: "error", message: String(e) });
    } finally {
      if (seq === seqRef.current) setBusy(false);
    }
  }, []);

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [load]);

  return (
    <div className="market-app">
      <header className="mk-header">
        <h1 className="mk-title">大盘</h1>
        <div className="mk-header-right">
          {state.kind === "ok" && (
            <span className="mk-time">{state.data.updated_at}</span>
          )}
          <button
            className="mk-refresh"
            disabled={busy}
            onClick={() => void load()}
            title={`每 ${REFRESH_MS / 1000} 秒自动刷新`}
          >
            {busy ? "刷新中…" : "刷新"}
          </button>
        </div>
      </header>

      {state.kind === "loading" && <div className="mk-hint">加载中…</div>}

      {state.kind === "error" && (
        <div className="mk-hint mk-warn" role="status">
          获取失败：{state.message}
        </div>
      )}

      {state.kind === "ok" && (
        <div className="mk-body">
          {/* 轮转取数说明：只在还有数据点没取到时显示，全齐了就不再占地方 */}
          {state.data.breadth_total.markets_counted < 3 && (
            <div className="mk-hint">
              部分市场本轮未取到，下一轮自动重试（稳态每轮轮转一个市场，约 3
              分钟轮完一遍）
            </div>
          )}

          <IndexCards
            indices={state.data.indices}
            error={state.data.indices_error}
            fetchedAt={state.data.indices_at}
          />
          <BreadthBar
            total={state.data.breadth_total}
            markets={state.data.markets}
          />
          <FlowPanel
            markets={state.data.markets}
            mainForceNet={state.data.main_force_net}
          />
        </div>
      )}
    </div>
  );
}
