<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import dayjs from "dayjs";
import {
  NAlert,
  NButton,
  NDropdown,
  NEmpty,
  NScrollbar,
  NSpace,
  NSpin,
  NSplit,
  NTable,
  NTag,
  useDialog,
  useMessage,
  type DropdownOption,
  } from "naive-ui";
import CommitGraph from "@/components/CommitGraph.vue";
import CommitDetail from "@/components/CommitDetail.vue";
import FilterBar from "@/components/FilterBar.vue";
import { useCommitStore } from "@/stores/commits";
import { useDetailStore } from "@/stores/detail";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";
import { specFor } from "@/api/spec";
import { usePrefsStore } from "@/stores/prefs";
import { isTypingTarget } from "@/shortcuts";
import type { Commit, CommitFilter } from "@/api/commit";

/**
 * 历史页（重构）：n-split 左右分栏，左栏用 n-table 展示提交记录，
 * 配合 CommitGraph 渲染 Git 提交网络图；右栏是选中提交的 Diff 详情。
 */
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
  return commitStore.commits.find((commit) => commit.id === sha) ?? detail.row;
});

const typeOptions = ref<string[]>([]);

const FALLBACK_TYPES = ["feat", "fix", "docs", "refactor", "style", "test", "perf", "build", "ci", "chore", "revert"];

const hasRows = computed(() => commitStore.commits.length > 0);

const LOAD_AHEAD_PX = 600;

function loadMore() {
  if (repoId.value !== null) commitStore.loadMore(repoId.value);
}

function onScrollbarScroll(e: Event) {
  const el = e.target as HTMLElement;
  if (el.scrollHeight - el.scrollTop - el.clientHeight > LOAD_AHEAD_PX) return;
  loadMore();
}

function pick(sha: string) {
  const commit = commitStore.commits.find((item) => item.id === sha) ?? null;
  if (detail.sha === sha) { detail.close(); return; }
  if (repoId.value === null) return;
  void detail.open(repoId.value, sha, commit);
}

const paneSize = ref(0.6);
function onSplitUpdate(val: number) { paneSize.value = val; }

/* ---- 传出的更改分节 ---- */
const outgoingBranch = computed(() => repos.repoState?.branch ?? null);
const firstUnpushedIndex = computed(() =>
  commitStore.commits.findIndex((c) => c.unpushed),
);

/* ---- 提交行 type 颜色 ---- */
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
  return commitStore.commits.findIndex((item) => item.id === sha);
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
  const commit = commitStore.commits.find((item) => item.id === sha) ?? null;
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
  const commit = commitStore.commits.find((item) => item.id === menu.sha);
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
  const commit = commitStore.commits.find((item) => item.id === menu.sha);
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

      <!-- 左右分栏：列表 + 提交详情；未选中提交时列表占满全宽 -->
      <n-split
        direction="horizontal"
        :size="detail.sha ? paneSize : 1"
        :min="0.2"
        :max="0.8"
        :resize-trigger-size="detail.sha ? 3 : 0"
        class="split"
        @update:size="onSplitUpdate"
      >
        <template #1>
          <div class="list-pane">
            <n-alert
              v-if="commitStore.error && !hasRows"
              type="error"
              :title="commitStore.error.message"
            >
              <div>错误码：{{ commitStore.error.code }}</div>
              <pre v-if="commitStore.error.detail" class="raw-output">{{ commitStore.error.detail }}</pre>
            </n-alert>

            <template v-else>
              <n-alert
                v-if="commitStore.graphError"
                type="warning"
                :title="`图列读取失败：${commitStore.graphError.message}`"
              >
                <div>错误码：{{ commitStore.graphError.code }}</div>
              </n-alert>

              <n-empty
                v-if="!commitStore.loading && commitStore.commits.length === 0"
                description="空仓库，尚无提交"
              />

              <!-- 提交记录列表：n-table + CommitGraph 网络图 -->
              <n-scrollbar
                v-if="commitStore.commits.length > 0"
                class="commit-scroll"
                @scroll="onScrollbarScroll"
              >
                <n-table :bordered="false" :single-line="false" size="small" class="commit-table">
                  <thead>
                    <tr>
                      <th class="col-graph"></th>
                      <th v-if="prefs.refs" class="col-branch">分支</th>
                      <th>提交信息</th>
                      <th v-if="prefs.sha" class="col-sha">哈希</th>
                      <th v-if="prefs.author" class="col-author">提交人</th>
                      <th v-if="prefs.time" class="col-time">提交时间</th>
                    </tr>
                  </thead>
                  <tbody>
                    <!-- 传出的更改分节标题 -->
                    <tr v-if="firstUnpushedIndex >= 0" class="outgoing-header-row">
                      <td :colspan="6">
                        <span class="outgoing-marker" />
                        <span class="outgoing-label">传出的更改</span>
                        <code v-if="outgoingBranch" class="outgoing-branch">{{ outgoingBranch }}</code>
                      </td>
                    </tr>
                    <tr
                      v-for="commit in commitStore.commits"
                      :key="commit.id"
                      class="commit-tr"
                      :class="{ selected: commit.id === detail.sha }"
                      @click="pick(commit.id)"
                      @contextmenu.prevent="openMenu($event, commit.id)"
                    >
                      <!-- Git 提交网络图 -->
                      <td class="cell-graph">
                        <commit-graph
                          :commit="commit"
                          :row="commitStore.rowFor(commit.id)"
                          :lanes="commitStore.graphLanes"
                        />
                      </td>
                      <!-- 分支引用 -->
                      <td v-if="prefs.refs" class="cell-refs">
                        <div class="refs">
                          <span v-if="commit.head && !commit.refs.some((b) => b.head)" class="badge current">HEAD</span>
                          <span
                            v-for="badge in commit.refs.slice(0, 4)"
                            :key="`${badge.kind}:${badge.name}`"
                            class="badge"
                            :class="[badge.kind, { head: badge.head }]"
                          >{{ badge.name }}</span>
                          <span v-if="commit.refs.length > 4" class="badge more">+{{ commit.refs.length - 4 }}</span>
                        </div>
                      </td>
                      <!-- 提交信息 -->
                      <td class="cell-subject">
                        <n-tag :type="typeColor(commit)" size="small" :bordered="false" class="type-tag">{{ typeLabel(commit) }}</n-tag>
                        <n-tag v-if="commit.scope" size="small" :bordered="false">{{ commit.scope }}</n-tag>
                        <n-tag v-if="commit.breaking" type="error" size="small" :bordered="false">!</n-tag>
                        <n-tag v-if="commit.merge" type="warning" size="small" :bordered="false">merge</n-tag>
                        <span class="subject" :title="commit.subject">{{ commit.subject }}</span>
                      </td>
                      <!-- 哈希 -->
                      <td v-if="prefs.sha" class="cell-sha">
                        <code>{{ commit.id.slice(0, 8) }}</code>
                      </td>
                      <!-- 提交人 -->
                      <td v-if="prefs.author" class="cell-author">{{ commit.authorName }}</td>
                      <!-- 提交时间 -->
                      <td v-if="prefs.time" class="cell-time">
                        {{ dayjs(commit.time * 1000).format("YYYY-MM-DD HH:mm") }}
                      </td>
                    </tr>
                  </tbody>
                </n-table>
              </n-scrollbar>
            </template>

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
          </div>
        </template>

        <template #2>
          <div v-if="detail.sha" class="detail-pane">
            <div class="detail-head">
              <span class="detail-title">提交详情</span>
              <n-button quaternary size="tiny" title="关闭" @click="detail.close()">✕</n-button>
            </div>
            <n-scrollbar class="detail-scroll">
              <commit-detail :repo-id="repoId" :commit="selectedCommit" />
            </n-scrollbar>
          </div>
        </template>
      </n-split>

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

.split {
  flex: 1;
  min-height: 0;
}
.split :deep(.n-split-pane) {
  height: 100%;
  overflow: hidden;
}

.list-pane {
  display: flex;
  flex-direction: column;
  gap: 8px;
  height: 100%;
  min-width: 0;
}

.commit-scroll {
  flex: 1;
  min-height: 0;
}

.status-line {
  flex: none;
  min-height: 24px;
}

.detail-pane {
  display: flex;
  flex-direction: column;
  height: 100%;
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
.detail-title { font-size: 12px; font-weight: 600; color: var(--text-2); }
.detail-scroll {
  flex: 1;
  min-height: 0;
}
.list-pane.full {
  display: flex;
  flex-direction: column;
  gap: 8px;
  height: 100%;
  min-width: 0;
}

/* 提交表 */
.commit-table {
  font-size: 12px;
  width: 100%;
  table-layout: fixed;
}

.commit-table :deep(thead th) {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-3);
  background: var(--surface);
  border-bottom: 1px solid var(--border);
  padding: 4px 8px;
  position: sticky;
  top: 0;
  z-index: 1;
}

.commit-table :deep(tbody tr) {
  cursor: pointer;
  transition: background 120ms ease;
}

.commit-table :deep(tbody tr:hover) {
  background: var(--surface-hover);
}

.commit-table :deep(tbody tr.selected) {
  background: var(--surface-selected);
  box-shadow: inset 2px 0 0 var(--accent);
}

.commit-table :deep(td) {
  padding: 0 8px;
  border-bottom: none;
  height: 30px;
  vertical-align: middle;
  box-shadow: inset 0 -1px 0 var(--border-soft);
}

.col-graph { width: 60px; padding: 0 !important; }
.cell-graph { padding: 0 !important; vertical-align: middle; border-bottom: none !important; line-height: 0; }
.commit-table :deep(td.cell-graph) { box-shadow: none !important; }


.col-branch { width: 180px; }
.cell-refs { white-space: nowrap; }

.refs { display: flex; align-items: center; gap: 3px; max-width: 200px; overflow: hidden; }

.badge {
  flex: none; max-width: 120px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;
  padding: 0 6px; border-radius: 9px; font-size: 11px; line-height: 15px;
}
.badge.branch { color: var(--badge-branch-text); background: var(--badge-branch-bg); }
.badge.remote { color: var(--badge-remote-text); background: var(--badge-remote-bg); }
.badge.tag { color: var(--badge-tag-text); background: var(--badge-tag-bg); }
.badge.current { color: var(--badge-head-text); background: var(--badge-head-bg); }
.badge.head { font-weight: 600; box-shadow: inset 0 0 0 1px currentColor; }
.badge.more { color: var(--badge-more-text); background: var(--badge-more-bg); }

.cell-subject { padding: 0 8px; display: flex; align-items: center; gap: 4px; overflow: hidden; }
.type-tag { flex: none; min-width: 44px; justify-content: center; }
.subject {
  margin-left: 6px;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.col-sha { width: 80px; }
.col-sha, .cell-sha { font-family: var(--font-mono); }
.cell-sha code { font-size: 11px; color: var(--text-3); }

.col-author { width: 100px; }
.col-author, .cell-author { white-space: nowrap; }
.cell-author { font-size: 11px; color: var(--text-2); }

.col-time { width: 140px; }
.col-time, .cell-time { white-space: nowrap; }
.cell-time { font-size: 11px; color: var(--text-3); }

/* 传出的更改分节标题 */
.outgoing-header-row td {
  background: var(--surface-hover);
  border-bottom: 1px solid var(--border);
  font-size: 12px;
  padding: 0 10px;
  height: 30px;
}
.outgoing-marker {
  display: inline-block; width: 7px; height: 7px; border-radius: 50%;
  background: var(--outgoing); margin-right: 6px; vertical-align: middle;
}
.outgoing-label { font-weight: 600; color: var(--text-1); margin-right: 8px; }
.outgoing-branch {
  padding: 0 6px; border-radius: 9px; font-size: 11px; line-height: 15px;
  color: var(--badge-branch-text); background: var(--badge-branch-bg);
}

.graph-waiting {
  display: flex; align-items: center; gap: 10px; height: 120px; padding: 0 16px;
}

.placeholder { padding-top: 20vh; }
.muted { font-size: 12px; opacity: 0.7; }
.scanned { color: var(--warn-text); opacity: 1; }
.raw-output { margin: 8px 0 0; white-space: pre-wrap; font-size: 12px; opacity: 0.8; }
</style>
