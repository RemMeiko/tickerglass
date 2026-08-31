import { direction, type Direction } from "./colors";

/**
 * 金额格式化。后端给的资金流向与成交额单位都是**元**（不缩放）。
 *
 * 为什么要有这个模块：`208907272192` 这种数字直接显示没人读得懂，
 * 而三个大盘子组件都要做同样的换算 —— 各写一份必然出现"这里显示亿、那里显示万"。
 */

const YI = 1e8;
const WAN = 1e4;

/**
 * 按中文习惯选单位（亿 / 万 / 元）。
 *
 * @param yuan 金额（元）
 * @param digits 小数位，默认 2
 */
export function formatMoney(yuan: number, digits = 2): string {
  if (!Number.isFinite(yuan)) return "—";
  const abs = Math.abs(yuan);
  if (abs >= YI) return `${(yuan / YI).toFixed(digits)} 亿`;
  if (abs >= WAN) return `${(yuan / WAN).toFixed(digits)} 万`;
  return `${yuan.toFixed(0)} 元`;
}

/**
 * 带符号的金额（资金净流入用）。
 *
 * 净流入必须显式带 `+` —— 资金流向的正负是这个面板最核心的信息，
 * 只靠颜色区分对色弱用户不友好。
 */
export function formatSignedMoney(yuan: number, digits = 2): string {
  if (!Number.isFinite(yuan)) return "—";
  const s = formatMoney(yuan, digits);
  return yuan > 0 ? `+${s}` : s;
}

/** 涨跌幅，显式带符号。 */
export function formatPercent(p: number, digits = 2): string {
  if (!Number.isFinite(p)) return "—";
  return `${p > 0 ? "+" : ""}${p.toFixed(digits)}%`;
}

/**
 * 涨跌方向 → CSS 类名后缀。
 *
 * **直接委托给 `colors.direction()`**，不自己判 —— 项目里已有一份方向判定，
 * 再写一份就等于两个真值源：将来若"平盘算涨还是算跌"的口径变了，
 * 两处必然分叉（这类分叉在本项目的 market/classify 上已经出过一次）。
 *
 * 注意方向 ≠ 颜色：A股红涨绿跌、美股绿涨红跌，映射在 `colors.changeToken`。
 * 大盘面板只做 A 股，故 `.mk-up` 直接上红色；见 market.css 的说明。
 */
export function trendClass(v: number): Direction {
  return direction(v);
}
