import { describe, expect, it } from "vitest";
import { displayCode } from "../format";
import { windowSizeFor } from "../appearance";

describe("displayCode：内部身份键 → 显示用裸码", () => {
  it("剥掉沪市前缀", () => {
    expect(displayCode("sh600519")).toBe("600519");
  });

  it("剥掉深市前缀", () => {
    expect(displayCode("sz000001")).toBe("000001");
  });

  it("剥掉指数前缀（上证指数与平安银行裸码相同，故身份键必须保留前缀）", () => {
    expect(displayCode("sh000001")).toBe("000001");
    expect(displayCode("sz000001")).toBe("000001");
    // 两者裸码一致 —— 正是"裸码不能作身份键"的原因
    expect(displayCode("sh000001")).toBe(displayCode("sz000001"));
  });

  it("剥掉港股前缀，保留前导零", () => {
    expect(displayCode("hk00700")).toBe("00700");
  });

  it("剥掉美股前缀，保留字母大小写", () => {
    expect(displayCode("usAAPL")).toBe("AAPL");
  });

  it("未知前缀原样返回（宁可多显示前缀，不要错剥）", () => {
    expect(displayCode("600519")).toBe("600519");
    expect(displayCode("xx123456")).toBe("xx123456");
  });

  it("空串与短串不崩", () => {
    expect(displayCode("")).toBe("");
    expect(displayCode("sh")).toBe("sh");
  });
});

describe("windowSizeFor：入参是边框盒尺寸", () => {
  it("向上取整并留 1px 安全余量", () => {
    // ceil(235.4)+1 = 237，ceil(183.2)+1 = 185
    expect(windowSizeFor(235.4, 183.2)).toEqual({ width: 237, height: 185 });
  });

  it("整数输入只加安全余量", () => {
    expect(windowSizeFor(300, 208)).toEqual({ width: 301, height: 209 });
  });

  it("不再自行补 padding —— padding 已由 borderBoxSize 覆盖", () => {
    // 回归防线：曾经的 bug 是调用方传 contentRect（不含 padding）而这里只加 2px，
    // 导致窗口窄「2×padding − 2」。若有人把 padding 补偿加回这里，
    // 与 borderBoxSize 叠加会让窗口反而过宽，此断言会失败。
    const { width } = windowSizeFor(100, 100);
    expect(width).toBe(101);
  });

  it("零与负值兜底为 1（不产生非法窗口尺寸）", () => {
    expect(windowSizeFor(0, 0)).toEqual({ width: 1, height: 1 });
    expect(windowSizeFor(-50, -50)).toEqual({ width: 1, height: 1 });
  });
});
