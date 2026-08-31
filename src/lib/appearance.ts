// 外观配置 → CSS 变量 / 尺寸计算。**纯函数**，进 vitest。
//
// 设计要点（D3）：缩放走 CSS 变量而不是 `transform: scale()` ——
// 后者会让文字渲染模糊，且不改变布局盒尺寸（窗口无法随内容自适应）。

import type { AppearanceConfig } from "../types";

/** 缩放前的基准尺寸（像素）。所有尺寸都由这些乘 scale 得到。 */
export const BASE = {
  /** 单行高度 */
  rowHeight: 44,
  /** 名称列宽 */
  nameWidth: 66,
  /** 分时图宽 */
  chartWidth: 104,
  /** 分时图高 */
  chartHeight: 30,
  /** 容器内边距 */
  padding: 6,
  /** 行间距 */
  rowGap: 2,
  /** 涨跌幅字号相对基准字号的比例 */
  percentRatio: 10 / 12,
} as const;

export const SCALE_MIN = 0.65;
export const SCALE_MAX = 1.6;

/** 把可能越界/非数的值钳到合法区间。配置文件是用户可手改的文本。 */
export function clampScale(v: number): number {
  if (!Number.isFinite(v)) return 1;
  return Math.min(SCALE_MAX, Math.max(SCALE_MIN, v));
}

function clamp01(v: number, fallback = 1): number {
  if (!Number.isFinite(v)) return fallback;
  return Math.min(1, Math.max(0, v));
}

/**
 * 像素值取整到 2 位小数。
 *
 * 必要性：`12 * 1.3` 在 IEEE 754 下是 `15.600000000000001`，
 * 直接拼进 CSS 会得到 `15.600000000000001px` —— 既难读，
 * 又可能让浏览器在相邻档位间产生无意义的亚像素抖动。
 */
function px(v: number): string {
  return `${Math.round(v * 100) / 100}px`;
}

/**
 * 配置 → CSS 自定义属性表。
 *
 * 三组不透明度**互不相乘**：它们分别作用在互不嵌套的 DOM 层上（D2），
 * 所以这里只是原样传递，不做任何乘算。背景板调到 0 时文字/曲线必须仍是原值。
 */
export function cssVars(a: AppearanceConfig): Record<string, string> {
  const scale = clampScale(a.scale);
  const font = Number.isFinite(a.font_size) ? a.font_size : 12;
  return {
    "--scale": String(scale),
    "--font-size": px(font * scale),
    "--font-size-sub": px(font * BASE.percentRatio * scale),
    "--row-height": px(BASE.rowHeight * scale),
    "--name-width": px(BASE.nameWidth * scale),
    "--padding": px(BASE.padding * scale),
    "--row-gap": px(BASE.rowGap * scale),
    "--curve-width": String(
      Number.isFinite(a.curve_width) ? a.curve_width : 1
    ),
    "--opacity-bg": String(clamp01(a.opacity_background, 0.85)),
    "--opacity-text": String(clamp01(a.opacity_text)),
    "--opacity-curve": String(clamp01(a.opacity_curve)),
  };
}

/** 分时图的缩放后尺寸。canvas 内部已有 devicePixelRatio 适配，不会模糊。 */
export function chartSize(a: AppearanceConfig): { width: number; height: number } {
  const scale = clampScale(a.scale);
  return {
    width: Math.round(BASE.chartWidth * scale),
    height: Math.round(BASE.chartHeight * scale),
  };
}

/**
 * 内容尺寸 → 上报给后端 `set_size` 的窗口尺寸。
 *
 * 入参必须是**边框盒**尺寸（`ResizeObserver` 的 `borderBoxSize`），它已含 padding 与 border。
 *
 * ⚠️ 曾经的 bug：这里的注释声称"加上容器边框与内边距"，但代码只加了 2px 边框、
 * 而调用方传的是 `contentRect`（内容盒，不含 padding）。结果窗口比内容窄/短
 * 「2×padding − 2」，表现为右边与下边被裁（scale=0.95 时约 9px）。
 * 现在 padding 已由 borderBoxSize 覆盖，这里只补 1px 的安全余量防亚像素取整下溢。
 */
export function windowSizeFor(
  borderBoxWidth: number,
  borderBoxHeight: number
): { width: number; height: number } {
  const safety = 1;
  return {
    width: Math.max(1, Math.ceil(borderBoxWidth) + safety),
    height: Math.max(1, Math.ceil(borderBoxHeight) + safety),
  };
}
