import { describe, expect, it } from "vitest";
import { applyBasePrice, resolvePercents } from "../alertCalc";
import type { AlertRule } from "../../types";

const EMPTY: AlertRule = {
  up_percent: null,
  down_percent: null,
  bull_price: null,
  bear_price: null,
  base_price: null,
};

describe("基准价 + 涨跌幅 → 目标价", () => {
  it("两个方向都填了涨跌幅时，两个目标价都推出来", () => {
    const r = applyBasePrice({
      ...EMPTY,
      base_price: 1000,
      up_percent: 3,
      down_percent: 3,
    });
    expect(r.bull_price).toBe(1030);
    // 跌幅存正数，用减法 —— 不是 1000*(1+(-3)/100) 那种双重取负
    expect(r.bear_price).toBe(970);
  });

  it("保留 2 位小数（浮点噪声不能漏进配置）", () => {
    // 1680.5 * 1.037 = 1742.6785 —— 未取整会写成一串小数进配置文件
    const r = applyBasePrice({ ...EMPTY, base_price: 1680.5, up_percent: 3.7 });
    expect(r.bull_price).toBe(1742.68);
  });

  it("基准价本身不被改动", () => {
    const r = applyBasePrice({ ...EMPTY, base_price: 1000, up_percent: 3 });
    expect(r.base_price).toBe(1000);
  });
});

describe("边界：缺字段时不猜、不清", () => {
  it("没填基准价 → 原样返回，已有目标价不被清掉", () => {
    // 关键：目标价可能是用户手填的，没基准价时绝不能清成 null
    const r = applyBasePrice({
      ...EMPTY,
      bull_price: 1500,
      bear_price: 1400,
      up_percent: 3,
    });
    expect(r.bull_price).toBe(1500);
    expect(r.bear_price).toBe(1400);
  });

  it("只填涨幅 → 只推 🐮 目标价，🐻 原样不动", () => {
    const r = applyBasePrice({
      ...EMPTY,
      base_price: 1000,
      up_percent: 3,
      bear_price: 888,
    });
    expect(r.bull_price).toBe(1030);
    expect(r.bear_price).toBe(888);
  });

  it("只填跌幅 → 只推 🐻 目标价，🐮 原样不动", () => {
    const r = applyBasePrice({
      ...EMPTY,
      base_price: 1000,
      down_percent: 5,
      bull_price: 1234,
    });
    expect(r.bear_price).toBe(950);
    expect(r.bull_price).toBe(1234);
  });

  it("填了基准价但两个涨跌幅都空 → 目标价一个都不推", () => {
    const r = applyBasePrice({ ...EMPTY, base_price: 1000 });
    expect(r.bull_price).toBeNull();
    expect(r.bear_price).toBeNull();
  });
});

describe("边界：非法基准价当作没填", () => {
  it("基准价为 0 → 不推（价格不可能是 0）", () => {
    const r = applyBasePrice({ ...EMPTY, base_price: 0, up_percent: 3 });
    expect(r.bull_price).toBeNull();
  });

  it("基准价为负 → 不推", () => {
    const r = applyBasePrice({ ...EMPTY, base_price: -10, up_percent: 3 });
    expect(r.bull_price).toBeNull();
  });

  it("基准价为 NaN/Infinity → 不推，且不产生 NaN 目标价", () => {
    // 配置文件是用户可手改的文本，NaN 能从那里进来
    const nan = applyBasePrice({ ...EMPTY, base_price: NaN, up_percent: 3 });
    expect(nan.bull_price).toBeNull();
    const inf = applyBasePrice({
      ...EMPTY,
      base_price: Infinity,
      up_percent: 3,
    });
    expect(inf.bull_price).toBeNull();
  });

  it("涨跌幅为 NaN → 该方向不推，不写出 NaN", () => {
    const r = applyBasePrice({
      ...EMPTY,
      base_price: 1000,
      up_percent: NaN,
      down_percent: 3,
    });
    expect(r.bull_price).toBeNull();
    expect(r.bear_price).toBe(970);
  });
});

describe("涨跌幅为 0 的特例", () => {
  it("涨幅 0 → 目标价等于基准价（0 是合法阈值，不该被当没填）", () => {
    const r = applyBasePrice({ ...EMPTY, base_price: 1000, up_percent: 0 });
    expect(r.bull_price).toBe(1000);
  });
});

// 「按现价生成」的涨跌幅取值链。这一组锁的是 2026-08-30 修的那个 bug：
// 原实现硬编码 ±3%，与用户填的涨跌幅无关，表现为"点了按钮没变化"。
describe("按现价生成：涨跌幅取值 逐股 → 全局默认 → 3", () => {
  it("逐股填了就用逐股的，全局默认被忽略", () => {
    const r = resolvePercents(
      { up_percent: 5, down_percent: 8 },
      { up: 3, down: 3 }
    );
    expect(r).toEqual({ up: 5, down: 8 });
  });

  it("逐股为空 → 回退全局默认（这就是「默认」的定义）", () => {
    const r = resolvePercents(
      { up_percent: null, down_percent: null },
      { up: 4, down: 6 }
    );
    expect(r).toEqual({ up: 4, down: 6 });
  });

  it("逐股与全局都为空 → 兜底 3（与改动前的硬编码值一致，行为不回退）", () => {
    const r = resolvePercents(
      { up_percent: null, down_percent: null },
      { up: null, down: null }
    );
    expect(r).toEqual({ up: 3, down: 3 });
  });

  it("两个方向各自独立回退（一个用逐股、另一个用全局）", () => {
    // 关键：不能因为其中一个方向缺失就整条回退，否则用户只填涨幅时跌幅会错
    const r = resolvePercents(
      { up_percent: 7, down_percent: null },
      { up: 3, down: 2 }
    );
    expect(r).toEqual({ up: 7, down: 2 });
  });

  it("逐股填 0 时取 0，不被当成没填", () => {
    // 0 是合法阈值（现价即目标价）。用 ?? 而非 || 才有这个行为，
    // 写成 || 会让 0 掉进回退链变成 3 —— 这是个容易踩的坑
    const r = resolvePercents(
      { up_percent: 0, down_percent: 0 },
      { up: 3, down: 3 }
    );
    expect(r).toEqual({ up: 0, down: 0 });
  });

  it("NaN 视为没填，走回退（脏数据不该算出 NaN 目标价）", () => {
    const r = resolvePercents(
      { up_percent: NaN, down_percent: NaN },
      { up: 4, down: 4 }
    );
    expect(r).toEqual({ up: 4, down: 4 });
  });

  it("与 applyBasePrice 串起来：现价 100 + 全局默认 5% → 105 / 95", () => {
    // 端到端锁住"按现价生成"的最终结果，而不只是取值那一步
    const { up, down } = resolvePercents(
      { up_percent: null, down_percent: null },
      { up: 5, down: 5 }
    );
    const r = applyBasePrice({
      ...EMPTY,
      base_price: 100,
      up_percent: up,
      down_percent: down,
    });
    expect(r.bull_price).toBe(105);
    expect(r.bear_price).toBe(95);
  });
});
