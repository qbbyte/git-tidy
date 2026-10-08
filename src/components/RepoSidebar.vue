<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { NButton, NEmpty, NInput, NSpace, NTag } from "naive-ui";
import RefPanel from "@/components/RefPanel.vue";
import StashPanel from "@/components/StashPanel.vue";
import OpJournal from "@/components/OpJournal.vue";
import ConflictResolver from "@/components/ConflictResolver.vue";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";
import { INTERRUPT_LABEL } from "@/api/refs";
import type { Repo } from "@/api/repo";

/**
 * 常驻左栏：仓库注册表 + 当前仓库的只读信息（照 Fork 的侧栏职责）。
 * 主区的历史页和提交页都靠这里切仓库，所以它不随页签卸载。
 */
const repos = useReposStore();
const writes = useWriteStore();

const path = ref("");
const url = ref("");
const showUrl = ref(false);
const editing = ref<{ id: number; name: string } | null>(null);

/** 克隆/补齐是长任务，期间所有写入口都要关掉，否则第二次点击会并发抢同一个目录 */
const busy = computed(() => repos.loading || repos.progress !== null);

/** 当前分支名。以 refs 扫出来的摘要为准，探测结果只做兜底 */
const branchText = computed(() => {
  const state = repos.repoState;
  if (!state) return repos.info?.branch ?? "游离 HEAD";
  if (state.branch) return state.branch;
  // 变基过程中 HEAD 是游离的，正在变基的分支名只有 interruptBranch 里有（§7.3）
  if (state.interruptBranch) {
    return `${state.interruptBranch}（${INTERRUPT_LABEL[state.interrupt]}）`;
  }
  return "游离 HEAD";
});

/**
 * 跟踪状态一句话说清。四种说法互斥：远程分支已删 > 没配跟踪分支 > 已同步 > 差多少。
 * ahead 为 null 是"git 没给这个数字"，已同步正好是这样（实测 [ahead] 只写非零那半）。
 */
const trackText = computed(() => {
  const state = repos.repoState;
  if (!state || state.branch === null) return "";
  // 空仓库里 HEAD 指向的是还没诞生的分支：它既没有跟踪分支，也谈不上"未设置"
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

const interruptText = computed(() =>
  repos.interrupted ? INTERRUPT_LABEL[repos.interrupt] : "正常",
);

/**
 * 本仓库的引用列表与它们的写入口都在 `RefPanel` 里：读的那一列常驻几十条，
 * 写入口悬停才露出来（§7.10 的确认强度与 §3 的期边界）。
 */

async function addPath() {
  const trimmed = path.value.trim();
  if (!trimmed) return;
  await repos.add(trimmed);
  // 失败时把路径留在框里，用户不必重新粘贴
  if (!repos.error) path.value = "";
}

async function addUrl() {
  const trimmed = url.value.trim();
  if (!trimmed) return;
  await repos.addByUrl(trimmed);
  if (!repos.error) {
    url.value = "";
    showUrl.value = false;
  }
}

// as const 是必须的：options 的字段类型被推宽成 boolean 时，
// open() 的条件返回类型会退化成 string[] | null
async function pickDirectory() {
  const picked = await open(
    { title: "选择 Git 仓库目录", directory: true, multiple: false } as const,
  );
  if (!picked) return;
  path.value = picked;
  await addPath();
}

function startRename(repo: Repo) {
  editing.value = { id: repo.id, name: repo.name };
}

function saveRename() {
  if (!editing.value) return;
  const { id, name } = editing.value;
  editing.value = null;
  repos.rename(id, name);
}

/** 移除只删注册记录，磁盘上的仓库不动（§6.1），所以不给确认弹窗 */
function removeRepo(repo: Repo) {
  if (editing.value?.id === repo.id) editing.value = null;
  repos.remove(repo.id);
}

function kindTag(repo: Repo) {
  return repo.kind === "worktree"
    ? { text: "本地", type: "success" as const }
    : { text: "只读", type: "info" as const };
}

onMounted(async () => {
  if (repos.repos.length === 0) await repos.load();
  // 远程命令的进度行可能在 promise 落地前就到了，所以订阅必须先于任何远程动作（§6.7）
  writes.watchProgress();
  if (repos.currentId !== null) await writes.loadAll();
});

/** 换仓库时 stash、日志、还原点都要跟着换：它们都是按仓库存的 */
watch(
  () => repos.currentId,
  async (id) => {
    writes.clearForRepo();
    if (id !== null) await writes.loadAll();
  },
);
</script>

<template>
  <aside class="side">
    <div class="brand">
      <span class="brand-title">Git Tidy</span>
      <span class="brand-sub">本地 Git 提交治理</span>
    </div>

    <div class="block">
      <div class="block-title">仓库</div>
      <div class="repo-list">
        <div
          v-for="repo in repos.repos"
          :key="repo.id"
          class="repo-row"
          :class="{ active: repo.id === repos.currentId }"
          @click="repos.select(repo.id)"
        >
          <div class="row-main">
            <span class="repo-name" :title="repo.path">{{ repo.name }}</span>
            <n-tag v-if="repo.id !== (editing?.id ?? -1)" :type="kindTag(repo).type" size="small">
              {{ kindTag(repo).text }}
            </n-tag>
          </div>
          <div class="row-actions">
            <template v-if="editing && editing.id === repo.id">
              <n-input
                v-model:value="editing.name"
                size="small"
                placeholder="留空回落到目录名"
                @keyup.enter="saveRename"
                @click.stop
              />
              <n-button size="small" type="primary" @click.stop="saveRename">存</n-button>
              <n-button size="small" quaternary @click.stop="editing = null">取消</n-button>
            </template>
            <template v-else>
              <n-button
                v-if="repo.kind === 'browse'"
                size="small"
                secondary
                type="primary"
                :disabled="busy"
                @click.stop="repos.materialize(repo.id)"
              >
                克隆
              </n-button>
              <n-button size="small" quaternary @click.stop="startRename(repo)">重命名</n-button>
              <n-button size="small" quaternary type="error" @click.stop="removeRepo(repo)">
                移除
              </n-button>
            </template>
          </div>
        </div>
        <n-empty v-if="!repos.repos.length" size="small" description="还没有仓库" />
      </div>

      <n-space vertical size="small">
        <n-input
          v-model:value="path"
          size="small"
          placeholder="本地仓库目录 D:\project\demo"
          @keyup.enter="addPath"
        />
        <n-space size="small">
          <n-button
            size="small"
            type="primary"
            :loading="busy"
            :disabled="!path.trim()"
            @click="addPath"
          >
            添加目录
          </n-button>
          <n-button size="small" :disabled="busy" @click="pickDirectory">浏览…</n-button>
          <n-button size="small" quaternary @click="showUrl = !showUrl">按地址</n-button>
        </n-space>
        <template v-if="showUrl">
          <n-input
            v-model:value="url"
            size="small"
            placeholder="https://… 或 git@host:org/repo.git"
            @keyup.enter="addUrl"
          />
          <n-button size="small" :loading="busy" :disabled="!url.trim()" @click="addUrl">
            只读浏览这个地址
          </n-button>
          <div class="hint">
            地址方式只下载提交对象，不建工作区：能读提交列表，不能暂存、不能提交。想要完整能力就添加之后点「克隆」。
          </div>
        </template>
      </n-space>
    </div>

    <div v-if="repos.info" class="block">
      <div class="block-title">{{ repos.current?.name }}</div>
      <div class="meta">
        <div><span class="key">分支</span>{{ branchText }}</div>
        <div v-if="trackText" :class="{ warn: trackStale }">
          <span class="key">跟踪</span>{{ trackText }}
        </div>
        <div v-if="repos.repoState" :class="{ warn: repos.interrupted }">
          <span class="key">状态</span>{{ interruptText }}
        </div>
        <div v-if="repos.info.headCommit">
          <span class="key">HEAD</span>
          <code>{{ repos.info.headCommit.slice(0, 8) }}</code>
        </div>
        <div v-else><span class="key">HEAD</span>空仓库</div>
        <div><span class="key">待提交</span>{{ repos.workingFiles.length }} 项</div>
        <div class="path" :title="repos.info.workTree">{{ repos.info.workTree }}</div>
        <div class="path muted" :title="repos.info.gitDir">{{ repos.info.gitDir }}</div>
        <div class="muted">git {{ repos.info.gitVersion }}</div>
      </div>
      <n-button
        v-if="repos.current?.kind === 'browse'"
        size="small"
        type="primary"
        secondary
        class="clone-button"
        :loading="busy"
        @click="repos.materialize(repos.current.id)"
      >
        克隆到本地
      </n-button>
    </div>

    <!--
      引用列表读自 §7.3 那一次 for-each-ref，不另起进程。
      **写**操作拆到 RefPanel：读的这一列常驻摆几十个分支，写入口悬停才露出来，
      而且每一个都要自己的确认（§7.10）。
    -->
    <ref-panel v-if="repos.canCommit" />

    <!-- 冲突解决器只在有未合并文件时占地方。§7.13：它在中断态里才有用，
         M2 阶段只有「一键退回」是唯一出口，M3 才有逐块取舍。 -->
    <div v-if="writes.hasConflicts" class="block">
      <conflict-resolver />
    </div>

    <div v-if="repos.canCommit" class="block">
      <stash-panel />
    </div>

    <div v-if="repos.currentId !== null" class="block">
      <op-journal />
    </div>
  </aside>
</template>

<style scoped>
.side {
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding: 14px;
  /* 不写 height:100%：项目没有 box-sizing 重置，100% 是内容高，再加上上下各 14px 内边距
     就变成 100vh + 28px，把文档撑出窗口，右侧因此多出一条整页滚动条。
     .shell 是 flex 行容器，交叉轴默认 stretch 已经把高度正好给到 100vh（含内边距） */
  overflow: auto;
  border-right: 1px solid var(--border);
  background: var(--surface-app);
  font-size: 12px;
}

.block {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.brand {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.brand-title {
  font-size: 15px;
  font-weight: 700;
  color: var(--accent);
}

.brand-sub {
  font-size: 11px;
  opacity: 0.65;
}

.block-title {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
}

.repo-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.repo-row {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 6px 8px;
  border-radius: 6px;
  cursor: pointer;
}

.repo-row:hover {
  background: var(--surface-hover);
}

.repo-row.active {
  background: var(--surface-selected);
}

.row-main {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.repo-name {
  font-size: 13px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.meta {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.key {
  display: inline-block;
  width: 56px;
  opacity: 0.65;
}

.ref-list {
  display: flex;
  flex-direction: column;
  gap: 1px;
  /* 分支几百条的仓库是常态：列表自己滚，别把上面的仓库列表和状态摘要顶出侧栏 */
  flex: none;
  max-height: 30vh;
  overflow: auto;
}

.ref-row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  padding: 1px 2px;
}

.ref-row.current .ref-name {
  font-weight: 600;
}

.ref-dot {
  flex: none;
  width: 6px;
  height: 6px;
  border-radius: 50%;
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

.ref-track {
  flex: none;
  font-size: 11px;
  opacity: 0.7;
}

/* .warn 下面那行是给全组件用的，这里要压掉自身的 opacity，否则警告色被冲淡到看不出来 */
.ref-track.warn {
  opacity: 1;
}

.path {
  font-size: 11px;
  opacity: 0.8;
  word-break: break-all;
}

/* 落后于远程、跟踪分支被删、有操作卡在半路——这三样都要一眼看见 */
.warn {
  color: var(--warn-text);
}

.clone-button {
  margin-top: 4px;
}

.hint {
  font-size: 11px;
  line-height: 1.5;
  opacity: 0.7;
}

.muted {
  font-size: 11px;
  opacity: 0.7;
}
</style>
