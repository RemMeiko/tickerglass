#!/usr/bin/env node
/**
 * 直接跑已构建的 exe 做验证，**不经过 .setup 安装**。
 *
 * 存在的理由：每次验证都重装一遍 setup 太慢，而 tauri build --no-bundle 产出的
 * exe 本身就是独立可执行的（前端资源已随 frontendDist 编进二进制，不依赖
 * localhost:1420 的 Vite dev server）。
 *
 * 它额外处理三件手工跑 exe 容易忘的事：
 *   ① 装过的版本与本 exe **共用同一个配置文件**（identifier 相同），
 *      同时开两个会互相盖设置 —— 先检测并拦下来。
 *   ② 日志重定向到文件，否则关掉终端就什么都看不到了。
 *   ③ 退出时把日志尾部打出来，省得再去翻路径。
 *
 * 用法：
 *   npm run app:run              # 跑 debug exe（默认）
 *   npm run app:run -- --release # 跑 release exe
 *   npm run app:run -- --log-only  # 只打印上次的日志尾部，不启动
 */

import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, openSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const PROJ = resolve(HERE, "..");

// target-dir 被 src-tauri/.cargo/config.toml 重定向到 D: 盘（F: 盘空间不足）。
// 这里从该文件里读，而不是硬编码，免得那边一改这里就失效。
function targetDir() {
  const cfg = join(PROJ, "src-tauri", ".cargo", "config.toml");
  if (existsSync(cfg)) {
    const m = /^\s*target-dir\s*=\s*"([^"]+)"/m.exec(readFileSync(cfg, "utf8"));
    if (m) return m[1];
  }
  return join(PROJ, "src-tauri", "target");
}

const args = process.argv.slice(2);
const isRelease = args.includes("--release");
const logOnly = args.includes("--log-only");
const profile = isRelease ? "release" : "debug";

const LOG_DIR = join(PROJ, "harness", "run-logs");
const LOG_FILE = join(LOG_DIR, `run-${profile}.log`);

function tailLog(n = 30) {
  if (!existsSync(LOG_FILE)) {
    console.log(`（无日志文件：${LOG_FILE}）`);
    return;
  }
  const lines = readFileSync(LOG_FILE, "utf8").split(/\r?\n/).filter(Boolean);
  console.log(`\n──── 日志尾部 ${Math.min(n, lines.length)}/${lines.length} 行 ────`);
  console.log(lines.slice(-n).join("\n"));
  console.log(`──── 完整日志：${LOG_FILE} ────`);
}

if (logOnly) {
  tailLog(60);
  process.exit(0);
}

const exe = join(targetDir(), profile, "tickerglass.exe");
if (!existsSync(exe)) {
  console.error(`找不到 exe：${exe}`);
  console.error(
    isRelease
      ? "先跑：npm run app:exe:release"
      : "先跑：npm run app:exe"
  );
  process.exit(1);
}

// ① 冲突检测：已在跑的同名进程会与本次共用配置文件
//
// 必须用 spawnSync 传**参数数组**，不能用 execSync 传整条命令行 ——
// 后者会经过 shell，`/FI "IMAGENAME eq tickerglass.exe"` 里的引号在
// bash→npm→node→cmd 这条链上被剥掉，tasklist 会报
// `Invalid argument/option - 'eq'`。而这个报错会被 catch 吞掉，
// 检测静默失效 → 拦不住第二个实例（本脚本第一版就踩了这个）。
let running = "";
{
  const r = spawnSync(
    "tasklist",
    ["/FI", "IMAGENAME eq tickerglass.exe", "/NH"],
    { encoding: "utf8", shell: false }
  );
  if (r.error || r.status !== 0) {
    console.warn(
      `注意：进程检测不可用（${r.error?.message ?? "exit " + r.status}），跳过冲突拦截。`
    );
  } else {
    running = r.stdout ?? "";
  }
}
if (/tickerglass\.exe/i.test(running)) {
  const pids = [...running.matchAll(/tickerglass\.exe\s+(\d+)/gi)].map((m) => m[1]);
  console.error(`已有 tickerglass.exe 在运行（pid ${pids.join(", ")}）。`);
  console.error(
    "它与本次启动**共用同一个配置文件**（identifier 相同），同时跑会互相覆盖设置。"
  );
  console.error(`先关掉：taskkill /F /IM tickerglass.exe`);
  process.exit(1);
}

mkdirSync(LOG_DIR, { recursive: true });
const fd = openSync(LOG_FILE, "w");

console.log(`启动 ${profile} exe：${exe}`);
console.log(`日志：${LOG_FILE}`);
console.log("按 Ctrl+C 结束（或直接关窗口）。\n");

const child = spawn(exe, [], {
  stdio: ["ignore", fd, fd],
  windowsHide: false,
});

child.on("error", (e) => {
  console.error("启动失败：", e.message);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  console.log(`\n进程结束（code=${code} signal=${signal ?? "-"}）`);
  tailLog(30);
  process.exit(code ?? 0);
});

// Ctrl+C 时连带收掉子进程，别留孤儿占着配置文件
for (const sig of ["SIGINT", "SIGTERM"]) {
  process.on(sig, () => {
    try {
      child.kill();
    } catch {
      /* 已经退了 */
    }
  });
}
