import { useState } from "react";
import type { AppConfig } from "../../types";
import { setAlwaysOnTop, setLocked } from "../../lib/ipc";

interface Props {
  config: AppConfig;
}

export default function WindowPanel({ config }: Props) {
  const w = config.window;
  const [pending, setPending] = useState(false);

  async function toggleTop(enabled: boolean) {
    setPending(true);
    try {
      await setAlwaysOnTop(enabled);
    } catch (e) {
      console.error("设置置顶失败:", e);
    } finally {
      setPending(false);
    }
  }

  async function toggleLock(locked: boolean) {
    setPending(true);
    try {
      await setLocked(locked);
    } catch (e) {
      console.error("设置锁定失败:", e);
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="panel">
      <label className="check">
        <input
          type="checkbox"
          checked={w.always_on_top}
          disabled={pending}
          onChange={(e) => void toggleTop(e.target.checked)}
        />
        窗口置顶
      </label>
      <p className="hint-text">关闭后悬浮窗会被其他应用窗口遮住。</p>

      <label className="check">
        <input
          type="checkbox"
          checked={w.locked}
          disabled={pending}
          onChange={(e) => void toggleLock(e.target.checked)}
        />
        锁定悬浮窗（不可拖动 + 鼠标点击穿透）
      </label>
      <p className="hint-text hint-strong">
        锁定后悬浮窗不再接受任何鼠标操作，双击也不会打开本设置窗。
        <br />
        解锁方式：<b>右键系统托盘图标 → 解锁悬浮窗</b>。托盘是锁定状态下的唯一入口。
      </p>

      <div className="group-title">当前窗口位置</div>
      <p className="hint-text">
        {w.x === null || w.y === null
          ? "尚未记录（拖动一次悬浮窗后会自动保存）"
          : `x = ${w.x}, y = ${w.y}`}
        <br />
        窗口被拖出屏幕外时会自动回弹，保证至少 60×30 像素可见。
      </p>
    </div>
  );
}
