import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import CandleChart from "./components/chart/CandleChart";
import { useConfig } from "./hooks/useConfig";
import { useTheme } from "./hooks/useTheme";
import IntradayLarge, {
  type LargeSegment,
} from "./components/chart/IntradayLarge";
import type {
  ChartPayload,
  ChartPeriod,
  Market,
  Snapshot,
  WatchItem,
} from "./types";
import { displayCode } from "./lib/format";

const PERIODS: { key: ChartPeriod; label: string }[] = [
  { key: "intraday", label: "分时" },
  { key: "5day", label: "5 日" },
  { key: "dayk", label: "日 K" },
];

/** 当日分时 30s 一刷；日K/5日 是历史数据，不自动刷 */
const INTRADAY_REFRESH_MS = 30_000;

function useChartSize() {
  const ref = useRef<HTMLDivElement | null>(null);
  const [size, setSize] = useState({ w: 800, h: 420 });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      // 这里用 contentRect 是对的：要的是 padding 以内的绘图区。
      // （悬浮窗的 D4 测量用的是 borderBoxSize，那边要连边框一起算，别混用）
      const box = entries[0]?.contentRect;
      if (!box) return;
      setSize({ w: Math.max(320, box.width), h: Math.max(220, box.height) });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return { ref, size };
}

export default function ChartApp() {
  // 主题：本窗口是独立 WebView，有自己的 document，
  // 所以必须自己挂一次 data-theme（设置窗写的不会传过来）。
  // 返回值往下传给两个 Canvas 图表 —— 它们的涨跌上色走 JS，CSS token 覆盖不到。
  const theme = useTheme(useConfig());

  // 首个标的由 URL query 带入，避免首帧不知道画谁而空转一轮
  const initialCode = useMemo(() => {
    const p = new URLSearchParams(window.location.search);
    return p.get("code") ?? "";
  }, []);

  const [code, setCode] = useState(initialCode);
  const [period, setPeriod] = useState<ChartPeriod>("intraday");
  const [payload, setPayload] = useState<ChartPayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [watchlist, setWatchlist] = useState<WatchItem[]>([]);
  // 名称只存在于报价响应里（WatchItem 只有代码与市场），故订阅快照取名
  const [names, setNames] = useState<Record<string, string>>({});
  const { ref: plotRef, size } = useChartSize();

  // 侧栏用自选股列表，方便在图窗内直接换股（不用回悬浮窗双击）
  useEffect(() => {
    invoke<{ watchlist: WatchItem[] }>("get_config")
      .then((cfg) => setWatchlist(cfg.watchlist ?? []))
      .catch(() => void 0);
  }, []);

  // 名称来自快照。大图窗不需要秒级刷新，取一次即可
  useEffect(() => {
    invoke<Snapshot>("get_snapshot")
      .then((snap) => {
        const m: Record<string, string> = {};
        for (const r of snap.rows) if (r.name) m[r.code] = r.name;
        setNames(m);
      })
      .catch(() => void 0);
  }, []);

  // 悬浮窗双击别的标的时，后端 emit chart-target 让本窗换股（而不是叠新窗）
  useEffect(() => {
    const un = listen<string>("chart-target", (e) => {
      if (e.payload) setCode(e.payload);
    });
    // ⚠️ 这个 .catch **不要删**（排查探针清理时也要留着）。
    //
    // `listen` 受 capability 门控（`core:event:allow-listen`）：窗口若没写进
    // `capabilities/default.json` 的 `windows` 列表，这里会**静默 reject** ——
    // 表现是"后端 emit 了、前端毫无反应"，控制台里也什么都看不到。
    // 大图窗就因为漏在列表外，换股功能一直不工作（2026-08-27 定位到）。
    //
    // 没有这个 catch，同类漏配将来还是查不出来。留一行日志的代价极小。
    un.catch((err) =>
      console.error(
        "[chart] chart-target 监听注册失败，多半是 capabilities/default.json " +
          "的 windows 列表漏了本窗口：",
        err
      )
    );
    return () => {
      void un.then((f) => f()).catch(() => void 0);
    };
  }, []);

  const load = useCallback(
    async (silent = false) => {
      if (!code) return;
      if (!silent) {
        setLoading(true);
        setError(null);
      }
      try {
        const p = await invoke<ChartPayload>("fetch_chart", { code, period });
        setPayload(p);
        setError(null);
      } catch (e) {
        // 静默刷新失败不清屏 —— 保留上次图，只是不更新
        if (!silent) {
          setPayload(null);
          setError(String(e));
        }
      } finally {
        if (!silent) setLoading(false);
      }
    },
    [code, period]
  );

  useEffect(() => {
    void load();
  }, [load]);

  // 只有当日分时需要自动刷新
  useEffect(() => {
    if (period !== "intraday") return;
    const t = window.setInterval(() => void load(true), INTRADAY_REFRESH_MS);
    return () => window.clearInterval(t);
  }, [period, load]);

  const current = watchlist.find((w) => w.tencent_code === code);
  const market: Market = current?.market ?? "ashare";

  // 两种后端结构归一成 IntradayLarge 的 segments
  const { segments, slots, showVolume } = useMemo((): {
    segments: LargeSegment[];
    slots: number;
    showVolume: boolean;
  } => {
    if (payload?.kind === "intraday") {
      const s = payload.series;
      return {
        segments: [
          {
            prevClose: s.prev_close,
            points: s.points.map((p) => ({
              slot: p.grid_index,
              price: p.price,
              breakBefore: p.break_before,
              label: String(p.hhmm).padStart(4, "0").replace(/(\d{2})(\d{2})/, "$1:$2"),
            })),
          },
        ],
        slots: s.grid_len,
        showVolume: false, // 腾讯当日分时通道不带量
      };
    }
    if (payload?.kind === "fiveDay") {
      return {
        segments: payload.days.map((d) => ({
          prevClose: d.prev_close,
          title: `${d.date.slice(4, 6)}-${d.date.slice(6, 8)}`,
          points: d.points.map((p, i) => ({
            // 5 日分时报文不带 grid_index，用序号铺满该段
            slot: i,
            price: p.price,
            volume: p.volume,
            label: `${d.date.slice(4, 6)}-${d.date.slice(6, 8)} ${String(p.hhmm)
              .padStart(4, "0")
              .replace(/(\d{2})(\d{2})/, "$1:$2")}`,
          })),
        })),
        slots: Math.max(
          1,
          ...payload.days.map((d) => d.points.length)
        ),
        showVolume: true,
      };
    }
    return { segments: [], slots: 1, showVolume: false };
  }, [payload]);

  return (
    <div className="chart-app">
      <header className="chart-head">
        <div className="chart-title">
          <span className="ct-name">{names[code] ?? displayCode(code)}</span>
          <span className="ct-code">{displayCode(code)}</span>
        </div>
        <div className="chart-tabs">
          {PERIODS.map((p) => (
            <button
              key={p.key}
              className={period === p.key ? "tab active" : "tab"}
              onClick={() => setPeriod(p.key)}
            >
              {p.label}
            </button>
          ))}
        </div>
        <button
          className="chart-refresh"
          onClick={() => void load()}
          title="重新拉取"
        >
          刷新
        </button>
      </header>

      <div className="chart-body-row">
        <aside className="chart-side">
          {watchlist.map((w) => (
            <button
              key={w.tencent_code}
              className={w.tencent_code === code ? "side-item active" : "side-item"}
              onClick={() => setCode(w.tencent_code)}
              title={w.tencent_code}
            >
              <span className="si-name">
                {names[w.tencent_code] ?? displayCode(w.tencent_code)}
              </span>
              <span className="si-code">{displayCode(w.tencent_code)}</span>
            </button>
          ))}
          {watchlist.length === 0 && (
            <div className="side-empty">自选股为空</div>
          )}
        </aside>

        <main className="chart-plot" ref={plotRef}>
          {loading && <div className="chart-msg">加载中…</div>}
          {!loading && error && (
            <div className="chart-msg chart-err">
              <div>{error}</div>
              <button onClick={() => void load()}>重试</button>
            </div>
          )}
          {!loading && !error && payload?.kind === "dailyK" && (
            <CandleChart
              bars={payload.bars}
              ma5={payload.ma5}
              ma20={payload.ma20}
              market={market}
              width={size.w}
              height={size.h}
              theme={theme}
            />
          )}
          {!loading && !error && payload && payload.kind !== "dailyK" && (
            <IntradayLarge
              segments={segments}
              slotsPerSegment={slots}
              market={market}
              width={size.w}
              height={size.h}
              showVolume={showVolume}
              theme={theme}
            />
          )}
        </main>
      </div>

      <footer className="chart-foot">
        <span className="legend ma5">MA5</span>
        <span className="legend ma20">MA20</span>
        <span className="legend avg">均价</span>
        <span className="foot-note">
          数据来自公开行情接口，仅供个人参考，不构成投资建议
        </span>
      </footer>
    </div>
  );
}
