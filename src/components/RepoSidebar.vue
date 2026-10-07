<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { NButton, NEmpty, NInput, NSpace, NTag } from "naive-ui";
import { useReposStore } from "@/stores/repos";
import { INTERRUPT_LABEL, type Ref } from "@/api/refs";
import type { Repo } from "@/api/repo";

/**
 * 常驻左栏：仓库注册表 + 当前仓库的只读信息（照 Fork 的侧栏职责）。
 * 主区的历史页和提交页都靠这里切仓库，所以它不随页签卸载。
 */
const repos = useReposStore();

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
 * 本仓库的全部引用。这里只读不写：切换分支会动 HEAD 和工作区，按第五节第 17 条
 * 要等 `write_guard` 的还原点先落地（§7.10 的写半边），所以列表上不给点击入口。
 */
const localBranches = computed(() => repos.refs.filter((item) => item.kind === "branch"));
const remoteBranches = computed(() => repos.refs.filter((item) => item.kind === "remote"));
const tagRefs = computed(() => repos.refs.filter((item) => item.kind === "tag"));

const currentBranch = computed(() => repos.repoState?.branch ?? repos.info?.branch ?? null);

/** 远程分支和标签默认收起：一个仓库几百个远程跟踪分支是常态，侧栏不该被它们挤没 */
const showRemote = ref(false);
const showTags = ref(false);

/** 与上方"跟踪"那一行同一套说法：git 没给数字时是"已同步"，不是 0/0 */
function trackOf(item: Ref) {
  if (item.upstreamGone) return "远程已删";
  if (item.ahead === null && item.behind === null) return item.upstream ? "已同步" : "";
  return `↑${item.ahead ?? 0} ↓${item.behind ?? 0}`;
}

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

onMounted(() => {
  if (repos.repos.length === 0) repos.load();
});
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
            <n-tag v-if="repo.id !== (editing?.id ?? -1)" :type="kindTag(repo).type" size="tiny">
              {{ kindTag(repo).text }}
            </n-tag>
          </div>
          <div class="row-actions">
            <template v-if="editing && editing.id === repo.id">
              <n-input
                v-model:value="editing.name"
                size="tiny"
                placeholder="留空回落到目录名"
                @keyup.enter="saveRename"
                @click.stop
              />
              <n-button size="tiny" type="primary" @click.stop="saveRename">存</n-button>
              <n-button size="tiny" quaternary @click.stop="editing = null">取消</n-button>
            </template>
            <template v-else>
              <n-button
                v-if="repo.kind === 'browse'"
                size="tiny"
                secondary
                type="primary"
                :disabled="busy"
                @click.stop="repos.materialize(repo.id)"
              >
                克隆
              </n-button>
              <n-button size="tiny" quaternary @click.stop="startRename(repo)">重命名</n-button>
              <n-button size="tiny" quaternary type="error" @click.stop="removeRepo(repo)">
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
      全部引用读自 §7.3 那一次 for-each-ref，不另起进程。只列不切：
      切换分支要动 HEAD 和工作区，等批 6 的还原点（write_guard）落地后按 §7.10 给。
    -->
    <div v-if="localBranches.length || remoteBranches.length || tagRefs.length" class="block">
      <div class="block-title">分支 {{ localBranches.length }}</div>
      <div class="ref-list">
        <div
          v-for="item in localBranches"
          :key="item.fullName"
          class="ref-row"
          :class="{ current: item.name === currentBranch }"
        >
          <span class="ref-dot" :class="{ on: item.name === currentBranch }"></span>
          <span class="ref-name" :title="item.fullName">{{ item.name }}</span>
          <span class="ref-track" :class="{ warn: item.upstreamGone }">{{ trackOf(item) }}</span>
        </div>
      </div>

      <template v-if="remoteBranches.length">
        <n-button size="tiny" quaternary @click="showRemote = !showRemote">
          远程分支 {{ remoteBranches.length }} {{ showRemote ? "▾" : "▸" }}
        </n-button>
        <div v-if="showRemote" class="ref-list">
          <div v-for="item in remoteBranches" :key="item.fullName" class="ref-row">
            <span class="ref-name" :title="item.fullName">{{ item.name }}</span>
          </div>
        </div>
      </template>

      <template v-if="tagRefs.length">
        <n-button size="tiny" quaternary @click="showTags = !showTags">
          标签 {{ tagRefs.length }} {{ showTags ? "▾" : "▸" }}
        </n-button>
        <div v-if="showTags" class="ref-list">
          <div v-for="item in tagRefs" :key="item.fullName" class="ref-row">
            <span class="ref-name" :title="item.fullName">{{ item.name }}</span>
          </div>
        </div>
      </template>
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
  border-right: 1px solid #e5e8ee;
  background: #fbfcfe;
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
  color: #1f5aa8;
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
  color: #8a94a6;
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
  background: #eef2f8;
}

.repo-row.active {
  background: #e3ecf7;
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
  background: #1f5aa8;
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
  color: #b45309;
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
