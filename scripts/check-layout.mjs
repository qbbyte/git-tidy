// 提交页那几个宽度：CSS 里写的与 lib/layout.ts 里算的必须一致。
//
// 为什么值得一个脚本：CSS 没法 import 常量，所以同一批数字必然有两份。它们一旦走偏，
// 症状是「某个窗口宽度下刚好溢出、多出一条横向滚动条」——很难联想到是宽度预算的问题，
// 而这类偏差在界面上只是"看着有点挤"，不报错、不留痕。
//
// 跑法：npm run check:layout
import { readFileSync } from "node:fs";

const { COMMIT_LAYOUT, DIVIDER_TOTAL, COMMIT_THREE_PANE_MIN } = await import(
  "../src/lib/layout.ts"
);

let failed = 0;

function check(name, actual, expected) {
  if (actual !== expected) {
    failed += 1;
    console.log(`FAIL ${name}\n  实际 ${actual}\n  期望 ${expected}`);
  } else {
    console.log(`ok   ${name}`);
  }
}

/** 取出 SFC 里 `<style>` 的内容，并去掉注释——注释里的选择器不该被匹配到 */
function styleOf(file) {
  const src = readFileSync(file, "utf8");
  const blocks = [...src.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)];
  if (blocks.length === 0) throw new Error(`${file} 里没有 <style>`);
  return blocks.map((m) => m[1].replace(/\/\*[\s\S]*?\*\//g, "")).join("\n");
}

/**
 * 按**精确选择器**取一条规则的声明体。
 *
 * 选择器必须紧跟在 `}` 或样式开头之后、紧挨着 `{`——所以 `.divider` 不会匹配到
 * `.divider::after`，`.preview-pane` 也不会匹配到 `.pane.narrow .preview-pane`。
 * 找不到就抛错：改名之后悄悄跳过检查，比不检查更糟。
 */
function rule(css, selector, where) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const found = css.match(new RegExp(`(?:^|\\})\\s*${escaped}\\s*\\{([^}]*)\\}`));
  if (!found) throw new Error(`${where} 里找不到规则 \`${selector}\``);
  return found[1];
}

/** 取一条声明的值。`flex` 不会匹配到 `flex-direction` */
function decl(body, prop) {
  const found = body.match(new RegExp(`(?:^|;)\\s*${prop}\\s*:\\s*([^;]+)`));
  if (!found) throw new Error(`找不到声明 \`${prop}\`，规则体是：${body.trim()}`);
  return found[1].trim();
}

/** `12px` → 12（裸 `0` 也是合法长度）。`1rem` 这类直接抛错，免得被当成 1 参与算术 */
function px(value) {
  const found = value.trim().match(/^(\d+(?:\.\d+)?)(px)?$/);
  if (!found || (found[2] === undefined && Number(found[1]) !== 0)) {
    throw new Error(`只认 px（或裸 0），拿到的是 \`${value}\``);
  }
  return Number(found[1]);
}

/** padding / margin 简写的左右之和 */
function horizontalTotal(value) {
  const parts = value.trim().split(/\s+/).map(px);
  if (parts.length === 1) return parts[0] * 2;
  if (parts.length === 4) return parts[1] + parts[3];
  return parts[1] * 2; // 两值与三值简写的左右都是第二段
}

/** `0 1 430px` 的第三段 */
function thirdOf(value) {
  const parts = value.trim().split(/\s+/);
  if (parts.length !== 3) throw new Error(`期望三段式 flex，拿到的是 \`${value}\``);
  return px(parts[2]);
}

const commit = styleOf("src/views/CommitView.vue");
const app = styleOf("src/App.vue");
const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const windowMinWidth = conf.app.windows[0].minWidth;

// ---- 提交页：CSS 与常量逐个对齐 ----

check(
  "分隔条本体 = COMMIT_LAYOUT.dividerWidth",
  px(decl(rule(commit, ".divider", "CommitView.vue"), "width")),
  COMMIT_LAYOUT.dividerWidth,
);
check(
  "分隔条左右外边距之和 = dividerMargin × 2",
  horizontalTotal(decl(rule(commit, ".divider", "CommitView.vue"), "margin")),
  COMMIT_LAYOUT.dividerMargin * 2,
);
check(
  "三栏差异栏下限 = COMMIT_LAYOUT.diffMin",
  px(decl(rule(commit, ".pane:not(.narrow) .preview-pane", "CommitView.vue"), "min-width")),
  COMMIT_LAYOUT.diffMin,
);
check(
  "窄布局差异栏下限 = COMMIT_LAYOUT.diffMinNarrow",
  px(decl(rule(commit, ".pane.narrow .preview-pane", "CommitView.vue"), "min-width")),
  COMMIT_LAYOUT.diffMinNarrow,
);

const form = rule(commit, ".form-column", "CommitView.vue");
const cssFormMin = px(decl(form, "min-width"));
check("表单栏下限 = COMMIT_LAYOUT.formMin", cssFormMin, COMMIT_LAYOUT.formMin);
check("表单栏首选宽度 = COMMIT_LAYOUT.formBasis", thirdOf(decl(form, "flex")), COMMIT_LAYOUT.formBasis);

// ---- 阈值：必须是加出来的，不是手写的 ----
check(
  "COMMIT_THREE_PANE_MIN = listMax + 分隔条 + diffMin + formMin",
  COMMIT_THREE_PANE_MIN,
  COMMIT_LAYOUT.listMax + DIVIDER_TOTAL + COMMIT_LAYOUT.diffMin + COMMIT_LAYOUT.formMin,
);

// ---- 外壳：n-layout 管侧栏与主区，不再手写宽度预算 ----
//
// 改用 n-layout-sider 后，侧栏宽度（280px）与折叠态（0px）由 NaiveUI 组件管理，
// 「最小窗口装不装得下」由 n-layout 的响应式行为兜底（窗口窄时侧栏可折叠收起）。
// 这里只保留 .content 内边距的一致性检查。
const contentPadding = horizontalTotal(decl(rule(app, ".content", "App.vue"), "padding"));
check("内容区左右内边距 = 32px（16px × 2）", contentPadding, 32);

console.log(
  [
    "",
    "提交页宽度预算：",
    `  三栏阈值 ${COMMIT_THREE_PANE_MIN}  = 清单上限 ${COMMIT_LAYOUT.listMax} + 分隔条 ${DIVIDER_TOTAL} + 差异栏 ${COMMIT_LAYOUT.diffMin} + 表单栏 ${COMMIT_LAYOUT.formMin}`,
    `  外壳由 n-layout 管理，侧栏可折叠（280px ↔ 0px）`,
  ].join("\n"),
);

console.log(failed === 0 ? "\n全部通过" : `\n${failed} 条失败`);
process.exit(failed === 0 ? 0 : 1);
