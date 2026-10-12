<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import dayjs from "dayjs";
import {
  NAlert,
  NButton,
  NDropdown,
  NEmpty,
  NSpace,
  NSpin,
  NTag,
  useDialog,
  useMessage,
  type DropdownOption,
} from "naive-ui";
import CommitGraphPanel from "@/components/CommitGraphPanel.vue";
import CommitDetail from "@/components/CommitDetail.vue";
import FilterBar from "@/components/FilterBar.vue";
import { useCommitStore } from "@/stores/commits";
import { useDetailStore } from "@/stores/detail";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";
import { specFor } from "@/api/spec";
import { usePrefsStore } from "@/stores/prefs";
import { usePaneDivider } from "@/composables/usePaneDivider";
import { isTypingTarget } from "@/shortcuts";
import type { Commit, CommitFilter, GraphRow } from "@/api/commit";

const repos = useReposStore();
const prefsStore = usePrefsStore();
const commitStore = useCommitStore();
const detail = useDetailStore();
const writes = useWriteStore();
const dialog = useDialog();
const message = useMessage();

const repoId = computed(() => repos.currentId);
const filterBar = ref<InstanceType<typeof FilterBar> | null>(null);

const prefs = computed(
  () =>
    prefsStore.prefs?.columns ?? { refs: true, author: true, time: true, sha: true },
);

const selectedCommit = computed(() => {
  const sha = detail.sha;
  if (sha === null) return null;
  return commitStore.commits.find((c) => c.id === sha) ?? detail.row;
});

const typeOptions = ref<string[]>([]);
const FALLBACK_TYPES = ["feat", "fix", "docs", "refactor", "style", "test", "perf", "build", "ci", "chore", "revert"];
const hasRows = computed(() => commitStore.commits.length > 0);
const LOAD_AHEAD_PX = 600;

function loadMore() {
  if (repoId.value !== null) commitStore.loadMore(repoId.value);
}

/** 唯一的滚动容器：分支图与提交列表共享，不需要手动同步滚动 */
const scrollEl = ref<HTMLElement | null>(null);

function onScroll() {
  const el = scrollEl.value;
  if (!el) return;
  if (el.scrollHeight - el.scrollTop - el.clientHeight > LOAD_AHEAD_PX) return;
  loadMore();
}

function pick(sha: string) {
  const commit = commitStore.commits.find((c) => c.id === sha) ?? null;
  if (detail.sha === sha) { detail.close(); return; }
  if (repoId.value === null) return;
  void detail.open(repoId.value, sha, commit);
}

/** 详情栏的分栏宽度 */
const splitContainer = ref<HTMLElement | null>(null);
const divider = usePaneDivider({
  container: splitContainer,
  storageKey: "git-tidy:history:detail",
  fallback: 0.4,
  min: 0.2,
  max: 0.6,
  unit: "ratio",
  containerMinWidth: 400,
});
const showDetail = computed(() => detail.sha !== null);

const outgoingBranch = computed(() => repos.repoState?.branch ?? null);
const firstUnpushedIndex = computed(() =>
  commitStore.commits.findIndex((c) => c.unpushed),
);

const TYPE_COLORS: Record<string, "success" | "error" | "warning" | "info"> = {
  feat: "success", fix: "error", perf: "warning", refactor: "warning",
  docs: "info", test: "info", build: "info", ci: "info",
};
function typeColor(commit: Commit) {
  return commit.commitType ? (TYPE_COLORS[commit.commitType] ?? "default") : "default";
}
function typeLabel(commit: Commit) {
  return commit.commitType ?? "非规范";
}

/* ---- 键盘导航 ---- */
const selectedIndex = computed(() => {
  const sha = detail.sha;
  if (sha === null) return -1;
  return commitStore.commits.findIndex((c) => c.id === sha);
});

function selectAt(index: number) {
  const commit = commitStore.commits[index];
  if (!commit || repoId.value === null) return;
  void detail.open(repoId.value, commit.id, commit);
}

function onKeyDown(event: KeyboardEvent) {
  if (event.ctrlKey || event.metaKey || event.altKey) return;
  if (isTypingTarget(event.target)) return;
  if (event.key === "Escape") {
    if (detail.sha === null) return;
    detail.close();
    event.preventDefault();
    return;
  }
  const commits = commitStore.commits;
  if (!commits.length) return;
  const at = selectedIndex.value;
  if (event.key === "ArrowDown" || event.key === "j") {
    selectAt(at === -1 ? 0 : Math.min(at + 1, commits.length - 1));
    event.preventDefault();
  } else if (event.key === "ArrowUp" || event.key === "k") {
    selectAt(at === -1 ? commits.length - 1 : Math.max(at - 1, 0));
    event.preventDefault();
  }
}

/* ---- 右键菜单 ---- */
const menu = reactive({ show: false, x: 0, y: 0, sha: "" });

function openMenu(event: MouseEvent, sha: string) {
  const commit = commitStore.commits.find((c) => c.id === sha) ?? null;
  if (repoId.value !== null && detail.sha !== sha) {
    void detail.open(repoId.value, sha, commit);
  }
  menu.sha = sha;
  menu.x = event.clientX;
  menu.y = event.clientY;
  menu.show = true;
}

function closeMenu() { menu.show = false; }

const menuOptions = computed<DropdownOption[]>(() => {
  const commit = commitStore.commits.find((c) => c.id === menu.sha);
  if (!commit) return [];
  const writable = writes.canWrite;
  return [
    { label: "摘取到当前分支", key: "pick", disabled: !writable },
    {
      label: commit.merge ? "回滚这条（合并提交需在详情里选主线）" : "回滚这条",
      key: "revert",
      disabled: !writable || commit.merge,
    },
    {
      label: "重置到这条",
      key: "reset",
      children: [
        { label: "soft：改动回到暂存区", key: "reset:soft", disabled: !writable },
        { label: "mixed：改动回到工作区", key: "reset:mixed", disabled: !writable },
        { label: "hard：丢弃这条之后的一切", key: "reset:hard", disabled: !writable },
      ],
    },
    { type: "divider", key: "divider-1" },
    {
      label: "从这里开始改写",
      key: "rewrite",
      disabled: !writable || commit.id === repos.info?.headCommit,
    },
    { label: "复制 sha", key: "copy-sha" },
    { label: "复制标题", key: "copy-subject" },
  ];
});

async function copyText(text: string, what: string) {
  try {
    await navigator.clipboard.writeText(text);
    message.success(`已复制${what}`);
  } catch {
    message.warning(`复制失败，${what}：${text}`);
  }
}

function onMenuSelect(key: string) {
  const commit = commitStore.commits.find((c) => c.id === menu.sha);
  closeMenu();
  if (!commit) return;
  if (key === "copy-sha") { void copyText(commit.id, "提交号"); return; }
  if (key === "copy-subject") { void copyText(commit.subject, "标题"); return; }
  if (key === "rewrite") { void writes.openRewrite(commit.id); return; }
  const run = async () => {
    if (key === "pick") await writes.pick(commit.id);
    else if (key === "revert") await writes.revert(commit.id, null);
    else if (key.startsWith("reset:")) await writes.reset(key.slice("reset:".length) as "soft" | "mixed" | "hard", commit.id);
  };
  if (key === "reset:hard") {
    dialog.warning({
      title: "重置到这条提交（hard）",
      content: `会扔掉 ${commit.id.slice(0, 8)} 之后的全部提交，以及工作区里未提交的改动。`,
      positiveText: "确认丢弃",
      negativeText: "算了",
      onPositiveClick: run,
    });
    return;
  }
  void run();
}

function applyFilter(next: CommitFilter) {
  if (repoId.value === null) return;
  detail.close();
  void commitStore.applyFilter(repoId.value, next);
}

function clearFilter() {
  if (repoId.value === null) return;
  detail.close();
  void commitStore.clearFilter(repoId.value);
}

async function loadTypes() {
  const id = repoId.value;
  if (id === null) return;
  try {
    const spec = await specFor(id);
    if (repoId.value !== id) return;
    typeOptions.value = spec.types.length > 0 ? [...spec.types] : [...FALLBACK_TYPES];
  } catch {
    if (repoId.value !== id) return;
    typeOptions.value = [...FALLBACK_TYPES];
  }
}

/** 分支图所需的 GraphRow 列表（只含有数据的行） */
const graphRows = computed<GraphRow[]>(() => {
  const result: GraphRow[] = [];
  for (const commit of commitStore.commits) {
    const row = commitStore.rowFor(commit.id);
    if (row) result.push(row);
  }
  return result;
});

onMounted(async () => {
  if (repos.repos.length === 0) await repos.load();
  detail.close();
  if (repoId.value !== null) {
    void commitStore.open(repoId.value);
    void loadTypes();
  }
});

watch(repoId, (id) => {
  detail.close();
  if (id === null) return;
  void commitStore.open(id);
  void loadTypes();
});

onMounted(() => window.addEventListener("keydown", onKeyDown));
onBeforeUnmount(() => window.removeEventListener("keydown", onKeyDown));

import { provideShell } from "@/shell";
let releaseSearch: (() => void) | undefined;
let releaseRefresh: (() => void) | undefined;
onMounted(() => {
  releaseSearch = provideShell("focusSearch", () => filterBar.value?.focusSearch());
  releaseRefresh = provideShell("refresh", () => {
    if (repoId.value !== null) void commitStore.open(repoId.value);
  });
});
onBeforeUnmount(() => {
  releaseSearch?.();
  releaseRefresh?.();
});
</script>

<template>
  <div class="page">
    <n-empty v-if="repoId === null" description="在左侧添加或打开一个仓库" class="placeholder" />

    <template v-else>
      <filter-bar
        ref="filterBar"
        :filter="commitStore.filter"
        :refs="repos.refs"
        :types="typeOptions"
        :busy="commitStore.loading"
        @apply="applyFilter"
        @clear="clearFilter"
      />

      <div ref="splitContainer" class="history-body">
        <!-- 左右共享滚动区：分支图 + 提交列表 -->
        <div ref="scrollEl" class="scroll-area" @scroll="onScroll">
          <n-alert
            v-if="commitStore.error && !hasRows"
            type="error"
            :title="commitStore.error.message"
            class="banner-error"
          >
            <div>错误码：{{ commitStore.error.code }}</div>
            <pre v-if="commitStore.error.detail" class="raw-output">{{ commitStore.error.detail }}</pre>
          </n-alert>

          <n-empty
            v-if="!commitStore.loading && commitStore.commits.length === 0"
            description="空仓库，尚无提交"
            class="empty-state"
          />

          <template v-if="hasRows">
            <!-- 分支图（左列，sticky 定位使其在水平方向不随列表滚动） -->
            <div class="graph-column">
              <n-alert
                v-if="commitStore.graphError"
                type="warning"
                :title="`图列读取失败：${commitStore.graphError.message}`"
                class="graph-error"
              >
                <div>错误码：{{ commitStore.graphError.code }}</div>
              </n-alert>
              <commit-graph-panel
                v-if="commitStore.graphReady"
                :rows="graphRows"
                :lanes="commitStore.graphLanes"
                :selected-sha="detail.sha"
              />
              <div v-else-if="!commitStore.graphError" class="graph-loading">
                <n-spin size="small" />
              </div>
            </div>

            <!-- 提交列表（右列） -->
            <div class="list-column">
              <!-- "传出的更改" 分节标题 -->
              <div v-if="firstUnpushedIndex >= 0" class="outgoing-divider">
                <span class="outgoing-marker" />
                <span class="outgoing-label">传出的更改</span>
                <code v-if="outgoingBranch" class="outgoing-branch">{{ outgoingBranch }}</code>
              </div>

              <div
                v-for="commit in commitStore.commits"
                :key="commit.id"
                class="commit-row"
                :class="{ selected: commit.id === detail.sha }"
                @click="pick(commit.id)"
                @contextmenu.prevent="openMenu($event, commit.id)"
              >
                <div class="row-main">
                  <n-tag :type="typeColor(commit)" size="small" :bordered="false" class="type-tag">
                    {{ typeLabel(commit) }}
                  </n-tag>
                  <n-tag v-if="commit.scope" size="small" :bordered="false">{{ commit.scope }}</n-tag>
                  <n-tag v-if="commit.breaking" type="error" size="small" :bordered="false">!</n-tag>
                  <n-tag v-if="commit.merge" type="warning" size="small" :bordered="false">merge</n-tag>
                  <span class="subject" :title="commit.subject">{{ commit.subject }}</span>
                </div>
                <div class="row-meta">
                  <div v-if="prefs.refs && commit.refs.length" class="refs">
                    <span v-if="commit.head && !commit.refs.some((b) => b.head)" class="badge current">HEAD</span>
                    <span
                      v-for="badge in commit.refs.slice(0, 4)"
                      :key="`${badge.kind}:${badge.name}`"
                      class="badge"
                      :class="[badge.kind, { head: badge.head }]"
                    >{{ badge.name }}</span>
                    <span v-if="commit.refs.length > 4" class="badge more">+{{ commit.refs.length - 4 }}</span>
                  </div>
                  <code v-if="prefs.sha" class="sha">{{ commit.id.slice(0, 8) }}</code>
                  <span v-if="prefs.author" class="author">{{ commit.authorName }}</span>
                  <span v-if="prefs.time" class="time">{{ dayjs(commit.time * 1000).format("MM-DD HH:mm") }}</span>
                </div>
              </div>
            </div>
          </template>
        </div>

        <!-- 分隔条 -->
        <div
          v-if="showDetail"
          class="divider"
          :class="{ dragging: divider.dragging.value }"
          @pointerdown="divider.onPointerDown"
          @dblclick="divider.reset"
        />

        <!-- 提交详情面板 -->
        <div
          v-if="showDetail"
          class="detail-pane"
          :style="{ flexBasis: `${divider.size.value * 100}%` }"
        >
          <div class="detail-head">
            <span class="detail-title">提交详情</span>
            <n-button quaternary size="tiny" title="关闭" @click="detail.close()">✕</n-button>
          </div>
          <div class="detail-scroll">
            <commit-detail :repo-id="repoId" :commit="selectedCommit" />
          </div>
        </div>
      </div>

      <!-- 状态行 -->
      <n-space align="center" class="status-line">
        <span class="muted">共 {{ commitStore.total }} 条</span>
        <span v-if="commitStore.commits.length" class="muted">已读出 {{ commitStore.commits.length }} 条</span>
        <span v-if="commitStore.graphLoading" class="muted">历史走向读取中…</span>
        <span v-if="commitStore.truncated" class="scanned">只扫了历史的前一段，下面可能还有</span>
        <template v-if="commitStore.error && hasRows">
          <span class="muted">读取下一页失败：{{ commitStore.error.message }}</span>
          <n-button size="small" @click="loadMore">重试</n-button>
        </template>
        <n-spin v-else-if="commitStore.loading" size="small" />
        <span v-else-if="hasRows && commitStore.loadedAll" class="muted">已全部加载</span>
      </n-space>

      <n-dropdown
        trigger="manual"
        placement="bottom-start"
        :show="menu.show"
        :x="menu.x"
        :y="menu.y"
        :options="menuOptions"
        @clickoutside="closeMenu"
        @select="onMenuSelect"
      />
    </template>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 8px;
  height: 100%;
}

/* ---- 历史主体：滚动区 + 详情面板 ---- */
.history-body {
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

/* ---- 唯一的滚动区：分支图与列表共享 ---- */
.scroll-area {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  overflow-x: hidden;
  display: flex;
  gap: 0;
}

/* ---- 左列：分支图（sticky 让它在水平方向不动） ---- */
.graph-column {
  flex: none;
  position: sticky;
  left: 0;
  z-index: 1;
  background: var(--surface-app);
  border-right: 1px solid var(--border);
  min-width: 0;
}

.graph-error {
  margin: 8px;
  flex: none;
}

.graph-loading {
  display: flex;
  justify-content: center;
  padding: 40px 8px;
}

/* ---- 右列：提交列表 ---- */
.list-column {
  flex: 1;
  min-width: 0;
}

/* ---- 分隔条 ---- */
.divider {
  flex: none;
  width: 3px;
  cursor: col-resize;
  background: var(--border);
  position: relative;
  transition: background 120ms ease;
}

.divider:hover,
.divider.dragging {
  background: var(--accent);
}

.divider::after {
  content: "";
  position: absolute;
  display: block;
  inset: 0 -6px;
}

/* ---- 详情面板 ---- */
.detail-pane {
  flex: none;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.detail-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex: none;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border);
  background: var(--surface);
}

.detail-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-2);
}

.detail-scroll {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

/* ---- 提交行 ---- */
.commit-row {
  display: flex;
  flex-direction: column;
  gap: 3px;
  height: 44px;
  padding: 5px 14px;
  border-bottom: 1px solid var(--border-soft);
  cursor: pointer;
  transition: background 100ms ease;
}

.commit-row:hover {
  background: var(--surface-hover);
}

.commit-row.selected {
  background: var(--surface-selected);
  box-shadow: inset 2px 0 0 var(--accent);
}

.row-main {
  display: flex;
  align-items: center;
  gap: 4px;
  overflow: hidden;
}

.type-tag {
  flex: none;
  min-width: 44px;
  justify-content: center;
}

.subject {
  margin-left: 6px;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-size: 13px;
  color: var(--text-1);
}

.row-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11.5px;
  color: var(--text-3);
  overflow: hidden;
}

.refs {
  display: flex;
  align-items: center;
  gap: 3px;
  max-width: 200px;
  overflow: hidden;
}

.badge {
  flex: none;
  max-width: 120px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  padding: 0 6px;
  border-radius: 9px;
  font-size: 10px;
  line-height: 14px;
}

.badge.branch { color: var(--badge-branch-text); background: var(--badge-branch-bg); }
.badge.remote { color: var(--badge-remote-text); background: var(--badge-remote-bg); }
.badge.tag { color: var(--badge-tag-text); background: var(--badge-tag-bg); }
.badge.current { color: var(--badge-head-text); background: var(--badge-head-bg); }
.badge.head { font-weight: 600; box-shadow: inset 0 0 0 1px currentColor; }
.badge.more { color: var(--badge-more-text); background: var(--badge-more-bg); }

.sha {
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--text-3);
  flex: none;
}

.author {
  flex: none;
  white-space: nowrap;
  max-width: 100px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.time {
  flex: none;
  white-space: nowrap;
}

/* 传出的更改 */
.outgoing-divider {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 44px;
  padding: 0 14px;
  background: var(--surface-hover);
  border-bottom: 1px solid var(--border);
  font-size: 12px;
}

.outgoing-marker {
  display: inline-block;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--outgoing);
}

.outgoing-label {
  font-weight: 600;
  color: var(--text-1);
}

.outgoing-branch {
  padding: 0 6px;
  border-radius: 9px;
  font-size: 11px;
  line-height: 15px;
  color: var(--badge-branch-text);
  background: var(--badge-branch-bg);
}

/* ---- 其他 ---- */
.banner-error {
  margin: 8px 14px;
  flex: none;
}

.empty-state {
  padding-top: 20vh;
}

.status-line {
  flex: none;
  min-height: 24px;
}

.placeholder {
  padding-top: 20vh;
}

.muted {
  font-size: 12px;
  opacity: 0.7;
}

.scanned {
  color: var(--warn-text);
  opacity: 1;
}

.raw-output {
  margin: 8px 0 0;
  white-space: pre-wrap;
  font-size: 12px;
  opacity: 0.8;
}

/* 窄容器：详情面板不再显示分隔条拖拽，改为占满一定比例 */
@media (max-width: 700px) {
  .scroll-area {
    flex-direction: column;
  }
  .graph-column {
    position: relative;
    border-right: none;
    border-bottom: 1px solid var(--border);
  }
}
</style>
