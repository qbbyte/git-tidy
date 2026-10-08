// token 漂移检查。
//
// 两头都要卡：
// - 用了 var(--x) 但 tokens.ts 里没有 x → 那个值解析不出来，颜色静默失效
//   （不报错，只是那一处不生效，是最难查的一类 bug）
// - tokens.ts 里定义了 x 但没人用 → 多半是拼错了名字，留着只会误导下一个人
//
// 跑法：npm run check:tokens
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

// 变量名转换规则从 tokens.ts 里拿，不在这里重写一遍：
// 两份实现早晚会走偏，而走偏的后果是颜色静默失效，不是报错。
const { tokens, toKebab } = await import("../src/styles/tokens.ts");
const tokensSrc = readFileSync("src/styles/tokens.ts", "utf8");

/** 从 tokens.ts 里抠出 `key: "#rrggbb"` 形式的定义 */
const defined = new Set();
for (const m of tokensSrc.matchAll(/^\s{2}(\w+):\s*"#[0-9a-fA-F]{3,8}",/gm)) {
  defined.add(m[1]);
}

const declared = new Set([...defined].map(toKebab));

function walk(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) out.push(...walk(full));
    else if (/\.(vue|ts|css)$/.test(full)) out.push(full);
  }
  return out;
}

const used = new Map(); // 变量名 -> 首个用到的位置
for (const file of walk("src")) {
  if (file.endsWith("tokens.ts")) continue;
  const src = readFileSync(file, "utf8");
  src.split("\n").forEach((line, i) => {
    // 两种消费方式都算「在用」：样式里的 var(--x)，以及 Naive 主题里的 tokens.x。
    // 只认真正是 token 键的属性名：`tokens.ts` 这种文件名里的 `ts` 不是键。
    for (const m of line.matchAll(/var\(--([a-z0-9-]+)/g)) {
      if (!used.has(m[1])) used.set(m[1], `${file}:${i + 1}`);
    }
    for (const m of line.matchAll(/\btokens\.(\w+)/g)) {
      if (!defined.has(m[1])) continue;
      const name = toKebab(m[1]);
      if (!used.has(name)) used.set(name, `${file}:${i + 1}  (tokens.${m[1]})`);
    }
  });
}

const unknown = [...used.keys()].filter((name) => !declared.has(name));
const unused = [...declared].filter((name) => !used.has(name));

let failed = 0;

// 值本身也要合法。一个写错的色值会让那一处静默失效，而且构建查不出来。
const badName = Object.keys(tokens)
  .map(toKebab)
  .filter((name) => !/^[a-z][a-z0-9-]*$/.test(name));
const badColor = Object.entries(tokens)
  .filter(([, value]) => !/^#[0-9a-fA-F]{3}([0-9a-fA-F]{3})?$/.test(value))
  .map(([key, value]) => `${key}=${value}`);

if (badName.length) {
  failed += 1;
  console.error(`\n不合法的变量名（CSS 会忽略）:\n  ${badName.join("\n  ")}`);
}
if (badColor.length) {
  failed += 1;
  console.error(`\n不合法的色值:\n  ${badColor.join("\n  ")}`);
}

if (unknown.length) {
  failed += 1;
  console.error(`\n用了未定义的 token（这些颜色不会生效）：`);
  for (const name of unknown) console.error(`  --${name}   ${used.get(name)}`);
}

if (unused.length) {
  failed += 1;
  console.error(`\n定义了但没人用（多半是拼错了名字）：`);
  for (const name of unused) console.error(`  --${name}`);
}

if (failed === 0) {
  console.log(`token 表一致：${declared.size} 个定义，${used.size} 个在用`);
}
process.exit(failed === 0 ? 0 : 1);