import { describe, expect, it } from "vitest";
import {
  formatMoney,
  formatPercent,
  formatSignedMoney,
  trendClass,
} from "../formatMoney";

describe("formatMoney", () => {
  it("亿级用亿", () => {
    // 大盘资金流向的典型量级：实测沪市主力净流入 74.2 亿
    expect(formatMoney(7_419_838_464)).toBe("74.20 亿");
    expect(formatMoney(1e8)).toBe("1.00 亿");
  });

  it("万级用万", () => {
    expect(formatMoney(12_345)).toBe("1.23 万");
    expect(formatMoney(1e4)).toBe("1.00 万");
  });

  it("小额用元且不带小数", () => {
    // 元级别再显示两位小数没有意义
    expect(formatMoney(999)).toBe("999 元");
    expect(formatMoney(0)).toBe("0 元");
  });

  it("负数按绝对值选单位", () => {
    // 单位必须看绝对值，否则 -74 亿会因为 -7.4e9 < 1e8 而被显示成「元」
    expect(formatMoney(-7_419_838_464)).toBe("-74.20 亿");
    expect(formatMoney(-12_345)).toBe("-1.23 万");
  });

  it("非有限值给占位符而不是 NaN", () => {
    // 界面上出现 "NaN 亿" 比 "—" 糟得多
    expect(formatMoney(Number.NaN)).toBe("—");
    expect(formatMoney(Number.POSITIVE_INFINITY)).toBe("—");
  });

  it("小数位可调", () => {
    expect(formatMoney(7_419_838_464, 0)).toBe("74 亿");
    expect(formatMoney(7_419_838_464, 1)).toBe("74.2 亿");
  });

  it("恰好在边界上时进大单位", () => {
    // 99_999_999 差 1 元到亿 → 仍是万；100_000_000 → 亿
    expect(formatMoney(99_999_999)).toBe("10000.00 万");
    expect(formatMoney(100_000_000)).toBe("1.00 亿");
  });
});

describe("formatSignedMoney", () => {
  it("正数显式带加号", () => {
    // 资金净流入的正负是面板最核心的信息，不能只靠颜色区分（色弱不可辨）
    expect(formatSignedMoney(7_419_838_464)).toBe("+74.20 亿");
  });

  it("负数自带减号不重复加符号", () => {
    expect(formatSignedMoney(-7_419_838_464)).toBe("-74.20 亿");
  });

  it("零不带符号", () => {
    expect(formatSignedMoney(0)).toBe("0 元");
  });

  it("非有限值给占位符", () => {
    expect(formatSignedMoney(Number.NaN)).toBe("—");
  });
});

describe("formatPercent", () => {
  it("正数带加号", () => {
    expect(formatPercent(0.17)).toBe("+0.17%");
  });

  it("负数带减号", () => {
    expect(formatPercent(-1.23)).toBe("-1.23%");
  });

  it("零不带符号", () => {
    expect(formatPercent(0)).toBe("0.00%");
  });

  it("非有限值给占位符", () => {
    expect(formatPercent(Number.NaN)).toBe("—");
  });
});

describe("trendClass", () => {
  it("正负零分别映射", () => {
    expect(trendClass(1)).toBe("up");
    expect(trendClass(-1)).toBe("down");
    expect(trendClass(0)).toBe("flat");
  });

  it("零归 flat 而不是 down", () => {
    // 平盘染成绿色会让人以为在跌
    expect(trendClass(0)).toBe("flat");
    expect(trendClass(-0)).toBe("flat");
  });

  it("非有限值归 flat", () => {
    expect(trendClass(Number.NaN)).toBe("flat");
  });

  it("与 colors.direction 保持一致（同一真值源）", async () => {
    // trendClass 委托给 colors.direction，这条锁住"不要又写一份判定"
    const { direction } = await import("../colors");
    for (const v of [3.5, -2, 0, Number.NaN, 1e-9, -1e-9]) {
      expect(trendClass(v)).toBe(direction(v));
    }
  });
});
