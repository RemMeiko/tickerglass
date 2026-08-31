// 把用户可见的产品名 "StockPet Plus" 改成新名字。
//
// 背景：本项目最初参考 GitHub 的 StockPet（MIT）扒接口用法，命名沿用了
// "StockPet Plus"。但实现早已完全分叉（Tauri vs Electron、双数据源、
// 提醒引擎、分页、主题系统都是自己的），继续叫 Plus 会让人误以为是那个项目的
// 增强版。改名以示区分。
//
// **只改用户可见字符串**，两处故意保留：
// ① Rust crate 名 stockpet_plus_lib —— 改它要动 70 个测试文件的 import，
//    纯机械风险、零用户收益
// ② 配置目录 %APPDATA%/StockPetPlus —— 改它会让用户现有配置全部孤立
//    （自选股、提醒规则、自定义音频都在里面）
// 这两个用户都看不到。

import { readFileSync, writeFileSync } from "node:fs";

const OLD = "StockPet Plus";
const NEW = "TickerGlass";

// 参考项目的引用要单独处理：那是出处说明，不该改成新名字，
// 但要说清"参考的是它、实现是我们自己的"
const FILES = [
  "src-tauri/tauri.conf.json",
  "package.json",
  "index.html",
  "settings.html",
  "chart.html",
  "market.html",
  "popup.html",
  "src-tauri/src/tray.rs",
  "src-tauri/src/window.rs",
  "src/components/settings/AboutPanel.tsx",
];

let total = 0;
for (const f of FILES) {
  let s;
  try {
    s = readFileSync(f, "utf8");
  } catch {
    console.log(`跳过(不存在): ${f}`);
    continue;
  }
  const before = s;
  s = s.split(OLD).join(NEW);
  if (s !== before) {
    const n = before.split(OLD).length - 1;
    writeFileSync(f, s, "utf8");
    console.log(`${f}: ${n} 处`);
    total += n;
  }
}
console.log(`合计替换 ${total} 处`);
