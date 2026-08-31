import { useEffect, useRef, useState } from "react";
import type { Market } from "../../types";
import { changeColor, type Theme } from "../../lib/colors";

/** 一个分时点。当日分时用后端给的 grid_index，5 日分时用序号。 */
export interface LargePoint {
  /** 横轴位置（0..slots-1）。当日分时由后端 grid_index 提供，前端不推算时段 */
  slot: number;
  price: number;
  volume?: number;
  /** 该点之前是否断线（午休）。当日分时由后端 break_before 标记 */
  breakBefore?: boolean;
  /** 悬停显示用的时间文本，由调用方按周期格式化 */
  label?: string;
}

/** 一段序列。5 日分时会有 5 段，各自有独立昨收基准。 */
export interface LargeSegment {
  points: LargePoint[];
  prevClose: number | null;
  /** 段标题（5 日分时下是日期） */
  title?: string;
}

interface Props {
  segments: LargeSegment[];
  /** 每段的横轴槽位数（当日分时=市场栅格长度） */
  slotsPerSegment: number;
  market: Market;
  width: number;
  height: number;
  /** 是否画成交量柱（5 日分时有量，当日分时的腾讯通道无量） */
  showVolume: boolean;
  /** 界面主题。由父组件传入，理由同 CandleChart（Canvas 上色，CSS 覆盖不到） */
  theme?: Theme;
}

const VOL_RATIO = 0.2;
const PAD = { left: 56, right: 44, top: 10, bottom: 20 };

/**
 * 分时大图：昨收基准线 + 涨跌区域填充 + 均价线 + 十字光标读数。
 *
 * 当日分时与 5 日分时共用此组件 —— 区别只在段数与槽位数，
 * 由调用方把两种后端结构都归一成 `LargeSegment[]`。
 *
 * 右侧 Y 轴显示涨跌幅百分比（相对昨收），这是分时图的惯例读法。
 */
export default function IntradayLarge({
  segments,
  slotsPerSegment,
  market,
  width,
  height,
  showVolume,
  theme = "dark",
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [hover, setHover] = useState<{ seg: number; idx: number; y: number } | null>(
    null
  );
  /**
   * 横向缩放。1 = 完整铺满，>1 = 放大。
   *
   * 只做横向：分时图的纵轴是"相对昨收的对称偏离"，纵向放大会让基准线跑出视野，
   * 反而看不出涨跌 —— 用户要放大看的是时间维度上的细节。
   */
  const [zoom, setZoom] = useState(1);
  /** 横向平移（像素）。缩放后用它把关注的时间段移进视野 */
  const [panX, setPanX] = useState(0);
  /**
   * 拖动起点。null = 未在拖动。
   *
   * 存 `panX` 的起始值而不是每帧累加增量：累加会让误差逐帧堆积，
   * 快速拖动时曲线会与光标越拖越偏。
   */
  const dragRef = useRef<{ x: number; panAt: number } | null>(null);

  const plotW = width - PAD.left - PAD.right;
  const plotH = height - PAD.top - PAD.bottom;
  const volH = showVolume ? plotH * VOL_RATIO : 0;
  const lineH = plotH - volH - (showVolume ? 8 : 0);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const allPts = segments.flatMap((s) => s.points);
    if (allPts.length === 0) {
      ctx.fillStyle = "rgba(232,234,237,0.5)";
      ctx.font = "12px 'Microsoft YaHei', sans-serif";
      ctx.textAlign = "center";
      ctx.fillText("该周期暂无数据", width / 2, height / 2);
      return;
    }

    // ── Y 轴：以「相对昨收的最大偏离」对称取范围，让基准线居中
    // 这是分时图的标准做法 —— 基准线不居中会让涨跌幅视觉上失真
    const base = segments[0].prevClose ?? allPts[0].price;
    let maxDev = 0;
    for (const s of segments) {
      const b = s.prevClose ?? base;
      for (const p of s.points) {
        const dev = Math.abs(p.price - b) / (b || 1);
        if (dev > maxDev) maxDev = dev;
      }
    }
    if (maxDev <= 0) maxDev = 0.01;
    maxDev *= 1.08; // 留一点余量，别让极值贴边

    const yOf = (price: number, segBase: number) => {
      const dev = (price - segBase) / (segBase || 1);
      return PAD.top + (1 - (dev / maxDev + 1) / 2) * lineH;
    };

    let volMax = 0;
    for (const p of allPts) if ((p.volume ?? 0) > volMax) volMax = p.volume ?? 0;
    const volTop = PAD.top + lineH + 8;
    const yVol = (v: number) =>
      volMax > 0 ? volTop + (1 - v / volMax) * volH : volTop + volH;

    // 横轴：每段等分，再叠加缩放与平移。
    //
    // **缩放只注入这一个函数**，8 个调用点（曲线/填充/量柱/网格/十字线…）
    // 全部自动跟随 —— 逐个改调用点必然漏掉一两处，表现是"放大后量柱和曲线错位"。
    const segW = plotW / segments.length;
    const xOf = (segIdx: number, slot: number) =>
      PAD.left +
      panX +
      zoom * (segW * segIdx + (slot / Math.max(1, slotsPerSegment - 1)) * segW);

    // ── 网格 + 双侧刻度（左价格、右涨跌幅）
    ctx.strokeStyle = "rgba(255,255,255,0.07)";
    ctx.font = "10px 'Microsoft YaHei', sans-serif";
    ctx.textBaseline = "middle";
    const TICKS = 4;
    for (let i = 0; i <= TICKS; i++) {
      const devRatio = maxDev * (1 - (2 * i) / TICKS);
      const price = base * (1 + devRatio);
      const y = PAD.top + (i / TICKS) * lineH;
      ctx.beginPath();
      ctx.moveTo(PAD.left, y);
      ctx.lineTo(PAD.left + plotW, y);
      ctx.stroke();

      const col = changeColor(devRatio, market, theme);
      ctx.fillStyle = i === TICKS / 2 ? "rgba(232,234,237,0.6)" : col;
      ctx.textAlign = "right";
      ctx.fillText(price.toFixed(2), PAD.left - 6, y);
      ctx.textAlign = "left";
      ctx.fillText(
        `${devRatio >= 0 ? "+" : ""}${(devRatio * 100).toFixed(2)}%`,
        PAD.left + plotW + 6,
        y
      );
    }

    // ── 以下内容随缩放平移，必须裁进绘图区
    //
    // 不裁的话放大后曲线会画到左侧价格刻度和右侧涨跌幅标签上面 ——
    // 数字被曲线穿过，比不能缩放更糟。
    // 网格与刻度画在 clip 之前（它们是固定坐标系，不该跟着动）。
    ctx.save();
    ctx.beginPath();
    ctx.rect(PAD.left, PAD.top, plotW, plotH);
    ctx.clip();

    // 段分隔竖线（5 日分时用）。走 xOf 而不是裸 segW*i —— 它也要跟着缩放，
    // 否则放大后分隔线会与实际段边界错位。
    if (segments.length > 1) {
      ctx.strokeStyle = "rgba(255,255,255,0.12)";
      for (let i = 1; i < segments.length; i++) {
        const x = xOf(i, 0);
        ctx.beginPath();
        ctx.moveTo(x, PAD.top);
        ctx.lineTo(x, PAD.top + plotH);
        ctx.stroke();
      }
    }

    // ── 逐段画线 + 填充 + 均价线
    segments.forEach((seg, si) => {
      if (seg.points.length === 0) return;
      const segBase = seg.prevClose ?? base;
      const lastPrice = seg.points[seg.points.length - 1].price;
      const color = changeColor(lastPrice - segBase, market, theme);

      // 涨跌区域填充（相对昨收）
      const zeroY = yOf(segBase, segBase);
      ctx.save();
      ctx.beginPath();
      let started = false;
      seg.points.forEach((p) => {
        const x = xOf(si, p.slot);
        const y = yOf(p.price, segBase);
        if (!started || p.breakBefore) {
          ctx.moveTo(x, y);
          started = true;
        } else ctx.lineTo(x, y);
      });
      ctx.lineTo(xOf(si, seg.points[seg.points.length - 1].slot), zeroY);
      ctx.lineTo(xOf(si, seg.points[0].slot), zeroY);
      ctx.closePath();
      const grad = ctx.createLinearGradient(0, PAD.top, 0, PAD.top + lineH);
      grad.addColorStop(0, `${color}33`);
      grad.addColorStop(1, `${color}05`);
      ctx.fillStyle = grad;
      ctx.fill();
      ctx.restore();

      // 价格线：只按后端给的 breakBefore 断线，前端不判断午休时刻
      ctx.strokeStyle = color;
      ctx.lineWidth = 1.4;
      ctx.beginPath();
      started = false;
      seg.points.forEach((p) => {
        const x = xOf(si, p.slot);
        const y = yOf(p.price, segBase);
        if (!started || p.breakBefore) {
          ctx.moveTo(x, y);
          started = true;
        } else ctx.lineTo(x, y);
      });
      ctx.stroke();

      // 均价线（累计成交额/累计量的近似：用价格累积均值，无量时退化为算术均值）
      ctx.strokeStyle = "rgba(232,180,76,0.85)";
      ctx.lineWidth = 1;
      ctx.setLineDash([4, 3]);
      ctx.beginPath();
      let sumPV = 0;
      let sumV = 0;
      let sumP = 0;
      started = false;
      seg.points.forEach((p, i) => {
        const v = p.volume ?? 0;
        sumPV += p.price * v;
        sumV += v;
        sumP += p.price;
        const avg = sumV > 0 ? sumPV / sumV : sumP / (i + 1);
        const x = xOf(si, p.slot);
        const y = yOf(avg, segBase);
        if (!started) {
          ctx.moveTo(x, y);
          started = true;
        } else ctx.lineTo(x, y);
      });
      ctx.stroke();
      ctx.setLineDash([]);

      // 量柱
      if (showVolume) {
        ctx.fillStyle = `${color}99`;
        const bw = Math.max(1, segW / Math.max(1, seg.points.length) * 0.7);
        seg.points.forEach((p) => {
          const v = p.volume ?? 0;
          if (v <= 0) return;
          const x = xOf(si, p.slot);
          const y = yVol(v);
          ctx.fillRect(x - bw / 2, y, bw, volTop + volH - y);
        });
      }

      // 段标题（5 日分时下是日期）。走 xOf 取段中点 —— 缩放后要跟着段走
      if (seg.title && segments.length > 1) {
        ctx.fillStyle = "rgba(232,234,237,0.45)";
        ctx.textAlign = "center";
        ctx.textBaseline = "top";
        ctx.fillText(
          seg.title,
          xOf(si, Math.max(1, slotsPerSegment - 1) / 2),
          PAD.top + plotH + 4
        );
      }
    });

    // 缩放内容画完，恢复裁剪 —— 下面的基准线与十字光标是全宽的固定元素
    ctx.restore();

    // 昨收基准线（画在最上层，虚线）
    ctx.strokeStyle = "rgba(255,255,255,0.28)";
    ctx.setLineDash([5, 4]);
    const zy = PAD.top + lineH / 2;
    ctx.beginPath();
    ctx.moveTo(PAD.left, zy);
    ctx.lineTo(PAD.left + plotW, zy);
    ctx.stroke();
    ctx.setLineDash([]);

    // 十字光标
    if (hover) {
      const seg = segments[hover.seg];
      const p = seg?.points[hover.idx];
      if (p) {
        const x = xOf(hover.seg, p.slot);
        ctx.strokeStyle = "rgba(255,255,255,0.35)";
        ctx.setLineDash([3, 3]);
        ctx.beginPath();
        ctx.moveTo(x, PAD.top);
        ctx.lineTo(x, PAD.top + plotH);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(PAD.left, hover.y);
        ctx.lineTo(PAD.left + plotW, hover.y);
        ctx.stroke();
        ctx.setLineDash([]);
      }
    }
  }, [
    segments,
    slotsPerSegment,
    market,
    width,
    height,
    showVolume,
    hover,
    plotW,
    plotH,
    lineH,
    volH,
    // 缩放与平移必须进依赖，否则滚轮改了值画面不重绘
    zoom,
    panX,
  ]);

  /**
   * 滚轮缩放。**以光标位置为锚点** —— 光标下那个时间点保持不动。
   *
   * 不做锚点的话（只改 zoom 不调 panX），放大时整段曲线会朝右侧滑走，
   * 用户想看的位置反而被推出视野，得再手动拖回来。
   *
   * 锚点公式推导：设光标屏幕位置 mx 对应的图内坐标为
   *   local = (mx - PAD.left - panX) / zoom
   * 缩放后要让同一个 local 仍落在 mx 上：
   *   mx = PAD.left + panX' + zoom' * local
   * 解出 panX' = mx - PAD.left - zoom' * local
   *
   * 用 ref + addEventListener(passive:false) 而不是 React 的 onWheel：
   * React 的 wheel 监听是 passive 的，里面调 preventDefault 无效
   * （控制台会警告），而不拦就会连带滚动大图窗页面。
   */
  useEffect(() => {
    const el = canvasRef.current;
    if (!el) return;

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = el.getBoundingClientRect();
      const mx = e.clientX - rect.left;

      // 每格滚轮缩放 1.15 倍。上限 12 倍：分时最密是 242 点，
      // 再放大就是在看插值噪声而非数据。下限 1 = 完整铺满，不允许缩得更小
      // （留白没有信息量）。
      const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
      setZoom((z) => {
        const next = Math.max(1, Math.min(12, z * factor));
        if (next === z) return z;

        setPanX((px) => {
          const local = (mx - PAD.left - px) / z;
          let nextPan = mx - PAD.left - next * local;
          // 夹住平移量：不允许左边界右移进绘图区、也不允许右边界左移进来，
          // 否则会露出空白。next === 1 时强制归零，回到完整视图。
          const maxPan = 0;
          const minPan = plotW - plotW * next;
          nextPan = Math.max(minPan, Math.min(maxPan, nextPan));
          return next === 1 ? 0 : nextPan;
        });
        return next;
      });
    };

    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [plotW]);

  /** 缩放状态在切换周期/标的时要重置，否则新数据会沿用上一只的视野 */
  useEffect(() => {
    setZoom(1);
    setPanX(0);
  }, [segments, slotsPerSegment]);

  // 参数类型收窄到两种事件的公共部分：hover 现在走 pointer 事件，
  // 但只用到 clientX/clientY/currentTarget，写成公共形状就不必区分来源
  const onMove = (e: {
    clientX: number;
    clientY: number;
    currentTarget: HTMLCanvasElement;
  }) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    if (mx < PAD.left || mx > PAD.left + plotW) {
      setHover(null);
      return;
    }
    const segW = plotW / segments.length;
    // **命中判定要走 xOf 的逆变换。**
    // 绘制时 x = PAD.left + panX + zoom * localX，
    // 所以反算 localX = (mx - PAD.left - panX) / zoom。
    // 少了这一步，放大后十字光标会锁到错误的时间点
    // （鼠标指在 14:00，读数却是 10:30）。
    const localX = (mx - PAD.left - panX) / zoom;
    const si = Math.min(
      segments.length - 1,
      Math.max(0, Math.floor(localX / segW))
    );
    const seg = segments[si];
    if (!seg || seg.points.length === 0) {
      setHover(null);
      return;
    }
    // 按 slot 找最近点（点位可能不连续，不能直接按索引比例取）
    const relSlot =
      ((localX - segW * si) / segW) * Math.max(1, slotsPerSegment - 1);
    let best = 0;
    let bestD = Infinity;
    seg.points.forEach((p, i) => {
      const d = Math.abs(p.slot - relSlot);
      if (d < bestD) {
        bestD = d;
        best = i;
      }
    });
    setHover({ seg: si, idx: best, y: my });
  };

  const hp = hover ? segments[hover.seg]?.points[hover.idx] : null;
  const hBase = hover
    ? segments[hover.seg]?.prevClose ?? segments[0].prevClose ?? null
    : null;
  const hPct =
    hp && hBase && hBase > 0 ? ((hp.price - hBase) / hBase) * 100 : null;

  return (
    <div className="candle-wrap">
      <canvas
        ref={canvasRef}
        style={{
          width,
          height,
          // 只在放大状态给 grab 光标：zoom=1 时拖不动（没有可平移的余量），
          // 显示 grab 会让用户以为能拖
          cursor: zoom > 1 ? (dragRef.current ? "grabbing" : "grab") : "default",
        }}
        /**
         * **hover 与拖动共用 Pointer Events 一条事件流。**
         *
         * 原先 hover 挂 onMouseMove、拖动挂 onPointerMove，两套并存。
         * 问题是拖动时 `setPointerCapture` 会把指针事件重定向到捕获元素，
         * mouse 事件在捕获期间不再可靠 —— 表现就是"能放大能拖，但十字光标没了"
         * （2026-08-31 用户报的现象）。
         *
         * 统一到 pointer 后只有一条路径：没在拖动就更新 hover，在拖动就平移。
         */
        onPointerMove={(e) => {
          const d = dragRef.current;
          if (d) {
            // 拖动中：平移，且不更新 hover（拖的时候十字跟着跳很乱）
            // 以起点为基准算绝对偏移，不逐帧累加（累加会积累误差）
            const raw = d.panAt + (e.clientX - d.x);
            // 夹在边界内：左右边界都不能移进绘图区，否则会拖出空白
            const minPan = plotW - plotW * zoom;
            setPanX(Math.max(minPan, Math.min(0, raw)));
            return;
          }
          onMove(e);
        }}
        onPointerLeave={() => setHover(null)}
        /**
         * 拖动平移。只在 zoom > 1 时生效 —— 完整视图没有可移动的余量。
         *
         * setPointerCapture：指针拖出画布边界后仍要继续收 move/up，
         * 否则用户拖快一点、光标掠出画布就"断线"，手感是拖到一半突然停住。
         */
        onPointerDown={(e) => {
          if (zoom <= 1 || e.button !== 0) return;
          dragRef.current = { x: e.clientX, panAt: panX };
          // 按下就先清掉十字：接下来是拖动，十字留着会与拖动的视觉打架
          setHover(null);
          e.currentTarget.setPointerCapture(e.pointerId);
        }}
        onPointerUp={(e) => {
          dragRef.current = null;
          e.currentTarget.releasePointerCapture(e.pointerId);
        }}
        onPointerCancel={() => {
          dragRef.current = null;
        }}
        // 双击复位。缩放进去之后没有别的回头路 —— 只靠反向滚轮要滚很多下，
        // 而且很难恰好停在 zoom=1
        onDoubleClick={() => {
          setZoom(1);
          setPanX(0);
        }}
        title={zoom > 1 ? "滚轮缩放，按住拖动平移，双击复位" : "滚轮放大"}
      />
      {/* 缩放倍数仅在放大时显示：zoom=1 时它没有信息量，
          常驻反而挤占读数区。同时它也是"双击可复位"的提示位。 */}
      {zoom > 1 && (
        <div className="zoom-badge" onDoubleClick={() => { setZoom(1); setPanX(0); }}>
          {zoom.toFixed(1)}× · 双击复位
        </div>
      )}
      <div className="candle-readout">
        {hp ? (
          <>
            {hp.label && <span className="ro-date">{hp.label}</span>}
            <span>价 {hp.price.toFixed(2)}</span>
            {hPct != null && (
              <span style={{ color: changeColor(hPct, market, theme) }}>
                {hPct >= 0 ? "+" : ""}
                {hPct.toFixed(2)}%
              </span>
            )}
            {hp.volume != null && hp.volume > 0 && (
              <span>量 {Math.round(hp.volume).toLocaleString()}</span>
            )}
          </>
        ) : (
          <span className="ro-hint">鼠标移到图上看分时详情</span>
        )}
      </div>
    </div>
  );
}
