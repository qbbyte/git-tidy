/**
 * 快捷键的**唯一定义处**（需求 7.23）。
 *
 * 同一个数组既驱动按键绑定，也渲染设置页里的说明表——所以说明不可能过期：
 * 删掉一条快捷键，说明表里那一行跟着消失；新增一条，绑定与说明同时出现。
 * 单独维护一张"文档表"是这类功能最常见的失效方式：功能改了、表忘了改，
 * 面板就开始骗人。
 *
 * `keys` 用一种接近人写的写法（`Ctrl+Shift+P`），`normalize` 负责翻译成
 * `KeyboardEvent` 的字段比较，中间不引入第二套映射表。
 */
export interface ShortcutContext {
  /** F5：重读当前页关心的数据 */
  refresh(): void;
  /** Ctrl+F：聚焦当前页的搜索框 */
  focusSearch(): void;
  /** Ctrl+Enter：在提交页提交 */
  submit(): void;
  /** Ctrl+Shift+P：聚焦仓库切换框 */
  switchRepo(): void;
  /** 数字键 1..5：直达主区页签 */
  gotoTab(index: number): void;
  /** Ctrl+,：打开设置（设置不在页签里，入口是左下角的齿轮） */
  openSettings(): void;
}

export interface Shortcut {
  id: string;
  keys: string[];
  label: string;
  /** 说明表里的分组标题 */
  group: string;
  /** 什么条件下生效，写给人看，也约束实现 */
  when: string;
  run(context: ShortcutContext): void;
}

export const SHORTCUTS: Shortcut[] = [
  {
    id: "refresh",
    keys: ["F5"],
    label: "刷新",
    group: "全局",
    when: "任何页面",
    run: (ctx) => ctx.refresh(),
  },
  {
    id: "focus-search",
    keys: ["Ctrl+F"],
    label: "在当前列表里搜索",
    group: "全局",
    when: "历史 / 文件页；输入框内不劫持",
    run: (ctx) => ctx.focusSearch(),
  },
  {
    id: "submit",
    keys: ["Ctrl+Enter"],
    label: "提交暂存区里的改动",
    group: "全局",
    when: "提交页，且当前草稿没有拦截级问题",
    run: (ctx) => ctx.submit(),
  },
  {
    id: "switch-repo",
    keys: ["Ctrl+Shift+P"],
    label: "切换仓库",
    group: "全局",
    when: "任何页面",
    run: (ctx) => ctx.switchRepo(),
  },
  {
    id: "settings",
    keys: ["Ctrl+,"],
    label: "打开设置",
    group: "全局",
    when: "任何页面（设置入口是左下角的齿轮）",
    run: (ctx) => ctx.openSettings(),
  },
];

/**
 * 页签直达：1..5 与主区页签顺序一致，顺序变了这里要跟着变。
 *
 * **设置不在这张表里**：它不是页签，是左下角齿轮打开的一个独立视图
 * （需求 7.23 的页签清单里有“设置”，但把它放在页签里会让一个一年可能点不到两次的
 * 设置页占住导航位；齿轮在左下角固定位置，既不占页签又始终可被发现）。
 */
export const TAB_SHORTCUTS: { key: string; label: string }[] = [
  { key: "1", label: "历史" },
  { key: "2", label: "文件" },
  { key: "3", label: "提交" },
  { key: "4", label: "报告" },
  { key: "5", label: "CHANGELOG" },
];

/** `Ctrl+Shift+P` → 比较用的形状。小写化，避免写两遍 Shift。 */
function normalize(combo: string): string[] {
  const parts = combo.split("+").map((part) => part.trim().toLowerCase());
  return parts;
}

export function comboOf(event: KeyboardEvent): string {
  const parts: string[] = [];
  if (event.ctrlKey || event.metaKey) parts.push("ctrl");
  if (event.shiftKey) parts.push("shift");
  if (event.altKey) parts.push("alt");
  parts.push(event.key.toLowerCase());
  return parts.join("+");
}

/**
 * 事件是不是这条快捷键。数字键与字母键用 `event.key` 比，
 * 所以 `Ctrl+1` 与 `1` 天然区分得开。
 */
export function matches(event: KeyboardEvent, combo: string): boolean {
  const wanted = normalize(combo);
  if (event.ctrlKey || event.metaKey) {
    if (!wanted.includes("ctrl")) return false;
  } else if (wanted.includes("ctrl")) {
    return false;
  }
  if (event.shiftKey !== wanted.includes("shift")) return false;
  if (event.altKey !== wanted.includes("alt")) return false;
  return event.key.toLowerCase() === wanted[wanted.length - 1];
}

/** 输入框里不劫持按键：用户在打字，不是在按快捷键 */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target.tagName === "INPUT" ||
    target.tagName === "TEXTAREA" ||
    target.tagName === "SELECT"
  );
}

/** 说明表用：把一组键渲染成 `Ctrl+Shift+P` 这样的字符串 */
export function formatKeys(keys: string[]): string {
  return keys.join(" / ");
}