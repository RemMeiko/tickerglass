import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { AlertEvent, Market, Snapshot } from "./types";
import { invoke } from "@tauri-apps/api/core";
import { changeColor } from "./lib/colors";
import { displayCode } from "./lib/format";
import { useConfig } from "./hooks/useConfig";
import { useTheme } from "./hooks/useTheme";
import { playAlertSound } from "./lib/sounds";
import { readAlertSound } from "./lib/ipc";

/** 弹窗停留时长。够看清但不至于挡住操作 —— 与悬浮窗内卡片的 12s 分开取值，
 *  因为这个窗口盖在屏幕正中央，停太久干扰更大。 */
const HOLD_MS = 4200;
/** 淡出动画时长，必须与 popup.css 的 .popup-card.leaving 一致 */
const FADE_MS = 320;

/**
 * 屏幕中心提醒弹窗。**独立窗口**，不是悬浮窗里的卡片。
 *
 * 为什么要独立窗口：提醒卡片本来就有（`AlertCard`），但它渲染在悬浮窗内部，
 * 而悬浮窗只有 ~228×360 且常被其他窗口遮住，所以"到价了"这件事很容易被漏掉。
 * 用户要的是"在电脑中心弹窗"，那就必须是一个能盖在所有窗口之上的独立窗口。
 *
 * 两者不冲突、各司其职：悬浮窗内卡片是常驻视野里的轻提示，
 * 这个是抢注意力的强提示。共用同一个 `alert` 事件，不会出现两套判定逻辑。
 *
 * 生命周期：显示 → HOLD_MS 后加 `.leaving` 触发淡出 → FADE_MS 后隐藏窗口。
 * **隐藏而不是关闭**：关掉后下次提醒又要走一次 `build()`（约几百毫秒），
 * 提醒是时效性的东西，等不起；隐藏则是即时的。
 */
export default function PopupApp() {
  const config = useConfig();
  // 主题在这里有两个用途：写 data-theme 换 CSS token，
  // 以及下面 changeColor 的第三参 —— 行价格是内联 style，CSS 覆盖不到它
  const theme = useTheme(config);
  const alerts = config?.alerts;
  const [event, setEvent] = useState<AlertEvent | null>(null);
  const [leaving, setLeaving] = useState(false);
  const [names, setNames] = useState<Record<string, string>>({});
  const [markets, setMarkets] = useState<Record<string, Market>>({});

  /** 名称与市场从快照取 —— 事件里只有代码 */
  useEffect(() => {
    invoke<Snapshot>("get_snapshot")
      .then((snap) => {
        const n: Record<string, string> = {};
        const m: Record<string, Market> = {};
        for (const r of snap.rows) {
          if (r.name) n[r.code] = r.name;
          m[r.code] = r.market;
        }
        setNames(n);
        setMarkets(m);
      })
      .catch(() => void 0);
  }, []);

  const dismiss = useCallback(() => {
    setLeaving(true);
    window.setTimeout(() => {
      void getCurrentWindow().hide();
      setEvent(null);
      setLeaving(false);
    }, FADE_MS);
  }, []);

  /**
   * 提示音。**这段是从悬浮窗的 AlertCard 搬过来的。**
   *
   * 2026-08-30 移除悬浮窗内提醒卡片时，声音差点被一起删掉 ——
   * 放音原本挂在那个组件里。中心弹窗接管视觉提醒后，声音也必须跟着搬，
   * 否则表现是"弹窗有了但没声音"。
   *
   * 走与设置页试听**同一个函数**：分两份实现必然出现"试听正常但提醒时不一样"。
   * 它内部已处理自定义音频读取失败退回内置音、音频完全不可用时静默，
   * 所以这里不需要 try/catch —— 视觉提醒不该被声音故障拖累。
   */
  const beep = useCallback(
    (side: "bull" | "bear") => {
      if (!alerts) return;
      const on = side === "bull" ? alerts.bull_sound : alerts.bear_sound;
      if (!on) return;
      const tone = side === "bull" ? alerts.bull_tone : alerts.bear_tone;
      void playAlertSound(
        tone,
        side === "bull",
        alerts.sound_volume,
        readAlertSound
      );
    },
    [alerts]
  );

  useEffect(() => {
    let timer: number | undefined;
    let disposed = false;

    const un = listen<AlertEvent[]>("alert", (e) => {
      const list = e.payload;
      if (disposed || list.length === 0) return;
      // 同周期可能多只同时越界。窗口只展示**最近一条** ——
      // 屏幕正中央堆叠多张卡片反而看不清。
      setEvent(list[list.length - 1]);
      setLeaving(false);
      // **窗口由这里显示，不由 Rust 侧显示。**
      // 后端 ensure_popup 只建窗口 + 摆位置，show 留给这一刻 ——
      // 内容与可见性同时发生，不会出现"窗口可见但里面是空的"。
      // 原先在 Rust 建完就 show，导致启动后屏幕中央挂一个永不消失的空窗口
      // （dismiss 的定时器只在收到事件时才调度，空窗口没人关）。
      void getCurrentWindow().show();
      // 一批里牛熊各响一次即可，不按条数叠加轰炸
      if (list.some((b) => b.side === "bull")) beep("bull");
      if (list.some((b) => b.side === "bear")) beep("bear");
      if (timer) window.clearTimeout(timer);
      timer = window.setTimeout(dismiss, HOLD_MS);
    });

    return () => {
      disposed = true;
      if (timer) window.clearTimeout(timer);
      void un.then((f) => f());
    };
    // beep 进依赖：不进的话监听器闭包会一直捕获旧的音色/音量，
    // 用户改了设置要重启才生效
  }, [dismiss, beep]);

  if (!event) return null;

  const market = markets[event.code] ?? "cn";
  // 涨跌方向按市场决定红绿（A股红涨绿跌、美股相反），与行情列表同一套规则
  const color = changeColor(
    event.percent ?? (event.side === "bull" ? 1 : -1),
    market,
    theme
  );

  const reason =
    event.trigger === "percent"
      ? `${event.side === "bull" ? "涨过" : "跌过"} ${event.threshold >= 0 ? "+" : ""}${event.threshold.toFixed(2)}%`
      : `${event.side === "bull" ? "达到止盈价" : "跌破止跌价"} ${event.threshold.toFixed(2)}`;

  return (
    <div
      className={`popup-card ${event.side} ${leaving ? "leaving" : ""}`}
      onClick={dismiss}
      title="点击关闭"
    >
      <div className="pc-name">{names[event.code] ?? displayCode(event.code)}</div>
      <div className="pc-price" style={{ color }}>
        {event.price.toFixed(2)}
        {event.percent != null && (
          <span className="pc-pct">
            {event.percent >= 0 ? "+" : ""}
            {event.percent.toFixed(2)}%
          </span>
        )}
      </div>
      <div className="pc-reason">{reason}</div>
    </div>
  );
}
