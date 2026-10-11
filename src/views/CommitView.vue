<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { NButton, NCheckbox, NEmpty, NTag } from "naive-ui";
import CommitForm from "@/components/CommitForm.vue";
import FilePreview from "@/components/FilePreview.vue";
import HookPanel from "@/components/HookPanel.vue";
import PartialStage from "@/components/PartialStage.vue";
import SyncPanel from "@/components/SyncPanel.vue";
import { usePaneDivider } from "@/composables/usePaneDivider";
import { COMMIT_LAYOUT, COMMIT_THREE_PANE_MIN } from "@/lib/layout";
import { isTypingTarget } from "@/shortcuts";
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
 * 默认 320 而不是 220：一行里除了路径还有勾选框、状态标签和两处间隔，固定开销就要
 * 70px 上下，220 的时候留给路径的不到 134px（约 18 个字符），
 * `commands/diff.rs` 与 `commands/commit.rs` 在一栏里长得一模一样。
 *
 * 三个数字都从 `lib/layout.ts` 来——那边同时是"与 CSS 对齐"的锚点，见 check:layout。
 */
const split = ref<HTMLElement | null>(null);
const divider = usePaneDivider({
  container: split,
  storageKey: "git-tidy:commit:split",
  fallback: COMMIT_LAYOUT.listDefault,
  min: COMMIT_LAYOUT.listMin,
  max: COMMIT_LAYOUT.listMax,
  unit: "px",
});

/**
 * 窄到三栏摆不下。
 *
 * 这时左区只摆一个：清单，或者差异。宽窗口下它们是并排的两栏，窄下来只能二选一——
 * 而**表单栏在任何一档都不被牺牲**：它虽是"写完就走"的那一栏，可提交页存在的意义就是它。
 *
 * 阈值是 `lib/layout.ts` 里那四项加出来的（清单上限 + 分隔条 + 差异栏下限 + 表单栏下限），
 * 不是这里手写的一个数：手写的话，改任何一个 CSS 宽度都会让它悄悄失准。
 *
 * 量的是**容器**宽度而不是视口宽度：侧栏宽度是用户可拖的，同样 1280 的窗口，
 * 主区能差出 200px 来。
 */
const narrow = computed(
  () =>
    divider.containerWidth.value > 0 &&
    divider.containerWidth.value < COMMIT_THREE_PANE_MIN,
);

/** 窄布局下差异栏是不是正盖着左区。宽布局下这个值不起作用 */
const covering = ref(false);

// 从宽变窄的那一刻回到清单：用户刚在看的多半还是那一屏的文件名
watch(narrow, (value) => {
  if (value) covering.value = false;
});

/**
 * 分段暂存面板是否展开。它占的就是中间那一栏。
 *
 * 入口在差异栏的标题上而不是列表行里：那个按钮一个就要 48px（本体 + 间隔），
 * 正好是「变更」列最缺的一段，而它的语义本来就是"对正在看的这个文件挑几段"。
 */
const partialOpen = ref(false);

/** 展开中的分段目标。只有选的是「变更」侧、且那个文件还在这一列时才成立 */
const partialFile = computed(() => {
  if (!partialOpen.value || preview.side !== "worktree" || preview.path === null) return null;
  return changed.value.find((file) => file.path === preview.path) ?? null;
});

/**
 * 能不能给当前这个文件分段。
 *
 * 三条判据：看的是未暂存那一半（分段暂存的输入正是它）、这一栏画的是差异而不是正文
 * （未跟踪文件走正文预览，它没有可分的暂存区版本）、预览没有给出取不到差异的原因。
 * 后两条都已经在 `preview` 里算过一次，这里不重算，免得两处口径漂移。
 */
const canPartial = computed(
  () =>
    preview.kind === "diff" &&
    preview.side === "worktree" &&
    preview.path !== null &&
    preview.blocked === null,
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

function baseName(path: string): string {
  const cut = path.lastIndexOf("/");
  return cut === -1 ? path : path.slice(cut + 1);
}

/**
 * 一行的路径拆成「目录」与「文件名」两段渲染，好让目录先被省略。
 *
 * 不拆的话整体右截断吃掉的是文件名，而它才是识别一个文件的唯一依据：
 * `src-tauri/src/comman...` 底下藏着 diff.rs 和 commit.rs 两个不同的文件。
 * 拆开之后目录带自己的省略号，文件名保持完整（完整路径仍挂在 title 上）。
 */
function dirOf(file: WorkingFile): string {
  const cut = file.path.lastIndexOf("/");
  return cut === -1 ? "" : file.path.slice(0, cut + 1);
}

/** 改名显示"旧名 → 新名"：只留新名字就看不出它原来叫什么，而那正是这条记录的全部 */
function nameOf(file: WorkingFile): string {
  const name = baseName(file.path);
  return file.fromPath ? `${baseName(file.fromPath)} → ${name}` : name;
}

/** 点一行就是"看这个文件的这一半"。换文件时退出分段模式，否则中间那栏还停在上一个文件上 */
function select(file: WorkingFile, side: PreviewSide) {
  const id = repoId.value;
  if (id === null) return;
  partialOpen.value = false;
  // 窄布局下差异栏是盖在左区上的，点一行就是要看它
  if (narrow.value) covering.value = true;
  void preview.open(id, file, side);
}

/** 「分段暂存」：中间那一栏从"看差异"换成"逐块勾选"，再点一次换回来 */
function togglePartial() {
  partialOpen.value = !partialOpen.value;
}

/**
 * 进来先替用户选上一个文件。
 *
 * 中间那一栏空着的时候，这一页就只剩两份文件名清单，"我到底要提交什么"依然没有答案，
 * 而这一栏有 700px 宽。优先「已暂存」的第一个——那才是提交按钮要落盘的内容。
 *
 * 已有选中（比如从别的页签切回来）就不动：那一次选择是用户自己做的。
 * 窄布局下也不选：自动盖上清单，等于把"有哪几个文件"这件事藏起来。
 */
function selectFirst() {
  const id = repoId.value;
  if (id === null || preview.path !== null || narrow.value) return;
  const fromStaged = staged.value.length > 0;
  const file = fromStaged ? staged.value[0] : changed.value[0];
  if (file === undefined) return;
  void preview.open(id, file, fromStaged ? "staged" : "worktree");
}

// ---------------------------------------------------------------- 键盘导航（§7.23）

/** 两列的滚动容器。键盘移动之后要把选中的那一行滚进视口 */
const stagedList = ref<HTMLElement | null>(null);
const changedList = ref<HTMLElement | null>(null);

/**
 * 左栏的行序：已暂存在上、变更在下，就是屏幕上看到的那个顺序。
 *
 * 两列合成一条序列，是为了让 ↑ 在「变更」的第一行上继续走到「已暂存」的最后一行。
 * 分两段各管各的话，走到列尾得先想一下该按哪个键。
 */
const rows = computed<{ file: WorkingFile; side: PreviewSide }[]>(() => [
  ...staged.value.map((file) => ({ file, side: "staged" as const })),
  ...changed.value.map((file) => ({ file, side: "worktree" as const })),
]);

const selectedIndex = computed(() =>
  rows.value.findIndex((row) => row.side === preview.side && row.file.path === preview.path),
);

/**
 * 某一行的元素。用 DOM 查询而不是给每行挂 ref：两列各有几十行，`v-for` 上的 ref
 * 一旦写成函数形式，每次重渲染都会换一次身份、把旧的那条以 null 回调进来，
 * 取到的反而可能是已经卸载的那个。
 */
function rowElement(side: PreviewSide, path: string): HTMLElement | undefined {
  const box = side === "staged" ? stagedList.value : changedList.value;
  if (box === null) return undefined;
  return Array.from(box.children).find(
    (child) => (child as HTMLElement).dataset.path === path,
  ) as HTMLElement | undefined;
}

function selectAt(index: number) {
  const row = rows.value[index];
  if (row === undefined) return;
  select(row.file, row.side);
  // 键盘往下走时视口不会自己跟：选中态跑出可视区，这一页就失去了反馈。
  // 窄布局下这条也顺带让"逐个看下一个文件"不必先退回清单——select 会把差异继续盖住左区
  rowElement(row.side, row.file.path)?.scrollIntoView({ block: "nearest" });
}

/** Space：把这一行勾上或取消掉。两列的方向正好相反，那就是它们各自的语义 */
function toggleChecked(row: { file: WorkingFile; side: PreviewSide }) {
  if (row.side === "staged") uncheck(row.file);
  else check(row.file);
}

/**
 * 提交页的键盘导航。
 *
 * 这一页的核心循环是"走一遍清单、逐个勾选"，纯鼠标做很累；历史页早有 ↑/↓ 选行，
 * 这里补上同一套语义（没选过时 ↓ = 选第一条，而不是跳过第一条）。
 *
 * 四条边界：
 * - 输入框里不劫持（`isTypingTarget`），否则写提交信息时按空格会跑去勾文件；
 * - **Space 只在焦点落在 body 上时才算勾选**：焦点在按钮上时那是"按这个按钮"，
 *   抢过来会让「全部暂存」按不动；
 * - 分段面板开着时 Space 归它，那一栏里勾的是 hunk 而不是整个文件；
 * - **弹层开着时不管**：naive-ui 的弹层自己收方向键与 Esc。历史页那边也没有这道闸，
 *   代价是弹层里按 ↑↓ 会把背后的列表也选走一行——这里沿用同一口径，不另立标准。
 */
function onKeyDown(event: KeyboardEvent) {
  if (event.ctrlKey || event.metaKey || event.altKey) return;
  if (isTypingTarget(event.target)) return;

  if (event.key === "Escape") {
    // 分段面板先收：它盖在预览上面，收掉之后才谈得上关预览
    if (partialOpen.value) {
      partialOpen.value = false;
      event.preventDefault();
      return;
    }
    // 窄布局下 Esc 只是"从差异退回清单"：预览并没有关掉，只是没显示出来，
    // 所以这里不 close——那样窗口放宽回来的时候选中就没了
    if (narrow.value) {
      if (!covering.value) return;
      covering.value = false;
      event.preventDefault();
      return;
    }
    if (preview.path === null) return;
    preview.close();
    event.preventDefault();
    return;
  }

  if (rows.value.length === 0) return;
  const at = selectedIndex.value;

  if (event.key === "ArrowDown" || event.key === "j") {
    selectAt(at === -1 ? 0 : Math.min(at + 1, rows.value.length - 1));
    event.preventDefault();
    return;
  }
  if (event.key === "ArrowUp" || event.key === "k") {
    selectAt(at === -1 ? rows.value.length - 1 : Math.max(at - 1, 0));
    event.preventDefault();
    return;
  }

  if (event.key !== " " || event.target !== document.body) return;
  const row = rows.value[at];
  if (row === undefined || busy.value || partialOpen.value) return;
  toggleChecked(row);
  event.preventDefault();
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
    if (partialOpen.value && partialFile.value === null) partialOpen.value = false;
    // 工作区是异步读进来的：直接落在提交页时挂载那一刻它还是空的，要等它到位再补选
    selectFirst();
  },
);

/** 换仓库时中间那栏必须清空：留着上一个仓库的 diff 比空着危险得多 */
watch(repoId, () => {
  preview.close();
  partialOpen.value = false;
});

/**
 * 从别的页签再进来时对一次账。
 *
 * 预览状态活在 store 里，不会随组件卸载消失；而这一页的 watch 只在工作区状态变化时
 * 才跑，重新进来时它并没有变过。
 */
onMounted(() => {
  window.addEventListener("keydown", onKeyDown);
  const id = repoId.value;
  if (id !== null) preview.sync(id, repos.workingFiles);
  selectFirst();
});

onBeforeUnmount(() => window.removeEventListener("keydown", onKeyDown));
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

  <div
    v-else
    ref="split"
    class="pane"
    :class="{ resizing: divider.dragging.value, narrow }"
  >
    <!--
      窄布局下这一栏与差异栏互斥（各占满左区），所以宽度交给 flex，
      不再用拖出来的那个像素值——那个值只在三栏并排时有意义。
    -->
    <section
      v-show="!narrow || !covering"
      class="column"
      :style="narrow ? undefined : { width: `${divider.size.value}px` }"
    >
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
      <div ref="stagedList" class="file-list">
        <div
          v-for="file in staged"
          :key="`staged-${file.path}`"
          class="file-row"
          :data-path="file.path"
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
          <!-- 目录与文件名分开：省略号落在目录上，文件名一直看得见（见 dirOf 的注释） -->
          <code class="file-path" :title="pathLabel(file)">
            <span v-if="dirOf(file)" class="dir">{{ dirOf(file) }}</span>
            <span class="name">{{ nameOf(file) }}</span>
          </code>
        </div>
        <n-empty v-if="!staged.length" size="small" description="暂存区是空的" />
      </div>

      <header class="column-head">
        <span>变更（{{ changed.length }}）</span>
        <n-button size="small" quaternary :disabled="!changed.length || busy" @click="stageAll">
          全部暂存
        </n-button>
      </header>
      <div ref="changedList" class="file-list">
        <div
          v-for="file in changed"
          :key="`changed-${file.path}`"
          class="file-row"
          :data-path="file.path"
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
          <!-- 分段暂存的入口在差异栏标题上：这里每行省下的 48px 都归文件名 -->
          <code class="file-path" :title="pathLabel(file)">
            <span v-if="dirOf(file)" class="dir">{{ dirOf(file) }}</span>
            <span class="name">{{ nameOf(file) }}</span>
          </code>
        </div>
        <n-empty v-if="!changed.length" size="small" description="工作区干净" />
      </div>
    </section>

    <!-- 窄布局下这一栏不摆：左区让给清单或差异，多一条分隔条只会白占 17px -->
    <div
      v-show="!narrow"
      class="divider"
      role="separator"
      aria-orientation="vertical"
      :aria-valuenow="Math.round(divider.size.value)"
      title="拖动调宽窄，双击恢复默认"
      @pointerdown="divider.onPointerDown"
      @dblclick="divider.reset"
    />

    <section v-show="!narrow || covering" class="preview-pane">
      <!-- 窄布局下差异盖住了清单，就得给一条回去的路；宽布局下它一直在，不需要 -->
      <div v-if="narrow" class="back-bar">
        <n-button size="small" quaternary @click="covering = false">← 返回清单</n-button>
        <span class="back-hint">↑ / ↓ 直接换文件</span>
      </div>
      <!-- 分段暂存自己带滚动与内边距，这里只负责给它一块够宽的地方 -->
      <div v-if="partialFile" class="scrolling">
        <partial-stage :file="partialFile" />
      </div>
      <file-preview
        v-else
        :can-partial="canPartial"
        :partial-open="partialOpen"
        @toggle-partial="togglePartial"
      />
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

/* 窄布局：清单与差异互斥，谁显示谁占满左区 */
.pane.narrow .column {
  flex: 1;
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

/**
 * 三栏并排时差异栏不许被压没。
 *
 * 没有这一条时它是 `flex: 1`（`flex-basis: 0%`）——基准为 0 意味着**只有富余宽度时
 * 才分得到**，富余不够就一点都拿不到，缺口全被表单栏（基准 430）吸收：1280 的默认窗口
 * 下它只有 194px，960 的最小窗口下是 **0px**。给了下限之后，flex 解算会回头把表单栏
 * 压下去让位（表单栏有 `flex-shrink: 1`）。
 *
 * 窄布局下不加这条：那时它独占左区，反而是清单在让位。
 */
.pane:not(.narrow) .preview-pane {
  min-width: 240px;
}

/**
 * 窄布局下差异栏独占左区，同样要有下限——表单栏的基准是 430，富余宽度先被它占着，
 * 不给下限的话最小窗口下差异栏只剩 211px，比三栏时的下限还窄。
 * 300px 之下 flex 解算会去压表单栏（它有 `flex-shrink: 1`，下限 290）。
 */
.pane.narrow .preview-pane {
  min-width: 300px;
}

/* 窄布局下盖在清单上面的那条退路 */
.back-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
  padding: 4px 8px;
  border-bottom: 1px solid var(--border);
  font-size: 12px;
}

.back-hint {
  color: var(--text-3);
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

/**
 * 路径拆成两段渲染，省略号落在目录上。
 *
 * 两个收缩系数差 100 倍是关键：flex 按「系数 × 基准宽度」分摊缺额，所以目录先被压到 0，
 * 之后才轮到文件名。反过来（整体右截断）先没的是文件名，而 `commands/diff.rs` 与
 * `commands/commit.rs` 在只剩 `src-tauri/src/comman...` 的时候长得一模一样。
 *
 * 两段各带一个省略号，所以空间不够时的中间形态是 `src-tauri/s…/diff.rs`。
 */
.file-path {
  display: flex;
  align-items: baseline;
  flex: 1;
  min-width: 0;
}

.file-path .dir {
  flex: 0 100 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-3);
}

.file-path .name {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 60vh;
}
</style>
