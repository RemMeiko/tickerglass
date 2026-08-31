import { useEffect, useRef, useState } from "react";
import type { AppConfig, SearchItem } from "../../types";
import {
  addWatchItem,
  fetchSnapshot,
  removeWatchItem,
  reorderWatchlist,
  subscribeSnapshot,
} from "../../lib/ipc";
import { setWatchHidden } from "../../lib/ipc";
import { moveItem } from "../../lib/dragReorder";
import { displayCode } from "../../lib/format";
import SearchBox from "./SearchBox";
import DirectCodeBox from "./DirectCodeBox";

const MAX_ITEMS = 50;

const MARKET_LABEL: Record<string, string> = {
  ashare: "A股",
  indexcn: "指数",
  hk: "港股",
  us: "美股",
};

interface Props {
  config: AppConfig;
}

export default function WatchlistPanel({ config }: Props) {
  const list = config.watchlist;
  const [toast, setToast] = useState<string | null>(null);
  /** 请求进行中禁用按钮，防重复提交（标准 23 的快速连点） */
  const [busy, setBusy] = useState(false);
  const [dragFrom, setDragFrom] = useState<number | null>(null);
  const [dragOver, setDragOver] = useState<number | null>(null);
  /**
   * 每一行的 DOM 节点，拖动时用它们的位置做命中测试。
   *
   * 为什么需要：Pointer Events 不像 HTML5 拖放那样自带 dragover 目标，
   * 落点得自己算 —— 拿每行的 getBoundingClientRect 与当前指针 Y 比较。
   */
  const rowsRef = useRef<(HTMLLIElement | null)[]>([]);
  /** 按下时的指针位置与行号。用来区分「点击」和「拖动」（4px 阈值） */
  const pressRef = useRef<{ y: number; index: number } | null>(null);
  /**
   * 代码 → 名称。**名称不在配置里** —— `WatchItem` 只存身份键与市场，
   * 名称是行情报文里的字段，只能从快照拿。
   */
  const [names, setNames] = useState<Record<string, string>>({});
  /**
   * 右键菜单位置与目标行。null = 未打开。
   *
   * 用 HTML 菜单而不是 Tauri 原生菜单：设置窗是**正常尺寸**的窗口，
   * 菜单画在 webview 里不会被裁（悬浮窗那边只有 ~60px 高才必须用原生菜单）。
   * 好处是样式能与设置窗统一，且不需要绕后端一趟。
   */
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    code: string;
    hidden: boolean;
  } | null>(null);

  // 点任意处 / 按 Esc 关菜单。**必须挂在 document 上** ——
  // 只在菜单自身上监听的话，点菜单外面不会触发关闭。
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenu(null);
    };
    // 延一帧再挂：否则唤出菜单的那次 contextmenu 之后紧跟的 click
    // 会立刻把菜单关掉（表现为"右键没反应"）
    const t = window.setTimeout(() => {
      document.addEventListener("click", close);
      document.addEventListener("contextmenu", close);
      document.addEventListener("keydown", onKey);
    }, 0);
    return () => {
      window.clearTimeout(t);
      document.removeEventListener("click", close);
      document.removeEventListener("contextmenu", close);
      document.removeEventListener("keydown", onKey);
    };
  }, [menu]);

  // 用**订阅**而不是定时轮询：后端每轮刷新都会 emit `snapshot`，
  // 再自己起一个 3 秒定时器是重复劳动（AlertPanel 那处是轮询，历史写法）。
  // 首屏另拉一次，避免错过订阅建立前的那个事件。
  useEffect(() => {
    let un: (() => void) | undefined;
    const apply = (snap: { rows: { code: string; name: string | null }[] }) => {
      const n: Record<string, string> = {};
      for (const r of snap.rows) {
        if (r.name) n[r.code] = r.name;
      }
      setNames(n);
    };
    subscribeSnapshot(apply)
      .then((f) => {
        un = f;
      })
      .catch(() => void 0);
    fetchSnapshot().then(apply).catch(() => void 0);
    return () => un?.();
  }, []);

  /** 用于 toast / title 的人类可读标签：优先名称，没有就用代码 */
  function label(code: string): string {
    return names[code] ?? displayCode(code);
  }

  function notify(msg: string) {
    setToast(msg);
    // 单一 toast，不叠多个
    window.setTimeout(() => setToast((cur) => (cur === msg ? null : cur)), 2400);
  }

  async function handlePick(item: SearchItem) {
    if (busy) return;
    setBusy(true);
    try {
      await addWatchItem({
        tencent_code: item.tencent_code,
        em_secid: item.em_secid,
        // market/is_index 由后端按 tencent_code 用 classify() 覆写（见
        // ipc::add_watch_item 文档）。前端拿不到可靠市场信息，这里传的值仅为满足类型 ——
        // 尤其港股/美股经搜索添加时这个猜测是错的，必须靠后端纠正。
        market: item.is_index ? "indexcn" : "ashare",
        is_index: item.is_index,
        // 新加的标的默认可见 —— 刚添加就藏起来毫无道理
        hidden: false,
      });
      notify(`已添加 ${item.name}`);
    } catch (e) {
      // 后端把"已达上限"/"已在自选股中"作为 Err 返回
      notify(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleRemove(tencentCode: string, name: string) {
    if (busy) return;
    setBusy(true);
    try {
      // 传 tencent_code（唯一键）。传裸码会同时删掉两个市场的同码标的
      await removeWatchItem(tencentCode);
      notify(`已删除 ${name}`);
    } catch (e) {
      notify(`删除失败：${String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function commitOrder(from: number, to: number) {
    const next = moveItem(list, from, to);
    // 原地放下：顺序没变就不发请求
    if (next.every((it, i) => it.tencent_code === list[i]?.tencent_code)) return;
    try {
      await reorderWatchlist(next.map((it) => it.tencent_code));
    } catch (e) {
      notify(`排序失败：${String(e)}`);
    }
  }

  /**
   * 上移 / 下移一格。
   *
   * 存在理由：HTML5 拖放在 WebView2 上**本质上不够稳**（用户报"有时按住不生效"，
   * 已修了 setData 与入场动画两个成因，但拖放依赖 mousedown→mousemove 的时序，
   * 边缘情况仍可能失手）。按钮是确定性的：点一下必然移动一格，
   * 不依赖任何时序或命中测试。
   *
   * 复用 `commitOrder` 而不是另写一套顺序计算 —— 两套算法必然分叉。
   */
  async function move(i: number, dir: -1 | 1) {
    if (busy) return;
    const to = i + dir;
    // 首行不能上移、末行不能下移。按钮那里也会 disabled，这里是双重保险
    if (to < 0 || to >= list.length) return;
    await commitOrder(i, to);
  }

  async function toggleHidden(code: string, hidden: boolean) {
    if (busy) return;
    setBusy(true);
    setMenu(null);
    try {
      await setWatchHidden(code, hidden);
      notify(hidden ? `已隐藏 ${label(code)}` : `已显示 ${label(code)}`);
    } catch (e) {
      notify(`操作失败：${String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="panel">
      <SearchBox onPick={handlePick} limitReached={list.length >= MAX_ITEMS} />

      {/* 两条添加通道并存：搜索按名称/拼音（依赖东财搜索接口，会限流），
          直接输码走报价接口（限流时仍可用）。见 DirectCodeBox 的文档 */}
      <DirectCodeBox
        onPick={handlePick}
        limitReached={list.length >= MAX_ITEMS}
      />

      <div className="list-header">
        <span>
          自选股 {list.length} / {MAX_ITEMS}
        </span>
        {/* 提示里把两种方式都写出来：拖放在 WebView2 上不够稳，
            用户失手时应该知道有按钮这条确定的路 */}
        <span className="hint-text">拖动行或用 ↑↓ 调整顺序</span>
      </div>

      {list.length === 0 ? (
        <div className="empty-list">还没有自选股，用上面的搜索框添加</div>
      ) : (
        <ul className="watch-list">
          {list.map((it, i) => (
            <li
              key={it.tencent_code}
              className={[
                "watch-row",
                dragFrom === i ? "dragging" : "",
                dragOver === i && dragFrom !== i ? "drag-over" : "",
                it.hidden ? "wr-is-hidden" : "",
              ]
                .filter(Boolean)
                .join(" ")}
              onContextMenu={(e) => {
                e.preventDefault();
                setMenu({
                  x: e.clientX,
                  y: e.clientY,
                  code: it.tencent_code,
                  hidden: it.hidden,
                });
              }}
              ref={(el) => {
                rowsRef.current[i] = el;
              }}
              /**
               * 拖动排序用 **Pointer Events 自己实现**，不用 HTML5 拖放。
               *
               * 为什么弃用 HTML5 拖放：它在 WebView2 上有一串隐式前提 ——
               * dragstart 必须 setData、effectAllowed 与 dropEffect 必须配对、
               * 全局 `user-select:none` 会阻止拖动启动、入场动画的 translate3d
               * 期间命中盒与绘制盒错位。前后修过三轮（补 setData、补
               * -webkit-user-drag），用户始终报"按住鼠标无法拖动换行"。
               * 每修好一个前提就冒出下一个，说明这条路的失败面太宽。
               *
               * Pointer Events 没有这些耦合：按下就收事件，落点自己算。
               * 代价是要手写命中测试与阈值判断，但行为完全可控。
               */
              onPointerDown={(e) => {
                // 只响应主键；右键要留给上下文菜单
                if (e.button !== 0 || busy) return;
                // 输入框/按钮上按下不启动拖动（否则点↑↓按钮会被当成拖）
                const t = e.target as HTMLElement;
                if (t.closest("button, input")) return;
                pressRef.current = { y: e.clientY, index: i };
                // **setPointerCapture 是关键**：捕获后即使指针移出这一行
                // （拖到别行上方、甚至移出列表）仍然由本元素接收 move/up，
                // 不必在每一行上都挂监听，也不会因为跨元素而丢事件。
                e.currentTarget.setPointerCapture(e.pointerId);
              }}
              onPointerMove={(e) => {
                const press = pressRef.current;
                if (!press) return;
                // 4px 阈值：小于它视作点击（用户只是想选中/右键），
                // 不设阈值会让每次轻微抖动都进入拖动态，行会闪一下透明度
                if (dragFrom === null) {
                  if (Math.abs(e.clientY - press.y) < 4) return;
                  setDragFrom(press.index);
                }
                // 命中测试：拿各行的实际位置与当前指针 Y 比，找落点。
                // 用 rect 而不是 elementFromPoint —— 后者在 pointer capture
                // 期间会一直返回捕获元素本身，永远算不出别的行。
                let target = press.index;
                for (let k = 0; k < rowsRef.current.length; k++) {
                  const el = rowsRef.current[k];
                  if (!el) continue;
                  const r = el.getBoundingClientRect();
                  if (e.clientY >= r.top && e.clientY <= r.bottom) {
                    target = k;
                    break;
                  }
                }
                setDragOver(target);
              }}
              onPointerUp={(e) => {
                const press = pressRef.current;
                pressRef.current = null;
                e.currentTarget.releasePointerCapture(e.pointerId);
                // 没进入拖动态 = 这是一次点击，什么都不做
                if (press === null || dragFrom === null) {
                  setDragFrom(null);
                  setDragOver(null);
                  return;
                }
                const to = dragOver;
                setDragFrom(null);
                setDragOver(null);
                if (to !== null && to !== dragFrom) {
                  void commitOrder(dragFrom, to);
                }
              }}
              onPointerCancel={() => {
                // 系统取消（比如触摸被手势接管）：清干净，别留下半拖状态
                pressRef.current = null;
                setDragFrom(null);
                setDragOver(null);
              }}
            >
              <span className="wr-handle" aria-hidden="true">
                ⋮⋮
              </span>
              <span className="wr-index">{i + 1}</span>
              {/* 名称为主、代码为辅。名称拿不到时（首屏未刷新 / 该标的取数失败）
                  退回显示代码 —— 绝不显示空白，那会让用户以为这行坏了。 */}
              <span className="wr-name">
                {names[it.tencent_code] ?? displayCode(it.tencent_code)}
              </span>
              <span className="wr-code">{displayCode(it.tencent_code)}</span>
              {/* 隐藏标记必须显眼：用户要能一眼看出这只还在、只是被藏了，
                  否则会以为删掉了又重复添加 */}
              {it.hidden && <span className="wr-hidden-tag">已隐藏</span>}
              <span className="wr-market">
                {MARKET_LABEL[it.market] ?? it.market}
              </span>
              {/* 上移/下移：拖放的确定性替代。点一下必然移动一格，
                  不依赖 mousedown→mousemove 时序，也不受命中测试影响。 */}
              <span className="wr-order">
                <button
                  className="wr-move"
                  disabled={busy || i === 0}
                  title={i === 0 ? "已在最上方" : `上移 ${label(it.tencent_code)}`}
                  aria-label={`上移 ${label(it.tencent_code)}`}
                  onClick={() => void move(i, -1)}
                >
                  ↑
                </button>
                <button
                  className="wr-move"
                  disabled={busy || i === list.length - 1}
                  title={
                    i === list.length - 1
                      ? "已在最下方"
                      : `下移 ${label(it.tencent_code)}`
                  }
                  aria-label={`下移 ${label(it.tencent_code)}`}
                  onClick={() => void move(i, 1)}
                >
                  ↓
                </button>
              </span>
              <button
                className="wr-remove"
                disabled={busy}
                title={`删除 ${label(it.tencent_code)}`}
                aria-label={`删除 ${label(it.tencent_code)}`}
                onClick={() =>
                  void handleRemove(it.tencent_code, label(it.tencent_code))
                }
              >
                删除
              </button>
            </li>
          ))}
        </ul>
      )}

      {list.length >= MAX_ITEMS && (
        <div className="warn-bar">已达 {MAX_ITEMS} 只上限，需先删除才能添加</div>
      )}

      {/* 右键菜单。position:fixed + clientX/Y：不受列表滚动影响。
          设置窗尺寸正常，菜单不会被裁（悬浮窗那边才必须用系统原生菜单）。 */}
      {menu && (
        <div
          className="wr-menu"
          style={{ left: menu.x, top: menu.y }}
          // 挡住冒泡：否则点菜单项自身会先触发 document 的 click 关闭逻辑
          onClick={(e) => e.stopPropagation()}
        >
          <button
            className="wr-menu-item"
            disabled={busy}
            onClick={() => void toggleHidden(menu.code, !menu.hidden)}
          >
            {menu.hidden ? "取消隐藏" : "隐藏（不删除）"}
          </button>
          <div className="wr-menu-sep" />
          <button
            className="wr-menu-item wr-menu-danger"
            disabled={busy}
            onClick={() => {
              setMenu(null);
              void handleRemove(menu.code, label(menu.code));
            }}
          >
            删除
          </button>
        </div>
      )}

      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
    </div>
  );
}
