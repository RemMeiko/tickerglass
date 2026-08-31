import { useRef } from "react";

export interface NumberInputProps {
  /** 当前值。null = 未填（显示 placeholder） */
  value: number | null;
  /** 每次滚轮/方向键的步长 */
  step: number;
  min?: number;
  max?: number;
  placeholder?: string;
  title?: string;
  /** 编辑中回调（每次输入都触发，父组件通常只更新 draft 不落盘） */
  onChange: (v: number | null) => void;
  /** 失焦回调（父组件在这里落盘） */
  onCommit?: () => void;
  /** 小数位。滚轮加减后按这个位数取整，避免浮点噪声 */
  decimals?: number;
}

/**
 * 数字输入框。**没有上下箭头，改用鼠标滚轮增减。**
 *
 * 为什么这么设计：原生 spinner 箭头在深色主题下是亮白的、很抢眼，
 * 而它是原生控件伪元素（Chromium 内部绘制），`color`/`fill`/`background`
 * 都改不动它 —— 前后试过压暗、invert、缩放四种办法都没能既保留箭头
 * 又让它不刺眼（详见 macos.css 里那段改动史）。
 * 最后的结论是：不要图形按钮，把"能微调"这个能力交给滚轮。
 *
 * # 滚轮实现的三个要点
 *
 * ① **只在获得焦点时响应滚轮。** 否则用户滚动整个设置页时，
 *    鼠标恰好掠过输入框就会悄悄改掉数值 —— 这种误改很难被发现。
 *    要求先点进去（focus）再滚，意图明确。
 *
 * ② **必须 `preventDefault`**，否则页面会跟着一起滚。
 *    但 React 的 onWheel 是 passive 监听，在里面调 preventDefault 无效
 *    （控制台会警告 "Unable to preventDefault inside passive event listener"），
 *    所以这里用 ref + `addEventListener(..., { passive: false })` 手动挂。
 *
 * ③ **取整到 decimals 位。** 0.01 步长连加几次会得到
 *    15.760000000000002 这种值，直接写进配置会很难看。
 */
export default function NumberInput({
  value,
  step,
  min,
  max,
  placeholder = "不启用",
  title,
  onChange,
  onCommit,
  decimals = 2,
}: NumberInputProps) {
  const ref = useRef<HTMLInputElement | null>(null);
  const focused = useRef(false);

  /** 挂 wheel 监听。用回调 ref 而不是 useEffect：节点换了也能正确重挂 */
  const attach = (el: HTMLInputElement | null) => {
    if (ref.current === el) return;
    ref.current = el;
    if (!el) return;

    el.addEventListener(
      "wheel",
      (e: WheelEvent) => {
        // 要点 ①：没聚焦不响应，避免滚页面时误改
        if (!focused.current) return;
        // 要点 ②：passive:false 才能真的拦住页面滚动
        e.preventDefault();

        const base = value ?? 0;
        // deltaY < 0 是向上滚 → 增大
        const dir = e.deltaY < 0 ? 1 : -1;
        let next = base + dir * step;
        if (min != null && next < min) next = min;
        if (max != null && next > max) next = max;
        // 要点 ③：去掉浮点噪声
        onChange(Number(next.toFixed(decimals)));
      },
      { passive: false }
    );
  };

  return (
    <input
      ref={attach}
      type="number"
      step={step}
      min={min}
      max={max}
      value={value == null ? "" : String(value)}
      placeholder={placeholder}
      title={title}
      onFocus={() => {
        focused.current = true;
      }}
      onBlur={() => {
        focused.current = false;
        onCommit?.();
      }}
      onChange={(e) => {
        const t = e.target.value.trim();
        if (t === "") {
          onChange(null);
          return;
        }
        const v = Number(t);
        onChange(Number.isFinite(v) && v > 0 ? v : null);
      }}
    />
  );
}
