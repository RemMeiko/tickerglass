// 把 Rust crate 名从 stockpet-plus / stockpet_plus_lib 改成 tickerglass / tickerglass_lib。
//
// 为什么之前跳过这一步：它牵连 25 个测试文件的 import 路径，纯机械但面广，
// 手改必漏。现在用户要上传 GitHub、要求名字彻底改掉，所以用脚本一次性做完。
//
// 影响面（三类，缺一不可）：
// ① Cargo.toml 的 [package] name —— 决定 **exe 文件名**
// ② Cargo.toml 的 [lib] name     —— 决定测试里的 import 路径
// ③ 25 个 .rs 文件里的 stockpet_plus_lib::  —— 集成测试引用 lib
//
// 另外 .cargo/config.toml 的 target-dir 也一起改：它指向 D:/stockpet-plus-build，
// 留着旧名会让产物目录与项目名不一致，交接时容易找错地方。

import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const REPL = [
  // 顺序重要：先替换更长的 lib 名，否则 "stockpet-plus" 会先把
  // "stockpet_plus_lib" 里的部分吃掉（虽然分隔符不同，但保险起见）
  ["stockpet_plus_lib", "tickerglass_lib"],
  ["stockpet-plus", "tickerglass"],
  ["stockpet_plus", "tickerglass"],
];

/** 递归收集要处理的文件，跳过构建产物与依赖 */
function collect(dir, out = []) {
  for (const name of readdirSync(dir)) {
    if (["target", "node_modules", "dist", ".git", "__pycache__"].includes(name)) continue;
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) collect(p, out);
    else if (/\.(rs|toml|json)$/.test(name)) out.push(p);
  }
  return out;
}

const files = collect("src-tauri");
// 根目录的 package.json 也可能提到，一并纳入
files.push("package.json");

let touched = 0;
let total = 0;
for (const f of files) {
  let s;
  try {
    s = readFileSync(f, "utf8");
  } catch {
    continue;
  }
  const before = s;
  let n = 0;
  for (const [from, to] of REPL) {
    const parts = s.split(from);
    if (parts.length > 1) {
      n += parts.length - 1;
      s = parts.join(to);
    }
  }
  if (s !== before) {
    writeFileSync(f, s, "utf8");
    console.log(`${f}: ${n} 处`);
    touched++;
    total += n;
  }
}
console.log(`\n共 ${touched} 个文件、${total} 处替换`);
