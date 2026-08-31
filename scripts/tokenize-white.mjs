// 把 macos.css 里硬编码的 rgba(255,255,255,α) 换成 rgba(var(--c-white-rgb),α)。
//
// 为什么要这么做：浅色主题靠 --c-white / --c-black 互换来适配所有
// "白色低透明度"元素（分隔线、内高光、hover 底）。硬编码的值不参与互换，
// 于是浅色下它们仍是白的、在白底上完全看不见 —— 用户反复报
// "浅色只换了背景""滚动条还是白色"，根因都在这里。
//
// **必须排除已在 [data-theme="light"] 块内的行**：那些是浅色专用规则，
// 里面的白就该是白（比如浅色卡片底 rgba(255,255,255,.72)），
// 换成 token 会被互换成黑，直接翻反。
//
// 用脚本而不是手改：43 处手改必然漏，且漏掉的那处只有用户看到才知道。

import { readFileSync, writeFileSync } from "node:fs";

const FILE = "src/macos.css";
const src = readFileSync(FILE, "utf8");
const lines = src.split("\n");

// 扫描出所有 [data-theme="light"] 规则块的行号区间
const lightRanges = [];
for (let i = 0; i < lines.length; i++) {
  if (!lines[i].includes('data-theme="light"')) continue;
  // 从这一行往下找到配对的 }
  let depth = 0;
  let started = false;
  for (let j = i; j < lines.length; j++) {
    for (const ch of lines[j]) {
      if (ch === "{") {
        depth++;
        started = true;
      } else if (ch === "}") depth--;
    }
    if (started && depth === 0) {
      lightRanges.push([i, j]);
      break;
    }
  }
}

const inLight = (n) => lightRanges.some(([a, b]) => n >= a && n <= b);

const PAT = /rgba\(255,\s*255,\s*255,\s*([0-9.]+)\)/g;
let changed = 0;
let skipped = 0;

const out = lines.map((line, n) => {
  if (!PAT.test(line)) return line;
  PAT.lastIndex = 0;
  if (inLight(n)) {
    skipped++;
    return line;
  }
  return line.replace(PAT, (_, a) => {
    changed++;
    return `rgba(var(--c-white-rgb), ${a})`;
  });
});

writeFileSync(FILE, out.join("\n"), "utf8");

console.log(`浅色块区间: ${lightRanges.length} 个`);
console.log(`已替换: ${changed} 处`);
console.log(`跳过(浅色块内): ${skipped} 行`);
