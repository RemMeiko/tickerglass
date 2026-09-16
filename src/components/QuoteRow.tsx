import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { RowSnapshot } from "../types";
import { changeColor } from "../lib/colors";
import {
  displayCode,
  formatPercent,
  PLACEHOLDER,
  quotePrice,
} from "../lib/format";
import MinuteChart from "./MinuteChart";

interface Props {
  row: RowSnapshot;
  /** 显示证券代码（关闭时元素不渲染，靠 flex 自然收紧，不留空洞） */
  showCode: boolean;
  showMarket: boolean;
  /** 显示主力/散户资金流（v0.3.0 需求 6，外观页可关） */
  showFundFlow: boolean;
  curveWidth: number;
  curveOpacity: number;
  chartWidth: number;
  chartHeight: number;
  /** 锁定态下不响应双击（鼠标事件本已被系统穿透，这里是双重保险） */
  locked: boolean;
  /** 双击该行 → 打开大图窗看这只（方案 A） */
  onOpenChart?: (code: string, name: string | null) => void;
  /** 右键该行 → 弹系统菜单，顶部带「隐藏 名称」 */
  onRowContextMenu?: (code: string) => void;
}

const MARKET_LABEL: Record<string, string> = {
  ashare: "A股",
  indexcn: "指数",
  hk: "港股",
  us: "美股",
};

/** 过期角标文案。报文时间不可得时降级为本地接收时间并标注来源 */
function staleLabel(row: RowSnapshot): string | null {
  if (row.state.kind !== "stale") return null;
  const { last_ok_at, from_report_time } = row.state;
  return from_report_time ? `数据过期 ${last_ok_at}` : `数据过期 ${last_ok_at}(本地)`;
}

/** 悬停资金流提示（v0.3.0 定稿）：
 *  悬停某行 → 一个独立小提示窗贴在**该行右缘外侧**显示主/散，
 *  **主窗尺寸完全不动**、图表与悬停读数零影响。
 *  提示窗内容/显隐全在后端（show_flow_tip 从快照取数），
 *  前端只负责算「行右缘」的物理屏幕坐标。 */
function useFlowTipHover(row: RowSnapshot, showFundFlow: boolean, locked: boolean) {
  /** 进出竞态序号：只认最后一次动作（快速划过多行时旧响应不得关掉新提示） */
  const seqRef = useRef(0);

  // **卸载兜底（2026-09-16 bug 修复）**：行被卸载（收起为球/滚轮换页/
  // 删除自选股/全部隐藏）时**不会派发 mouseleave** —— React 卸载不产生
  // 鼠标事件，提示卡就永远悬在屏幕上。必须在清理函数里主动收掉，
  // 并作废在途的 show 请求（否则迟到的 show 会把刚收掉的又亮出来）。
  useEffect(
    () => () => {
      seqRef.current++;
      void invoke("hide_flow_tip").catch(() => void 0);
    },
    []
  );

  const onEnter = (e: React.MouseEvent<HTMLDivElement>) => {
    if (locked || !showFundFlow || !row.fund_flow) return;
    const seq = ++seqRef.current;
    const rect = e.currentTarget.getBoundingClientRect();
    const win = getCurrentWindow();
    Promise.all([win.outerPosition(), win.scaleFactor()])
      .then(([pos, sf]) => {
        if (seq !== seqRef.current) return;
        // 行右缘 + 2px 间隙；垂直居中减半个提示窗高（后端还有显示器边界钳制）
        const x = pos.x + Math.round(rect.right * sf) + 2;
        const y =
          pos.y + Math.round((rect.top + rect.height / 2) * sf) - 17 * Math.round(sf);
        return invoke("show_flow_tip", { x, y, code: row.code });
      })
      .catch((err) => console.error("资金流提示显示失败:", err));
  };

  const onLeave = () => {
    seqRef.current++;
    void invoke("hide_flow_tip").catch(() => void 0);
  };

  return { onEnter, onLeave };
}

export default function QuoteRow({
  row,
  showCode,
  showMarket,
  showFundFlow,
  curveWidth,
  curveOpacity,
  chartWidth,
  chartHeight,
  locked,
  onOpenChart,
  onRowContextMenu,
}: Props) {
  const { quote, market } = row;
  const percent = quote?.change.percent ?? null;
  const color = changeColor(percent, market);

  const isStale = row.state.kind === "stale";
  const isError = row.state.kind === "error";
  const stale = staleLabel(row);

  const { onEnter: onFlowEnter, onLeave: onFlowLeave } = useFlowTipHover(
    row,
    showFundFlow,
    locked
  );

  return (
    <div
      className="row"
      style={{ opacity: isStale ? 0.62 : 1 }}
      title={locked ? "" : "双击看大图（分时 / 5 日 / 日K）"}
      onMouseEnter={onFlowEnter}
      onMouseLeave={onFlowLeave}
      onDoubleClick={(e) => {
        // 锁定态不响应（此时鼠标事件本已被系统穿透，这里是双重保险）
        if (locked) return;
        // 阻止冒泡：否则会连带触发 content-layer 上的"双击开设置窗"
        e.stopPropagation();
        onOpenChart?.(row.code, row.name ?? null);
      }}
      onContextMenu={(e) => {
        if (locked) return;
        // 必须自己 preventDefault + stopPropagation：
        // stopPropagation 挡住外层 .app 的通用菜单（否则两个菜单一起弹），
        // 而一旦挡了冒泡，外层那句 preventDefault 就不会执行 ——
        // webview 自带的"检查元素"菜单得由这里挡掉。
        e.preventDefault();
        e.stopPropagation();
        onRowContextMenu?.(row.code);
      }}
    >
      <div className="row-name" title={row.code}>
        {/* 名称拿不到时显示代码，不显示空白。
            兜底也走 displayCode：否则会显示内部身份键 sh600519，
            与下面那个裸码 span 不一致（同一行出现两种代码写法很怪）。 */}
        <span className="name-text">{row.name ?? displayCode(row.code)}</span>
        {/* 条件渲染而非 visibility:hidden —— 关闭后布局自动收紧（标准 13） */}
        {/* 显示裸码（600519）而不是内部身份键（sh600519）—— 与行情软件一致。
            身份键必须保留前缀（东财裸码不唯一），故只在显示层剥。 */}
        {showCode && (
          <span className="code-text">{displayCode(row.code)}</span>
        )}
        {showMarket && (
          <span className="market-text">
            {MARKET_LABEL[row.market] ?? row.market}
          </span>
        )}
        {stale && <span className="badge" title={stale}>过期</span>}
        {isError && <span className="badge badge-err" title={row.state.kind === "error" ? row.state.message : ""}>无数据</span>}
      </div>

      <div className="row-chart">
        <MinuteChart
          series={row.minute}
          available={row.minute_available}
          changePercent={percent}
          width={chartWidth}
          height={chartHeight}
          lineWidth={curveWidth}
          alpha={curveOpacity}
        />
      </div>

      <div className="row-value">
        {/* 数据未到达时是占位符「—」，绝不显示 0 */}
        <div className="price" style={{ color }}>
          {quotePrice(quote, market)}
        </div>
        <div className="percent" style={{ color }}>
          {quote ? formatPercent(percent) : PLACEHOLDER}
        </div>
      </div>
    </div>
  );
}
