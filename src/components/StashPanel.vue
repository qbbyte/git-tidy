<script setup lang="ts">
import { computed, ref } from "vue";
import dayjs from "dayjs";
import { NButton, NCheckbox, NEmpty, NInput } from "naive-ui";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";

/**
 * stash 面板（§7.9）。
 *
 * 五件事：存、取（apply）、取出并删（pop）、删、从 stash 建分支。
 *
 * M2 的边界写在界面上：**冲突不解决，只给一键退回**。pop 撞冲突时 stash 条目仍在
 * （Rust 侧不让它被消费掉），界面把"退回"指出来，用户要么退，要么在终端里处理。
 */
const repos = useReposStore();
const writes = useWriteStore();

const message = ref("");
const includeUntracked = ref(true);
const selectedPaths = ref<string[]>([]);
const branchFrom = ref<{ reference: string; name: string } | null>(null);

/**
 * 面板自己折叠，不用外层再套一层折叠器。
 *
 * 套 NCollapse 就得把面板自己的表头拿掉（否则标题与计数出现两遍），而这个表头里
 * 装的是「工作区有 N 项改动」与 stash 数量——收起了就看不全。
 * 改成表头本身可点：标题与计数一直在，按钮也都在。
 */
const collapsed = ref(false);

const entries = computed(() => writes.stashes);
const hasWork = computed(() => repos.workingFiles.length > 0);

function canWrite() {
  return writes.canWrite && !writes.busy;
}

/** 部分存：只收选中的文件。全不选就是整仓存——那是 `git stash push` 的默认行为 */
const pathsToStash = computed(() => (selectedPaths.value.length > 0 ? [...selectedPaths.value] : null));

async function push() {
  const done = await writes.pushStash(
    pathsToStash.value,
    includeUntracked.value,
    message.value.trim() || null,
  );
  if (done !== null) {
    message.value = "";
    selectedPaths.value = [];
  }
}

async function stashToBranch(reference: string, name: string) {
  branchFrom.value = null;
  if (!name.trim()) return;
  await writes.branchFromStash(reference, name.trim());
}

function togglePath(path: string, checked: boolean) {
  selectedPaths.value = checked
    ? [...selectedPaths.value, path]
    : selectedPaths.value.filter((item) => item !== path);
}

function isPicked(path: string) {
  return selectedPaths.value.includes(path);
}
</script>

<template>
  <div class="stash">
    <div class="head toggle" :class="{ collapsed }" @click="collapsed = !collapsed">
      <span class="chevron" aria-hidden="true" />
      <span>stash {{ entries.length }}</span>
      <span class="muted">
        {{ hasWork ? `工作区有 ${repos.workingFiles.length} 项改动` : "工作区干净" }}
      </span>
    </div>

    <template v-if="!collapsed">

    <div v-if="repos.interrupted" class="warn">
      现在停在「{{ repos.interrupt }}」上，先退回或处理完再动 stash。
      <n-button size="small" :loading="writes.busy" @click="writes.abort()">一键退回</n-button>
    </div>

    <n-input v-model:value="message" size="small" placeholder="说明（留空＝git 默认的 WIP 描述）" />
    <n-checkbox v-model:checked="includeUntracked" :disabled="!canWrite()">
      连未跟踪的文件一起存（<code>-u</code>）
    </n-checkbox>

    <div v-if="repos.workingFiles.length" class="pick">
      <div class="muted">只存选中的文件（全不选＝整仓存）：</div>
      <label v-for="file in repos.workingFiles" :key="file.path" class="pick-row">
        <n-checkbox
          :checked="isPicked(file.path)"
          :disabled="!canWrite()"
          @update:checked="(checked) => togglePath(file.path, checked)"
        />
        <code>{{ file.path }}</code>
      </label>
    </div>

    <n-button size="small" type="primary" :disabled="!canWrite() || !hasWork" @click="push">
      存起来
    </n-button>

    <div v-if="entries.length" class="list">
      <div v-for="entry in entries" :key="entry.reference" class="row">
        <div class="row-main">
          <code class="ref">{{ entry.reference }}</code>
          <span class="msg" :title="entry.message">{{ entry.message }}</span>
        </div>
        <div class="row-meta muted">
          <span v-if="entry.branch">{{ entry.branch }}</span>
          <span>{{ dayjs(entry.time * 1000).format("MM-DD HH:mm") }}</span>
        </div>
        <div class="row-actions">
          <n-button size="small" quaternary :disabled="!canWrite()" @click="writes.applyStash(entry.reference)">
            取
          </n-button>
          <n-button size="small" quaternary :disabled="!canWrite()" @click="writes.popStash(entry.reference)">
            取并删
          </n-button>
          <n-button
            size="small"
            quaternary
            :disabled="!canWrite()"
            @click="branchFrom = { reference: entry.reference, name: '' }"
          >
            建分支
          </n-button>
          <n-button size="small" quaternary type="error" :disabled="!canWrite()" @click="writes.dropStash(entry.reference)">
            删
          </n-button>
        </div>
        <div v-if="branchFrom && branchFrom.reference === entry.reference" class="branch-from">
          <n-input
            v-model:value="branchFrom.name"
            size="small"
            placeholder="新分支名"
            @keyup.enter="stashToBranch(entry.reference, branchFrom.name)"
          />
          <n-button
            size="small"
            type="primary"
            @click="stashToBranch(entry.reference, branchFrom.name)"
          >
            建并切过去
          </n-button>
        </div>
      </div>
    </div>
    <n-empty v-else size="small" description="没有 stash" />

    <div class="muted">
      取出撞冲突时本版本不解决冲突：条目会保留，顶部给一键退回，或在终端里处理完。
    </div>
    </template>
  </div>
</template>

<style scoped>
.stash {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.head {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
}

/* 可折叠的表头。点击区给满整行，否则要点中文字才收得起 */
.head.toggle {
  cursor: pointer;
  user-select: none;
  padding: 2px 4px;
  margin: -2px -4px;
  border-radius: 4px;
}

.head.toggle:hover {
  background: var(--surface-hover);
}

/*
 * 箭头用 CSS 三角形画，不为一个图标引入 @vicons——
 * 这个图标只需要一个方向，旋转一个 border 三角就够。
 */
.chevron {
  flex: none;
  width: 0;
  height: 0;
  border: 4px solid transparent;
  border-top-color: currentColor;
  margin-top: 3px;
  transition: transform 120ms ease;
}

.head.collapsed .chevron {
  transform: rotate(-90deg);
  margin-top: 0;
}

.pick {
  max-height: 120px;
  overflow: auto;
  border: 1px solid var(--border-soft);
  border-radius: 6px;
  padding: 4px 6px;
  font-size: 11px;
}

.pick-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.list {
  display: flex;
  flex-direction: column;
}

.row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 8px;
  padding: 3px 0;
  border-bottom: 1px solid var(--surface-sunken);
}

.row-main {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
}

.ref {
  flex: none;
  color: var(--text-3);
  font-size: 11px;
}

.msg {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row-meta {
  display: flex;
  gap: 8px;
  flex: none;
}

.row-actions {
  display: none;
  gap: 2px;
  flex: none;
}

.row:hover .row-actions {
  display: flex;
}

.branch-from {
  flex-basis: 100%;
  display: flex;
  gap: 4px;
  padding: 2px 0 4px;
}

.warn {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 8px;
  background: var(--warn-bg);
  border: 1px solid var(--warn-border);
  border-radius: 6px;
  font-size: 11px;
}

.muted {
  font-size: 11px;
  opacity: 0.7;
}
</style>