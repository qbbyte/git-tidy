<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { NAlert, NButton, NCheckbox, NEmpty, NSpin } from "naive-ui";
import { useDetailStore } from "@/stores/detail";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";
import {
  filePartialSupport,
  filesStageHunks,
  worktreeFileDiff,
  type HunkSelection,
  type PartialSupport,
} from "@/api/write";
import type { Diff, LineKind } from "@/api/detail";
import type { WorkingFile } from "@/api/status";

/**
 * 逐行 / 分块暂存（§7.8）。
 *
 * 三条硬边界都在这里落地，而且**界面不给不能做的入口**（§3 的期边界）：
 * - 未跟踪文件没有暂存区版本可比 → 只给整文件暂存，并说清原因；
 * - 受 `.gitattributes` eol 规则约束的文件 → 暂存区与工作区的行尾本来就不同，
 *   行号对不上，出来的补丁会改错内容 → 明确禁用；
 * - 二进制与超阈值文件 → 只给整文件操作。
 *
 * 勾选的形态是"第几段 + 第几行"，补丁由 Rust 侧从 git 自己的 diff 原文里裁：
 * 让前端拼补丁文本等于让它自己算行号，而行号错一位就是暂存错内容，且不会报错。
 */
const repos = useReposStore();
const detail = useDetailStore();
const writes = useWriteStore();

const props = defineProps<{ file: WorkingFile }>();

const diff = ref<Diff | null>(null);
const support = ref<PartialSupport | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);
/** 选中的：段号 → 行号集合（空集合＝整段） */
const picked = ref<Record<number, number[]>>({});

const repoId = computed(() => repos.currentId);
/** 二进制 / 超阈值的 diff 仍然读得出来，只是没有可勾的行——那种情况下整块不可勾 */
const hunkCount = computed(() => diff.value?.hunks.length ?? 0);

/** hunk 内每一行在 diff 里的下标：前端按显示顺序编号，Rust 按这个下标裁 */
const rows = computed(() => {
  const out: { hunk: number; line: number; kind: LineKind; text: string; no: number | null }[] = [];
  diff.value?.hunks.forEach((hunk, hunkIndex) => {
    hunk.lines.forEach((line, lineIndex) => {
      out.push({
        hunk: hunkIndex,
        line: lineIndex,
        kind: line.kind,
        text: line.text,
        no: line.kind === "delete" ? line.oldNo : line.newNo,
      });
    });
  });
  return out;
});

function isWholeHunkPicked(hunk: number) {
  const lines = picked.value[hunk];
  return lines !== undefined && lines.length === 0;
}

function isLinePicked(hunk: number, line: number) {
  const lines = picked.value[hunk];
  if (lines === undefined) return false;
  if (lines.length === 0) return true; // 整段都算选中
  return lines.includes(line);
}

/** 勾一行＝这一行的改动。勾走时整段选退化成"逐行列"的形式 */
function toggleLine(hunk: number, line: number, checked: boolean) {
  const current = picked.value[hunk];
  const asList = current === undefined || current.length === 0 ? [] : [...current];
  const next = checked
    ? [...asList.filter((item) => item !== line), line].sort((a, b) => a - b)
    : asList.filter((item) => item !== line);
  const copy = { ...picked.value };
  if (next.length === 0) {
    delete copy[hunk];
  } else {
    copy[hunk] = next;
  }
  picked.value = copy;
}

function toggleHunk(hunk: number, checked: boolean) {
  const copy = { ...picked.value };
  if (checked) {
    copy[hunk] = [];
  } else {
    delete copy[hunk];
  }
  picked.value = copy;
}

const selection = computed<HunkSelection[]>(() =>
  Object.entries(picked.value)
    .map(([hunk, lines]) => ({ hunk: Number(hunk), lines }))
    .sort((a, b) => a.hunk - b.hunk),
);

async function load() {
  const id = repoId.value;
  if (id === null) return;
  loading.value = true;
  error.value = null;
  picked.value = {};
  diff.value = null;
  try {
    // 先问能不能分段暂存：不能做时连 diff 都不必取
    support.value = await filePartialSupport(id, props.file.path);
    if (!support.value.supported) return;
    diff.value = await worktreeFileDiff(id, props.file.path);
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    loading.value = false;
  }
}

async function stageSelected() {
  const id = repoId.value;
  if (id === null || selection.value.length === 0) return;
  loading.value = true;
  error.value = null;
  try {
    const staged = await filesStageHunks(id, props.file.path, selection.value);
    // 以 git 返回的整份列表为准：暂存之后索引变了，界面不自己猜
    repos.workingFiles = staged.files;
    detail.close();
    picked.value = {};
    diff.value = null;
    support.value = null;
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    loading.value = false;
  }
}

watch(() => props.file.path, load, { immediate: true });

defineExpose({ reload: load });
</script>

<template>
  <div class="partial">
    <div class="head">
      <span class="muted">分段暂存</span>
      <n-button
        size="small"
        quaternary
        :loading="loading"
        :disabled="writes.busy"
        @click="load"
      >
        重读
      </n-button>
    </div>

    <n-alert v-if="error" type="error" :title="error" class="block" />

    <div v-else-if="loading" class="waiting">
      <n-spin size="small" />
      <span class="muted">正在读这个文件的未暂存改动…</span>
    </div>

    <template v-else-if="support && !support.supported">
      <!-- 不能分段暂存时不给开关，只说原因：给了再报错是设计缺陷 -->
      <div class="muted reason">{{ support.reason }}。用左侧的整文件勾选暂存。</div>
    </template>

    <n-empty v-else-if="hunkCount === 0" size="small" description="这个文件没有未暂存的改动" />

    <template v-else>
      <div v-for="(hunk, index) in diff?.hunks ?? []" :key="index" class="hunk">
        <div class="hunk-head">
          <n-checkbox
            :checked="isWholeHunkPicked(index)"
            :disabled="writes.busy"
            @update:checked="(checked) => toggleHunk(index, checked)"
          >
            第 {{ index + 1 }} 段 · 旧 {{ hunk.oldStart }} → 新 {{ hunk.newStart }}
          </n-checkbox>
        </div>
        <div
          v-for="line in rows.filter((row) => row.hunk === index)"
          :key="`${index}-${line.line}`"
          class="line"
          :class="line.kind"
        >
          <n-checkbox
            :checked="isLinePicked(index, line.line)"
            :disabled="line.kind !== 'add' && line.kind !== 'delete'"
            @update:checked="(checked) => toggleLine(index, line.line, checked)"
          />
          <span class="no">{{ line.no ?? "" }}</span>
          <span class="text">{{ line.text }}</span>
        </div>
      </div>

      <n-button
        size="small"
        type="primary"
        :disabled="selection.length === 0 || writes.busy"
        :loading="loading"
        @click="stageSelected"
      >
        暂存选中的 {{ selection.length }} 段
      </n-button>
      <div class="muted">
        只勾增删行也可以，上下文会自动带上 3 行（<code>git apply</code> 靠它定位）。
        整块预验不通过时索引一动不动，不会出现"半个 hunk 入栈"。
      </div>
    </template>
  </div>
</template>

<style scoped>
.partial {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 6px 8px;
  background: var(--surface-app);
  border-top: 1px solid var(--border-soft);
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-3);
}

.hunk {
  border: 1px solid var(--border);
  border-radius: 6px;
  overflow: hidden;
  background: var(--surface);
}

.hunk-head {
  padding: 2px 6px;
  background: var(--surface-sunken);
  border-bottom: 1px solid var(--border-soft);
  font-size: 11px;
}

.line {
  display: flex;
  align-items: flex-start;
  gap: 4px;
  padding: 0 4px;
  font-family: Consolas, "Courier New", monospace;
  font-size: 12px;
  line-height: 18px;
}

.line.add {
  background: var(--ok-bg);
}

.line.delete {
  background: var(--error-bg);
}

.no {
  flex: none;
  width: 30px;
  text-align: right;
  color: var(--text-3);
  user-select: none;
}

.text {
  white-space: pre-wrap;
  word-break: break-all;
  padding-right: 4px;
}

.waiting {
  display: flex;
  align-items: center;
  gap: 8px;
}

.reason {
  line-height: 1.6;
}

.muted {
  font-size: 11px;
  opacity: 0.75;
}
</style>