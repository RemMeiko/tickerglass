import { useEffect, useRef, useState } from "react";
import type { RowSnapshot } from "../types";
import { changeColor } from "../lib/colors";
import { PLACEHOLDER, formatPercent } from "../lib/format";
import {
  ROTATE_DURATION_MS,
  ROTATE_INTERVAL_MS,
  ballSize,
  clampIndex,
  nextIndex,
  shouldRotate,
} from "../lib/ballRotate";
import type { Theme } from "../lib/colors";

/**
 * 判定"这是一次点击"还是"用户想拖窗口"的位移阈值（CSS 像素）。
 *
 * 为什么需要它：球态下**不能挂 `data-tauri-drag-region`** —— 挂了之后
 * mousedown 立刻被系统接管去拖窗口，click 事件永远不触发，球就点不开了。
 * 所以两个手势都由我们自己分派：按下后位移小于阈值 = 点击（展开），
 * 超过阈值 = 调 `startDragging()` 交给系统拖窗。
 *
 * 取 4px 的理由：鼠标点击时手抖通常在 1~2px；触控板轻点可能到 3px。
 * 再大（如 8px）会让"想拖一点点"变成误展开。
 */
const CLICK_SLOP_PX = 4;

export default function StockBall({
  rows,
  locked,
  scale,
  theme,
  onExpand,
}: {
  rows: RowSnapshot[];
  locked: boolean;
  scale: number;
  theme: Theme;
  onExpand: () => void;
}) {
  /**
   * 当前显示第几只（0 起）。
   *
   * 这里**不做钳制**，渲染时才用 `clampIndex` 折进合法范围 ——
   * 因为 rows.length 会随用户增删而变，把钳制放在 setState 里就要在
   * rows 变化时再同步一次，多一个 effect 多一处竞态。
   */
  const [index, setIndex] = useState(0);
  /** 鼠标悬停时暂停轮播，让用户能看清当前这只 */
  const [hover, setHover] = useState(false);

  const len = rows.length;
  const safeIndex = clampIndex(index, len);
  const row = rows[safeIndex] ?? null;

  /**
   * 正在"转出去"的那一只。水晶球内旋转的关键 —— **两面必须同时存在**。
   *
   * 原来只有一个 `.ball-face`、key 绑索引，React 会销毁旧节点再挂载新节点：
   * 旧内容瞬间消失、新内容淡入，用户反馈的"消失出现"就是这个。
   * 真正的内旋转需要旧面转出（0 → -90°）与新面转进（90° → 0）同框，
   * 像球里一块牌子绕轴转过去。
   *
   * 存 row 快照而不是索引：转场这 620ms 里用户可能删股票，
   * 存索引会导致旧面突然指向另一只（或越界变空白）。
   */
  const [outgoing, setOutgoing] = useState<RowSnapshot | null>(null);
  /** 上一帧的索引，用来判断"这次渲染是不是发生了切换" */
  const prevIndexRef = useRef(safeIndex);
  /** 上一帧的 row，切换发生时它就是要转出去的那一只 */
  const prevRowRef = useRef(row);

  // ── 切换检测：索引变了就把上一只推入 outgoing，620ms 后清掉
  //
  // 为什么用 useEffect 而不是在渲染里直接比较：setState 必须在提交后调用。
  // 依赖只有 safeIndex —— rows 内容变化（行情刷新）不该触发转场动画，
  // 只有"换了另一只"才算切换。
  useEffect(() => {
    if (prevIndexRef.current === safeIndex) return;
    const leaving = prevRowRef.current;
    prevIndexRef.current = safeIndex;
    if (!leaving) return;

    setOutgoing(leaving);
    const timer = window.setTimeout(
      () => setOutgoing(null),
      ROTATE_DURATION_MS
    );
    // 清理很关键：切换比动画还快时（用户手动连点/极短周期）
    // 不清会留下永不消失的旧面
    return () => window.clearTimeout(timer);
  }, [safeIndex]);

  // prevRowRef 每帧同步，且**必须在上面那个 effect 之后** ——
  // 否则切换那一帧读到的已经是新 row，转出去的会是新面自己
  useEffect(() => {
    prevRowRef.current = row;
  }, [row]);

  // ── 5s 自动轮播
  //
  // 依赖里带 len：自选股数量变化时重建定时器，否则闭包里捕获的是旧长度，
  // 环绕位置会错（比如从 3 只加到 5 只，仍在前 3 只之间转）。
  // 带 hover：悬停时清掉定时器而不是"跳过一次"，这样移开鼠标后
  // 是完整的 5s 而不是残余的零点几秒。
  useEffect(() => {
    if (locked) return; // 锁定态下鼠标穿透，用户看不到也点不到，没必要重渲染
    if (hover) return;
    if (!shouldRotate(len)) return;

    const timer = window.setInterval(() => {
      setIndex((i) => nextIndex(i, len));
    }, ROTATE_INTERVAL_MS);

    return () => window.clearInterval(timer);
  }, [len, hover, locked]);

  // ── 点击 vs 拖窗的手势分派
  const down = useRef<{ x: number; y: number; dragging: boolean } | null>(null);

  const handlePointerDown = (e: React.PointerEvent) => {
    if (locked) return;
    // 只响应左键。右键要留给 context menu（外层 .app 上挂着）
    if (e.button !== 0) return;
    down.current = { x: e.clientX, y: e.clientY, dragging: false };
  };

  const handlePointerMove = (e: React.PointerEvent) => {
    const d = down.current;
    if (!d || d.dragging) return;
    const moved = Math.hypot(e.clientX - d.x, e.clientY - d.y);
    if (moved <= CLICK_SLOP_PX) return;
    // 超过阈值 → 交给系统拖窗。
    // 标记 dragging 后不再重复调用，也让 pointerup 知道这次不是点击。
    d.dragging = true;
    // 动态 import：@tauri-apps/api/window 只在这一个交互里用到，
    // 顶层 import 会把它算进悬浮窗首屏包体
    void import("@tauri-apps/api/window")
      .then(({ getCurrentWindow }) => getCurrentWindow().startDragging())
      .catch((err) => console.error("启动拖窗失败:", err));
  };

  const handlePointerUp = () => {
    const d = down.current;
    down.current = null;
    if (!d || d.dragging) return;
    onExpand();
  };

  const size = ballSize(scale);
  const percent = row?.quote?.change.percent ?? null;
  const color = row ? changeColor(percent, row.market, theme) : undefined;

  return (
    <div
      className="ball-wrap"
      style={{ width: size.diameter, height: size.diameter }}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      // 指针离开窗口时也要清状态，否则下次 pointerup 会误判成点击
      onPointerLeave={() => {
        down.current = null;
        setHover(false);
      }}
      onPointerEnter={() => setHover(true)}
      title={locked ? "" : "单击展开列表 · 拖动可移动 · 右键菜单"}
    >
      {/* 球体。四层叠出星空的深邃与体积（顺序即绘制顺序，后者盖前者）：
          .ball-body  = 主球面（深蓝径向渐变 + 两片星云）+ 外缘暗部
          .ball-core  = 内部深层（inset 阴影往里收，制造"球里还有东西"的体积感）
          .ball-stars = 星点层（box-shadow 铺 14 组坐标 ×3 层 = 42 颗，带闪烁与慢自转）
          .ball-gloss = 高光斑（左上小片镜面反射，让表面像玻璃而不是磨砂纸）
          四层都 pointer-events:none，命中测试留给外层 .ball-wrap 统一处理 */}
      <div className="ball-body" aria-hidden="true" />
      <div className="ball-core" aria-hidden="true" />
      {/* ⚠️ 星点层**必须排在 .ball-core 之后** —— 排在前面时会被 core 的
          渐变（中心 62%/70%，偏右下）盖住，右下那几颗暗星直接消失。
          真实夜空里星光穿透一切，不该被"球壳阴影"压住。 */}
      {/* 两层嵌套是**必须的**，不是多余包装：
          外层 .ball-stars 挂 150s 慢自转，内层 .ball-stars-inner 挂星图与闪烁。
          一个元素上 `transform` 是单一属性，挂不了"自转 + 闪烁"两条不同周期的
          动画（后声明的会整条覆盖前者）。自转用 transform、闪烁用 opacity，
          分到两层才互不干扰。 */}
      <div className="ball-stars" aria-hidden="true">
        <div className="ball-stars-inner" />
      </div>
      <div className="ball-gloss" aria-hidden="true" />

      {/* 文字层。**滚动播放**：转场期间两面在同一条轨道上横向平移，
          旧面滑出左侧、新面从右侧滑入，前后紧挨着 ——
          任意时刻球心都被其中一面占满，不存在空档。

          为什么不是绕轴旋转（上一版的做法）：两面绕同一根中心轴转，
          中点时各在 ±45°，横向投影压缩到 71% 且各自侧移 15.6px、
          中心间距 31.1px，而球直径只有 60px —— 球心必然露出空档。
          用户反馈的"中间消失一部分再出现"就是这个几何造成的，不是 bug。
          滚动的本质是平移，不是旋转。

          .ball-viewport 单独一层负责裁剪：`overflow: hidden` 会强制拍平
          3D 上下文，不能与 `preserve-3d` 放在同一元素上，所以裁剪在外、
          3D 在内（.ball-spindle）。

          key 绑索引 → 换股票时 React 重建新面节点 → CSS 动画重放。
          不加 key 的话 DOM 被复用，animation 只在首次挂载跑一次
          （这个坑列表分页和设置窗 Tab 都踩过）。 */}
      <div className="ball-viewport" aria-hidden={false}>
        <div className="ball-spindle">
        {outgoing && (
          <div className="ball-face is-leaving">
            <div className="ball-name" style={{ fontSize: size.nameFont }}>
              {outgoing.name ?? outgoing.code}
            </div>
            <div
              className="ball-percent"
              style={{
                fontSize: size.percentFont,
                color: changeColor(
                  outgoing.quote?.change.percent ?? null,
                  outgoing.market,
                  theme
                ),
              }}
            >
              {formatPercent(outgoing.quote?.change.percent ?? null)}
            </div>
          </div>
        )}
        {row ? (
          /* ⚠️ `is-entering` 只在 outgoing 存在时才加 —— 用户要求"只有 5s 切换
             才滚动"。outgoing 仅在索引真的变了、且上一帧有内容时被设上，
             所以首次挂载（无上一帧）是静态出现，不滚。
             无条件加类的话每次节点重建都会滚一次，首屏也会莫名滑进来。 */
          <div
            key={safeIndex}
            className={`ball-face ${outgoing ? "is-entering" : ""}`}
          >
            <div className="ball-name" style={{ fontSize: size.nameFont }}>
              {row.name ?? row.code}
            </div>
            <div
              className="ball-percent"
              style={{ fontSize: size.percentFont, color }}
            >
              {/* formatPercent 自己对 null/NaN 返回 PLACEHOLDER，
                  这里不要再写一份兜底 —— 两处占位符早晚会分叉 */}
              {formatPercent(percent)}
            </div>
          </div>
        ) : (
          <div className="ball-face">
            <div className="ball-name" style={{ fontSize: size.nameFont }}>
              无自选股
            </div>
            <div className="ball-percent" style={{ fontSize: size.percentFont }}>
              {PLACEHOLDER}
            </div>
          </div>
        )}
        </div>
      </div>

      {/* 这里原本有一圈底部指示点（表示"还会转到别的股票"）。
          2026-09-01 用户要求去掉 —— 星空球面上多一排等距白点，
          既破坏星点的不规则感，也把"深邃"压成了一个 UI 控件。
          轮播仍在跑（见上面的 useEffect），只是不再有可视指示。 */}
    </div>
  );
}
