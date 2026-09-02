import { useEffect, useState } from "react";
import type { Snapshot } from "../types";
import { fetchSnapshot, subscribeSnapshot } from "../lib/ipc";

/**
 * 行情快照 hook。事件订阅 + 首屏 invoke 双通道（与 `useConfig` 同一套防竞态写法）。
 *
 * 为什么从 `Rows` 里提上来：球态与列表态**都要**这份数据，
 * 留在 `Rows` 内部会让球态另起一条订阅 —— 两条订阅各自持有一份 state，
 * 切换形态时新挂载的那侧要等下一轮推送（最长 60s）才有内容，
 * 中间是空白球/空白列表。提到 App 层后两态共用同一份，切换即时有数据。
 */
export function useSnapshot(): Snapshot | null {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    subscribeSnapshot((snap) => {
      if (!cancelled) setSnapshot(snap);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((e) => console.error("订阅快照失败:", e));

    fetchSnapshot()
      .then((snap) => {
        // 已有事件推送过来时不要用首屏结果回退覆盖
        if (!cancelled) setSnapshot((prev) => prev ?? snap);
      })
      .catch((e) => console.error("获取首屏快照失败:", e));

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return snapshot;
}
