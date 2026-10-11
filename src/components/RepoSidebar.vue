<script setup lang="ts">
import { computed, h, reactive, ref } from "vue";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { GitTidyError } from "@/api/client";
import { openTerminal } from "@/api/prefs";
import {
  NButton,
  NDropdown,
  NEmpty,
  NIcon,
  NInput,
  NScrollbar,
  NTree,
  type DropdownOption,
  type TreeOption,
  useDialog,
} from "naive-ui";
import Settings from "@vicons/tabler/es/Settings";
import FolderIcon from "@vicons/tabler/es/Folder";
import GitBranchIcon from "@vicons/tabler/es/GitBranch";
import CloudIcon from "@vicons/tabler/es/Cloud";
import TagIcon from "@vicons/tabler/es/Tag";
import DotsIcon from "@vicons/tabler/es/DotsVertical";
import MonitorIcon from "@vicons/tabler/es/DeviceDesktop";
import CheckIcon from "@vicons/tabler/es/Check";
import PlusIcon from "@vicons/tabler/es/Plus";
import RefreshIcon from "@vicons/tabler/es/Refresh";
import ListIcon from "@vicons/tabler/es/List";
import { useReposStore } from "@/stores/repos";
import { INTERRUPT_LABEL } from "@/api/refs";
import type { Repo } from "@/api/repo";
import { useWriteStore } from "@/stores/write";
import type { Ref as GitRef } from "@/api/refs";

const repos = useReposStore();
const writes = useWriteStore();
const dialog = useDialog();

const emit = defineEmits<{
  (e: "open-settings"): void;
  (e: "open-add-repo"): void;
  (e: "open-new-branch"): void;
  (e: "open-branch-manager"): void;
}>();

const editing = ref<{ id: number; name: string } | null>(null);
const repoQuery = ref("");
const branchFilter = ref("");

const SEARCH_THRESHOLD = 8;

const filteredRepos = computed(() => {
  const q = repoQuery.value.trim().toLowerCase();
  if (!q) return repos.repos;
  return repos.repos.filter(
    (r) => r.name.toLowerCase().includes(q) || r.path.toLowerCase().includes(q),
  );
});

const busy = computed(() => repos.loading || repos.progress !== null);

/* ---- 钉住区 ---- */
const branchText = computed(() => {
  const state = repos.repoState;
  if (!state) return repos.info?.branch ?? "游离 HEAD";
  if (state.branch) return state.branch;
  if (state.interruptBranch) return `${state.interruptBranch}（${INTERRUPT_LABEL[state.interrupt]}）`;
  return "游离 HEAD";
});

const trackText = computed(() => {
  const state = repos.repoState;
  if (!state || state.branch === null) return "";
  if (!repos.info?.headCommit) return "";
  if (state.upstreamGone) return "远程分支已删除";
  if (!state.upstream) return "未设置跟踪分支";
  if (state.ahead === null) return `${state.upstream}（已同步）`;
  return `${state.upstream}　↑${state.ahead ?? 0} ↓${state.behind ?? 0}`;
});

const trackStale = computed(() => {
  const state = repos.repoState;
  if (!state) return false;
  return state.upstreamGone || (state.behind ?? 0) > 0;
});

/* ---- 分支 / 远程 / 标签数据 ---- */
const localBranches = computed(() => repos.refs.filter((r) => r.kind === "branch"));
const remoteBranches = computed(() => repos.refs.filter((r) => r.kind === "remote"));
const tags = computed(() => repos.refs.filter((r) => r.kind === "tag"));

const remoteName = computed(() => {
  const remote = remoteBranches.value[0]?.name.split("/")[0];
  return remote ?? "origin";
});

/* ---- 树展开键 ---- */
const treeExpandedKeys = ref<string[]>(["group:local", "group:remote", "group:tags"]);

/* ---- 树数据 ---- */
const treeOptions = computed<TreeOption[]>(() => {
  const q = branchFilter.value.trim().toLowerCase();
  const match = (r: GitRef) => !q || r.name.toLowerCase().includes(q);
  const currentBranch = repos.repoState?.branch ?? null;
  const result: TreeOption[] = [];

  const localFiltered = localBranches.value.filter(match);
  result.push({
    key: "group:local",
    label: `本地（${localFiltered.length}）`,
    _type: "group-local",
    children: localFiltered.map((r) => ({
      key: `branch:${r.fullName}`,
      label: r.name,
      isLeaf: true,
      _type: "branch",
      _ref: r,
      _current: r.name === currentBranch,
    }) as TreeOption),
  });

  const remoteFiltered = remoteBranches.value.filter(match);
  if (remoteFiltered.length) {
    const groups: Record<string, GitRef[]> = {};
    for (const r of remoteFiltered) {
      const remote = r.name.split("/")[0];
      (groups[remote] ??= []).push(r);
    }
    result.push({
      key: "group:remote",
      label: `远端（${remoteFiltered.length}）`,
      _type: "group-remote",
      children: Object.entries(groups).map(([remote, refs]) => ({
        key: `remote-sub:${remote}`,
        label: remote,
        _type: "remote-sub",
        children: refs.map((r) => ({
          key: `remote-branch:${r.fullName}`,
          label: r.name.split("/").slice(1).join("/"),
          isLeaf: true,
          _type: "remote-branch",
          _ref: r,
        }) as TreeOption),
      }) as TreeOption),
    });
  }

  const tagFiltered = tags.value.filter(match);
  if (tagFiltered.length) {
    result.push({
      key: "group:tags",
      label: `标签（${tagFiltered.length}）`,
      _type: "group-tags",
      children: tagFiltered.map((r) => ({
        key: `tag:${r.fullName}`,
        label: r.name,
        isLeaf: true,
        _type: "tag-node",
        _ref: r,
      }) as TreeOption),
    });
  }

  return result;
});

/* ---- 树渲染 ---- */
function renderTreePrefix({ option }: { option: TreeOption }) {
  const t = (option as Record<string, unknown>)._type as string | undefined;
  const iconMap: Record<string, unknown> = {
    "group-local": MonitorIcon,
    "group-remote": CloudIcon,
    "group-tags": TagIcon,
    "remote-sub": CloudIcon,
    branch: GitBranchIcon,
    "remote-branch": GitBranchIcon,
    "tag-node": TagIcon,
  };
  const icon = iconMap[t ?? ""];
  if (!icon) return undefined;
  if (t === "branch" && (option as Record<string, unknown>)._current) {
    return h(NIcon, { size: 14, class: "tree-prefix current" }, { default: () => h(CheckIcon) });
  }
  return h(NIcon, { size: 14, class: "tree-prefix" }, { default: () => h(icon as never) });
}

/* naive-ui 的 node-props 回调参数是 `{ option }`，不是 option 本身。
   之前直接把参数当 option 用，`_type` / `_ref` 永远是 undefined，
   这里返回的事件一个都没挂上，双击与右键菜单自然都不响应。 */
function treeNodeProps({ option }: { option: TreeOption }) {
  const t = (option as Record<string, unknown>)._type as string | undefined;
  const gitRef = (option as Record<string, unknown>)._ref as GitRef | undefined;
  if ((t === "branch" || t === "remote-branch") && gitRef) {
    return {
      onContextmenu: (e: MouseEvent) => openBranchMenu(e, gitRef),
      onDblclick: () => void confirmSwitch(gitRef),
    };
  }
  return {};
}

async function confirmSwitch(gitRef: GitRef) {
  const currentBranch = repos.repoState?.branch ?? null;
  if (gitRef.name === currentBranch) return;
  if (repos.workingFiles.length > 0) {
    dialog.warning({
      title: "工作区有改动",
      content: `工作区还有 ${repos.workingFiles.length} 处改动，切换分支会被拒绝。`,
      positiveText: "知道了",
    });
    return;
  }
  dialog.warning({
    title: "切换分支",
    content: `切换到分支 ${gitRef.name}？`,
    positiveText: "切换",
    negativeText: "取消",
    onPositiveClick: () => void writes.switchBranch(gitRef.name, false),
  });
}

/* ---- 仓库行操作 ---- */
function repoActions(repo: Repo): DropdownOption[] {
  return [
    ...(repo.kind === "browse" ? [{ label: "克隆到本地", key: "clone", disabled: busy.value }] : []),
    { label: "重命名", key: "rename" },
    { label: "复制路径", key: "copy-path" },
    { label: "在资源管理器中显示", key: "reveal" },
    { label: "在终端中打开", key: "terminal" },
    { type: "divider", key: "d1" },
    { label: "移除", key: "remove" },
  ];
}

async function onRepoAction(repo: Repo, key: string) {
  if (key === "clone") void repos.materialize(repo.id);
  else if (key === "rename") editing.value = { id: repo.id, name: repo.name };
  else if (key === "copy-path") await navigator.clipboard.writeText(repo.path);
  else if (key === "reveal") {
    try { await revealItemInDir(repo.path); }
    catch (err) { repos.error = new GitTidyError("shell_reveal_failed", String(err)); }
  } else if (key === "terminal") {
    try { await openTerminal(repo.id); }
    catch (err) { repos.error = err instanceof GitTidyError ? err : new GitTidyError("shell_failed", String(err)); }
  } else if (key === "remove") {
    if (editing.value?.id === repo.id) editing.value = null;
    repos.remove(repo.id);
  }
}

function saveRename() {
  if (!editing.value) return;
  const { id, name } = editing.value;
  editing.value = null;
  repos.rename(id, name);
}

/* ---- 分支右键菜单 ---- */
const branchMenu = reactive({ show: false, x: 0, y: 0, ref: null as GitRef | null });

function branchMenuOptions(gitRef: GitRef): DropdownOption[] {
  const isCurrent = gitRef.name === (repos.repoState?.branch ?? null);
  const opts: DropdownOption[] = [];
  if (!isCurrent) opts.push({ label: "切换到此分支", key: "switch", disabled: !writes.canWrite });
  if (gitRef.upstream && !gitRef.upstreamGone) opts.push({ label: "强推（lease）", key: "force-push", disabled: !writes.canWrite });
  opts.push({ label: "重命名", key: "rename", disabled: !writes.canWrite || isCurrent });
  opts.push({ type: "divider", key: "d1" });
  opts.push({ label: "删除", key: "delete", disabled: !writes.canWrite || isCurrent });
  return opts;
}

function openBranchMenu(event: MouseEvent, gitRef: GitRef) {
  event.preventDefault();
  branchMenu.ref = gitRef;
  branchMenu.x = event.clientX;
  branchMenu.y = event.clientY;
  branchMenu.show = true;
}

function closeBranchMenu() { branchMenu.show = false; }

async function onBranchMenuSelect(key: string) {
  const target = branchMenu.ref;
  closeBranchMenu();
  if (!target) return;
  if (key === "switch") {
    if (repos.workingFiles.length > 0) {
      dialog.warning({ title: "工作区有改动", content: `工作区还有 ${repos.workingFiles.length} 处改动，切换分支会被拒绝。`, positiveText: "知道了" });
      return;
    }
    await writes.switchBranch(target.name, false);
    return;
  }
  if (key === "rename") {
    const newName = ref(target.name);
    dialog.create({
      title: "重命名分支",
      content: () => h(NInput, { value: newName.value, "onUpdate:value": (v: string) => (newName.value = v), size: "small", placeholder: "新分支名" }),
      positiveText: "保存",
      negativeText: "取消",
      onPositiveClick: () => {
        const trimmed = newName.value.trim();
        if (trimmed && trimmed !== target.name) void writes.renameBranch(target.name, trimmed);
      },
    });
    return;
  }
  if (key === "force-push") {
    dialog.warning({
      title: `强推 ${target.name}？`, content: "远程被别人抢先推过时这次会失败（lease 保护）。",
      positiveText: "强推", negativeText: "算了",
      onPositiveClick: () => void writes.push(remoteName.value, target.name, false, true),
    }); return;
  }
  if (key === "delete") {
    const probe = await writes.probeDeletable(target.name);
    if (probe === null) return;
    dialog.warning({
      title: `删除分支 ${target.name}？`,
      content: probe.merged
        ? "已合并进别的分支，删除不会丢提交（reflog 里还找得回来）。"
        : `有 ${probe.unmerged ?? "?"} 个未合并的提交，删除会丢掉它们。`,
      positiveText: "删除", negativeText: "算了",
      onPositiveClick: () => void writes.deleteBranch(target.name, !probe.merged),
    }); return;
  }
}

/* ---- 工具栏 ---- */
function toggleExpandAll() {
  const allExpanded = treeExpandedKeys.value.length >= 3;
  treeExpandedKeys.value = allExpanded ? [] : ["group:local", "group:remote", "group:tags"];
}
</script>

<template>
  <aside class="side">
    <!-- 搜索 -->
    <div v-if="repos.repos.length > SEARCH_THRESHOLD" class="search-wrap">
      <n-input
        v-model:value="repoQuery"
        size="small"
        clearable
        placeholder="过滤仓库…"
      />
    </div>

    <n-scrollbar class="main-scroll">
      <!-- 仓库列表 -->
    <div class="section-head">
      <span class="section-label">仓库</span>
      <n-button size="tiny" quaternary type="primary" @click="emit('open-add-repo')">+ 添加</n-button>
    </div>
    <div class="repo-list">
      <div
        v-for="repo in filteredRepos"
        :key="repo.id"
        class="repo-card"
        :class="{ active: repo.id === repos.currentId }"
        @click="repos.select(repo.id)"
      >
        <template v-if="editing && editing.id === repo.id">
          <n-input
            v-model:value="editing.name"
            size="tiny"
            placeholder="仓库名"
            @keyup.enter="saveRename"
            @click.stop
          />
          <n-button size="tiny" type="primary" @click.stop="saveRename">✓</n-button>
          <n-button size="tiny" quaternary @click.stop="editing = null">✕</n-button>
        </template>
        <template v-else>
          <n-icon :component="FolderIcon" size="16" class="repo-icon" :class="repo.kind" />
          <div class="repo-info">
            <span class="repo-name" :title="repo.path">{{ repo.name }}</span>
            <span class="repo-kind">{{ repo.kind === "worktree" ? "本地" : "只读" }}</span>
          </div>
          <n-dropdown
            trigger="click"
            placement="bottom-end"
            :options="repoActions(repo)"
            @select="onRepoAction(repo, $event as string)"
          >
            <button class="repo-dots" @click.stop>
              <n-icon :component="DotsIcon" size="14" />
            </button>
          </n-dropdown>
        </template>
      </div>
      <n-empty v-if="!repos.repos.length" size="small" description="还没有仓库" style="margin: 20px 0" />
    </div>

    <!-- 当前仓库信息 -->
    <div v-if="repos.info" class="pinned" :class="{ switching: repos.switching }">
      <div class="pinned-title">{{ repos.current?.name }}</div>
      <div class="pinned-line">
        <span class="branch" :title="branchText">{{ branchText }}</span>
        <span v-if="trackText" class="track" :class="{ warn: trackStale }">{{ trackText }}</span>
      </div>
      <div class="pinned-line">
        <code v-if="repos.info.headCommit">{{ repos.info.headCommit.slice(0, 8) }}</code>
        <span v-else class="muted">空仓库</span>
        <span class="sep">·</span>
        <span :class="{ strong: repos.workingFiles.length > 0 }">
          {{ repos.workingFiles.length }} 待提交
        </span>
      </div>
      <button v-if="repos.current?.kind === 'browse'" class="clone-btn" @click="repos.materialize(repos.current.id)">
        克隆到本地
      </button>
    </div>

    <!-- 分支 / 远端 / 标签树 -->
    <div v-if="repos.currentId !== null" class="branch-panel">
      <div class="branch-toolbar">
        <span class="branch-toolbar-title">分支</span>
        <div class="branch-toolbar-actions">
          <n-button size="tiny" quaternary title="新建分支" @click="emit('open-new-branch')">
            <template #icon><n-icon :component="PlusIcon" size="14" /></template>
          </n-button>
          <n-button size="tiny" quaternary title="刷新" @click="repos.refreshAll()">
            <template #icon><n-icon :component="RefreshIcon" size="14" /></template>
          </n-button>
          <n-button size="tiny" quaternary title="展开/折叠全部" @click="toggleExpandAll">
            <template #icon><n-icon :component="ListIcon" size="14" /></template>
          </n-button>
          <n-button size="tiny" quaternary title="分支管理" @click="emit('open-branch-manager')">
            <template #icon><n-icon :component="Settings" size="14" /></template>
          </n-button>
        </div>
      </div>
      <div class="branch-filter">
        <n-input v-model:value="branchFilter" size="tiny" clearable placeholder="过滤分支…" />
      </div>
      <div class="branch-tree-wrap">
        <n-tree
          :data="treeOptions"
          :expanded-keys="treeExpandedKeys"
          @update:expanded-keys="treeExpandedKeys = $event"
          :render-prefix="renderTreePrefix"
          :node-props="treeNodeProps"
          block-line
          :selectable="false"
          key-field="key"
          label-field="label"
          children-field="children"
          size="small"
          :indent="16"
          :animated="false"
        />
      </div>
    </div>

    </n-scrollbar>

    <!-- 设置（sticky 固定底部） -->
    <div class="foot">
      <n-button quaternary size="small" block @click="emit('open-settings')">
        <template #icon><n-icon :component="Settings" size="14" /></template>
        设置
      </n-button>
    </div>
    <!-- 分支右键菜单 -->
    <n-dropdown
      trigger="manual"
      placement="bottom-start"
      :show="branchMenu.show"
      :x="branchMenu.x"
      :y="branchMenu.y"
      :options="branchMenu.ref ? branchMenuOptions(branchMenu.ref) : []"
      @clickoutside="closeBranchMenu"
      @select="onBranchMenuSelect"
    />
  </aside>
</template>

<style scoped>
.side {
  display: flex;
  flex-direction: column;
  height: 100%;
  padding: 6px 6px;
  gap: 4px;
  font-size: 13px;
  background: var(--surface-app);
  overflow: hidden;
}

/* ---- 搜索 ---- */
.search-wrap { flex: none; padding: 0 2px; }

/* 主滚动区：吃掉剩余高度，内容超出时自己滚 */
.main-scroll {
  flex: 1;
  min-height: 0;
  padding: 0 2px;
}

/* ---- 分区标题 ---- */
.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 6px 3px;
}
.section-label {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
}

/* ---- 仓库列表 ---- */
.repo-list {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 0 2px;
}

.repo-card {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  border-radius: 6px;
  cursor: pointer;
  transition: background 100ms ease;
}

.repo-card:hover { background: var(--surface-hover); }

.repo-card.active {
  background: var(--surface-selected);
}

.repo-icon {
  flex: none;
  color: var(--text-3);
  transition: color 120ms ease;
}
.repo-icon.worktree { color: var(--ok-text); }

.repo-info {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 5px;
}
.repo-name {
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.repo-card.active .repo-name { font-weight: 600; }
.repo-kind {
  font-size: 10px;
  color: var(--text-3);
  padding: 0 4px;
  border-radius: 3px;
  background: var(--surface-sunken);
}

.repo-dots {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: 0;
  border-radius: 4px;
  background: none;
  color: var(--text-3);
  cursor: pointer;
  opacity: 0;
  transition: opacity 120ms ease;
}
.repo-card:hover .repo-dots { opacity: 1; }
.repo-dots:hover { background: var(--border); color: var(--text-1); }

/* ---- 钉住区 ---- */
.pinned {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 6px 8px;
  margin: 8px 2px 0;
  border-radius: 6px;
  background: var(--surface);
  border: 1px solid var(--border-soft);
}
.pinned.switching { opacity: 0.5; pointer-events: none; }
.pinned-title { font-size: 13px; font-weight: 600; color: var(--text-1); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pinned-line { display: flex; align-items: center; gap: 6px; font-size: 11px; color: var(--text-2); min-width: 0; }
.pinned-line .branch { font-weight: 600; color: var(--text-1); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pinned-line .track { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pinned-line .sep { opacity: 0.4; }
.pinned-line .strong { font-weight: 600; color: var(--text-1); }
.pinned-line code { font-family: var(--font-mono); font-size: 11px; color: var(--text-3); }

.clone-btn {
  margin-top: 4px;
  padding: 3px 10px;
  border: 1px solid var(--accent);
  border-radius: 6px;
  background: transparent;
  color: var(--accent);
  font-family: inherit;
  font-size: 12px;
  cursor: pointer;
  align-self: flex-start;
  transition: all 120ms ease;
}
.clone-btn:hover { background: var(--accent); color: white; }

.muted { color: var(--text-2); opacity: 0.7; }


/* 引用列表 */
/* ---- 操作面板 ---- */

/* ---- 底栏 ---- */
.foot {
  flex: none;
  display: flex;
  padding: 6px 2px;
  border-top: 1px solid var(--border-soft);
  background: var(--surface-app);
}

/* ---- 分支面板 ---- */
.branch-panel {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-top: 8px;
}
.branch-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px 6px 2px;
}
.branch-toolbar-title {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
}
.branch-toolbar-actions {
  display: flex;
  align-items: center;
  gap: 0;
}
.branch-filter { padding: 0 6px 2px; }

.branch-tree-wrap {
  padding: 0 2px;
}

.branch-tree-wrap :deep(.n-tree) {
  font-size: 12px;
}
.branch-tree-wrap :deep(.n-tree-node-content) {
  border-radius: 4px;
  padding: 1px 4px;
  transition: background 80ms ease;
}
.branch-tree-wrap :deep(.n-tree-node-content:hover) {
  background: var(--surface-hover);
}
.branch-tree-wrap :deep(.n-tree-node--pending .n-tree-node-content) {
  background: var(--surface-hover);
}
.branch-tree-wrap :deep(.tree-prefix) {
  color: var(--text-3);
  margin-right: 4px;
  flex-shrink: 0;
}
.branch-tree-wrap :deep(.tree-prefix.current) {
  color: var(--accent);
}
</style>
