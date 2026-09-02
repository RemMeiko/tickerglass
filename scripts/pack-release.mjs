#!/usr/bin/env node
/**
 * 把已构建的 release exe 打成成品交付包。
 *
 * 成品目录（用户 2026-08-31 指定）：
 *   E:\61.AI_tools\TickerGlass_release-v{version}\
 *     ├── TickerGlass_release-v{version}.exe
 *     └── 使用说明.txt
 *
 * 为什么要这个脚本，而不是记在文档里让人手动拷：
 *   ① 版本号只在 tauri.conf.json 里写一次，目录名和 exe 名都由它推导 ——
 *      升版本时不会漏改某一处（改名那次就漏了 scripts/*.mjs 的 7 处旧名）。
 *   ② 拷完立刻比 sha1，确保成品和构建产物逐字节一致。手动拷贝拷错、
 *      拷成旧版都不会报错，这是本项目「假完成」踩过的坑。
 *   ③ target-dir 被 .cargo/config.toml 重定向到 D 盘，路径从那里读而非硬编码。
 *
 * 注意：这里只做「拷贝 + 改名 + 校验」，不触发编译。
 * 因为 cargo 在 exe 运行中会 `failed to remove file`，编译时机要人来定。
 * 没有 release 产物时脚本会明确报错并给出该跑的命令。
 *
 * 用法：
 *   npm run pack              # 出到 E 盘成品目录
 *   npm run pack -- --zip     # 额外打一个 zip（同目录）
 *   npm run pack -- --dry     # 只打印将要做什么，不写盘
 */

import { existsSync, mkdirSync, readFileSync, copyFileSync, statSync, readdirSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const HERE = dirname(fileURLToPath(import.meta.url));
const PROJ = resolve(HERE, "..");

// 成品根目录。用户指定放 E 盘的 AI 工具集里，与 D 盘的构建中间产物分开。
const OUT_ROOT = "E:\\61.AI_tools";

const argv = process.argv.slice(2);
const WANT_ZIP = argv.includes("--zip");
const DRY = argv.includes("--dry");

function die(msg, hint) {
  console.error(`\n✗ ${msg}`);
  if (hint) console.error(`  ${hint}`);
  process.exit(1);
}

function sha1(p) {
  return createHash("sha1").update(readFileSync(p)).digest("hex");
}

function mb(bytes) {
  return (bytes / 1048576).toFixed(2) + " MB";
}

/**
 * target-dir 被 src-tauri/.cargo/config.toml 重定向（本机 F 盘空间不足，指到 D 盘）。
 * 从该文件读而非硬编码 —— 那边一改这里就跟着变，换机器删掉该文件也能回落默认值。
 * 与 run-exe.mjs 的同名函数保持一致行为。
 */
function targetDir() {
  const cfg = join(PROJ, "src-tauri", ".cargo", "config.toml");
  if (existsSync(cfg)) {
    const m = /^\s*target-dir\s*=\s*"([^"]+)"/m.exec(readFileSync(cfg, "utf8"));
    if (m) return m[1];
  }
  return join(PROJ, "src-tauri", "target");
}

// ── 版本号：唯一真源是 tauri.conf.json，目录名与 exe 名都由它推导 ──
const confPath = join(PROJ, "src-tauri", "tauri.conf.json");
if (!existsSync(confPath)) die(`找不到 ${confPath}`);
const conf = JSON.parse(readFileSync(confPath, "utf8"));
const version = conf.version;
const productName = conf.productName;
if (!version) die("tauri.conf.json 里没有 version 字段");

// 交付名统一为 {productName}_release-v{version}，目录与 exe 同名
const stem = `${productName}_release-v${version}`;
const outDir = join(OUT_ROOT, stem);

// ── 源产物 ──
const relDir = join(targetDir(), "release");
// Cargo 用 package name 命名 exe（tickerglass），与 productName（TickerGlass）不同
const srcExe = join(relDir, "tickerglass.exe");
const srcTxt = join(PROJ, "packaging", "使用说明.txt");

if (!existsSync(srcExe)) {
  die(`没有 release 产物：${srcExe}`,
    "先跑 `npm run app:exe:release`（编译前记得关掉正在运行的 exe，否则 failed to remove file）");
}
if (!existsSync(srcTxt)) {
  die(`没有 ${srcTxt}`, "packaging/使用说明.txt 是随包发布的说明文档，它是唯一真源，不该缺失");
}

// ── 构建产物新鲜度自检 ──
// exe 比源码旧 = 改了代码没重编。这是本项目「假完成」的头号成因，
// 所以宁可啰嗦也要在出包时拦一次。
const exeMtime = statSync(srcExe).mtimeMs;
const newer = [];
for (const sub of ["src", "src-tauri/src"]) {
  const base = join(PROJ, sub);
  const stack = [base];
  while (stack.length) {
    const d = stack.pop();
    let es;
    try { es = readdirSync(d, { withFileTypes: true }); } catch { continue; }
    for (const e of es) {
      const fp = join(d, e.name);
      if (e.isDirectory()) { stack.push(fp); continue; }
      if (!/\.(rs|tsx?|css)$/.test(e.name)) continue;
      try { if (statSync(fp).mtimeMs > exeMtime) newer.push(fp.slice(PROJ.length + 1)); } catch {}
    }
  }
}

const outExe = join(outDir, `${stem}.exe`);
const outTxt = join(outDir, "使用说明.txt");

console.log(`\n${productName} v${version} → 成品包`);
console.log(`  源 exe    ${srcExe}`);
console.log(`            ${mb(statSync(srcExe).size)}  ${new Date(exeMtime).toLocaleString("zh-CN")}`);
console.log(`  目标目录  ${outDir}`);
console.log(`  目标 exe  ${stem}.exe`);

if (newer.length) {
  console.log(`\n⚠ 有 ${newer.length} 个源文件比 exe 新，这个包可能不含最新改动：`);
  for (const f of newer.slice(0, 8)) console.log(`    ${f}`);
  if (newer.length > 8) console.log(`    …还有 ${newer.length - 8} 个`);
  console.log(`  要包含它们请先重跑 npm run app:exe:release`);
}

if (DRY) {
  console.log("\n--dry：未写盘。");
  process.exit(0);
}

// ── 拷贝 ──
if (!existsSync(OUT_ROOT)) {
  die(`成品根目录不存在：${OUT_ROOT}`, "E 盘没挂载？或路径已变更，改本脚本顶部的 OUT_ROOT");
}
mkdirSync(outDir, { recursive: true });
copyFileSync(srcExe, outExe);
copyFileSync(srcTxt, outTxt);

// ── 逐字节校验：手动拷贝拷错/拷成旧版都不会报错，这里必须自证 ──
const a = sha1(srcExe), b = sha1(outExe);
if (a !== b) die(`exe 拷贝后 sha1 不一致（源 ${a.slice(0, 12)} vs 成品 ${b.slice(0, 12)}）`,
  "磁盘写入异常或目标文件被占用，重跑一次");
const ta = sha1(srcTxt), tb = sha1(outTxt);
if (ta !== tb) die("使用说明.txt 拷贝后 sha1 不一致");

console.log(`\n✓ exe    ${mb(statSync(outExe).size)}  sha1=${b.slice(0, 12)}（与源一致）`);
console.log(`✓ 说明   ${statSync(outTxt).size}B  sha1=${tb.slice(0, 12)}`);

// ── 可选 zip ──
// 用 PowerShell 的 Compress-Archive，不用 Windows 自带的 tar.exe。三个实测理由：
//   ① 编码。包里有「使用说明.txt」，bsdtar 按系统 ANSI 代码页（中文机上是 GBK）写
//      文件名且不置 UTF-8 标志位（general purpose bit 11），别人在非中文 locale
//      解压会拿到乱码文件名。Compress-Archive 走 .NET ZipFile，写 UTF-8 且置位
//      —— 实测中央目录里是 e4bdbf...（UTF-8）+ flags 0x0800。
//   ② 裸写 "tar" 在 Git Bash 下会解析到 GNU tar，而 GNU tar 造不出 zip。
//   ③ tar 把含盘符冒号的参数当远程主机（`E:\x.zip` → "Cannot connect to E:"）。
// 命令里不出现中文字面量，用通配符规避 CJK 参数在 shell 间的转码问题；
// cwd 设成 outDir，参数全相对，顺带也绕开冒号。
if (WANT_ZIP) {
  const zipName = `${stem}.zip`;
  const zip = join(outDir, zipName);
  const r = spawnSync("powershell", ["-NoProfile", "-Command",
    `Compress-Archive -Path '.\\*.exe','.\\*.txt' -DestinationPath '.\\${zipName}' -Force`],
    { cwd: outDir, encoding: "utf8" });
  if (r.status === 0 && existsSync(zip)) {
    console.log(`✓ zip    ${mb(statSync(zip).size)}  ${zipName}（文件名 UTF-8）`);
  } else {
    console.log(`⚠ zip 失败（${(r.stderr || r.error?.message || "未知").trim().slice(0, 120)}）`);
    console.log(`  exe 和说明已就位，手动压缩即可`);
  }
}

console.log(`\n成品目录：${outDir}`);
console.log(`发 GitHub Release：Releases → Draft a new release → 把上面的文件拖到 Attach binaries`);
console.log(`（gh CLI 本机未装；要用命令行发得先 winget install GitHub.cli）\n`);
