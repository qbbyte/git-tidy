<script setup lang="ts">
import { computed } from "vue";
import { NEmpty, NSpin } from "naive-ui";
import type { TreeEntry } from "@/api/file";
import { formatBytes } from "@/format";

/**
 * 某个修订的文件树（§7.6）。
 *
 * Rust 侧给的是**扁平列表**（`ls-tree -r` 的原样输出，按路径排序），树形是这里按路径
 * 分段推出来的：只比相邻两级的层号，不在 Rust 侧再拼一次树——那一份数据还要再序列化一遍，
 * 而界面上真正要的只是"缩进几格"。
 */
const props = defineProps<{
  entries: TreeEntry[];
  loading: boolean;
  selected: string | null;
}>();

const emit = defineEmits<{ pick: [TreeEntry] }>();

interface Node {
  entry: TreeEntry;
  /** 文件名之外的目录前缀（`src/git/`），空表示根目录下的文件 */
  dir: string;
  /** 该目录的缩进层数 */
  depth: number;
}

const nodes = computed<Node[]>(() => {
  const out: Node[] = [];
  let previousDir = "";
  for (const entry of props.entries) {
    const cut = entry.path.lastIndexOf("/");
    const dir = cut === -1 ? "" : entry.path.slice(0, cut);
    // 层数只比相邻两条：按路径排序过，同一个目录下的文件是连续的，
    // 前缀相同时层数必然相同；前缀变了才可能变深或变浅
    const depth =
      dir === previousDir
        ? out[out.length - 1]?.depth ?? 0
        : dir === ""
          ? 0
          : dir.split("/").length;
    out.push({ entry, dir, depth });
    previousDir = dir;
  }
  return out;
});

/** 目录分组：同一目录下的文件并在一次标题里，不用每行都画一遍目录名 */
interface Group {
  dir: string;
  depth: number;
  entries: TreeEntry[];
}

const groups = computed<Group[]>(() => {
  const out: Group[] = [];
  for (const node of nodes.value) {
    const head = out[out.length - 1];
    if (head && head.dir === node.dir) {
      head.entries.push(node.entry);
      continue;
    }
    out.push({ dir: node.dir, depth: node.depth, entries: [node.entry] });
  }
  return out;
});

/** 目录里的文件还要按子目录排一遍，不然 `a/x.txt` 会排在 `a-b.txt` 后面 */
function nameOf(entry: TreeEntry) {
  const cut = entry.path.lastIndexOf("/");
  return cut === -1 ? entry.path : entry.path.slice(cut + 1);
}

function sortEntries(entries: TreeEntry[]) {
  return [...entries].sort((a, b) => nameOf(a).localeCompare(nameOf(b)));
}

function sizeOf(entry: TreeEntry): string {
  if (entry.kind === "commit") return "子模块";
  return entry.size === undefined ? "" : formatBytes(entry.size);
}

/** 可执行位变了要看得出来：`100755` 标一下，免得以为内容没变就没人管 */
function execOf(entry: TreeEntry) {
  return entry.mode === "100755" ? "可执行" : "";
}
</script>

<template>
  <div class="tree">
    <div v-if="loading" class="waiting">
      <n-spin size="small" />
      <span class="muted">正在读这个修订的文件树…</span>
    </div>

    <n-empty v-else-if="nodes.length === 0" description="这个修订里没有文件" class="placeholder" />

    <div v-else class="body">
      <div v-for="group in groups" :key="group.dir" class="group">
        <div v-if="group.dir" class="dir" :style="{ paddingLeft: `${group.depth * 14 + 8}px` }">
          {{ group.dir }}/
        </div>
        <div
          v-for="entry in sortEntries(group.entries)"
          :key="entry.path"
          class="file"
          :class="{ picked: entry.path === selected, plain: entry.kind === 'commit' }"
          :style="{ paddingLeft: `${(group.dir ? group.depth : group.depth) * 14 + 20}px` }"
          @click="emit('pick', entry)"
        >
          <span class="name">{{ nameOf(entry) }}</span>
          <span v-if="execOf(entry)" class="flag">{{ execOf(entry) }}</span>
          <span class="size muted">{{ sizeOf(entry) }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tree {
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: auto;
  font-size: 12px;
}

.body {
  padding: 4px 0;
}

.dir {
  padding-top: 6px;
  font-weight: 600;
  color: var(--text-2);
}

.file {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 8px;
  cursor: pointer;
  white-space: nowrap;
}

.file:hover {
  background: var(--surface-hover);
}

.file.picked {
  background: var(--surface-selected);
}

/* 子模块指针只有一行指针，没有正文可看——点它只能看到"这是个子模块" */
.file.plain {
  cursor: default;
  opacity: 0.75;
}

.name {
  overflow: hidden;
  text-overflow: ellipsis;
}

.flag {
  flex: none;
  color: var(--warn-text);
  font-size: 10px;
}

.size {
  flex: none;
  margin-left: auto;
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