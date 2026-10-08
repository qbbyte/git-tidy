<script setup lang="ts">
import { computed, ref } from "vue";
import dayjs from "dayjs";
import {
  NAlert,
  NButton,
  NEmpty,
  NPopconfirm,
  NSelect,
  NSpace,
  NSpin,
  NSwitch,
  NTag,
} from "naive-ui";
import DiffView from "@/components/DiffView.vue";
import { formatDelta } from "@/format";
import type { Change, ChangeStatus } from "@/api/detail";
import type { Commit } from "@/api/commit";
import { useDetailStore } from "@/stores/detail";
import { useWriteStore } from "@/stores/write";

/**
 * 「历史」页右半边：选中提交的信息 + 它改了哪些文件 + 其中一个文件的差异。
 * 清单和差异都是现取的，行数据本身仍来自左边那一列（`commit`），这里不重复取一次提交。
 */
const props = defineProps<{
  repoId: number | null;
  commit: Commit | null;
}>();

const store = useDetailStore();
const writes = useWriteStore();

/**
 * 回滚合并提交时必须显式选主线（§7.11）：`-m 1` 与 `-m 2` 回掉的是相反的一侧，
 * 默认一个就是替用户做了一次不可逆的决定。所以这里不给默认值。
 */
const mainline = ref<number | null>(null);

const STATUS_LETTER: Record<ChangeStatus, string> = {
  add: "A",
  modify: "M",
  delete: "D",
  rename: "R",
  copy: "C",
  typechange: "T",
};

const STATUS_CLASS: Record<ChangeStatus, string> = {
  add: "s-add",
  modify: "s-modify",
  delete: "s-delete",
  rename: "s-rename",
  copy: "s-rename",
  typechange: "s-modify",
};

const selectedPath = computed(() => store.file?.path ?? null);

/** 一次提交总共改了多少：合并提交那句提示要说清它是相对谁的 */
const totals = computed(() => {
  let added = 0;
  let deleted = 0;
  for (const change of store.detail?.changes ?? []) {
    added += change.added ?? 0;
    deleted += change.deleted ?? 0;
  }
  return { added, deleted };
});

function pick(change: Change) {
  // 子模块指针那两个号是提交号不是 blob，点开只会得到一个查不到的对象（§7.4）
  if (props.repoId === null || change.gitlink) return;
  store.pickFile(props.repoId, change);
}

function countsOf(change: Change): string {
  if (change.gitlink) {
    // 全 40 位在这一栏摆不下也没必要：认前 8 位足够跳到子仓库里去找那一条
    const from = change.oldOid?.slice(0, 8) ?? "新增";
    const to = change.newOid?.slice(0, 8) ?? "移除";
    return `子模块指针 ${from} → ${to}`;
  }
  if (change.modeOnly) {
    return `仅权限 ${(change.oldMode ?? "?").slice(-3)} → ${(change.newMode ?? "?").slice(-3)}`;
  }
  if (change.binary) {
    const sizes = formatDelta(change.oldSize, change.newSize);
    return sizes === "" ? "二进制文件" : `二进制 ${sizes}`;
  }
  return `+${change.added ?? 0} −${change.deleted ?? 0}`;
}

function toggleWhiteSpace(value: boolean) {
  if (props.repoId === null) return;
  store.setIgnoreWhiteSpace(props.repoId, value);
}

function retry() {
  if (props.repoId === null) return;
  store.retryDiff(props.repoId);
}

/** 摘取这一条到当前分支。冲突时 Rust 侧回中断态，顶部提示条给一键退回 */
async function pickThis() {
  if (props.commit === null) return;
  await writes.pick(props.commit.id);
}

async function revertThis() {
  if (props.commit === null) return;
  // 合并提交没选主线就拒：这个选择不能由工具替用户做
  if (props.commit.merge && mainline.value === null) return;
  await writes.revert(props.commit.id, mainline.value);
  mainline.value = null;
}

/** reset 到这条提交。soft / mixed / hard 三档各是一次确认 */
async function resetToThis(mode: "soft" | "mixed" | "hard") {
  if (props.commit === null) return;
  await writes.reset(mode, props.commit.id);
}
</script>

<template>
  <div class="detail">
    <n-empty v-if="commit === null" description="选一条提交看它改了什么" class="placeholder" />

    <template v-else>
      <div class="head">
        <code class="sha" :title="commit.id">{{ commit.id.slice(0, 10) }}</code>
        <span class="subject">{{ commit.subject }}</span>
      </div>
      <!--
        这一条的写操作。全部经 write_guard：脏工作区、中断态、HEAD 被别处挪动
        都会在 Rust 侧被拒，界面只负责把拒绝的理由显示出来。
      -->
      <n-space size="small" class="ops" align="center">
        <n-button size="tiny" quaternary :disabled="!writes.canWrite" @click="pickThis">
          摘取到当前分支
        </n-button>
        <n-popconfirm :disabled="!writes.canWrite" positive-text="回滚" negative-text="算了" @positive-click="revertThis">
          <template #trigger>
            <n-button size="tiny" quaternary :disabled="!writes.canWrite">回滚这条</n-button>
          </template>
          <div class="revert-note">
            <div>回滚是在当前分支上新增一条反向提交，历史不会被删除。</div>
            <div v-if="commit.merge" class="warn">
              这是合并提交，<b>必须选主线</b>：选错会撤掉另一侧的工作。
            </div>
            <n-space v-if="commit.merge" size="small" align="center">
              <n-select
                v-model:value="mainline"
                size="small"
                class="mainline"
                placeholder="选主线"
                :options="[
                  { label: '主线 1（第一父）', value: 1 },
                  { label: '主线 2（第二父）', value: 2 },
                ]"
              />
            </n-space>
          </div>
        </n-popconfirm>
        <n-popconfirm positive-text="reset --soft" negative-text="算了" @positive-click="resetToThis('soft')">
          <template #trigger>
            <n-button size="tiny" quaternary :disabled="!writes.canWrite">重置到此（soft）</n-button>
          </template>
          提交留在历史里，改动回到暂存区。
        </n-popconfirm>
        <n-popconfirm positive-text="reset --hard" negative-text="算了" @positive-click="resetToThis('hard')">
          <template #trigger>
            <n-button size="tiny" quaternary type="error" :disabled="!writes.canWrite">
              丢弃到此处（hard）
            </n-button>
          </template>
          <b>会扔掉这条之后的全部提交与工作区里的改动。</b>工作区脏时这一档会被拒绝。
        </n-popconfirm>
      </n-space>
      <div class="meta muted">
        {{ commit.authorName }} &lt;{{ commit.authorEmail }}&gt; ·
        {{ dayjs(commit.time * 1000).format("YYYY-MM-DD HH:mm") }}
      </div>
      <pre v-if="commit.body" class="body">{{ commit.body }}</pre>

      <n-alert
        v-if="store.error"
        type="error"
        :title="store.error.message"
        class="block"
      >
        <div>错误码：{{ store.error.code }}</div>
        <pre v-if="store.error.detail" class="raw-output">{{ store.error.detail }}</pre>
      </n-alert>

      <div v-else-if="store.loading" class="waiting">
        <n-spin size="small" />
        <span class="muted">正在读这条提交改了哪些文件…</span>
      </div>

      <template v-else-if="store.detail">
        <div class="files">
          <div class="files-line muted">
            <span>{{ store.detail.changes.length }} 个文件</span>
            <span>+{{ totals.added }} −{{ totals.deleted }}</span>
            <!--
              合并提交给的是对**第一父**的差集（Rust 侧显式 diff <sha>^1 <sha>），
              不写这一句，看到"只有一两个文件"会以为这次合并没带别的东西进来。
            -->
            <span v-if="store.detail.merge" class="merge-note">
              合并提交：相对第一父 {{ store.detail.parents[0]?.slice(0, 8) }}
            </span>
          </div>
          <div
            v-for="change in store.detail.changes"
            :key="`${change.path}:${change.oldPath ?? ''}`"
            class="file"
            :class="{ picked: change.path === selectedPath, plain: change.gitlink }"
            @click="pick(change)"
          >
            <span class="letter" :class="STATUS_CLASS[change.status]">
              {{ STATUS_LETTER[change.status] }}
            </span>
            <span class="path" :title="change.oldPath ? `${change.oldPath} → ${change.path}` : change.path">
              {{ change.path }}
              <span v-if="change.oldPath" class="muted">（原 {{ change.oldPath }}）</span>
            </span>
            <span class="counts muted">{{ countsOf(change) }}</span>
            <n-tag v-if="change.score !== null && change.score < 100" size="tiny" :bordered="false">
              {{ change.score }}%
            </n-tag>
          </div>
        </div>

        <div class="diff-line">
          <span class="file-title">{{ store.file?.path ?? "" }}</span>
          <label class="ws muted">
            <n-switch size="small" :value="store.ignoreWhiteSpace" @update:value="toggleWhiteSpace" />
            忽略空白
          </label>
        </div>

        <diff-view :diff="store.diff" :loading="store.diffLoading" :error="store.diffError" @retry="retry" />
      </template>
    </template>
  </div>
</template>

<style scoped>
.detail {
  display: flex;
  flex-direction: column;
  gap: 6px;
  height: 100%;
  min-height: 0;
  padding: 10px 12px;
  font-size: 13px;
}

.head {
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.ops {
  flex-wrap: wrap;
}

.mainline {
  width: 150px;
}

.revert-note {
  font-size: 12px;
  line-height: 1.6;
  max-width: 320px;
}

.revert-note .warn {
  color: var(--warn-text);
  margin: 4px 0;
}

.sha {
  flex: none;
  color: var(--accent);
  font-size: 12px;
}

.subject {
  font-weight: 600;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.meta {
  font-size: 12px;
}

.body {
  margin: 4px 0 0;
  padding: 8px;
  background: var(--surface-sunken);
  border-radius: 6px;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: inherit;
  font-size: 12px;
  /* 正文可以很长：这一栏给个上限，剩下的滚动看，不能把文件列表挤出可视区 */
  max-height: 120px;
  overflow: auto;
}

.block {
  margin-top: 6px;
}

.waiting {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 0;
}

.files {
  flex: none;
  max-height: 30vh;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--surface);
  margin-top: 6px;
}

.files-line {
  display: flex;
  gap: 10px;
  padding: 5px 8px;
  border-bottom: 1px solid var(--border-soft);
  font-size: 12px;
  background: var(--surface-sunken);
  position: sticky;
  top: 0;
}

.merge-note {
  color: var(--warn-text);
}

.file {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 8px;
  border-bottom: 1px solid var(--surface-sunken);
  cursor: pointer;
}

.file:hover {
  background: var(--surface-hover);
}

.file.picked {
  background: var(--surface-selected);
}

/* 子模块指针没有差异可看，就不该装作能点 */
.file.plain {
  cursor: default;
}

.letter {
  flex: none;
  width: 18px;
  height: 18px;
  line-height: 18px;
  text-align: center;
  border-radius: 4px;
  font-size: 11px;
  font-family: Consolas, monospace;
  color: var(--surface);
}

.s-add {
  background: var(--ok-text);
}

.s-modify {
  background: var(--warn-text);
}

.s-delete {
  background: var(--error-text);
}

.s-rename {
  background: var(--status-rename-bg);
}

.path {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.counts {
  flex: none;
}

.diff-line {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 6px;
  padding-bottom: 4px;
  border-bottom: 1px solid var(--border-soft);
}

.file-title {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-family: Consolas, monospace;
  font-size: 12px;
}

.ws {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: none;
}

.muted {
  font-size: 12px;
  opacity: 0.7;
}

.raw-output {
  margin: 8px 0 0;
  white-space: pre-wrap;
  font-size: 12px;
  opacity: 0.8;
}

.placeholder {
  padding-top: 18vh;
}
</style>
