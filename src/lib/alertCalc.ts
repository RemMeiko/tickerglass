// 提醒规则的数值推导。**纯函数**，进 vitest。
//
// 为什么单独一个文件而不是留在 AlertPanel 里：目标价推导有一堆边界
// （缺基准价、缺涨跌幅、跌幅存正数、手填值不该被覆盖），
// 这些分支在界面上要一个个手点才能碰到，放纯函数里单测能全覆盖。

import type { AlertRule } from "../types";

/** 价格保留 2 位小数 —— 行情本身就是 2 位精度，多余位数只是浮点噪声。 */
function round2(v: number): number {
  return Number(v.toFixed(2));
}

/**
 * 基准价 + 涨跌幅 → 上下目标价。
 *
 * 语义（用户 2026-08-29 提出）：「基准价是目标价与涨跌幅的共同计算基础」。
 * 所以这里只做一件事 —— 把 `base_price` 按 `up_percent`/`down_percent`
 * 推成 `bull_price`/`bear_price`。**不改动判定逻辑**：
 * 提醒触发仍看目标价与当日涨跌幅，`base_price` 自身不参与判定。
 *
 * 三条边界，每条都有单测：
 * 1. 没填基准价 → 原样返回（不清目标价，那可能是用户手填的）
 * 2. 只填了一个方向的涨跌幅 → 只推那个方向，另一个方向不动
 * 3. 跌幅字段存的是**正数**（用户填 3.0 表示 −3.0%）→ 用减法
 */
/**
 * 「按现价生成」用的涨跌幅取值。**逐股 → 全局默认 → 3**。
 *
 * 为什么单独抽出来：这条回退链是 2026-08-30 修 bug 的核心语义，
 * 埋在组件里没法单测，将来很容易被改回硬编码 ±3%（那正是原来的 bug）。
 *
 * 原实现是 `p * 1.03` / `p * 0.97` 写死，与用户填的涨跌幅完全无关 ——
 * 逐股填 5% 再点按钮，出来仍是 ±3% 的数，表现为"点了按现价生成没变化"。
 *
 * 兜底取 3 而不是 null：null 会让 `applyBasePrice` 不推该方向的目标价，
 * 按钮就变成"只填了基准价、目标价还是空的"，等于没生效。
 * 3 与改动前的硬编码值一致，行为不回退。
 */
export function resolvePercents(
  rule: Pick<AlertRule, "up_percent" | "down_percent">,
  defaults: { up: number | null; down: number | null }
): { up: number; down: number } {
  const pick = (own: number | null | undefined, def: number | null): number => {
    if (own != null && Number.isFinite(own)) return own;
    if (def != null && Number.isFinite(def)) return def;
    return 3;
  };
  return {
    up: pick(rule.up_percent, defaults.up),
    down: pick(rule.down_percent, defaults.down),
  };
}

export function applyBasePrice(rule: AlertRule): AlertRule {
  const base = rule.base_price;
  // 非有限值/非正数一律当没填。基准价是价格，0 与负数无意义。
  if (base == null || !Number.isFinite(base) || base <= 0) {
    return rule;
  }

  const next: AlertRule = { ...rule };
  if (rule.up_percent != null && Number.isFinite(rule.up_percent)) {
    next.bull_price = round2(base * (1 + rule.up_percent / 100));
  }
  if (rule.down_percent != null && Number.isFinite(rule.down_percent)) {
    next.bear_price = round2(base * (1 - rule.down_percent / 100));
  }
  return next;
}
