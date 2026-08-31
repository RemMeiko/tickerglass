import {
  Component,
  type ErrorInfo,
  type ReactNode,
  useEffect,
  useRef,
  useState,
} from "react";
import type { AppConfig, Snapshot } from "./types";
import {
  dismissConfigNotice,
  fetchConfigNotice,
  fetchSnapshot,
  openChartWindow,
  openSettingsWindow,
  resizeMainWindow,
  showContextMenu,
  subscribeSnapshot,
} from "./lib/ipc";
import { useConfig } from "./hooks/useConfig";
import { chartSize, cssVars, windowSizeFor } from "./lib/appearance";
import QuoteRow from "./components/QuoteRow";
import EmptyHint from "./components/EmptyHint";

/** 兜底，保证任何渲染异常都不会让悬浮窗变成空白 */
class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("渲染异常:", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return <div className="fatal">界面异常：{this.state.error.message}</div>;
    }
    return this.props.children;
  }
}

/** 尺寸上报防抖（D4）。不防抖会让拖动缩放滑块时每帧一次 set_size，造成窗口抖动 */
const RESIZE_DEBOUNCE_MS = 100;

/**
 * 悬浮窗每页最多显示几只。
 *
 * 取 8 的理由：悬浮窗高度按内容量出来（D4），8 行加上页码点大约 400px，
 * 在 1080p 上仍是"瞄一眼"的尺寸；再多就开始挡住桌面内容。
 * 超出的放到下一页，鼠标悬停时滚轮换页。
 */
const ROWS_PER_PAGE = 8;

function Rows({
  config,
  locked,
  onOpenChart,
  onRowContextMenu,
}: {
  config: AppConfig | null;
  locked: boolean;
  onOpenChart: (code: string, name: string | null) => void;
  onRowContextMenu: (code: string) => void;
}) {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  /**
   * 当前页（0 起）。超过 ROWS_PER_PAGE 只的自选股分页显示，滚轮换页。
   *
   * **必须声明在所有提前 return 之前** —— 下面有三处早退
   * （无自选股 / 全部隐藏 / 首屏骨架），hook 放在它们之后会违反调用顺序规则。
   */
  const [page, setPage] = useState(0);
  /**
   * 上一次翻页的方向。1 = 往后翻（新页从下方滑入），-1 = 往前翻（从上方滑入）。
   *
   * 为什么要存方向：动画方向必须跟手 —— 往下滚时新内容从下面上来、
   * 往上滚时从上面下来，才符合"页面在纵向排队"的直觉。
   * 固定一个方向的话，往回翻会感觉在倒放。
   */
  const [pageDir, setPageDir] = useState<1 | -1>(1);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    // 事件订阅 + 首屏 invoke 双通道，避免错过启动瞬间的首个事件
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

  // 首屏骨架由后端的 Pending 行提供；rows 为空才是"无自选股"
  if (snapshot && snapshot.rows.length === 0) {
    return <EmptyHint />;
  }

  // 隐藏过滤**只发生在这一层**。后端快照保留隐藏行（设置页要用它取名称，
  // 提醒判定也照常跑），所以这里是"隐藏"这个功能真正生效的地方。
  const rows = (snapshot?.rows ?? []).filter((r) => !r.hidden);

  // 全部隐藏：绝不能显示"暂无自选股" —— 用户会以为股票被删了。
  // 必须说清"还在，只是藏起来了"以及去哪里取消。
  if (snapshot && snapshot.rows.length > 0 && rows.length === 0) {
    return <AllHiddenHint count={snapshot.rows.length} />;
  }

  const appearance = config?.appearance ?? null;
  const chart = appearance
    ? chartSize(appearance)
    : { width: 104, height: 30 };

  // ── 分页
  //
  // 超过 8 只就分页，滚轮换页。为什么要分页而不是让窗口一直长下去：
  // 悬浮窗高度是按内容量出来的，50 只自选股会顶到屏幕外，
  // 且常驻桌面的东西占满整屏就失去"瞄一眼"的意义。
  const totalPages = Math.max(1, Math.ceil(rows.length / ROWS_PER_PAGE));
  // 钳制页码：用户删股票后当前页可能已越界（比如停在第 2 页却只剩 5 只）。
  // 不钳制会渲染出空白页，且滚轮要滚回去才有内容。
  const safePage = Math.min(page, totalPages - 1);
  const pageRows = rows.slice(
    safePage * ROWS_PER_PAGE,
    safePage * ROWS_PER_PAGE + ROWS_PER_PAGE
  );

  return (
    <div
      className="rows"
      // 滚轮换页。**不需要 preventDefault** —— 悬浮窗没有可滚动的溢出内容，
      // 所以不会连带滚动页面（这里与设置页的 NumberInput 情况不同，
      // 那边在可滚动面板里必须拦）。
      onWheel={(e) => {
        if (totalPages <= 1) return;
        const dir = e.deltaY > 0 ? 1 : -1;
        setPage((p) => {
          // 用 safePage 而不是 p 做基准：p 可能是越界的旧值
          const cur = Math.min(p, totalPages - 1);
          const next = Math.max(0, Math.min(totalPages - 1, cur + dir));
          // 只在真的换了页时记方向：到头后继续滚不该重放动画
          if (next !== cur) setPageDir(dir === 1 ? 1 : -1);
          // 到头就停住，不循环 —— 循环会让用户分不清自己在第几页
          return next;
        });
      }}
    >
      {/* 换页滑入动画。
          key 绑页码 → React 重建这个节点 → CSS animation 每次换页都重放。
          不加 key 的话同一个 DOM 节点被复用，animation 只在首次挂载跑一次
          （这个坑设置窗的 Tab 切换踩过，见 SettingsApp 的注释）。
          只动 transform + opacity（合成层属性），不触发重排 ——
          悬浮窗高度由 ResizeObserver 量内容上报，若动画改变布局尺寸
          会让窗口跟着抽动。 */}
      <div
        key={safePage}
        className={`page-slide ${pageDir === 1 ? "from-below" : "from-above"}`}
      >
        {pageRows.map((row) => (
          <QuoteRow
            key={row.code}
            row={row}
            showCode={appearance?.show_code ?? false}
            showMarket={appearance?.show_market ?? false}
            curveWidth={appearance?.curve_width ?? 1}
            curveOpacity={appearance?.opacity_curve ?? 1}
            chartWidth={chart.width}
            chartHeight={chart.height}
            locked={locked}
            onOpenChart={onOpenChart}
            onRowContextMenu={onRowContextMenu}
          />
        ))}
      </div>
      {/* 页码指示：只在真的有多页时出现，单页时不占地方。
          点圆点也能跳页 —— 滚轮是主要手段，但只给滚轮的话
          用户不容易发现"还有第二页"。 */}
      {totalPages > 1 && (
        <div className="page-dots" title={`第 ${safePage + 1} / ${totalPages} 页，滚轮换页`}>
          {Array.from({ length: totalPages }, (_, i) => (
            <button
              key={i}
              type="button"
              className={`page-dot ${i === safePage ? "active" : ""}`}
              aria-label={`第 ${i + 1} 页`}
              aria-current={i === safePage}
              onClick={(e) => {
                // 挡住冒泡：外层 content-layer 有"双击开设置窗"
                e.stopPropagation();
                // 方向按"跳到的页在当前页前面还是后面"算 ——
                // 不记的话会沿用上一次滚轮的方向，出现"往前跳却从下面滑入"
                if (i !== safePage) setPageDir(i > safePage ? 1 : -1);
                setPage(i);
              }}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * 全部自选股都被隐藏时的提示。
 *
 * **与 `EmptyHint` 严格区分。** 显示"暂无自选股"会让用户以为股票被删了，
 * 进而重复添加、或以为配置丢了。这里必须说明三件事：
 * 还在（N 只）、是被隐藏了、去哪里取消。
 */
function AllHiddenHint({ count }: { count: number }) {
  return (
    <div className="empty-hint">
      <div className="empty-title">{count} 只自选股已全部隐藏</div>
      <div className="empty-sub">右键 → 设置 → 自选股，右键行可取消隐藏</div>
    </div>
  );
}

/** 配置损坏/迁移后的可见提示条（标准 19） */
function ConfigNotice() {
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    fetchConfigNotice()
      .then(setNotice)
      .catch((e) => console.error("读取配置提示失败:", e));
  }, []);

  if (!notice) return null;
  return (
    <div className="notice-bar" role="status">
      <span className="notice-text">{notice}</span>
      <button
        className="notice-close"
        aria-label="关闭提示"
        onClick={() => {
          setNotice(null);
          void dismissConfigNotice().catch(() => {});
        }}
      >
        ×
      </button>
    </div>
  );
}

export default function App() {
  const config = useConfig();
  const measureRef = useRef<HTMLDivElement | null>(null);

  // 内容尺寸变化 → 防抖后上报后端 set_size（D4）
  useEffect(() => {
    const el = measureRef.current;
    if (!el) return;
    let timer: number | undefined;

    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      // 必须用 borderBoxSize 而非 contentRect：
      // contentRect 是**内容盒**，不含 `.content-layer` 的 padding（--padding，随 scale 变化）。
      // 用它上报会让窗口比实际内容窄/短「两倍 padding - 2px 边框」，
      // 表现为右边与下边被裁掉（scale=0.95 时约 9px，实测窗口 226x175 而内容需 ~236x184）。
      // borderBoxSize 含 padding 与 border，正是 set_size 需要的量。
      const box = entry.borderBoxSize?.[0];
      const width = box ? box.inlineSize : entry.contentRect.width;
      const height = box ? box.blockSize : entry.contentRect.height;
      if (width <= 0 || height <= 0) return;
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const size = windowSizeFor(width, height);
        void resizeMainWindow(size.width, size.height).catch((e) =>
          console.error("上报窗口尺寸失败:", e)
        );
      }, RESIZE_DEBOUNCE_MS);
    });

    observer.observe(el);
    return () => {
      window.clearTimeout(timer);
      observer.disconnect();
    };
  }, []);

  const locked = config?.window.locked ?? false;
  // CSS 变量注入（D3）：缩放走变量而不是 transform: scale()
  const style = config
    ? (cssVars(config.appearance) as React.CSSProperties)
    : undefined;

  // 原先这里有一份「代码 → 名称/市场」映射 + 一条快照订阅，专供已删除的
  // 提醒卡片使用。卡片移到中心弹窗后一并删除 —— PopupApp 自己取快照，
  // 留在这里就是每轮白订阅一次。

  // 双击某一行 → 大图窗看这只（方案 A）。行的 onDoubleClick 已 stopPropagation，
  // 故不会连带触发下方 content-layer 的"双击开设置窗"。
  const handleOpenChart = (code: string, name: string | null) => {
    void openChartWindow(code, name).catch((e) =>
      console.error("打开大图窗失败:", e)
    );
  };

  // 右键落在某一行上 → 菜单顶部多一项「隐藏 名称」（用户 2026-08-29 明确要求
  // 是**股票行上**右键，不是窗口级菜单）。行组件已 stopPropagation，
  // 所以不会连带弹出下面那个通用菜单。
  const handleRowContextMenu = (code: string) => {
    if (locked) return;
    void showContextMenu(code).catch((err) =>
      console.error("弹出行右键菜单失败:", err)
    );
  };

  return (
    <div
      className="app"
      style={style}
      // 右键菜单挂在**最外层**，这样窗口上任何位置都能唤出 —— 包括股票行上。
      //
      // 用户反馈「双击空白处很难，空白处很小」：悬浮窗尺寸是按内容量出来的（D4），
      // 一只自选股时几乎没有空白可点。右键不依赖空白区，是更可靠的入口。
      // 双击开设置的老路径保留（习惯了的人还能用）。
      onContextMenu={(e) => {
        // 必须挡掉 webview 自带的检查元素菜单，否则两个菜单会一起弹
        e.preventDefault();
        // 锁定态下鼠标事件已被系统穿透，这里收不到；写这个判断是双重保险
        if (locked) return;
        void showContextMenu().catch((err) =>
          console.error("弹出右键菜单失败:", err)
        );
      }}
    >
      {/*
        背景板是**独立兄弟层**（D2）：它不是文字/曲线的祖先元素，
        所以把它的 opacity 调到 0 不会让文字与曲线一起消失。
      */}
      <div className="bg-layer" aria-hidden="true" />

      {/*
        被测量的元素是这层包裹（不是 content-layer）：提醒卡片必须一起计入，
        否则卡片落在窗口尺寸之外会被裁掉 —— 只有一只自选股时窗口才 ~60px 高，
        而一张卡片就有 ~44px，几乎全被切掉。
        它自身不设 padding/边框，故测得的盒子等于「内容层 + 卡片栈」的并集。
      */}
      <div className="measure" ref={measureRef}>
        {/*
          拖拽区挂在内容层而**不是**背景板上：背景板 opacity:0 时窗口仍须可拖动。
          锁定时移除 data-tauri-drag-region（配合后端 set_ignore_cursor_events）。
        */}
        <div
          className="content-layer"
          {...(locked ? {} : { "data-tauri-drag-region": true })}
          onDoubleClick={() => {
            // 锁定态下不响应双击（此时鼠标事件本已被系统穿透，这里是双重保险）
            if (locked) return;
            void openSettingsWindow().catch((e) =>
              console.error("打开设置窗失败:", e)
            );
          }}
          title={locked ? "" : "右键打开菜单（也可双击此处开设置）"}
        >
          <ConfigNotice />
          <ErrorBoundary>
            <Rows
              config={config}
              locked={locked}
              onOpenChart={handleOpenChart}
              onRowContextMenu={handleRowContextMenu}
            />
          </ErrorBoundary>
        </div>

        {/* 提醒卡片已移除（2026-08-30）。
            到价提醒改由**屏幕中心的独立弹窗**承担（popup 窗口 / PopupApp.tsx），
            理由是悬浮窗只有 ~228×360 且常被其他窗口遮住，卡片在这里很容易被漏看。
            提示音也跟着搬到了 PopupApp —— 原先放音挂在 AlertCard 里，
            删组件时若不搬会变成"弹窗有了但没声音"。 */}
      </div>
    </div>
  );
}
