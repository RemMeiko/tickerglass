import { describe, expect, it } from "vitest";
import { moveItem } from "../dragReorder";

const L = ["a", "b", "c", "d"];

describe("moveItem", () => {
  it("把最后一项拖到第一位", () => {
    // 标准 5 的主场景
    expect(moveItem(L, 3, 0)).toEqual(["d", "a", "b", "c"]);
  });

  it("把第一项拖到最后", () => {
    expect(moveItem(L, 0, 3)).toEqual(["b", "c", "d", "a"]);
  });

  it("相邻两项互换", () => {
    expect(moveItem(L, 1, 2)).toEqual(["a", "c", "b", "d"]);
  });

  it("原地放下时顺序不变且不报错", () => {
    // 标准 5 边界：把第一项拖到第一项
    expect(moveItem(L, 0, 0)).toEqual(L);
    expect(moveItem(L, 2, 2)).toEqual(L);
  });

  it("不修改入参（纯函数）", () => {
    const src = ["a", "b", "c"];
    moveItem(src, 0, 2);
    expect(src).toEqual(["a", "b", "c"]);
  });

  it("from 越界时原样返回，不丢项", () => {
    // 拖到列表外空白区域会给出非法索引
    expect(moveItem(L, 9, 0)).toEqual(L);
    expect(moveItem(L, -1, 0)).toEqual(L);
  });

  it("to 越界时原样返回，不丢项", () => {
    expect(moveItem(L, 0, 9)).toEqual(L);
    expect(moveItem(L, 0, -1)).toEqual(L);
  });

  it("非整数索引原样返回", () => {
    expect(moveItem(L, NaN, 0)).toEqual(L);
    expect(moveItem(L, 0, 1.5)).toEqual(L);
  });

  it("空列表与单元素列表不崩", () => {
    expect(moveItem([], 0, 0)).toEqual([]);
    expect(moveItem(["only"], 0, 0)).toEqual(["only"]);
  });

  it("移动后长度始终不变", () => {
    expect(moveItem(L, 3, 1)).toHaveLength(4);
    expect(moveItem(L, 9, 1)).toHaveLength(4);
  });

  it("50 只列表的中间移动保持全部元素", () => {
    const big = Array.from({ length: 50 }, (_, i) => `s${i}`);
    const out = moveItem(big, 49, 0);
    expect(out).toHaveLength(50);
    expect(out[0]).toBe("s49");
    expect(new Set(out).size).toBe(50);
  });
});
