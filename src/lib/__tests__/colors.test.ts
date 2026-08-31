import { describe, expect, it } from "vitest";
import { changeToken } from "../colors";

// 三态 × 两套市场规则 = 6 个强制用例。
// 零态分支只能靠单测保证 —— 标的当日恰好平盘时运行时根本测不到。

describe("A 股/港股规则：红涨绿跌", () => {
  it("上涨为红", () => {
    expect(changeToken(1.45, "ashare")).toBe("red");
    // 港股与指数同规则
    expect(changeToken(1.24, "hk")).toBe("red");
    expect(changeToken(0.04, "indexcn")).toBe("red");
  });

  it("下跌为绿", () => {
    expect(changeToken(-1.45, "ashare")).toBe("green");
    expect(changeToken(-1.24, "hk")).toBe("green");
    expect(changeToken(-0.04, "indexcn")).toBe("green");
  });

  it("平盘为中性色", () => {
    expect(changeToken(0, "ashare")).toBe("neutral");
    expect(changeToken(0, "hk")).toBe("neutral");
  });
});

describe("美股规则：绿涨红跌", () => {
  it("上涨为绿", () => {
    expect(changeToken(0.36, "us")).toBe("green");
  });

  it("下跌为红", () => {
    expect(changeToken(-0.36, "us")).toBe("red");
  });

  it("平盘为中性色", () => {
    expect(changeToken(0, "us")).toBe("neutral");
  });
});

describe("涨跌不可得时不染色", () => {
  it("null / undefined / NaN 一律中性色", () => {
    // 断网或停牌时涨跌是 null，不能被染成红绿（那会暗示一个不存在的方向）
    expect(changeToken(null, "ashare")).toBe("neutral");
    expect(changeToken(undefined, "us")).toBe("neutral");
    expect(changeToken(NaN, "hk")).toBe("neutral");
  });
});
