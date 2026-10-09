<script setup lang="ts">
import { computed, ref } from "vue";
import {
  NAlert,
  NButton,
  NEmpty,
  NInput,
  NPopconfirm,
  NSelect,
  NSpace,
} from "naive-ui";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";
import { usePrefsStore } from "@/stores/prefs";
import type { Ref } from "@/api/refs";

/**
 * 分支与标签的写操作（§7.10）。
 *
 * 与 §7.3 那一批只读列表分开成另一个面板，是因为**写与读的确认强度完全不同**：
 * 读只影响摆什么，写会移动 HEAD、重写工作区、删掉本地引用，所以每一个写动作
 * 都要经过自己的确认，而读的那一列不给任何写入口（§3 的期边界）。
 *
 * 所有写操作都经 `write_guard`：工作区脏、中断态、HEAD 被别处挪动，都会在 Rust 侧被拒，
 * 面板只负责把拒绝的理由显示出来，不在这里另做一套判断。
 */
const repos = useReposStore();
const writes = useWriteStore();
const prefsStore = usePrefsStore();

/** Git Flow 的三类分支（需求 7.16：自组合，不依赖系统里装了 git-flow） */
const FLOW_KINDS = ["feature", "hotfix", "release"] as const;
type FlowKind = (typeof FLOW_KINDS)[number];

function flowPrefix(kind: FlowKind): string {
  return prefsStore.prefs?.flowPrefixes[kind] ?? "";
}

const branches = computed(() => repos.refs.filter((item) => item.kind === "branch"));
const remoteBranches = computed(() => repos.refs.filter((item) => item.kind === "remote"));
const tags = computed(() => repos.refs.filter((item) => item.kind === "tag"));
const currentBranch = computed(() => repos.repoState?.branch ?? repos.info?.branch ?? null);

/** 远程名取自远程跟踪分支的前缀（origin/…）。没有远程跟踪分支时给 origin */
const remoteName = computed(() => {
  const remote = remoteBranches.value[0]?.name.split("/")[0];
  return remote ?? "origin";
});

const newBranch = ref("");
const newBranchStart = ref<string | null>(null);
const newTag = ref("");
const newTagMessage = ref("");
const renaming = ref<{ from: string; to: string } | null>(null);
const upstreamFor = ref<string | null>(null);
const upstreamChoice = ref<string | null>(null);
const confirmDelete = ref<{ name: string; typed: string } | null>(null);

const startOptions = computed(() => [
  { label: "当前 HEAD", value: "HEAD" },
  { label: "工作区基线（暂存区）", value: "" },
  ...branches.value.map((item) => ({ label: item.name, value: item.name })),
]);

const upstreamOptions = computed(() => [
  ...remoteBranches.value.map((item) => ({ label: item.name, value: item.name })),
]);

function trackOf(item: Ref) {
  if (item.upstreamGone) return "远程已删";
  if (item.ahead === null && item.behind === null) return item.upstream ? "已同步" : "";
  return `↑${item.ahead ?? 0} ↓${item.behind ?? 0}`;
}

function canWrite() {
  return writes.canWrite && !writes.busy;
}

async function createBranch(switchToIt: boolean) {
  const name = newBranch.value.trim();
  if (!name) return;
  const start = newBranchStart.value === "HEAD" ? null : newBranchStart.value || null;
  const outcome = await writes.createBranch(name, start, switchToIt);
  if (outcome !== null) {
    newBranch.value = "";
    newBranchStart.value = null;
  }
}

/** 切换前先看一眼工作区：脏的时候切换会被 Rust 侧拒，不如自己先说清 */
const dirtyHint = computed(() =>
  repos.workingFiles.length === 0
    ? ""
    : `工作区还有 ${repos.workingFiles.length} 处改动，切换分支会被拒绝（先提交或存起来）`,
);

async function switchTo(name: string) {
  await writes.switchBranch(name, false);
}

async function startRename(item: Ref) {
  renaming.value = { from: item.name, to: item.name };
}

async function saveRename() {
  if (!renaming.value) return;
  const { from, to } = renaming.value;
  renaming.value = null;
  const trimmed = to.trim();
  if (!trimmed || trimmed === from) return;
  await writes.renameBranch(from, trimmed);
}

/** 点删除先查"会丢多少"：`-D` 是真删，得先把数字摆出来 */
async function askDelete(item: Ref) {
  const probe = await writes.probeDeletable(item.name);
  if (probe === null) return;
  if (probe.merged) {
    await writes.deleteBranch(item.name, false);
    return;
  }
  confirmDelete.value = { name: item.name, typed: "" };
}

const unmergedOf = (name: string) => writes.deletable[name]?.unmerged ?? null;

async function forceDelete() {
  if (!confirmDelete.value || confirmDelete.value.typed !== confirmDelete.value.name) return;
  const name = confirmDelete.value.name;
  confirmDelete.value = null;
  await writes.deleteBranch(name, true);
}

function openUpstream(item: Ref) {
  upstreamFor.value = upstreamFor.value === item.name ? null : item.name;
  upstreamChoice.value = item.upstream;
}

async function saveUpstream(name: string) {
  const upstream = upstreamChoice.value;
  upstreamFor.value = null;
  await writes.setUpstream(name, upstream);
}

async function createTag() {
  const name = newTag.value.trim();
  if (!name) return;
  const message = newTagMessage.value.trim();
  const done = await writes.createTag(name, message === "" ? null : message);
  if (done !== null) {
    newTag.value = "";
    newTagMessage.value = "";
  }
}

/** 删远程分支属远程写：要求手输分支名（§7.12 的确认强度） */
const remoteDeleteTyped = ref("");
const remoteDeleting = ref<string | null>(null);

async function deleteRemote(name: string) {
  if (remoteDeleteTyped.value !== name) return;
  const target = remoteDeleting.value;
  remoteDeleting.value = null;
  remoteDeleteTyped.value = "";
  if (target === null) return;
  await writes.deleteRemoteBranch(remoteName.value, target);
}

function shortSha(sha: string) {
  return sha.slice(0, 8);
}
</script>

<template>
  <div class="block">
    <div class="block-title">分支 {{ branches.length }}</div>

    <n-alert v-if="writes.error" type="error" :title="writes.error.message" class="inline-alert">
      <div>错误码：{{ writes.error.code }}</div>
    </n-alert>

    <div class="ref-list">
      <div
        v-for="item in branches"
        :key="item.fullName"
        class="ref-row"
        :class="{ current: item.name === currentBranch }"
      >
        <span class="ref-dot" :class="{ on: item.name === currentBranch }"></span>
        <span class="ref-name" :title="item.fullName">
          <template v-if="renaming && renaming.from === item.name">
            <n-input
              v-model:value="renaming.to"
              size="small"
              @keyup.enter="saveRename"
              @click.stop
            />
            <n-button size="small" type="primary" @click.stop="saveRename">存</n-button>
          </template>
          <template v-else>{{ item.name }}</template>
        </span>
        <span class="ref-track" :class="{ warn: item.upstreamGone }">{{ trackOf(item) }}</span>

        <span class="ref-actions" @click.stop>
          <n-button
            v-if="item.name !== currentBranch"
            size="small"
            quaternary
            :disabled="!canWrite()"
            :title="dirtyHint"
            @click="switchTo(item.name)"
          >
            切换
          </n-button>
          <n-button size="small" quaternary :disabled="!canWrite()" @click="openUpstream(item)">
            上游
          </n-button>
          <!--
            强推只给「配了上游」的分支，而且只用 lease：对方抢先推过时它会失败，
            而裸 `--force` 会把别人的提交抹掉（§7.12）
          -->
          <n-popconfirm
            v-if="item.upstream && !item.upstreamGone"
            positive-text="强推"
            negative-text="算了"
            @positive-click="writes.push(remoteName, item.name, false, true)"
          >
            <template #trigger>
              <n-button size="small" quaternary :disabled="!canWrite()">强推</n-button>
            </template>
          </n-popconfirm>
          <n-button size="small" quaternary :disabled="!canWrite()" @click="startRename(item)">
            改名
          </n-button>
          <n-button
            size="small"
            quaternary
            type="error"
            :disabled="!canWrite() || item.name === currentBranch"
            @click="askDelete(item)"
          >
            删除
          </n-button>
        </span>

        <span v-if="unmergedOf(item.name) !== null" class="unmerged">
          未合并 {{ unmergedOf(item.name) }} 个提交
        </span>

        <div v-if="upstreamFor === item.name" class="inline-form" @click.stop>
          <n-select
            v-model:value="upstreamChoice"
            :options="upstreamOptions"
            size="small"
            clearable
            filterable
            tag
            placeholder="选一个远程跟踪分支"
          />
          <n-button size="small" type="primary" @click="saveUpstream(item.name)">设定</n-button>
        </div>
      </div>
      <n-empty v-if="!branches.length" size="small" description="还没有本地分支" />
    </div>

    <!-- 删分支：未合并时要求手输名字（真删引用，不是"取消暂存"那种可逆操作） -->
    <n-popconfirm
      v-if="confirmDelete"
      positive-text="强制删除"
      negative-text="算了"
      @positive-click="forceDelete"
    >
      <template #trigger>
        <div class="confirm">
          <span>
            删除 <b>{{ confirmDelete.name }}</b> 会丢掉
            <b>{{ unmergedOf(confirmDelete.name) ?? "?" }}</b> 个未合并的提交。
            输入分支名确认：
          </span>
          <n-input v-model:value="confirmDelete.typed" size="small" placeholder="分支名" />
        </div>
      </template>
    </n-popconfirm>

    <n-space size="small" align="center">
      <!--
        Git Flow 的三个前缀来自个人偏好（设置页可改）。这里只做“填名字”不做自动建分支：
        名字后面还要接用户自己写的内容，自动建会多出一堆 `feature/feature-x`。
      -->
      <n-space size="small" :wrap="false">
        <n-button
          v-for="flow in FLOW_KINDS"
          :key="flow"
          size="small"
          quaternary
          :disabled="!canWrite()"
          @click="newBranch = flowPrefix(flow)"
        >
          {{ flowPrefix(flow) || flow }}
        </n-button>
      </n-space>
      <n-input
        v-model:value="newBranch"
        size="small"
        placeholder="新分支名"
        @keyup.enter="createBranch(true)"
      />
      <n-select
        v-model:value="newBranchStart"
        :options="startOptions"
        size="small"
        class="start"
        placeholder="起点"
      />
      <n-button size="small" :disabled="!canWrite() || !newBranch.trim()" @click="createBranch(true)">
        建并切换
      </n-button>
      <n-button
        size="small"
        quaternary
        :disabled="!canWrite() || !newBranch.trim()"
        @click="createBranch(false)"
      >
        只建
      </n-button>
    </n-space>
    <div v-if="dirtyHint" class="hint">{{ dirtyHint }}</div>

    <template v-if="remoteBranches.length">
      <div class="block-title">远程 {{ remoteName }}</div>
      <n-space size="small" align="center">
        <n-button size="small" :loading="writes.syncAction === 'fetch'" @click="writes.fetch(remoteName)">
          抓取
        </n-button>
        <n-button
          size="small"
          :disabled="!currentBranch"
          @click="writes.pull(remoteName, 'ff_only')"
        >
          拉取（快进）
        </n-button>
        <n-button
          size="small"
          :disabled="!currentBranch"
          @click="writes.pull(remoteName, 'rebase')"
        >
          拉取（变基）
        </n-button>
        <n-button
          size="small"
          type="primary"
          :disabled="!currentBranch"
          @click="writes.push(remoteName, currentBranch ?? '', !repos.repoState?.upstream, false)"
        >
          推送
        </n-button>
      </n-space>
      <div class="hint">
        推送只推当前分支；没有上游时用 <code>-u</code> 建立跟踪。改写过历史要推，
        请在下面的分支列表里用「强推（lease）」，它不会覆盖别人抢先推的提交。
      </div>

      <div class="remote-list">
        <div v-for="item in remoteBranches" :key="item.fullName" class="ref-row">
          <span class="ref-name" :title="item.fullName">{{ item.name }}</span>
          <span class="ref-actions" @click.stop>
            <n-button
              v-if="remoteDeleting === item.name"
              size="small"
              type="error"
              :disabled="remoteDeleteTyped !== item.name"
              @click="deleteRemote(item.name)"
            >
              确认删除
            </n-button>
            <n-button
              v-else
              size="small"
              quaternary
              type="error"
              @click="remoteDeleting = item.name; remoteDeleteTyped = ''"
            >
              删远程
            </n-button>
          </span>
          <div v-if="remoteDeleting === item.name" class="inline-form" @click.stop>
            <n-input v-model:value="remoteDeleteTyped" size="small" placeholder="输入分支名确认" />
          </div>
        </div>
      </div>
    </template>

    <div class="block-title">标签 {{ tags.length }}</div>
    <div class="ref-list">
      <div v-for="item in tags" :key="item.fullName" class="ref-row">
        <span class="ref-name" :title="item.fullName">{{ item.name }}</span>
        <code v-if="item.target" class="muted">{{ shortSha(item.target) }}</code>
        <n-button
          size="small"
          quaternary
          type="error"
          :disabled="!canWrite()"
          @click="writes.deleteTag(item.name)"
        >
          删除
        </n-button>
      </div>
      <n-empty v-if="!tags.length" size="small" description="还没有标签" />
    </div>
    <n-space size="small" align="center">
      <n-input v-model:value="newTag" size="small" placeholder="标签名" />
      <n-input v-model:value="newTagMessage" size="small" placeholder="说明（留空＝轻量标签）" />
      <n-button size="small" :disabled="!canWrite() || !newTag.trim()" @click="createTag">
        打标签
      </n-button>
    </n-space>

    <div v-if="writes.syncLines.length" class="sync">
      <span class="muted">{{ writes.syncAction }}：</span>
      <code>{{ writes.syncLines[writes.syncLines.length - 1] }}</code>
    </div>

    <div v-if="writes.lastBackupRef" class="muted">
      上一步的还原点：<code>{{ writes.lastBackupRef }}</code>
    </div>
  </div>
</template>

<style scoped>
.block {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.block-title {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
  margin-top: 4px;
}

.ref-list,
.remote-list {
  display: flex;
  flex-direction: column;
}

.ref-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  padding: 2px 0;
}

.ref-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--border);
  flex: none;
}

.ref-dot.on {
  background: var(--accent);
}

.ref-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ref-track,
.unmerged {
  flex: none;
  font-size: 11px;
  opacity: 0.7;
}

.ref-track.warn,
.unmerged {
  color: var(--warn-text);
  opacity: 1;
}

.ref-actions {
  display: none;
  flex: none;
  gap: 2px;
}

/* 悬停才露出写入口：这一列一屏要放几十个分支，常驻按钮会把它挤成按钮墙 */
.ref-row:hover .ref-actions {
  display: flex;
}

.inline-form {
  flex-basis: 100%;
  display: flex;
  gap: 4px;
  padding: 2px 0 4px 12px;
}

.confirm {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
  background: var(--error-bg);
  border: 1px solid var(--error-border);
  border-radius: 6px;
  font-size: 11px;
}

.start {
  width: 130px;
}

.hint {
  font-size: 11px;
  opacity: 0.7;
  line-height: 1.5;
}

.sync {
  font-size: 11px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.inline-alert {
  margin: 0;
}

.muted {
  font-size: 11px;
  opacity: 0.7;
}
</style>