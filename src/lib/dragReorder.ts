// 拖拽排序的顺序计算。**纯函数**，进 vitest。
//
// 抽出来的理由：拖放的 DOM 事件难自动化测试，但"从 from 移到 to 之后列表长什么样"
// 是纯计算，可以完整锁住（含原地放下、越界索引等边界）。

/**
 * 把 `from` 位置的元素移动到 `to` 位置。
 *
 * 返回**新数组**，不改入参。索引非法（越界/NaN）或 `from === to` 时原样返回副本 ——
 * 标准 5 要求"原地放下顺序不变且不报错"、"拖到列表外空白区域不丢项"。
 */
export function moveItem<T>(list: readonly T[], from: number, to: number): T[] {
  const next = list.slice();
  if (!Number.isInteger(from) || !Number.isInteger(to)) return next;
  if (from < 0 || from >= next.length) return next;
  if (to < 0 || to >= next.length) return next;
  if (from === to) return next;

  const [moved] = next.splice(from, 1);
  next.splice(to, 0, moved);
  return next;
}
