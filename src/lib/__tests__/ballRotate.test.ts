import { describe, expect, it } from "vitest";
import {
  BALL_BASE,
  ROTATE_DURATION_MS,
  ROTATE_INTERVAL_MS,
  ballSize,
  clampIndex,
  nextIndex,
  shouldRotate,
} from "../ballRotate";

describe("clampIndex", () => {
  it("范围内原样返回", () => {
    expect(clampIndex(0, 3)).toBe(0);
    expect(clampIndex(1, 3)).toBe(1);
    expect(clampIndex(2, 3)).toBe(2);
  });

  it("超界环绕（用户删股票后当前索引越界）", () => {
    // 这是真实场景：球停在第 6 只，用户删到只剩 3 只
    expect(clampIndex(5, 3)).toBe(2);
    expect(clampIndex(3, 3)).toBe(0);
    expect(clampIndex(100, 7)).toBe(2);
  });

  it("负索引补成正的", () => {
    // JS 的 % 对负数返回负值：(-1 % 3) === -1。不补一轮会得到负索引 → 空白球
    expect(clampIndex(-1, 3)).toBe(2);
    expect(clampIndex(-3, 3)).toBe(0);
    expect(clampIndex(-4, 3)).toBe(2);
  });

  it("空列表返回 0 而不是 NaN", () => {
    expect(clampIndex(0, 0)).toBe(0);
    expect(clampIndex(5, 0)).toBe(0);
    expect(clampIndex(-2, 0)).toBe(0);
  });

  it("非数与小数不产生 NaN 或非整索引", () => {
    expect(clampIndex(NaN, 3)).toBe(0);
    expect(clampIndex(Infinity, 3)).toBe(0);
    expect(clampIndex(1.7, 3)).toBe(1);
  });
});

describe("nextIndex", () => {
  it("逐个前进", () => {
    expect(nextIndex(0, 3)).toBe(1);
    expect(nextIndex(1, 3)).toBe(2);
  });

  it("到末尾环绕回 0（与列表翻页故意不同，那边到头停住）", () => {
    expect(nextIndex(2, 3)).toBe(0);
    expect(nextIndex(0, 1)).toBe(0);
  });

  it("从越界索引前进也落在合法范围", () => {
    // 删股票后紧接着定时器触发：入参已越界，出参必须合法
    expect(nextIndex(9, 3)).toBe(1);
    expect(nextIndex(-1, 3)).toBe(0);
  });

  it("空列表返回 0", () => {
    expect(nextIndex(0, 0)).toBe(0);
    expect(nextIndex(3, 0)).toBe(0);
  });

  it("连续前进 n 次回到原点", () => {
    const len = 4;
    let i = 0;
    for (let k = 0; k < len; k++) i = nextIndex(i, len);
    expect(i).toBe(0);
  });
});

describe("shouldRotate", () => {
  it("2 只以上才轮播", () => {
    expect(shouldRotate(2)).toBe(true);
    expect(shouldRotate(50)).toBe(true);
  });

  it("0 或 1 只不启动定时器（转来转去还是同一只）", () => {
    expect(shouldRotate(0)).toBe(false);
    expect(shouldRotate(1)).toBe(false);
  });
});

describe("ballSize", () => {
  it("scale=1 时等于基准值", () => {
    const s = ballSize(1);
    expect(s.diameter).toBe(BALL_BASE.diameter);
    expect(s.nameFont).toBe(BALL_BASE.nameFont);
    expect(s.percentFont).toBe(BALL_BASE.percentFont);
  });

  it("scale > 1 时等比放大", () => {
    // 用 2 / 1.5 这类二进制可精确表示的 scale，避免断言被浮点舍入卡住
    expect(ballSize(2).diameter).toBe(120);
    expect(ballSize(1.5).diameter).toBe(90);
  });

  it("scale < 1 被钳住，直径不低于窗口可见下限 60px", () => {
    // window.rs 的 MIN_VISIBLE_W=60 / MIN_VISIBLE_H=30 是硬地板：
    // 球直径若低于 60，后端把窗口顶回 60 而 DOM 更小，
    // 表现为球周围一圈透明留白（且拖动热区比看到的球大）。
    // 基准直径已等于 60，所以生效 scale 下钳到 1 —— 球只放大不缩小。
    expect(ballSize(0.65).diameter).toBe(60);
    expect(ballSize(0.5).diameter).toBe(60);
    expect(ballSize(0.99).diameter).toBe(60);
    expect(BALL_BASE.diameter).toBeGreaterThanOrEqual(60);
  });

  it("字号跟着一起被钳住（球不缩了字也不能缩，否则比例崩）", () => {
    // 钳的是 scale 而非单独钳 diameter，所以低 scale 下字号也停在基准
    const s = ballSize(0.65);
    expect(s.nameFont).toBe(BALL_BASE.nameFont);
    expect(s.percentFont).toBe(BALL_BASE.percentFont);
  });

  it("字号保留 1 位小数，不整数取整", () => {
    // 整数取整会把两个字号的相对关系拉歪：真实比值 11/9 = 1.222，
    // 取整后名称与涨跌幅的视觉层级跟着变。CSS 接受小数 px，没理由取整。
    const s = ballSize(1.5);
    expect(s.nameFont).toBeCloseTo(13.5, 5);
    expect(s.percentFont).toBeCloseTo(16.5, 5);
    // 关键是小数没被丢掉
    expect(Number.isInteger(s.nameFont)).toBe(false);
  });

  it("非数回落到 scale=1", () => {
    expect(ballSize(NaN).diameter).toBe(BALL_BASE.diameter);
    expect(ballSize(Infinity).diameter).toBe(BALL_BASE.diameter);
  });
});

describe("时间常量", () => {
  it("轮播间隔是用户指定的 5 秒", () => {
    expect(ROTATE_INTERVAL_MS).toBe(5000);
  });

  it("旋转时长明显短于轮播间隔（否则一直在动）", () => {
    expect(ROTATE_DURATION_MS).toBeLessThan(ROTATE_INTERVAL_MS / 5);
  });
});
