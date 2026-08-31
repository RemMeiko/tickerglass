import { useState } from "react";

export interface SliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  /** 右侧数值显示。返回带单位的字符串，如 `95%` / `1.0 px` / `300s` */
  format: (v: number) => string;
  onInput: (v: number) => void;
  /** 行样式：外观页用 slider-row，提醒页用 row-slider（两页历史样式不同） */
  rowClass?: string;
}

/**
 * 滑块。**拖动期间走本地态，不等 IPC 往返。**
 *
 * 抽成共用组件的理由：2026-08-30 第一次修"滑块迟钝"时只改了外观页，
 * 提醒页那三个（提示音音量 / 卡片不透明度 / 冷却期）是各自写的裸
 * `<input type="range">`，于是用户报"背景板滑动条很丝滑，但是提示音音量
 * 的这还是有卡顿感"。同一个交互缺陷分散在两处实现里，改一处必然漏另一处，
 * 所以这里收敛成一个组件，两个面板都用它。
 *
 * # 为什么纯受控会卡
 *
 * 原写法 `value={config.xxx}` + 每次 input 发 IPC：滑块要等
 * `commit → emit("config") → useConfig 收到 → 重渲染` 一整圈**跨进程异步**
 * 才移动一格。拖快了就出现"手指已经到了、滑块在后面追"的粘滞感。
 *
 * # 现在的做法
 *
 * 拖动时先更新 `local` 立即重绘（同步、零延迟），同时照常发 IPC
 * 让悬浮窗实时预览。松手/失焦后清掉 `local`，重新以 config 为准 ——
 * 这一步是必要的：后端 `sanitize()` 会把超界值钳制回合法区间，
 * 交回受控才能让钳制结果正确回显。
 *
 * **没用防抖**：预览必须实时（标准 12 要求 ≤200ms 生效），
 * 防抖会让悬浮窗在拖动过程中不动，那只是把卡顿换了个位置。
 */
export default function Slider({
  label,
  value,
  min,
  max,
  step,
  format,
  onInput,
  rowClass = "slider-row",
}: SliderProps) {
  const id = `slider-${label}`;
  const [local, setLocal] = useState<number | null>(null);
  // 拖动中以本地值为准；松手后回到受控值
  const shown = local ?? value;

  const emit = (v: number) => {
    setLocal(v);
    onInput(v);
  };

  return (
    <label className={rowClass} htmlFor={id}>
      <span>{label}</span>
      <input
        id={id}
        className="slider"
        type="range"
        min={min}
        max={max}
        step={step}
        value={shown}
        aria-valuetext={format(shown)}
        onInput={(e) => emit(Number((e.target as HTMLInputElement).value))}
        onChange={(e) => emit(Number(e.target.value))}
        // 松手/失焦/松键即交回受控：后端 sanitize 钳制过的值要能正确回显
        onPointerUp={() => setLocal(null)}
        onBlur={() => setLocal(null)}
        onKeyUp={() => setLocal(null)}
      />
      <em>{format(shown)}</em>
    </label>
  );
}
