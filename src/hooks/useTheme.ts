import { useEffect } from "react";
import type { AppConfig } from "../types";
import type { Theme } from "../lib/colors";

/**
 * 把配置里的主题写到 `<html data-theme>` 上，并返回生效的主题。
 *
 * # 为什么挂在 `<html>` 而不是某个容器
 *
 * token 覆盖块的选择器是 `:root[data-theme="light"]`，`:root` 就是 `<html>`。
 * 挂在更内层的 div 上，`:root` 匹配不到，整块覆盖静默失效
 * （不报错、只是颜色不变）—— 这是最容易踩且最难查的一种。
 *
 * # 为什么每个窗口入口都要调
 *
 * 四个窗口是**四个独立 WebView**，各有自己的 document。
 * 在设置窗里写 `data-theme` 不会影响大图窗的 DOM，所以每个入口都得挂一次。
 *
 * # 悬浮窗为什么传 null
 *
 * 悬浮窗是 transparent 窗口，铺浅色底就是桌面上一块白斑；
 * 且它靠 `opacity_background` 调透明度，与主题两套机制会打架。
 * 传 null 让它固定深色。
 */
export function useTheme(config: AppConfig | null): Theme {
  const theme: Theme = config?.appearance.theme === "light" ? "light" : "dark";

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  return theme;
}
