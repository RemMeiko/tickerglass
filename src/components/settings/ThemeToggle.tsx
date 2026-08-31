import type { AppConfig } from "../../types";
import { updateAppearance } from "../../lib/ipc";

interface Props {
  config: AppConfig | null;
}

/**
 * 日/月主题切换。**图标按钮，不用文字。**
 *
 * 放在设置窗 Tab 栏的右端：主题是全局开关，藏在「外观」页里的话
 * 用户得先找到那一页才能切换。参考壁纸工具（Wallhaven）的做法 ——
 * 标题栏右侧一个图标，点一下就翻。
 *
 * 图标语义按"点了会变成什么"来选，不是"现在是什么"：
 * 当前深色 → 显示太阳（点它去白天），当前浅色 → 显示月亮（点它回夜间）。
 * 这是系统设置类开关的通行约定，反过来会让人犹豫。
 *
 * SVG 内联而不是引图标库：两个图标而已，加一个依赖不值得，
 * 且内联能直接用 `currentColor` 跟着主题走。
 */
export default function ThemeToggle({ config }: Props) {
  const a = config?.appearance;
  const isLight = a?.theme === "light";

  const toggle = () => {
    if (!a) return;
    void updateAppearance({
      ...a,
      theme: isLight ? "dark" : "light",
    }).catch((e) => console.error("切换主题失败:", e));
  };

  return (
    <button
      type="button"
      role="switch"
      aria-checked={isLight}
      className="theme-toggle"
      onClick={toggle}
      // 标题写"切到什么"，与图标语义一致
      title={isLight ? "切换到夜间模式" : "切换到白天模式"}
      aria-label={isLight ? "切换到夜间模式" : "切换到白天模式"}
      disabled={!a}
    >
      {isLight ? (
        /* 当前白天 → 显示月亮（点它回夜间） */
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
        </svg>
      ) : (
        /* 当前夜间 → 显示太阳（点它去白天） */
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <circle cx="12" cy="12" r="4" />
          <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
        </svg>
      )}
    </button>
  );
}
