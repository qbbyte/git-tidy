<script setup lang="ts">
import { computed, reactive, watch } from "vue";
import { NEmpty, NSpin } from "naive-ui";
import type { TreeEntry } from "@/api/file";
import { formatBytes } from "@/format";

/**
 * 某个修订的文件树（§7.6）。
 *
 * Rust 侧给的是**扁平列表**（`ls-tree -r -l -z` 的原样输出，按路径排序），树在这里现折，
 * 只做两件事：按 `/` 分段挂到父目录下、每层排一次序。不在 Rust 侧拼好再发——那份数据还要
 * 再序列化一遍，而界面真正要的只是"挂在谁下面、缩进几格"。
 *
 * 默认**全展开**：改造前就是一屏全列出来，所以收起集合 `collapsed` 是"反着存"的——
 * 什么都不知道时与旧行为一致，不会因为多出折叠反而让人少看见东西。
 */
const props = defineProps<{
  entries: TreeEntry[];
  loading: boolean;
  selected: string | null;
}>();

const emit = defineEmits<{ pick: [TreeEntry] }>();

/** 每层缩进多少像素 */
const STEP = 12;
/** 最外层也要留的一点左内边距，别让引导线贴着面板边 */
const BASE = 6;

interface DirNode {
  type: "dir";
  /** 目录名本身（不含父路径）。根节点是空串且不渲染 */
  name: string;
  /** 目录全路径，同时当 key 与折叠集合的键 */
  path: string;
  depth: number;
  children: Node[];
}

/** 一行文件前面那个记号：要么是彩色字母，要么是矢量图形，要么是一页空白 */
interface FileIcon {
  /** 上色用的 class（`t-js` / `t-vue`…）。认不出来的留空 ⇒ 落到三级灰 */
  cls: string;
  /** 字母记号（`JS` / `TS` / `RS`）。有它就渲染文字 */
  text?: string;
  /** 图形记号的 symbol id（`fi-vue`…）。没有 `text` 时用它 */
  sym?: string;
}

interface FileNode {
  type: "file";
  name: string;
  path: string;
  depth: number;
  entry: TreeEntry;
  icon: FileIcon;
}

type Node = DirNode | FileNode;

/**
 * 折树。
 *
 * `ls-tree` 是路径排序的，所以父目录一定先于子项出现，遇到没见过的目录现建父级即可，
 * 不需要第二次遍历去找"这个目录的父目录是谁"。
 */
const tree = computed<DirNode>(() => {
  const root: DirNode = { type: "dir", name: "", path: "", depth: -1, children: [] };
  const dirs = new Map<string, DirNode>([["", root]]);

  for (const entry of props.entries) {
    const cut = entry.path.lastIndexOf("/");
    const dirPath = cut === -1 ? "" : entry.path.slice(0, cut);
    const parent = dirOf(dirPath);
    const name = cut === -1 ? entry.path : entry.path.slice(cut + 1);
    parent.children.push({
      type: "file",
      name,
      path: entry.path,
      depth: parent.depth + 1,
      entry,
      icon: iconOf(name),
    });
  }

  sortTree(root);
  return root;

  function dirOf(dirPath: string): DirNode {
    const hit = dirs.get(dirPath);
    if (hit) return hit;
    const cut = dirPath.lastIndexOf("/");
    const parent = dirOf(cut === -1 ? "" : dirPath.slice(0, cut));
    const node: DirNode = {
      type: "dir",
      name: cut === -1 ? dirPath : dirPath.slice(cut + 1),
      path: dirPath,
      depth: parent.depth + 1,
      children: [],
    };
    parent.children.push(node);
    dirs.set(dirPath, node);
    return node;
  }

  /**
   * 目录在前、文件在后，各自按名字排。
   *
   * 跟旧版（照 `ls-tree` 的路径序分组）不同：路径序里 `src-tauri/` 会排在 `src/` 后面、
   * 根下的文件会夹在目录中间，扫一眼分不出层级。`numeric` 让 `file2` 排在 `file10` 前面。
   */
  function sortTree(node: DirNode) {
    node.children.sort((a, b) => {
      if (a.type !== b.type) return a.type === "dir" ? -1 : 1;
      return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
    });
    for (const child of node.children) if (child.type === "dir") sortTree(child);
  }
});

/** 收起来的目录。存"收起的"而不是"展开的"，默认全展开就是这么来的 */
const collapsed = reactive(new Set<string>());

/** 摊平成一维：折叠的目录不下钻，模板里就不用递归了 */
const rows = computed<Node[]>(() => {
  const out: Node[] = [];
  const walk = (node: DirNode) => {
    for (const child of node.children) {
      out.push(child);
      if (child.type === "dir" && !collapsed.has(child.path)) walk(child);
    }
  };
  walk(tree.value);
  return out;
});

function onRow(node: Node) {
  if (node.type === "dir") {
    if (collapsed.has(node.path)) collapsed.delete(node.path);
    else collapsed.add(node.path);
    return;
  }
  // 子模块指针点开只有一行指针，不给这个入口（§7.4 同一条）
  if (node.entry.kind === "commit") return;
  emit("pick", node.entry);
}

// 选中的文件如果被收在了折叠的目录里（从历史/归属跳回来时会这样），把祖先全展开——
// 否则高亮落在一个看不见的行上，看起来像"什么都没选中"
watch(
  () => props.selected,
  (path) => {
    if (!path) return;
    const parts = path.split("/");
    for (let i = 1; i < parts.length; i += 1) collapsed.delete(parts.slice(0, i).join("/"));
  },
);

/** 后缀。以点开头的文件（`.gitignore`）也走这条路：`gitignore` 就是它的后缀 */
function extOf(name: string): string {
  const cut = name.lastIndexOf(".");
  return cut === -1 ? "" : name.slice(cut + 1).toLowerCase();
}

/**
 * 颜色族：一个族共用一个 token，不按后缀逐个配色（几十种后缀配几十个色只会变成噪声）。
 * 认不出来的留空 ⇒ 落到三级灰。
 */
const FAMILY: Record<string, string> = {
  js: "js", mjs: "js", cjs: "js", jsx: "js",
  ts: "ts", mts: "ts", cts: "ts", tsx: "ts",
  vue: "vue",
  rs: "rust",
  css: "style", scss: "style", sass: "style", less: "style",
  html: "markup", htm: "markup", xml: "markup",
  json: "data", yaml: "data", yml: "data", toml: "data", ini: "data", lock: "data",
  md: "doc", markdown: "doc", txt: "doc", rst: "doc",
  sh: "shell", bash: "shell", zsh: "shell", ps1: "shell", psm1: "shell",
  bat: "shell", cmd: "shell",
  png: "image", jpg: "image", jpeg: "image", gif: "image", webp: "image",
  ico: "image", bmp: "image", svg: "image",
};

/**
 * 字母记号：只有"字母本身就把类型说清"的这几族才用。其余交给图形（见 `GLYPHS`）。
 *
 * 键是 `FAMILY` 的值（族名）而不是后缀——查表查的是族。写成后缀会静默查空：
 * 图标一个不画、也不报错，只有把 DOM 拉出来量 bbox 才发现。
 */
const MARKS: Record<string, string> = { js: "JS", ts: "TS", rust: "RS" };

/**
 * 图形记号：**一族一个、互不重样**。
 *
 * 上一版让 js/ts/vue/rust 共用一个 `</>`，只靠颜色区分——一屏文件扫下来全是同一个
 * 图标，类型反而读不出来。现在每个族给一个认得出的形状（`.vue` 是 Vue 的 V、
 * `.html` 是 `<>`、`.json` 是 `{}`、`.css` 是 `#`、脚本是 `>_`…），颜色只做辅助。
 */
const GLYPHS: Record<string, string> = {
  vue: "fi-vue",
  markup: "fi-markup",
  data: "fi-data",
  doc: "fi-doc",
  style: "fi-style",
  shell: "fi-shell",
  image: "fi-image",
};

/** 文件行：认得出族就给对应记号，认不出来的退化成一页空白（三级灰） */
function iconOf(name: string): FileIcon {
  const f = FAMILY[extOf(name)];
  if (!f) return { cls: "", sym: "fi-none" };
  const text = MARKS[f];
  return text ? { cls: `t-${f}`, text } : { cls: `t-${f}`, sym: GLYPHS[f] };
}

function sizeOf(entry: TreeEntry): string {
  return entry.size === undefined ? "" : formatBytes(entry.size);
}

/** 可执行位变了要看得出来：`100755` 标一下，免得以为内容没变就没人管 */
function isExec(entry: TreeEntry): boolean {
  return entry.mode === "100755";
}
</script>

<template>
  <div class="tree">
    <!--
      图标形状表：只定义一次，每行用 <use> 引用。
      用内联 SVG 而不是图标库——这一处不值得为它多背一个依赖。
      形状一律"一族一个"、互不重样；细笔画 + 按类型上色，比一排实心色块安静。
    -->
    <svg class="sheet" aria-hidden="true">
      <defs>
        <symbol id="fi-none" viewBox="0 0 16 16">
          <path
            d="M4.6 2.4h4.3l2.6 2.6v7.7c0 .5-.4.9-.9.9H4.6c-.5 0-.9-.4-.9-.9V3.3c0-.5.4-.9.9-.9Z"
            fill="none"
            stroke="currentColor"
            stroke-width="1.15"
            stroke-linejoin="round"
          />
          <path
            d="M8.9 2.4v2.6h2.6"
            fill="none"
            stroke="currentColor"
            stroke-width="1.15"
            stroke-linejoin="round"
          />
        </symbol>

        <!-- Vue 的 V：外层实心三角、内层按 evenodd 挖空，不依赖行底色 -->
        <symbol id="fi-vue" viewBox="0 0 16 16">
          <path
            fill="currentColor"
            fill-rule="evenodd"
            d="M1.3 3.1h3.5L8 9.5l3.2-6.4h3.5L8 14.1zM4.8 3.1h2.1L8 5.9l1.1-2.8h2.1L8 9.5z"
          />
        </symbol>

        <symbol id="fi-markup" viewBox="0 0 16 16">
          <g
            fill="none"
            stroke="currentColor"
            stroke-width="1.35"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M5.7 4.9 2.6 8l3.1 3.1" />
            <path d="M10.3 4.9 13.4 8l-3.1 3.1" />
          </g>
        </symbol>

        <symbol id="fi-data" viewBox="0 0 16 16">
          <g
            fill="none"
            stroke="currentColor"
            stroke-width="1.3"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path
              d="M6.8 3.7c-1.2 0-1.9.7-1.9 1.9v1.2c0 1.1-.6 1.6-1.6 1.6 1 0 1.6.5 1.6 1.6v1.2c0 1.2.7 1.9 1.9 1.9"
            />
            <path
              d="M9.2 3.7c1.2 0 1.9.7 1.9 1.9v1.2c0 1.1.6 1.6 1.6 1.6-1 0-1.6.5-1.6 1.6v1.2c0 1.2-.7 1.9-1.9 1.9"
            />
          </g>
        </symbol>

        <symbol id="fi-doc" viewBox="0 0 16 16">
          <path
            d="M4.1 1.9h4.5l2.9 2.9v8.2c0 .6-.4 1-1 1H4.1c-.6 0-1-.4-1-1V2.9c0-.6.4-1 1-1Z"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linejoin="round"
          />
          <path
            d="M8.4 1.9v2.9h2.9"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linejoin="round"
          />
          <g fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round">
            <path d="M5.2 8.5h5.4" />
            <path d="M5.2 10.7h3.3" />
          </g>
        </symbol>

        <symbol id="fi-style" viewBox="0 0 16 16">
          <g fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round">
            <path d="M6.3 3.4 5.2 12.6" />
            <path d="M10.8 3.4 9.7 12.6" />
            <path d="M3.6 6.2h9" />
            <path d="M3.1 9.8h9" />
          </g>
        </symbol>

        <symbol id="fi-shell" viewBox="0 0 16 16">
          <g
            fill="none"
            stroke="currentColor"
            stroke-width="1.35"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M3.4 4.9 6.5 8l-3.1 3.1" />
            <path d="M7.9 11.1h4.7" />
          </g>
        </symbol>

        <symbol id="fi-image" viewBox="0 0 16 16">
          <rect
            x="2.2"
            y="3.1"
            width="11.6"
            height="9.8"
            rx="1.7"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
          />
          <path
            d="M3.4 11.5 6.5 8.4l1.9 1.9 1.5-1.5 2.7 2.7"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
          <circle cx="6" cy="6.4" r="0.95" fill="currentColor" />
        </symbol>
      </defs>
    </svg>

    <div v-if="loading" class="waiting">
      <n-spin size="small" />
      <span class="muted">正在读这个修订的文件树…</span>
    </div>

    <n-empty
      v-else-if="rows.length === 0"
      description="这个修订里没有文件"
      class="placeholder"
    />

    <div v-else class="body" role="tree">
      <div
        v-for="node in rows"
        :key="node.path"
        class="row"
        :class="{
          dir: node.type === 'dir',
          picked: node.type === 'file' && node.path === selected,
          plain: node.type === 'file' && node.entry.kind === 'commit',
          folded: node.type === 'dir' && collapsed.has(node.path),
        }"
        :style="{ paddingLeft: `${BASE + node.depth * STEP}px` }"
        :title="node.type === 'dir' ? `${node.path}/` : node.path"
        :role="node.type === 'dir' ? 'button' : 'treeitem'"
        :aria-expanded="node.type === 'dir' ? !collapsed.has(node.path) : undefined"
        @click="onRow(node)"
      >
        <!-- 引导线：每个祖先一条、贯穿整行，不然层级只靠缩进数像素是数不清的 -->
        <span
          v-for="level in node.depth"
          :key="level"
          class="guide"
          :style="{ left: `${BASE + (level - 1) * STEP + 11}px` }"
          aria-hidden="true"
        />

        <span v-if="node.type === 'dir'" class="slot" aria-hidden="true">
          <svg class="folder" viewBox="0 0 16 16" width="13" height="13">
            <path
              d="M1.5 4.2c0-.7.6-1.2 1.2-1.2h2.6l1.3 1.4h6.7c.7 0 1.2.5 1.2 1.2v6.2c0 .7-.5 1.2-1.2 1.2H2.7c-.6 0-1.2-.5-1.2-1.2z"
              fill="none"
              stroke="currentColor"
              stroke-width="1.1"
              stroke-linejoin="round"
            />
          </svg>
        </span>
        <span v-else class="slot">
          <span v-if="node.icon.text" class="mark" :class="node.icon.cls">{{ node.icon.text }}</span>
          <svg
            v-else
            class="file-icon"
            :class="node.icon.cls"
            width="14"
            height="14"
            aria-hidden="true"
          >
            <use :href="`#${node.icon.sym}`" />
          </svg>
        </span>

        <span class="name">{{ node.name }}</span>

        <span v-if="node.type === 'dir'" class="twist" aria-hidden="true" />
        <template v-else>
          <span v-if="isExec(node.entry)" class="chip exec">可执行</span>
          <span v-if="node.entry.kind === 'commit'" class="chip sub">子模块</span>
          <span v-else class="size">{{ sizeOf(node.entry) }}</span>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tree {
  position: relative;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow-y: auto;
  /* 纵深很深的路径把左内边距顶上来时，宁可让名字省略号，也不给一条横向滚动条 */
  overflow-x: hidden;
  font-size: 12px;
}

.body {
  padding: 4px 0;
}

.row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 4px;
  height: 24px;
  padding-right: 8px;
  cursor: pointer;
  white-space: nowrap;
}

.row:hover {
  background: var(--surface-hover);
}

.row.picked {
  background: var(--surface-selected);
}

/* 子模块指针只有一行指针，没有正文可看——点它只能看到"这是个子模块" */
.row.plain {
  cursor: default;
}

.row.plain .name {
  opacity: 0.75;
}

.guide {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 1px;
  background: var(--border-soft);
}

/* 图标槽与徽标槽同宽，同一层的目录名和文件名才能对齐成一条线 */
.slot {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 100%;
}

.folder {
  color: var(--text-3);
}

/* 形状表本身不占位：它只负责提供 symbol，真正渲染的是每行里的那个 .file-icon */
.sheet {
  position: absolute;
  width: 0;
  height: 0;
}

.file-icon {
  display: block;
  flex: none;
  /* 认不出来的类型：一页空白，三级灰 */
  color: var(--text-3);
}

/* 字母记号：只有 JS / TS / RS 这几族用。等宽、加粗、字号压到 9px——
   再大字就撑破 22px 的槽，再小就和旁边 14px 的图形记号不是一个重量 */
.mark {
  flex: none;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, "Courier New", monospace;
  font-size: 9px;
  font-weight: 700;
  line-height: 1;
  letter-spacing: -0.3px;
}

/* 一族一色。放在 .file-icon / .mark 之后，同特异性下按源序盖掉默认灰 */
.t-js {
  color: var(--file-js);
}
.t-ts {
  color: var(--file-ts);
}
.t-vue {
  color: var(--file-vue);
}
.t-rust {
  color: var(--file-rust);
}
.t-shell {
  color: var(--file-shell);
}
.t-style {
  color: var(--file-style);
}
.t-markup {
  color: var(--file-markup);
}
.t-data {
  color: var(--file-data);
}
.t-doc {
  color: var(--file-doc);
}
.t-image {
  color: var(--file-image);
}

.row.dir .name {
  font-weight: 600;
  color: var(--text-2);
}

.name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 折起来朝右、展开朝下。CSS 三角：本身 0 尺寸，靠 border 画出来 */
.twist {
  flex: none;
  width: 0;
  height: 0;
  margin: 0 4px 0 2px;
  border: 4px solid transparent;
  border-left-color: var(--text-3);
  transition: transform 120ms ease;
}

.row.dir:not(.folded) .twist {
  transform: rotate(90deg);
}

.chip {
  flex: none;
  padding: 0 4px;
  border-radius: 3px;
  font-size: 10px;
  line-height: 15px;
}

.chip.exec {
  color: var(--warn-text);
  background: var(--warn-bg);
}

.chip.sub {
  color: var(--text-2);
  background: var(--surface-sunken);
}

/* 固定宽度右对齐：一列数字才会对齐成一条竖线，而不是随位数左右跳 */
.size {
  flex: none;
  min-width: 52px;
  text-align: right;
  color: var(--text-3);
}

.waiting {
  display: flex;
  gap: 8px;
  padding: 16px;
}

.placeholder {
  padding-top: 20%;
}

.muted {
  font-size: 11px;
  opacity: 0.7;
}
</style>
