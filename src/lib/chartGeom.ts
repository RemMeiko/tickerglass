import type { MinutePoint } from "../types";

/**
 * X 轴映射的唯一口径：只用后端给的 `grid_index` 与该市场栅格长度。
 *
 * 前端**不**自行按点序号均分 —— 那会让午休段与盘后缺点的标的横向错位；
 * 也**不**自行推算栅格长度（`grid_len` 随序列一起下发）。
 */
export function xOfGridIndex(gridIndex: number, gridLen: number, width: number): number {
  if (gridLen <= 1) return 0;
  return (gridIndex / (gridLen - 1)) * width;
}

export interface YScale {
  min: number;
  max: number;
  /** 价格 → 像素 Y（画布坐标，上小下大） */
  toY: (price: number) => number;
}

/**
 * Y 轴映射。极值只取已落栅格的点（盘后重复点已被后端丢弃）。
 * 昨收与均价也纳入范围，保证基准线/均价线始终可见 ——
 * 均价线（v0.3.0）若不进范围，价格窄幅震荡时它会画出画布外。
 * 上下留 padding，避免曲线贴边。
 */
export function makeYScale(
  points: MinutePoint[],
  prevClose: number | null,
  height: number,
  padding = 3
): YScale | null {
  if (points.length === 0) return null;

  let min = points[0].price;
  let max = points[0].price;
  const consider = (v: number | null) => {
    if (v == null || !Number.isFinite(v)) return;
    if (v < min) min = v;
    if (v > max) max = v;
  };
  for (const p of points) {
    consider(p.price);
    consider(p.avg);
  }
  if (prevClose !== null && Number.isFinite(prevClose) && prevClose > 0) {
    consider(prevClose);
  }

  // 全平（所有点同价）时给一个极小区间，避免除零把曲线画到画布外
  if (max - min < 1e-9) {
    const eps = Math.max(Math.abs(max) * 0.001, 0.01);
    min -= eps;
    max += eps;
  }

  const usable = Math.max(1, height - padding * 2);
  return {
    min,
    max,
    toY: (price: number) => padding + ((max - price) / (max - min)) * usable,
  };
}
