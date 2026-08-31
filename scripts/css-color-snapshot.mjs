// 抽取**构建产物**里所有落地颜色，输出成可 diff 的扁平快照。
//
// 用途：证明「抽 token」这类重构没有改变观感。抽 token 前后各跑一次，
// 两份快照完全相同 = 每条规则的最终颜色一模一样。
//
// 为什么读 dist 而不是 src：var() 只有在层叠解析后才有确定值。
// 读源码只能看到 `var(--c-text)` 这种字面量，看不出它最终是什么颜色 ——
// 而"最终是什么颜色"恰恰是本脚本唯一关心的事。
//
// 输出格式（每行一条，已排序）：
//   "bundle|selector|prop|#rrggbb[aa]"
// alpha 折进 hex 尾部，空白全剥，便于逐字节 diff。

import fs from 'fs';
import path from 'path';

const DIST = process.argv[2] || 'dist/assets';
const OUT = process.argv[3] || 'snap.json';

/** 只看这些属性 —— 其余属性不含颜色，收进来只是噪声。 */
const COLOR_PROPS = new Set([
  'color', 'background', 'background-color', 'border', 'border-color',
  'border-top', 'border-bottom', 'border-left', 'border-right',
  'border-top-color', 'border-bottom-color',
  'border-left-color', 'border-right-color',
  'outline', 'outline-color', 'box-shadow', 'text-shadow',
  'fill', 'stroke', 'caret-color', 'accent-color', 'column-rule-color',
  '-webkit-text-fill-color',
]);

const hx = (n) => Math.round(n).toString(16).padStart(2, '0');

/** rgb()/rgba() → #rrggbb[aa]（alpha=1 时不带尾字节，与 hex 写法对齐） */
function rgbToHex(r, g, b, a) {
  let s = `#${hx(r)}${hx(g)}${hx(b)}`;
  if (a !== undefined && a !== null && Number(a) < 1) {
    s += hx(Number(a) * 255);
  }
  return s;
}

/** 把一段值里的所有颜色写法归一。非颜色部分原样留（如 `1px solid`）。 */
function normalizeColors(v) {
  let s = v;
  // rgb()/rgba()
  s = s.replace(
    /rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)/gi,
    (m, r, g, b, a) => rgbToHex(+r, +g, +b, a)
  );
  // 现代空格语法 rgb(R G B / A)
  s = s.replace(
    /rgba?\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*(?:\/\s*([\d.]+%?)\s*)?\)/gi,
    (m, r, g, b, a) => {
      let al = a;
      if (al && al.endsWith('%')) al = String(parseFloat(al) / 100);
      return rgbToHex(+r, +g, +b, al);
    }
  );
  // 3 位 hex → 6 位
  s = s.replace(/#([0-9a-f])([0-9a-f])([0-9a-f])\b/gi,
    (m, a, b, c) => `#${a}${a}${b}${b}${c}${c}`);
  return s.toLowerCase();
}

/** 收集 :root 等处定义的自定义属性，供 var() 解析用。 */
function collectVars(css) {
  const vars = new Map();
  for (const m of css.matchAll(/(--[\w-]+)\s*:\s*([^;}]+)/g)) {
    // 后定义覆盖先定义（近似层叠；本项目 token 只在 :root 定义一次）
    vars.set(m[1], m[2].trim());
  }
  return vars;
}

/** 递归展开 var()，带深度上限防自引用死循环。 */
function resolveVars(value, vars, depth = 0) {
  if (depth > 12 || !value.includes('var(')) return value;
  const next = value.replace(
    /var\(\s*(--[\w-]+)\s*(?:,\s*([^)]*))?\)/g,
    (m, name, fallback) => {
      if (vars.has(name)) return vars.get(name);
      return fallback !== undefined ? fallback : m;
    }
  );
  if (next === value) return next;
  return resolveVars(next, vars, depth + 1);
}

const results = new Set();
let unresolved = 0;

const files = fs.readdirSync(DIST).filter((f) => f.endsWith('.css'));

/**
 * 变量表必须**跨 bundle 全局收集**。
 *
 * vite 按入口切 bundle：`:root` 写在 styles.css 里，被打进 macos bundle；
 * 而 chart.css 是独立 bundle，里面 38 处 `var(--c-*)` 引用了那些 token
 * 但自身不含定义。运行时 chart.tsx 同时 import 两个文件，浏览器在同一个
 * 文档里解析，所以拿得到值 —— 按单文件收集就会假报 38 个未解析。
 */
const globalVars = new Map();
for (const file of files) {
  const css = fs.readFileSync(path.join(DIST, file), 'utf8');
  for (const [k, v] of collectVars(css)) {
    if (!globalVars.has(k)) globalVars.set(k, v);
  }
}

for (const file of files) {
  const css = fs.readFileSync(path.join(DIST, file), 'utf8');
  // bundle 名去掉 vite 的内容哈希（macos-Dsz-xNER.css → macos）
  const bundle = file.replace(/-[A-Za-z0-9_-]{8}\.css$/, '').replace(/\.css$/, '');
  // 本文件定义优先，缺的回落到全局表
  const vars = new Map([...globalVars, ...collectVars(css)]);

  // 剥注释，避免注释里的色值被当规则
  const clean = css.replace(/\/\*[\s\S]*?\*\//g, '');

  // 逐个规则块。选择器可能含逗号，拆开各记一条（与基线一致）
  for (const m of clean.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const selRaw = m[1].trim();
    const body = m[2];
    // 跳过 @规则的头部（@media/@keyframes 等），它们的内层块会被单独匹配到
    if (selRaw.startsWith('@')) continue;

    for (const decl of body.split(';')) {
      const idx = decl.indexOf(':');
      if (idx === -1) continue;
      const prop = decl.slice(0, idx).trim().toLowerCase();
      if (!COLOR_PROPS.has(prop)) continue;

      let val = decl.slice(idx + 1).trim();
      val = resolveVars(val, vars);
      if (val.includes('var(')) unresolved++;
      val = normalizeColors(val).replace(/\s+/g, '');
      if (!val) continue;

      for (const sel of selRaw.split(',')) {
        const s = sel.trim().replace(/\s+/g, ' ');
        if (!s) continue;
        results.add(`${bundle}|${s}|${prop}|${val}`);
      }
    }
  }
}

const sorted = [...results].sort();
fs.writeFileSync(OUT, JSON.stringify(sorted, null, 1) + '\n');
console.log(`TRIPLES=${sorted.length}`);
console.log(`UNRESOLVED_VAR=${unresolved}`);
console.log(`BUNDLES=${files.length}`);
console.log(`OUT=${OUT}`);
