import { useEffect, useState } from "react";
import type { AppConfig } from "../types";
import { fetchConfig, subscribeConfig } from "../lib/ipc";

/**
 * 配置状态 hook。事件订阅 + 首屏 invoke 双通道（与快照同样的防竞态写法）。
 *
 * 跨窗口生效的原理：设置窗改配置 → 后端 `emit("config")` 广播 →
 * 悬浮窗这个 hook 收到 → 立即重渲染。**没有"应用"按钮**（标准 12）。
 */
export function useConfig(): AppConfig | null {
  const [config, setConfig] = useState<AppConfig | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    subscribeConfig((cfg) => {
      if (!cancelled) setConfig(cfg);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((e) => console.error("订阅配置失败:", e));

    fetchConfig()
      .then((cfg) => {
        // 已有事件推来的更新时不要用首屏结果回退覆盖
        if (!cancelled) setConfig((prev) => prev ?? cfg);
      })
      .catch((e) => console.error("获取配置失败:", e));

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return config;
}
