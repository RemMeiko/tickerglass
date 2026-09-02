// 球态悬浮窗的轮播索引与尺寸计算。**纯函数**，进 vitest。
//
// 为什么单独一个模块：索引环绕与"列表变短后钳制"这两件事在界面上很难手点到
// （要删到刚好比当前索引短），但错了就是空白球。抽成纯函数后用单测覆盖边界。

/** 轮播间隔（毫秒）。用户 2026-09-01 指定 5 秒。 */
export const ROTATE_INTERVAL_MS = 5000;

/**
 * 旋转过渡时长（毫秒）。
 *
 * 取 620ms 的理由：2026-09-01 改成"水晶球内旋转"后，一次切换是**两面各转 90°**
 * （旧面转出 + 新面转进），而不是原来单面淡入。460ms 分给两段各 230ms
 * 太仓促，读不出"牌子在球里转过去"，只剩一次翻动。
 *
 * 上限受测试约束：必须 < ROTATE_INTERVAL_MS / 5 = 1000ms，
 * 否则 5s 周期里动的时间占比过高，常驻桌面会显得躁。
 */
export const ROTATE_DURATION_MS = 620;

/**
 * 缩放前的球体基准尺寸（像素）。与 appearance.ts 的 BASE 同一套路。
 *
 * 2026-09-01 用户要求「缩小四分之一」→ 按**面积** 1/4 理解，即直径减半
 * （112 → 56）。首版 112px 实测在桌面上偏大。
 *
 * ⚠️ 字号**不随直径等比缩** —— 直径减半时 13/15px 等比会变成 6.5/7.5px，
 * 小到读不出来。文字有独立的可读性下限，所以只降到 9/11px
 * （约 0.7 倍而非 0.5 倍）。
 *
 * ⚠️ 直径取 60 而不是算出来的 56：`window.rs` 的 `MIN_VISIBLE_W = 60` 是硬地板，
 * 后端会把窗口宽度顶到 60。若 DOM 只有 56，球周围就多出一圈透明留白，
 * 且拖动热区比看到的球大（右键不受影响 —— 菜单挂在最外层 `.app` 上；
 * 只是左键点那圈留白不展开）。56 → 60 视觉上差 7%，不影响「缩小四分之一」的诉求，
 * 但一次消掉整类留白 Bug —— 包括默认 scale=1.0 的情况。
 * 见 `ballSize` 对 scale 的下钳。
 */
export const BALL_BASE = {
  /** 直径。与 window.rs 的 MIN_VISIBLE_W 对齐，勿低于 60。 */
  diameter: 60,
  /** 股票名称字号 */
  nameFont: 9,
  /** 涨跌幅字号 */
  percentFont: 11,
} as const;

/**
 * 把索引钳到 `[0, len)`，支持负数与超界。
 *
 * 必要性：用户在设置页删股票后，球正显示的索引可能已越界
 * （比如停在第 6 只却只剩 3 只）。不钳制会渲染出空白球，
 * 且要等 5s 后自动前进才可能回到有效范围 —— 与列表模式的 `safePage` 同一个道理。
 *
 * `len <= 0` 返回 0：调用方自己判空（没有可显示的行时不该渲染球体内容），
 * 这里返回 0 只是保证不产生 NaN 或负索引。
 */
export function clampIndex(index: number, len: number): number {
  if (!Number.isFinite(index) || len <= 0) return 0;
  const i = Math.trunc(index);
  // 先取模再补一轮，处理负数：(-1 % 3) 在 JS 里是 -1，不是 2
  return ((i % len) + len) % len;
}

/**
 * 前进到下一只，到末尾环绕回 0。
 *
 * 与列表模式的翻页**故意不同**：那边到头就停住（循环会让用户分不清第几页），
 * 而球态是无人值守的自动轮播，停在最后一只就等于失效了。
 */
export function nextIndex(index: number, len: number): number {
  if (len <= 0) return 0;
  return clampIndex(clampIndex(index, len) + 1, len);
}

/**
 * 缩放后的球体尺寸。
 *
 * ⚠️ 生效 scale 下钳到 1 —— 球只放大不缩小。
 *
 * 这是被 `window.rs` 的 `MIN_VISIBLE_W = 60` 逼出来的：基准直径已等于该下限，
 * 再乘任何 < 1 的 scale 都会让 DOM 小于后端顶出的窗口宽度，
 * 于是球周围出现一圈透明留白（见 `BALL_BASE` 注释）。
 *
 * 钳 scale 而不是单独钳 diameter：否则球停止缩小而字号继续缩，
 * 文字与球的比例会崩。代价是 scale < 1 的用户在球态下调不小球体
 * —— 列表态不受影响，那边没有这个地板。
 */
export function ballSize(scale: number): {
  diameter: number;
  nameFont: number;
  percentFont: number;
} {
  const raw = Number.isFinite(scale) ? scale : 1;
  const s = Math.max(raw, 1);
  return {
    diameter: Math.round(BALL_BASE.diameter * s),
    // 字号保留 1 位小数而不整数取整：以用户当前 scale=1.15 为例，整数取整给出
    // 10 / 13（比值 1.30），而真实比值是 11/9 = 1.222；1 位小数给出 10.4 / 12.7
    // （比值 1.221）。整数取整会把两个字号的相对关系拉歪，
    // 名称与涨跌幅的视觉层级跟着变。CSS 接受小数 px，没有理由取整。
    nameFont: Math.round(BALL_BASE.nameFont * s * 10) / 10,
    percentFont: Math.round(BALL_BASE.percentFont * s * 10) / 10,
  };
}

/**
 * 轮播是否该启动。
 *
 * 只有 1 只时不启动定时器 —— 转来转去还是同一只，纯粹浪费一次重渲染，
 * 而且会让"旋转"这个动作失去含义（用户会以为卡了）。
 */
export function shouldRotate(len: number): boolean {
  return len >= 2;
}
