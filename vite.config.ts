import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "path";

// Tauri 期望固定端口，端口漂移会导致 devUrl 指向空页面（白屏）
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    // 多页面入口：悬浮窗 + 独立设置窗。
    // 漏掉 settings.html 会让设置窗在打包后 404（dev 下靠 devServer 仍能打开，
    // 所以这个坑只在 tauri build 后才暴露）
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        settings: resolve(__dirname, "settings.html"),
        chart: resolve(__dirname, "chart.html"),
        market: resolve(__dirname, "market.html"),
        popup: resolve(__dirname, "popup.html"),
      },
    },
  },
});
