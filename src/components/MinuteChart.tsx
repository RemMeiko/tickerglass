import { useEffect, useMemo, useRef, useState } from "react";
import type { MinutePoint, MinuteSeries } from "../types";
import { changeColor } from "../lib/colors";
import { makeYScale, xOfGridIndex } from "../lib/chartGeom";

/** `930` → `"09:30"`。后端下发的是数值形式，显示层补零加冒号 */
function hhmmLabel(hhmm: number): string {
  return String(hhmm).padStart(4, "0").replace(/(\d{2})(\d{2})/, "$1:$2");
}

interface Props {
  series: MinuteSeries | null;
  /** false 时不画曲线。拿不到序列就留空，绝不补点伪造 */
  available: boolean;
  changePercent: number | null;
  width: number;
  height: number;
  /** 曲线线宽（配置驱动） */
  lineWidth: number;
  /** 曲线不透明度。用 canvas globalAlpha，**不影响文字层**（标准 11 的独立性） */
  alpha: number;
}

/**
 * Canvas 2D 自绘分时折线。
 *
 * 绘制规则只有一条：`break_before === true` 就 `moveTo`，否则 `lineTo`。
 * 前端不出现任何 1130 / 1200 之类的时间常量 —— 午休在哪由后端标注。
 *
 * **不存在"点数不足时补点"的分支**：点少就画少，一个点都没有就不画。
 */
export default function MinuteChart({
  series,
  available,
  changePercent,
  width,
  height,
  lineWidth,
  alpha,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  // 悬停命中的点（含屏幕 x，供画竖线）。移出即 null
  const [hover, setHover] = useState<{ pt: MinutePoint; x: number } | null>(
    null
  );

  // 命中测试用：把可绘制的点按屏幕 x 排一遍，取最近的。
  // 空/不可用时为空数组，hover 永远命不中。
  const hitPoints = useMemo(() => {
    if (!series || !available) return [] as { x: number; pt: MinutePoint }[];
    return series.points.map((pt) => ({
      x: xOfGridIndex(pt.grid_index, series.grid_len, width),
      pt,
    }));
  }, [series, available, width]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // devicePixelRatio 适配，避免曲线模糊
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    if (!series) return;

    const prevClose = series.prev_close;

    // 昨收基准线始终可画（它是基准线，不是行情曲线）
    if (prevClose !== null && Number.isFinite(prevClose) && prevClose > 0) {
      const scaleForBase = makeYScale(
        available ? series.points : [],
        prevClose,
        height
      );
      const y = scaleForBase ? scaleForBase.toY(prevClose) : height / 2;
      ctx.save();
      ctx.strokeStyle = "rgba(200,205,215,0.35)";
      ctx.lineWidth = 1;
      ctx.setLineDash([2, 2]);
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(width, y);
      ctx.stroke();
      ctx.restore();
    }

    // 分时序列不可用 → 只留基准线，不画任何曲线
    if (!available || series.points.length === 0) return;

    const scale = makeYScale(series.points, prevClose, height);
    if (!scale) return;

    ctx.save();
    // 曲线不透明度用 globalAlpha：只作用于本 canvas 的绘制，
    // 文字在另一个 DOM 层，故调到 0 时文字不会变浅（标准 11 的独立性）
    ctx.globalAlpha = Number.isFinite(alpha)
      ? Math.min(1, Math.max(0, alpha))
      : 1;
    ctx.strokeStyle = changeColor(changePercent, series.market);
    ctx.lineWidth = Number.isFinite(lineWidth) && lineWidth > 0 ? lineWidth : 1.2;
    ctx.lineJoin = "round";
    ctx.beginPath();

    series.points.forEach((p, i) => {
      const x = xOfGridIndex(p.grid_index, series.grid_len, width);
      const y = scale.toY(p.price);
      // 首点显式 moveTo（不依赖 canvas「beginPath 后首个 lineTo 等价 moveTo」的隐式行为）；
      // 其余点唯一规则：break_before 就断线
      if (i === 0 || p.break_before) {
        ctx.moveTo(x, y);
      } else {
        ctx.lineTo(x, y);
      }
    });
    ctx.stroke();
    ctx.restore();

    // 悬停十字线与命中点。画在曲线之后，且**不受 alpha 影响** ——
    // 曲线不透明度调到 0 时读数线仍须可见，否则读数框有数字却看不到位置。
    if (hover) {
      const hx = hover.x;
      const hy = scale.toY(hover.pt.price);
      ctx.save();
      ctx.strokeStyle = "rgba(230,235,245,0.55)";
      ctx.lineWidth = 1;
      ctx.setLineDash([2, 2]);
      ctx.beginPath();
      ctx.moveTo(hx, 0);
      ctx.lineTo(hx, height);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.fillStyle = changeColor(changePercent, series.market);
      ctx.beginPath();
      ctx.arc(hx, hy, 2.2, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
    }
  }, [series, available, changePercent, width, height, lineWidth, alpha, hover]);

  const prevClose = series?.prev_close ?? null;
  // 读数里的涨跌幅按「该刻价 vs 昨收」现算，不能复用整行的 changePercent ——
  // 那是最新价的涨跌幅，鼠标停在上午某一刻时那个数字是错的
  const hoverPct =
    hover && prevClose !== null && prevClose > 0
      ? ((hover.pt.price - prevClose) / prevClose) * 100
      : null;

  return (
    <div
      style={{ position: "relative", width, height }}
      onMouseMove={(e) => {
        if (hitPoints.length === 0) return;
        const rect = e.currentTarget.getBoundingClientRect();
        const mx = e.clientX - rect.left;
        // 取屏幕 x 最近的点。240~391 个点挤在 ~100px 里，逐点比距离足够快
        let best = hitPoints[0];
        let bestD = Math.abs(best.x - mx);
        for (const c of hitPoints) {
          const d = Math.abs(c.x - mx);
          if (d < bestD) {
            best = c;
            bestD = d;
          }
        }
        setHover({ pt: best.pt, x: best.x });
      }}
      onMouseLeave={() => setHover(null)}
    >
      <canvas
        ref={canvasRef}
        style={{ width, height, display: "block" }}
        // 拖拽区留着：data-tauri-drag-region 只在 mousedown 生效，
        // 与 hover 读数互不干扰（悬停能读数，按下仍能拖窗口）
        data-tauri-drag-region
      />
      {hover && (
        // 绝对定位是硬要求：读数框若进常规流会撑大 .measure，
        // 触发 D4 的尺寸上报 → 悬浮窗宽高随鼠标抖动
        <div className="mc-readout">
          <span className="mc-t">{hhmmLabel(hover.pt.hhmm)}</span>
          <span
            className="mc-p"
            style={{ color: changeColor(hoverPct, series?.market ?? "ashare") }}
          >
            {hover.pt.price.toFixed(2)}
          </span>
          {hoverPct !== null && (
            <span
              className="mc-c"
              style={{ color: changeColor(hoverPct, series?.market ?? "ashare") }}
            >
              {hoverPct >= 0 ? "+" : ""}
              {hoverPct.toFixed(2)}%
            </span>
          )}
        </div>
      )}
    </div>
  );
}
