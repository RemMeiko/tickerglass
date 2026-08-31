import { describe, expect, it } from "vitest";
import type { AppearanceConfig } from "../../types";
import {
  BASE,
  SCALE_MAX,
  SCALE_MIN,
  chartSize,
  clampScale,
  cssVars,
  windowSizeFor,
} from "../appearance";

function cfg(over: Partial<AppearanceConfig> = {}): AppearanceConfig {
  return {
    scale: 1,
    curve_width: 1,
    font_size: 12,
    show_code: false,
    show_market: false,
    opacity_background: 0.85,
    opacity_text: 1,
    opacity_curve: 1,
    theme: null,
    ...over,
  };
}

describe("clampScale", () => {
  it("区间内的值原样返回", () => {
    expect(clampScale(1)).toBe(1);
    expect(clampScale(1.3)).toBe(1.3);
  });

  it("超上限钳到 160%", () => {
    expect(clampScale(9)).toBe(SCALE_MAX);
  });

  it("低于下限钳到 65%", () => {
    expect(clampScale(0.1)).toBe(SCALE_MIN);
  });

  it("非数回落 1", () => {
    expect(clampScale(NaN)).toBe(1);
    expect(clampScale(Infinity)).toBe(1);
  });
});

describe("cssVars 缩放尺寸计算", () => {
  it("100% 时字号等于基准字号", () => {
    expect(cssVars(cfg()) ["--font-size"]).toBe("12px");
  });

  it("130% 时字号同比放大", () => {
    expect(cssVars(cfg({ scale: 1.3 }))["--font-size"]).toBe("15.6px");
  });

  it("65% 时行高同比缩小", () => {
    // 44 * 0.65 = 28.6，字面量写死以免表达式自身带入浮点尾差
    const v = cssVars(cfg({ scale: 0.65 }));
    expect(v["--row-height"]).toBe("28.6px");
  });

  it("160% 时名称列宽同比放大", () => {
    // 66 * 1.6 在 IEEE 754 下是 105.60000000000001，CSS 输出须是整洁的 105.6px
    const v = cssVars(cfg({ scale: 1.6 }));
    expect(v["--name-width"]).toBe("105.6px");
  });

  it("像素值不出现浮点尾差", () => {
    // 防回归：任何 --* 像素变量都不该出现 15.600000000000001px 这种输出
    for (const scale of [0.65, 0.8, 1, 1.3, 1.6]) {
      const v = cssVars(cfg({ scale }));
      for (const [k, val] of Object.entries(v)) {
        if (val.endsWith("px")) {
          expect(val, `${k} @ scale=${scale}`).not.toMatch(/\d{6,}px$/);
        }
      }
    }
  });

  it("越界 scale 在 CSS 变量里已被钳制", () => {
    expect(cssVars(cfg({ scale: 99 }))["--scale"]).toBe(String(SCALE_MAX));
  });

  it("自定义字号参与缩放", () => {
    expect(cssVars(cfg({ font_size: 16, scale: 1.5 }))["--font-size"]).toBe("24px");
  });

  it("副字号按比例小于主字号", () => {
    const v = cssVars(cfg({ font_size: 12, scale: 1 }));
    expect(v["--font-size-sub"]).toBe("10px");
  });
});

describe("cssVars 三组不透明度互相独立（标准 11）", () => {
  it("背景为 0 时文字与曲线仍是原值（不相乘）", () => {
    const v = cssVars(cfg({ opacity_background: 0, opacity_text: 1, opacity_curve: 1 }));
    expect(v["--opacity-bg"]).toBe("0");
    expect(v["--opacity-text"]).toBe("1");
    expect(v["--opacity-curve"]).toBe("1");
  });

  it("曲线为 0 时文字不受影响", () => {
    const v = cssVars(cfg({ opacity_curve: 0 }));
    expect(v["--opacity-curve"]).toBe("0");
    expect(v["--opacity-text"]).toBe("1");
  });

  it("文字为 0 时曲线不受影响", () => {
    const v = cssVars(cfg({ opacity_text: 0 }));
    expect(v["--opacity-text"]).toBe("0");
    expect(v["--opacity-curve"]).toBe("1");
  });

  it("三组各自独立取值互不干扰", () => {
    const v = cssVars(
      cfg({ opacity_background: 0.2, opacity_text: 0.5, opacity_curve: 0.9 })
    );
    expect(v["--opacity-bg"]).toBe("0.2");
    expect(v["--opacity-text"]).toBe("0.5");
    expect(v["--opacity-curve"]).toBe("0.9");
  });

  it("越界不透明度被钳到 0..1", () => {
    const v = cssVars(cfg({ opacity_background: -3, opacity_text: 7 }));
    expect(v["--opacity-bg"]).toBe("0");
    expect(v["--opacity-text"]).toBe("1");
  });

  it("曲线宽度原样传递", () => {
    expect(cssVars(cfg({ curve_width: 2.5 }))["--curve-width"]).toBe("2.5");
  });
});

describe("chartSize", () => {
  it("100% 时等于基准尺寸", () => {
    expect(chartSize(cfg())).toEqual({
      width: BASE.chartWidth,
      height: BASE.chartHeight,
    });
  });

  it("160% 时同比放大且取整", () => {
    expect(chartSize(cfg({ scale: 1.6 }))).toEqual({
      width: Math.round(BASE.chartWidth * 1.6),
      height: Math.round(BASE.chartHeight * 1.6),
    });
  });

  it("65% 时同比缩小", () => {
    const s = chartSize(cfg({ scale: 0.65 }));
    expect(s.width).toBe(Math.round(BASE.chartWidth * 0.65));
    expect(s.height).toBeGreaterThan(0);
  });

  it("越界 scale 也产出正整数尺寸", () => {
    const s = chartSize(cfg({ scale: NaN }));
    expect(Number.isInteger(s.width)).toBe(true);
    expect(s.width).toBeGreaterThan(0);
  });
});

// 2026-08-24 语义变更：入参从「内容盒尺寸」改为「边框盒尺寸」。
// 原因：调用方原先传 ResizeObserver 的 contentRect（不含 padding），
// 而本函数只加 2px 边框，导致窗口比内容窄/短「2×padding − 2」，
// 表现为右边与下边被裁（scale=0.95 时约 9px，实测窗口 226x175 而内容需 ~236x184）。
// 现改为传 borderBoxSize（已含 padding 与 border），本函数只补 1px 防亚像素下溢。
describe("windowSizeFor", () => {
  it("边框盒尺寸只补 1px 安全余量（不再自行加边框）", () => {
    expect(windowSizeFor(300, 208)).toEqual({ width: 301, height: 209 });
  });

  it("小数尺寸向上取整后再补 1px（避免裁切最后一像素）", () => {
    expect(windowSizeFor(299.2, 207.4)).toEqual({ width: 301, height: 209 });
  });

  it("0 尺寸不产出 0 或负数窗口", () => {
    const s = windowSizeFor(0, 0);
    expect(s.width).toBeGreaterThan(0);
    expect(s.height).toBeGreaterThan(0);
  });
});
