<script setup lang="ts">
/**
 * 分支管理页（表格风格）。
 *
 * 从原弹窗 RefsModal 迁移为独立路由页面，不再遮挡其他内容，
 * 表格有更多空间展示分支详情。
 */
import { computed, ref, watch, onMounted } from "vue";
import dayjs from "dayjs";
import {
  NButton,
  NCheckbox,
  NIcon,
  NInput,
  NPopconfirm,
  NSpace,
  NTabPane,
  NTabs,
  useDialog,
} from "naive-ui";
import RefreshIcon from "@vicons/tabler/es/Refresh";
import TrashIcon from "@vicons/tabler/es/Trash";
import GitBranchIcon from "@vicons/tabler/es/GitBranch";
import NewBranchModal from "@/components/NewBranchModal.vue";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";

const repos = useReposStore();
const writes = useWriteStore();
const dialog = useDialog();

interface BranchDetail {
  name: string;
  upstream: string | null;
  target: string;
  time: number;
  authorName: string;
  upstreamGone: boolean;
}

const branchDetails = ref<BranchDetail[]>([]);
const newBranchVisible = ref(false);
const loading = ref(false);

const ACTIVE_DAYS = 30;
function isActive(time: number): boolean {
  return dayjs().diff(dayjs(time * 1000), "day") <= ACTIVE_DAYS;
}

async function fetchBranchDetails() {
  const repoId = repos.currentId;
  if (repoId === null) return;
  loading.value = true;
  try {
    // 预留：后端实现后改为真实调用
    // const data = await call<BranchDetail[]>("refs_branch_details", { id: repoId });
    // branchDetails.value = data;

    const branches = repos.refs.filter((r) => r.kind === "branch");
    branchDetails.value = branches.map((b) => ({
      name: b.name,
      upstream: b.upstream,
      target: b.target,
      time: 0,
      authorName: "",
      upstreamGone: b.upstreamGone,
    }));
  } finally {
    loading.value = false;
  }
}

onMounted(() => void fetchBranchDetails());
watch(() => repos.currentId, () => void fetchBranchDetails());

const activeTab = ref("all");
const searchQuery = ref("");
const sortBy = ref<"time" | null>(null);
const sortDesc = ref(true);

const activeCount = computed(() => branchDetails.value.filter((b) => isActive(b.time)).length);
const inactiveCount = computed(() => branchDetails.value.length - activeCount.value);

const filteredBranches = computed(() => {
  let list = branchDetails.value;
  if (activeTab.value === "active") list = list.filter((b) => isActive(b.time));
  else if (activeTab.value === "inactive") list = list.filter((b) => !isActive(b.time));
  const q = searchQuery.value.trim().toLowerCase();
  if (q) list = list.filter((b) => b.name.toLowerCase().includes(q));
  if (sortBy.value === "time") {
    list = [...list].sort((a, b) => sortDesc.value ? b.time - a.time : a.time - b.time);
  }
  return list;
});

const checkedKeys = ref<Set<string>>(new Set());
const allChecked = computed(() =>
  filteredBranches.value.length > 0 && filteredBranches.value.every((b) => checkedKeys.value.has(b.name)),
);
const someChecked = computed(() =>
  filteredBranches.value.some((b) => checkedKeys.value.has(b.name)) && !allChecked.value,
);

function toggleAll(value: boolean) {
  if (value) filteredBranches.value.forEach((b) => checkedKeys.value.add(b.name));
  else checkedKeys.value.clear();
}

function toggleRow(name: string, value: boolean) {
  if (value) checkedKeys.value.add(name);
  else checkedKeys.value.delete(name);
}

const selectedRow = ref<string | null>(null);

async function deleteBranch(name: string) {
  const probe = await writes.probeDeletable(name);
  if (probe === null) return;
  dialog.warning({
    title: `删除分支 ${name}？`,
    content: probe.merged
      ? "已合并进别的分支，删除不会丢提交。"
      : `有 ${probe.unmerged ?? "?"} 个未合并的提交，删除会丢掉它们。`,
    positiveText: "删除",
    negativeText: "取消",
    onPositiveClick: async () => {
      await writes.deleteBranch(name, !probe.merged);
      void fetchBranchDetails();
    },
  });
}

function onRefresh() {
  void repos.refreshRefs().then(() => void fetchBranchDetails());
}

function formatTime(t: number): string {
  if (t === 0) return "—";
  return dayjs(t * 1000).format("YYYY-MM-DD HH:mm:ss");
}

const currentBranch = computed(() => repos.repoState?.branch ?? null);
</script>

<template>
  <div class="branch-page">
    <div class="branch-mgr">
      <!-- 头部 -->
      <div class="mgr-header">
        <span class="mgr-title">分支管理</span>
        <div class="mgr-header-right">
          <n-tabs v-model:value="activeTab" type="segment" size="small" class="mgr-tabs">
            <n-tab-pane name="all" :tab="`全部 (${branchDetails.length})`" />
            <n-tab-pane name="active" :tab="`活跃 (${activeCount})`" />
            <n-tab-pane name="inactive" :tab="`不活跃 (${inactiveCount})`" />
          </n-tabs>
        </div>
      </div>

      <!-- 筛选栏 -->
      <div class="mgr-filter">
        <n-space size="small" align="center">
          <n-button quaternary size="small" title="刷新" :loading="loading" @click="onRefresh">
            <template #icon><n-icon :component="RefreshIcon" size="14" /></template>
          </n-button>
          <n-button size="small" type="primary" @click="newBranchVisible = true">
            新建分支
          </n-button>
        </n-space>
        <n-input v-model:value="searchQuery" size="small" clearable placeholder="关键字" class="mgr-search" />
      </div>

      <!-- 表格 -->
      <div class="mgr-table-wrap">
        <table class="mgr-table">
          <thead>
            <tr>
              <th class="col-check">
                <n-checkbox :checked="allChecked" :indeterminate="someChecked" @update:checked="toggleAll" />
              </th>
              <th class="col-name">分支名称</th>
              <th class="col-track">跟踪分支</th>
              <th class="col-hash">最新提交</th>
              <th class="col-time sortable" @click="sortBy = 'time'; sortDesc = !sortDesc">
                提交时间
                <span v-if="sortBy === 'time'" class="sort-arrow">{{ sortDesc ? '↓' : '↑' }}</span>
              </th>
              <th class="col-author">提交人</th>
              <th class="col-actions">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="branch in filteredBranches"
              :key="branch.name"
              class="mgr-row"
              :class="{ selected: selectedRow === branch.name, current: branch.name === currentBranch }"
              @click="selectedRow = branch.name"
            >
              <td class="col-check" @click.stop>
                <n-checkbox :checked="checkedKeys.has(branch.name)" @update:checked="(v: boolean) => toggleRow(branch.name, v)" />
              </td>
              <td class="col-name">
                <div class="name-cell">
                  <n-icon :component="GitBranchIcon" size="13" class="branch-icon" />
                  <span class="branch-name">{{ branch.name }}</span>
                  <span v-if="branch.name === currentBranch" class="current-badge">HEAD</span>
                </div>
              </td>
              <td class="col-track">
                <span v-if="branch.upstream" :class="{ gone: branch.upstreamGone }">{{ branch.upstream }}</span>
                <span v-else class="muted">无远程跟踪分支</span>
              </td>
              <td class="col-hash"><code>{{ branch.target.slice(0, 8) }}</code></td>
              <td class="col-time">{{ formatTime(branch.time) }}</td>
              <td class="col-author">{{ branch.authorName || '—' }}</td>
              <td class="col-actions" @click.stop>
                <n-popconfirm positive-text="删除" negative-text="取消" @positive-click="deleteBranch(branch.name)">
                  <template #trigger>
                    <n-button quaternary size="tiny" type="error" :disabled="branch.name === currentBranch">
                      <template #icon><n-icon :component="TrashIcon" size="14" /></template>
                    </n-button>
                  </template>
                  确认删除分支 {{ branch.name }}？
                </n-popconfirm>
              </td>
            </tr>
            <tr v-if="filteredBranches.length === 0">
              <td colspan="7" class="empty-row">没有匹配的分支</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
    <new-branch-modal v-model:show="newBranchVisible" />
  </div>
</template>

<style scoped>
.branch-page {
  height: 100%;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.branch-mgr {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--surface);
  border-radius: 6px;
  border: 1px solid var(--border);
  overflow: hidden;
}

.mgr-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px 8px;
  border-bottom: 1px solid var(--border);
  flex: none;
}
.mgr-title {
  font-size: 15px;
  font-weight: 700;
  color: var(--text-1);
}
.mgr-header-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.mgr-filter {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 16px;
  border-bottom: 1px solid var(--border);
  flex: none;
}
.mgr-search { width: 200px; }

.mgr-table-wrap {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.mgr-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
  table-layout: fixed;
}

.mgr-table thead th {
  position: sticky;
  top: 0;
  z-index: 1;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-3);
  background: var(--surface);
  border-bottom: 1px solid var(--border);
  padding: 8px 12px;
  text-align: left;
  white-space: nowrap;
}

.mgr-table th.sortable { cursor: pointer; user-select: none; }
.sort-arrow { margin-left: 2px; color: var(--accent); }

.mgr-table tbody tr {
  cursor: pointer;
  transition: background 80ms ease;
  border-bottom: 1px solid var(--border-soft);
}
.mgr-table tbody tr:hover { background: var(--surface-hover); }
.mgr-table tbody tr.selected { background: var(--surface-selected); }
.mgr-table tbody tr.current { background: var(--surface-selected); }

.mgr-table td {
  padding: 6px 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  vertical-align: middle;
}

.col-check { width: 40px; text-align: center; }
.col-name { width: 160px; }
.col-track { width: 140px; }
.col-hash { width: 90px; }
.col-time { width: 160px; }
.col-author { width: 100px; }
.col-actions { width: 50px; text-align: center; }

.col-hash code { font-family: var(--font-mono); font-size: 11px; color: var(--text-3); }

.name-cell { display: flex; align-items: center; gap: 5px; }
.branch-icon { flex: none; color: var(--text-3); }
.branch-name { overflow: hidden; text-overflow: ellipsis; }
.current-badge {
  flex: none; font-size: 10px; font-weight: 600; padding: 0 4px;
  border-radius: 3px; color: var(--accent); background: var(--surface-selected);
}

.col-track .gone { color: var(--warn-text); text-decoration: line-through; }
.muted { color: var(--text-3); opacity: 0.6; }
.empty-row { text-align: center; padding: 40px 0; color: var(--text-3); }
</style>