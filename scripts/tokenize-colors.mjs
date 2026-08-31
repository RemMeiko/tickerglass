// styles.css / chart.css 的硬编码色值 → 语义 token。
//
// 每个 token 出两种形态：
//   --c-x      : #rrggbb        实色直接用
//   --c-x-rgb  : r, g, b        供 alpha 变体用 rgba(var(--c-x-rgb), .7)
//
// 为什么必须有第二种：本项目有 33 处是"已 token 化基色 + 不同 alpha"。
// 只换实色的话，浅色主题下 --c-text 翻成深色，而它的 70% 变体仍是硬编码的浅色，
// 在白底上直接消失。三元组形态让 alpha 变体跟着主题走。
//
// 本步只做机械替换，**色值不变**。观感不变由 css-color-snapshot.mjs 前后对比证明。
// 唯一预期 diff：chart.css:261 的 rgba(232,180,76) 统一到 232,182,76（原手误，差 2/255）。

import fs from 'fs';
import path from 'path';

const DRY = process.argv.includes('--dry');
const SRC = process.argv[2] || 'src';
const FILES = ['styles.css', 'chart.css'];

/** token 名 → [r, g, b]。只收 styles.css + chart.css 代码区真实出现的色。 */
export const PALETTE = {
  // 文字层级（由亮到暗）
  text:          [232, 234, 237],
  'text-strong': [241, 243, 244],
  'text-2':      [196, 200, 204],
  'text-3':      [154, 160, 166],
  'text-4':      [107, 112, 117],
  // 背景层级（由深到浅）
  'bg-sunken':   [22, 22, 22],
  bg:            [31, 31, 31],
  'bg-elev':     [38, 38, 38],
  'bg-elev-2':   [38, 38, 42],
  'bg-toast':    [35, 35, 35],
  // 涨跌
  up:            [245, 72, 74],
  'up-soft':     [229, 115, 115],
  // 强调
  accent:        [76, 154, 255],
  'accent-text': [207, 224, 255],
  'accent-chart': [91, 155, 213],
  // 警告/过期
  warn:          [230, 162, 60],
  'warn-soft':   [240, 192, 120],
  'warn-2':      [232, 182, 76],
  // 中性基色（overlay / shadow 的 alpha 变体基底）
  white:         [255, 255, 255],
  black:         [0, 0, 0],
};

/** 手误归并：文件里的 232,180,76 视为 warn-2(232,182,76)。见文件头说明。 */
const ALIAS_RGB = { '232,180,76': 'warn-2' };

const hx = (n) => n.toString(16).padStart(2, '0');
const toHex = ([r, g, b]) => `#${hx(r)}${hx(g)}${hx(b)}`;

/** hex（小写，6 位）→ token 名 */
const HEX2TOKEN = new Map(
  Object.entries(PALETTE).map(([k, v]) => [toHex(v), k])
);
/** "r,g,b" → token 名（含手误别名） */
const RGB2TOKEN = new Map([
  ...Object.entries(PALETTE).map(([k, v]) => [v.join(','), k]),
  ...Object.entries(ALIAS_RGB).map(([rgb, k]) => [rgb, k]),
]);

const counts = {};
const bump = (k) => (counts[k] = (counts[k] || 0) + 1);

/**
 * 只替换注释外的内容。
 *
 * 必要性：注释里大量写着色值（我自己上一轮去蓝时就写了「原为 rgba(18,22,30)」），
 * 替换进去会把说明文字改成 var(...)，读起来就不知所云了。
 */
function mapOutsideComments(text, fn) {
  let out = '';
  let i = 0;
  while (i < text.length) {
    const start = text.indexOf('/*', i);
    if (start === -1) {
      out += fn(text.slice(i));
      break;
    }
    out += fn(text.slice(i, start));
    const end = text.indexOf('*/', start + 2);
    if (end === -1) {
      out += text.slice(start); // 未闭合注释，原样留
      break;
    }
    out += text.slice(start, end + 2);
    i = end + 2;
  }
  return out;
}

function tokenize(chunk) {
  // ① 先处理 rgba(r,g,b,a) —— 必须在实色之前，否则 hex 替换会先把里面拆坏
  chunk = chunk.replace(
    /rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*([0-9.]+)\s*\)/g,
    (m, r, g, b, a) => {
      const tok = RGB2TOKEN.get([r, g, b].join(','));
      if (!tok) return m;
      bump(`${tok}-rgb`);
      return `rgba(var(--c-${tok}-rgb), ${a})`;
    }
  );
  // ② 3 位 hex 先展开成 6 位（#fff → #ffffff），否则漏替
  chunk = chunk.replace(/#([0-9a-fA-F])([0-9a-fA-F])([0-9a-fA-F])\b/g,
    (m, a, b, c) => `#${a}${a}${b}${b}${c}${c}`);
  // ③ 实色
  chunk = chunk.replace(/#[0-9a-fA-F]{6}\b/g, (m) => {
    const tok = HEX2TOKEN.get(m.toLowerCase());
    if (!tok) return m;
    bump(tok);
    return `var(--c-${tok})`;
  });
  return chunk;
}

let touched = 0;
for (const f of FILES) {
  const p = path.join(SRC, f);
  const before = fs.readFileSync(p, 'utf8');
  const after = mapOutsideComments(before, tokenize);
  if (before !== after) {
    touched++;
    if (!DRY) {
      fs.writeFileSync(p + '.bak', before);
      fs.writeFileSync(p, after);
    }
  }
  const n = (after.match(/var\(--c-/g) || []).length;
  console.log(`${f}: var(--c-) = ${n}`);
}

const total = Object.values(counts).reduce((a, b) => a + b, 0);
const alpha = Object.entries(counts)
  .filter(([k]) => k.endsWith('-rgb'))
  .reduce((a, [, v]) => a + v, 0);

console.log('---');
for (const [k, v] of Object.entries(counts).sort((a, b) => b[1] - a[1])) {
  console.log(`  ${k}: ${v}`);
}
console.log('---');
console.log(`TOTAL=${total}  实色=${total - alpha}  alpha变体=${alpha}`);
console.log(`FILES_TOUCHED=${touched}  MODE=${DRY ? 'DRY(未写盘)' : '已写盘(.bak 已留)'}`);

// :root 定义块单独出文件 —— 必须在替换**之后**再插进 styles.css，
// 否则这一轮替换会把自己的定义也换成 var(--c-text: var(--c-text))。
const lines = ['/* ── 语义色 token（本块由 scripts/tokenize-colors.mjs 生成）', ' *',
  ' * 每色两种形态：--c-x 实色，--c-x-rgb 三元组（供 rgba(var(--c-x-rgb), .7) 用）。',
  ' * 浅色主题只需在 [data-theme="light"] 里覆盖这些值，不必改任何规则。 */',
  ':root {'];
for (const [k, v] of Object.entries(PALETTE)) {
  lines.push(`  --c-${k}: ${toHex(v)};`);
  lines.push(`  --c-${k}-rgb: ${v.join(', ')};`);
}
lines.push('}');
fs.writeFileSync(path.join(SRC, '..', 'root-tokens.txt'), lines.join('\n') + '\n');
console.log('ROOT_TOKENS_WRITTEN=root-tokens.txt');

