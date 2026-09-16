import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { changeColor } from "./lib/colors";
import { formatFlowCompact } from "./lib/formatMoney";
import type { FlowTipPayload } from "./types";
import "./flowtip.css";

/**
 * 悬停资金流提示窗（v0.3.0 定稿方案）。
 *
 * 鼠标悬停悬浮窗某一行时，主窗右侧贴着该行末尾浮现本窗，
 * 显示该股的主力/散户当日净流入。**主窗尺寸完全不动** —— 这是
 * 用户三轮反馈的核心诉求（常驻太宽/盖图表/整窗扩宽都被否）。
 *
 * 内容到达走双通道（与 useConfig/useSnapshot 同一套防竞态思路）：
 * - 事件 `flow-tip`（稳态路径）
 * - 启动时 invoke `flow_tip_current` 拉一次当前值（防"窗口刚建好、
 *   页面还没挂上 listen、第一个事件已发完"的首帧空白 —— 大图窗
 *   换股功能就栽过这个坑，见 ChartApp 的注释）
 */
function FlowTip() {
  const [payload, setPayload] = useState<FlowTipPayload | null>(null);

  useEffect(() => {
    const un = listen<FlowTipPayload>("flow-tip", (e) => setPayload(e.payload));
    un.catch((err) =>
      console.error("[资金流提示] 事件监听注册失败（查 capabilities 的 windows 列表）:", err)
    );
    invoke<FlowTipPayload | null>("flow_tip_current")
      .then((p) => setPayload((prev) => prev ?? p))
      .catch(() => void 0);
    return () => {
      void un.then((f) => f()).catch(() => void 0);
    };
  }, []);

  if (!payload) return null;
  const market = payload.market ?? "ashare";
  return (
    <div className="flow-tip">
      <span
        className="ft-chip"
        style={{ color: changeColor(payload.main_net, market) }}
      >
        主力 {formatFlowCompact(payload.main_net)}
      </span>
      <span className="ft-sep">·</span>
      <span
        className="ft-chip"
        style={{ color: changeColor(payload.retail_net, market) }}
      >
        散户 {formatFlowCompact(payload.retail_net)}
      </span>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<FlowTip />);
