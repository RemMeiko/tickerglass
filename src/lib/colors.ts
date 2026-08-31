import type { Market } from "../types";

/**
 * 涨跌方向（**语义**，与市场无关）。颜色由 `changeColor` 按市场决定。
 * 方向与配色分成两步，避免"翻转 token"和"按市场换调色板"两处都翻一次而互相抵消。
 */
export type Direction = "up" | "down" | "flat";

/** 配色 token（**视觉**）。red/green 是最终落到界面上的颜色语义 */
export type ColorToken = "red" | "green" | "neutral";

/** 界面主题。与 `AppearanceConfig.theme` 同值域 */
export type Theme = "dark" | "light";

/**
 * 两套调色板。
 *
 * **为什么行价格的颜色不走 CSS token**：红涨绿跌要按市场翻转
 * （A股/港股红涨绿跌，美股绿涨红跌），这个判断在 JS 里做，
 * 结果用内联 style 打到 DOM 上，所以 `[data-theme="light"]` 覆盖不到它。
 * 加浅色主题时这里**必须单独处理**，否则深色的红绿会直接留在白底上。
 *
 * 浅色值不是深色值的简单变换，是按白底对比度重取的：
 * `#f5484a` / `#22b573` 在白底上都偏亮、饱和度不够压不住，各压深一档。
 * neutral（平盘）反过来要**变深**：深色下它是浅灰 #c9cdd4，
 * 浅色下得用中灰才看得见。
 */
export const PALETTES: Record<Theme, Record<ColorToken, string>> = {
  dark: {
    red: "#f5484a",
    green: "#22b573",
    neutral: "#c9cdd4",
  },
  light: {
    red: "#d92b2e",
    green: "#0f8f56",
    neutral: "#6e757c",
  },
};

/**
 * 深色调色板。**保留这个导出是为了不破坏既有引用**
 * （Canvas 绘图等处直接取过色值），新代码请用 `PALETTES`。
 */
export const TOKEN_HEX: Record<ColorToken, string> = PALETTES.dark;

/**
 * 涨跌方向。纯函数，无 DOM、无副作用。
 * `percent` 为 null / NaN（涨跌不可得）或 0（平盘）都返回 flat ——
 * 这两种情况在视觉上都不该被染成红或绿。
 */
export function direction(percent: number | null | undefined): Direction {
  if (percent === null || percent === undefined || !Number.isFinite(percent) || percent === 0) {
    return "flat";
  }
  return percent > 0 ? "up" : "down";
}

/**
 * 方向 + 市场 → 配色 token。两套市场规则：
 * - A 股 / 港股 / 指数：**红涨绿跌**（中国大陆与香港习惯）
 * - 美股：**绿涨红跌**（欧美习惯）
 */
export function changeToken(percent: number | null | undefined, market: Market): ColorToken {
  const dir = direction(percent);
  if (dir === "flat") return "neutral";
  const westernPalette = market === "us";
  if (westernPalette) {
    return dir === "up" ? "green" : "red";
  }
  return dir === "up" ? "red" : "green";
}

/**
 * 涨跌 → 实际颜色值。
 *
 * `theme` 默认 `"dark"`：既有调用点（十余处）不必逐个改就保持原行为，
 * 只有需要跟随主题的地方显式传值。
 */
export function changeColor(
  percent: number | null | undefined,
  market: Market,
  theme: Theme = "dark"
): string {
  return PALETTES[theme][changeToken(percent, market)];
}
