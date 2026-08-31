import { defineConfig } from "vitest/config";

// Sprint 1 只承载 colors.ts 的纯函数单测；完整前端测试体系属 Sprint 4
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/__tests__/**/*.test.ts"],
  },
});
