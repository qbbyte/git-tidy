<script setup lang="ts">
import { computed, ref } from "vue";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { GitTidyError } from "@/api/client";
import { openTerminal } from "@/api/prefs";
import {
  NButton,
  NCollapse,
  NCollapseItem,
  NDropdown,
  NEmpty,
  NIcon,
  NInput,
  NScrollbar,
  NTag,
  type DropdownOption,
} from "naive-ui";
import Settings from "@vicons/tabler/es/Settings";
import FolderIcon from "@vicons/tabler/es/Folder";
import GitBranchIcon from "@vicons/tabler/es/GitBranch";
import CloudIcon from "@vicons/tabler/es/Cloud";
import TagIcon from "@vicons/tabler/es/Tag";
import DotsIcon from "@vicons/tabler/es/DotsVertical";
import { useReposStore } from "@/stores/repos";
import { INTERRUPT_LABEL } from "@/api/refs";
import type { Repo } from "@/api/repo";

/**
 * 常驻左栏（参考 UGit 风格）：
 * - 仓库列表：卡片式行，彩色图标 + 仓库名 + 状态指示
 * - 分支/标签：NCollapse 折叠区
 * - 当前仓库信息与操作面板
 */
const repos = useReposStore();

const emit = defineEmits<{
  (e: "open-settings"): void;
  (e: "open-add-repo"): void;
}>();

const editing = ref<{ id: number; name: string } | null>(null);
const repoQuery = ref("");
/** 当前展开的折叠面板 */
const activeSections = ref<string[]>(["branches"]);

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

    <!-- 分支 / 标签 -->
    <n-collapse v-if="repos.currentId !== null" v-model:expanded-names="activeSections" :display-directive="'show'" arrow-placement="left" class="refs-collapse">
      <n-collapse-item v-if="localBranches.length" name="branches">
        <template #header>
          <n-icon :component="GitBranchIcon" size="14" />
          <span class="collapse-label">本地分支（{{ localBranches.length }}）</span>
        </template>
        <div class="ref-list">
          <div
            v-for="ref in localBranches"
            :key="ref.fullName"
            class="ref-item"
          >
            <span class="ref-name">{{ ref.name }}</span>
            <n-tag v-if="repos.repoState?.branch === ref.name" size="tiny" type="info" :bordered="false">HEAD</n-tag>
            <span v-if="ref.ahead || ref.behind" class="ahead-behind">↑{{ ref.ahead ?? 0 }} ↓{{ ref.behind ?? 0 }}</span>
          </div>
        </div>
      </n-collapse-item>

      <n-collapse-item v-if="remoteBranches.length" name="remote">
        <template #header>
          <n-icon :component="CloudIcon" size="14" />
          <span class="collapse-label">远程分支（{{ remoteBranches.length }}）</span>
        </template>
        <div class="ref-list">
          <div v-for="ref in remoteBranches" :key="ref.fullName" class="ref-item remote">
            <span class="ref-name">{{ ref.name }}</span>
          </div>
        </div>
      </n-collapse-item>

      <n-collapse-item v-if="tags.length" name="tags">
        <template #header>
          <n-icon :component="TagIcon" size="14" />
          <span class="collapse-label">标签（{{ tags.length }}）</span>
        </template>
        <div class="ref-list">
          <div v-for="ref in tags" :key="ref.fullName" class="ref-item tag">
            <span class="ref-name">{{ ref.name }}</span>
          </div>
        </div>
      </n-collapse-item>
    </n-collapse>

    </n-scrollbar>

    <!-- 设置（sticky 固定底部） -->
    <div class="foot">
      <n-button quaternary size="small" block @click="emit('open-settings')">
        <template #icon><n-icon :component="Settings" size="14" /></template>
        设置
      </n-button>
    </div>
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
  box-shadow: inset 2px 0 0 var(--accent);
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

/* ---- 折叠区 ---- */
.refs-collapse { margin: 8px 0 0; }
.refs-collapse :deep(.n-collapse-item__header) {
  padding: 4px 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-2);
  align-items: center;
}
.refs-collapse :deep(.n-collapse-item__header:hover) {
  color: var(--text-1);
}
.collapse-label { margin-left: 4px; }
.refs-collapse :deep(.n-collapse-item__content-inner) {
  padding-top: 2px;
}

/* 引用列表 */
.ref-list {
  display: flex;
  flex-direction: column;
  gap: 0;
  padding: 0 4px 4px;
}
.ref-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 8px;
  border-radius: 5px;
  font-size: 12px;
  cursor: default;
}
.ref-item:hover { background: var(--surface-hover); }
.ref-item .ref-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-2);
}
.ref-item.remote .ref-name { color: var(--text-3); }
.ahead-behind { font-size: 10px; color: var(--text-3); }

/* ---- 操作面板 ---- */

/* ---- 底栏 ---- */
.foot {
  flex: none;
  display: flex;
  padding: 6px 2px;
  border-top: 1px solid var(--border-soft);
  background: var(--surface-app);
}
</style>
