import { useEffect, useRef, useState } from "react";
import type { KlineBar, MaLine, Market } from "../../types";
import { changeColor, type Theme } from "../../lib/colors";

interface Props {
  bars: KlineBar[];
  /** 均线组（周期+逐根值）。v0.3.0 起由配置决定，数量与周期都不再固定 */
  mas: MaLine[];
  market: Market;
  width: number;
  height: number;
  /**
   * 界面主题。**必须由父组件传入**，不在这里自己订阅配置：
   * 本组件在 Canvas 上逐帧绘制，多一个配置订阅就多一次整图重绘。
   * 蜡烛与涨跌文字用内联/Canvas 上色，CSS token 覆盖不到，所以只能走参数。
   */
  theme?: Theme;
}

/** 量柱区占总高的比例，其余留给蜡烛区 */
const VOL_RATIO = 0.22;
const PAD = { left: 52, right: 12, top: 10, bottom: 20 };

/**
 * 均线配色。前两位是历史色（MA5 黄 / MA20 蓝，v0.2.x 的既有观感，
 * 配置默认 [5,20] 时按列表顺序取色正好复现）；其余按列表顺序轮转。
 * 不按"周期→颜色"做稳定映射：8 色轮转下 5/10/15/20 这类等差周期
 * 会撞同一色相，按顺序轮转的可区分性更好。
 */
const MA_PALETTE = [
  "#e8b64c", // 黄（MA5 位置）
  "#5b9bd5", // 蓝（MA20 位置）
  "#c77dff", // 紫
  "#7fd18c", // 绿
  "#f2907b", // 橙红
  "#8fc9e8", // 浅蓝
  "#d5b899", // 沙色
  "#a0a8b8", // 灰蓝
];

/** 鼠标悬停时的十字光标信息 */
interface HoverInfo {
  index: number;
  x: number;
  y: number;
}

/**
 * 日K 蜡烛图：蜡烛 + MA5/MA20 + 成交量柱 + 十字光标读数。
 *
 * 自绘 Canvas 不引图表库 —— 依赖增量为零，且蜡烛的涨跌配色要按市场切换
 * （A股港股红涨绿跌、美股绿涨红跌），现成库反而要绕。
 */
export default function CandleChart({
  bars,
  mas,
  market,
  width,
  height,
  theme = "dark",
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [hover, setHover] = useState<HoverInfo | null>(null);
  /**
   * 横向缩放。1 = 完整铺满，>1 = 放大。与分时图同一套机制。
   *
   * 只做横向：纵轴是价格区间，纵向放大会让部分蜡烛跑出视野，
   * 而看日K要的是"时间轴上放大看细节"。
   */
  const [zoom, setZoom] = useState(1);
  /** 横向平移（像素） */
  const [panX, setPanX] = useState(0);
  /** 拖动起点。存起始 panX 而非逐帧累加，避免误差堆积 */
  const dragRef = useRef<{ x: number; panAt: number } | null>(null);

  const plotW = width - PAD.left - PAD.right;
  const plotH = height - PAD.top - PAD.bottom;
  const volH = plotH * VOL_RATIO;
  const candleH = plotH - volH - 8;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || bars.length === 0) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    // ── Y 轴范围：蜡烛区取最高最低，均线也要纳入否则会画出框外
    let hi = -Infinity;
    let lo = Infinity;
    for (const b of bars) {
      if (b.high > hi) hi = b.high;
      if (b.low < lo) lo = b.low;
    }
    for (const line of mas) {
      for (const v of line.values) {
        if (v == null) continue;
        if (v > hi) hi = v;
        if (v < lo) lo = v;
      }
    }
    if (!Number.isFinite(hi) || !Number.isFinite(lo) || hi <= lo) {
      hi = lo + 1;
    }
    const span = hi - lo;
    const yOf = (p: number) => PAD.top + (1 - (p - lo) / span) * candleH;

    let volMax = 0;
    for (const b of bars) if (b.volume > volMax) volMax = b.volume;
    const volTop = PAD.top + candleH + 8;
    const yVol = (v: number) =>
      volMax > 0 ? volTop + (1 - v / volMax) * volH : volTop + volH;

    // 每根蜡烛的水平槽宽
    const slot = plotW / bars.length;
    // 蜡烛实体宽度跟着缩放走：放大时蜡烛要变粗，否则放大后只是间距变宽、
    // 蜡烛仍是细线，看不出"放大看细节"的效果。上限 14px 防止过度放大后糊成色块
    const bodyW = Math.max(1, Math.min(slot * zoom * 0.62, 14));
    // 缩放与平移**只注入这一个函数**，蜡烛/影线/均线/量柱/日期/十字线
    // 全部自动跟随 —— 逐个改调用点必然漏一两处，表现是"放大后量柱和蜡烛错位"
    const xOf = (i: number) => PAD.left + panX + zoom * (slot * (i + 0.5));

    // ── 网格与价格刻度
    ctx.strokeStyle = "rgba(255,255,255,0.07)";
    ctx.fillStyle = "rgba(232,234,237,0.55)";
    ctx.font = "10px 'Microsoft YaHei', sans-serif";
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    const TICKS = 5;
    for (let i = 0; i <= TICKS; i++) {
      const p = lo + (span * i) / TICKS;
      const y = yOf(p);
      ctx.beginPath();
      ctx.moveTo(PAD.left, y);
      ctx.lineTo(PAD.left + plotW, y);
      ctx.stroke();
      ctx.fillText(p.toFixed(2), PAD.left - 6, y);
    }
    // 量柱区上沿
    ctx.beginPath();
    ctx.moveTo(PAD.left, volTop + volH);
    ctx.lineTo(PAD.left + plotW, volTop + volH);
    ctx.stroke();

    // ── 以下内容随缩放平移，必须裁进绘图区
    //
    // 不裁的话放大后蜡烛会画到左侧价格刻度上面 —— 数字被蜡烛穿过，
    // 比不能缩放更糟。网格与刻度画在 clip 之前（它们是固定坐标系，不跟着动）。
    ctx.save();
    ctx.beginPath();
    ctx.rect(PAD.left, PAD.top, plotW, plotH);
    ctx.clip();

    // ── 蜡烛与量柱
    for (let i = 0; i < bars.length; i++) {
      const b = bars[i];
      // 与前一根收盘比：这是行情软件的惯例（不是与本根开盘比）
      const prevClose = i > 0 ? bars[i - 1].close : b.open;
      const delta = b.close - prevClose;
      const color = changeColor(delta, market, theme);
      const rising = b.close >= b.open;

      const x = xOf(i);
      // 影线
      ctx.strokeStyle = color;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x, yOf(b.high));
      ctx.lineTo(x, yOf(b.low));
      ctx.stroke();

      // 实体：涨用空心（只描边）、跌用实心 —— 与主流看盘软件一致
      const yOpen = yOf(b.open);
      const yClose = yOf(b.close);
      const top = Math.min(yOpen, yClose);
      const h = Math.max(1, Math.abs(yClose - yOpen));
      if (rising) {
        ctx.strokeStyle = color;
        ctx.strokeRect(x - bodyW / 2, top, bodyW, h);
      } else {
        ctx.fillStyle = color;
        ctx.fillRect(x - bodyW / 2, top, bodyW, h);
      }

      // 量柱同色
      ctx.fillStyle = color;
      const vy = yVol(b.volume);
      ctx.fillRect(x - bodyW / 2, vy, bodyW, volTop + volH - vy);
    }

    // ── 均线（数量与周期由配置决定，颜色按列表顺序取调色板）
    const drawMA = (arr: (number | null)[], stroke: string) => {
      ctx.strokeStyle = stroke;
      ctx.lineWidth = 1.2;
      ctx.beginPath();
      let started = false;
      for (let i = 0; i < arr.length; i++) {
        const v = arr[i];
        if (v == null) {
          started = false;
          continue;
        }
        const x = xOf(i);
        const y = yOf(v);
        if (started) ctx.lineTo(x, y);
        else {
          ctx.moveTo(x, y);
          started = true;
        }
      }
      ctx.stroke();
    };
    mas.forEach((line, i) => {
      drawMA(line.values, MA_PALETTE[i % MA_PALETTE.length]);
    });

    // 缩放内容画完，恢复裁剪。
    // **必须在日期刻度之前恢复** —— 日期画在 plotH + 4（绘图区下方），
    // 仍在 clip 里会被整行裁掉，表现是"放大后横轴日期消失"。
    ctx.restore();

    // ── 日期刻度（首、中、末三处，避免拥挤）
    ctx.fillStyle = "rgba(232,234,237,0.5)";
    ctx.textBaseline = "top";
    const marks = [0, Math.floor(bars.length / 2), bars.length - 1];
    for (const i of marks) {
      const b = bars[i];
      if (!b) continue;
      ctx.textAlign = i === 0 ? "left" : i === bars.length - 1 ? "right" : "center";
      ctx.fillText(b.date.slice(5), xOf(i), PAD.top + plotH + 4);
    }

    // ── 十字光标
    if (hover) {
      const b = bars[hover.index];
      if (b) {
        const x = xOf(hover.index);
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
    // 缩放与平移必须进依赖，否则滚轮改了值画面不重绘
  }, [
    bars,
    mas,
    market,
    width,
    height,
    hover,
    plotW,
    plotH,
    candleH,
    volH,
    theme,
    zoom,
    panX,
  ]);

  /**
   * 滚轮缩放。以光标位置为锚点 —— 光标下那根蜡烛保持不动。
   *
   * 不做锚点的话（只改 zoom 不调 panX），放大时整段会朝一侧滑走，
   * 用户想看的位置被推出视野，还得手动拖回来。
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
      // 每格 1.15 倍。上限 12：日K 常见 120~250 根，再放大就只剩几根占满全屏
      const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
      setZoom((z) => {
        const next = Math.max(1, Math.min(12, z * factor));
        if (next === z) return z;
        setPanX((px) => {
          const local = (mx - PAD.left - px) / z;
          const raw = mx - PAD.left - next * local;
          const minPan = plotW - plotW * next;
          // next === 1 时强制归零，干净回到完整视图
          return next === 1 ? 0 : Math.max(minPan, Math.min(0, raw));
        });
        return next;
      });
    };

    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [plotW]);

  /** 换标的/换周期时重置视野，否则新数据会沿用上一只的缩放位置 */
  useEffect(() => {
    setZoom(1);
    setPanX(0);
  }, [bars]);

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
    if (mx < PAD.left || mx > PAD.left + plotW || bars.length === 0) {
      setHover(null);
      return;
    }
    const slot = plotW / bars.length;
    // **命中判定要走 xOf 的逆变换。**
    // 绘制时 x = PAD.left + panX + zoom * (slot * (i + 0.5))，
    // 所以反算 local = (mx - PAD.left - panX) / zoom。
    // 少这一步，放大后十字光标会锁到错误的那根蜡烛
    // （鼠标指着某根，读数是另一根的）。
    const local = (mx - PAD.left - panX) / zoom;
    const idx = Math.min(bars.length - 1, Math.max(0, Math.floor(local / slot)));
    setHover({ index: idx, x: mx, y: my });
  };

  const hb = hover ? bars[hover.index] : null;
  const hPrevClose =
    hover && hover.index > 0 ? bars[hover.index - 1].close : hb?.open ?? null;
  const hPct =
    hb && hPrevClose && hPrevClose > 0
      ? ((hb.close - hPrevClose) / hPrevClose) * 100
      : null;

  return (
    <div className="candle-wrap">
      <canvas
        ref={canvasRef}
        style={{
          width,
          height,
          // 只在放大状态给 grab：zoom=1 时没有可平移余量，显示 grab 会误导
          cursor: zoom > 1 ? (dragRef.current ? "grabbing" : "grab") : "default",
        }}
        /**
         * **hover 与拖动共用 Pointer Events 一条事件流。**
         *
         * 原先 hover 挂 onMouseMove、拖动挂 onPointerMove，两套并存。
         * 拖动时 `setPointerCapture` 会把指针事件重定向到捕获元素，
         * mouse 事件在捕获期间不再可靠 —— 表现是"能放大能拖，但十字光标没了"。
         * 统一到 pointer 后只有一条路径：没在拖动就更新 hover，在拖动就平移。
         */
        onPointerMove={(e) => {
          const d = dragRef.current;
          if (d) {
            // 拖动中：平移，且不更新 hover（拖的时候十字跟着跳很乱）
            // 以起点为基准算绝对偏移，不逐帧累加（累加会积累误差）
            const raw = d.panAt + (e.clientX - d.x);
            const minPan = plotW - plotW * zoom;
            setPanX(Math.max(minPan, Math.min(0, raw)));
            return;
          }
          onMove(e);
        }}
        onPointerLeave={() => setHover(null)}
        // 拖动平移。setPointerCapture：指针拖出画布后仍要继续收 move/up，
        // 否则拖快一点掠出边界就"断线"
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
        onDoubleClick={() => {
          setZoom(1);
          setPanX(0);
        }}
        title={zoom > 1 ? "滚轮缩放，按住拖动平移，双击复位" : "滚轮放大"}
      />
      {/* 缩放倍数徽标，同时是"双击可复位"的提示位。zoom=1 时不显示（无信息量） */}
      {zoom > 1 && (
        <div
          className="zoom-badge"
          onDoubleClick={() => {
            setZoom(1);
            setPanX(0);
          }}
        >
          {zoom.toFixed(1)}× · 双击复位
        </div>
      )}
      <div className="candle-readout">
        {hb ? (
          <>
            <span className="ro-date">{hb.date}</span>
            <span>开 {hb.open.toFixed(2)}</span>
            <span>高 {hb.high.toFixed(2)}</span>
            <span>低 {hb.low.toFixed(2)}</span>
            <span>收 {hb.close.toFixed(2)}</span>
            {hPct != null && (
              <span style={{ color: changeColor(hPct, market, theme) }}>
                {hPct >= 0 ? "+" : ""}
                {hPct.toFixed(2)}%
              </span>
            )}
            {mas.map((line, i) => (
              <span
                key={line.period}
                style={{ color: MA_PALETTE[i % MA_PALETTE.length] }}
              >
                MA{line.period}{" "}
                {hover && line.values[hover.index] != null
                  ? line.values[hover.index]!.toFixed(2)
                  : "—"}
              </span>
            ))}
          </>
        ) : (
          <span className="ro-hint">鼠标移到图上看每日详情</span>
        )}
      </div>
    </div>
  );
}
