<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { NButton, NCheckbox, NEmpty, NTag } from "naive-ui";
import CommitForm from "@/components/CommitForm.vue";
import FilePreview from "@/components/FilePreview.vue";
import HookPanel from "@/components/HookPanel.vue";
import PartialStage from "@/components/PartialStage.vue";
import SyncPanel from "@/components/SyncPanel.vue";
import { usePaneDivider } from "@/composables/usePaneDivider";
import { usePreviewStore, type PreviewSide } from "@/stores/preview";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";
import type { WorkingFile } from "@/api/status";

/**
 * 提交页（§7.8）：左栏两列勾选——已暂存 / 变更，中间画选中文件的差异，右边规范表单。
 *
 * 一个文件可以同时出现在两列里（暂存了一半又改了），这时上列勾着、下列没勾：
 * 勾下列是把剩下那半也暂存，取消上列是把暂存那半退回工作区。勾选态一律以 git
 * 返回的整份列表为准，不做乐观更新。
 *
 * 中间那一栏是这一页的关键：**两列各画自己那一半**。不分开的话，"我到底要提交什么"
 * 就只能靠文件名和状态字去猜，而 `MM` 恰恰是最常见也最需要看清的一种状态。
 */
const repos = useReposStore();
const writes = useWriteStore();
const preview = usePreviewStore();

const repoId = computed(() => repos.currentId);
const staged = computed(() => repos.stagedFiles);
const changed = computed(() => repos.changedFiles);
/** 一次 IPC 能带一串路径，"全部"就是把它当成一次多文件操作 */
const busy = computed(() => repos.staging || writes.busy);

/**
 * 左栏的宽度。像素制而不是比例：它只是一份文件名清单，跟着窗口变宽只会多出几百像素
 * 空白，而中间那栏差异才是要宽的那一个（与文件页的文件树同一条理由）。
 *
 * 上下限比"能拖多宽"保守得多：这一页右边还有固定 430px 的表单，960px 的最小窗口下
 * 三栏加起来只有 700px 出头，拖过头就是把差异栏压成一条缝。
 */
const split = ref<HTMLElement | null>(null);
const divider = usePaneDivider({
  container: split,
  storageKey: "git-tidy:commit:split",
  fallback: 220,
  min: 170,
  max: 380,
  unit: "px",
});

/**
 * 正在分段暂存的那个文件。它占的也是中间那一栏。
 *
 * 分段暂存原来挂在列表行的下面就地展开，而列表栏现在只有 200px 出头：一行的复选框
 * 加行号加正文根本铺不开。挪到中间这一栏既解决了宽度，也让"看"和"挑"落在同一个位置。
 */
const partialPath = ref<string | null>(null);

const partialFile = computed(() =>
  partialPath.value === null
    ? null
    : changed.value.find((file) => file.path === partialPath.value) ?? null,
);

function statusOf(file: WorkingFile) {
  if (file.conflict) return { text: "冲突", type: "error" as const };
  // 索引列是"这次要提交的内容"，工作区列是"还没提交的部分"，界面只说最要紧的那个
  const code = file.staged && file.worktreeStatus === " " ? file.indexStatus : file.worktreeStatus;
  const labels: Record<string, { text: string; type: "success" | "warning" | "info" | "default" }> =
    {
      A: { text: "新增", type: "success" },
      M: { text: "修改", type: "info" },
      D: { text: "删除", type: "warning" },
      R: { text: "重命名", type: "info" },
      C: { text: "复制", type: "info" },
      "?": { text: "未跟踪", type: "default" },
      " ": { text: "仅暂存", type: "success" },
    };
  return labels[code.trim() || " "] ?? { text: code, type: "default" as const };
}

function check(file: WorkingFile) {
  repos.stage([file.path]);
}

function uncheck(file: WorkingFile) {
  repos.unstage([file.path]);
}

function stageAll() {
  repos.stage(changed.value.map((file) => file.path));
}

function unstageAll() {
  repos.unstage(staged.value.map((file) => file.path));
}

function pathLabel(file: WorkingFile) {
  return file.fromPath ? `${file.fromPath} → ${file.path}` : file.path;
}

/** 点一行就是"看这个文件的这一半"。换文件时退出分段模式，否则中间那栏还停在上一个文件上 */
function select(file: WorkingFile, side: PreviewSide) {
  const id = repoId.value;
  if (id === null) return;
  partialPath.value = null;
  void preview.open(id, file, side);
}

/** 「分段」：中间那一栏换成这个文件的逐块勾选，再点一次收回普通预览 */
function togglePartial(file: WorkingFile) {
  if (partialPath.value === file.path) {
    partialPath.value = null;
    return;
  }
  const id = repoId.value;
  if (id === null) return;
  partialPath.value = file.path;
  void preview.open(id, file, "worktree");
}

/**
 * 暂存 / 取消暂存之后对一次账：选中的文件可能换了列，差异也不再是原来那一份。
 *
 * 放在 watch 里而不是每个勾选动作里各写一次：勾选有三个入口（单行、两列的「全部」、
 * 分段暂存的「暂存选中的 N 段」），逐个去接早晚漏一个。
 */
watch(
  () => repos.workingFiles,
  (files) => {
    const id = repoId.value;
    if (id !== null) preview.sync(id, files);
    // 这个文件整份都进索引之后就不在「变更」列了，分段面板跟着收起来
    if (partialPath.value !== null && partialFile.value === null) partialPath.value = null;
  },
);

/** 换仓库时中间那栏必须清空：留着上一个仓库的 diff 比空着危险得多 */
watch(repoId, () => {
  preview.close();
  partialPath.value = null;
});

/**
 * 从别的页签再进来时对一次账。
 *
 * 预览状态活在 store 里，不会随组件卸载消失；而这一页的 watch 只在工作区状态变化时
 * 才跑，重新进来时它并没有变过。
 */
onMounted(() => {
  const id = repoId.value;
  if (id !== null) preview.sync(id, repos.workingFiles);
});
</script>

<template>
  <div v-if="repoId === null || !repos.canCommit" class="placeholder">
    <n-empty
      :description="
        repos.current && repos.current.kind === 'browse'
          ? '这是只读浏览的仓库，先「克隆到本地」才能暂存和提交'
          : '在左侧打开一个本地仓库'
      "
    />
  </div>

  <div v-else ref="split" class="pane" :class="{ resizing: divider.dragging.value }">
    <section class="column" :style="{ width: `${divider.size.value}px` }">
      <header class="column-head">
        <span>已暂存（{{ staged.length }}）</span>
        <n-button
          size="small"
          quaternary
          :disabled="!staged.length || busy"
          @click="unstageAll"
        >
          全部取消暂存
        </n-button>
      </header>
      <div class="file-list">
        <div
          v-for="file in staged"
          :key="`staged-${file.path}`"
          class="file-row"
          :class="{ picked: preview.side === 'staged' && preview.path === file.path }"
          @click="select(file, 'staged')"
        >
          <!--
            勾选框单独停掉冒泡：点它是"取消暂存"，同时也是在选中这一行的话，
            同一次点击会既把它退回工作区、又把中间那栏换成退回之后的内容。
          -->
          <n-checkbox
            :checked="true"
            :disabled="busy"
            @click.stop
            @update:checked="uncheck(file)"
          />
          <n-tag :type="statusOf(file).type" size="small" :bordered="false">
            {{ statusOf(file).text }}
          </n-tag>
          <code class="file-path" :title="pathLabel(file)">{{ pathLabel(file) }}</code>
        </div>
        <n-empty v-if="!staged.length" size="small" description="暂存区是空的" />
      </div>

      <header class="column-head">
        <span>变更（{{ changed.length }}）</span>
        <n-button size="small" quaternary :disabled="!changed.length || busy" @click="stageAll">
          全部暂存
        </n-button>
      </header>
      <div class="file-list">
        <div
          v-for="file in changed"
          :key="`changed-${file.path}`"
          class="file-row"
          :class="{ picked: preview.side === 'worktree' && preview.path === file.path }"
          @click="select(file, 'worktree')"
        >
          <n-checkbox
            :checked="false"
            :disabled="busy"
            @click.stop
            @update:checked="check(file)"
          />
          <n-tag :type="statusOf(file).type" size="small" :bordered="false">
            {{ statusOf(file).text }}
          </n-tag>
          <code class="file-path" :title="pathLabel(file)">{{ pathLabel(file) }}</code>
          <!-- 分段暂存的入口：点开之后中间那一栏换成逐块勾选 -->
          <n-button size="small" quaternary @click.stop="togglePartial(file)">
            {{ partialPath === file.path ? "收起" : "分段" }}
          </n-button>
        </div>
        <n-empty v-if="!changed.length" size="small" description="工作区干净" />
      </div>
    </section>

    <div
      class="divider"
      role="separator"
      aria-orientation="vertical"
      :aria-valuenow="Math.round(divider.size.value)"
      title="拖动调宽窄，双击恢复默认"
      @pointerdown="divider.onPointerDown"
      @dblclick="divider.reset"
    />

    <section class="preview-pane">
      <!-- 分段暂存自己带滚动与内边距，这里只负责给它一块够宽的地方 -->
      <div v-if="partialFile" class="scrolling">
        <partial-stage :file="partialFile" />
      </div>
      <file-preview v-else />
    </section>

    <section class="form-column">
      <commit-form
        :repo-id="repoId"
        :locked="repos.interrupted"
        @committed="repos.refreshAll()"
      />
      <!-- 提交与推送是连着的两步：提交完 `ahead` 一变，这里立刻显示「↑ N 待推送」 -->
      <sync-panel />
      <hook-panel :repo-id="repoId" />
    </section>
  </div>
</template>

<style scoped>
/* 栏距一律由分隔条自己的左右外边距给：分隔条按指针位置算宽度，中间再垫一层 gap
   会让"拖着拖着就偏了"变成常驻现象 */
.pane {
  display: flex;
  align-items: stretch;
  height: 100%;
}

.column {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: none;
  min-width: 0;
}

.divider {
  position: relative;
  flex: none;
  width: 3px;
  margin: 0 7px;
  cursor: col-resize;
  border-radius: 2px;
  transition: background 120ms ease;
}

/* 热区扩到 17px，不占布局——理由见 CommitsView 里的同一条注释 */
.divider::after {
  content: "";
  position: absolute;
  inset: 0 -7px;
}

.divider:hover,
.pane.resizing .divider {
  background: var(--accent);
}

.preview-pane {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  /* 里面自己滚：正文长起来不能把这一栏整体撑高，把窗口拖出滚动条 */
  overflow: hidden;
}

.scrolling {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

/**
 * 表单栏。可以收窄到 290px，因为它和差异栏抢的是同一行的宽度：最小窗口下三栏
 * 只有 700px 出头，430px 原样不动的话差异栏就没地方了。
 *
 * 290px 是表单里最宽那一行（type / scope 两个下拉并排）还铺得开的下限，再窄会开始折行。
 */
.form-column {
  flex: 0 1 430px;
  min-width: 290px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  max-height: 100%;
  overflow: auto;
}

.column-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-3);
  padding: 2px 4px;
}

.file-list {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 6px 8px;
  max-height: 38vh;
  overflow: auto;
}

.file-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 4px;
  margin: 0 -4px;
  border-radius: 4px;
  font-size: 12px;
  cursor: pointer;
}

.file-row:hover {
  background: var(--surface-hover);
}

/* 选中态要够显眼：中间那一栏画的是"哪一个文件的哪一半"，认错了就是对着别的文件做决定 */
.file-row.picked {
  background: var(--surface-hover);
  box-shadow: inset 2px 0 0 var(--accent);
}

.file-path {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 60vh;
}
</style>
