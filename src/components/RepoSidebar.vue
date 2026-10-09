<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { GitTidyError } from "@/api/client";
import { openTerminal } from "@/api/prefs";
import { useRoute, useRouter } from "vue-router";
import { NIcon } from "naive-ui";
// 逐个图标路径 import（不是整包）：整包引入会把三千多个图标全打进包里，
// 而这里只需要这一个。tree-shaking 之后单个图标 1–2 KB。
import Settings from "@vicons/tabler/es/Settings";
import { provideShell } from "@/shell";
import { RADIUS_CONTROL } from "@/styles/tokens";
import { NButton, NCollapse, NCollapseItem, NDropdown, NEmpty, NInput, NSpace, type DropdownOption } from "naive-ui";
import RefPanel from "@/components/RefPanel.vue";
import RewritePanel from "@/components/RewritePanel.vue";
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
const route = useRoute();
const router = useRouter();

/** 设置页不是页签，是左下角齿轮打开的独立视图，所以它自己管激活态 */
const settingsOpen = computed(() => route.name === "settings");
function toggleSettings() {
  if (settingsOpen.value) router.push({ name: "history" });
  else router.push({ name: "settings" });
}

const path = ref("");
const url = ref("");
const showUrl = ref(false);
const editing = ref<{ id: number; name: string } | null>(null);
const repoQuery = ref("");
/** 添加表单默认收起：它常驻时占五六行，而大多数时候根本不用添加仓库 */
const showAdd = ref(false);
/** 「仓库详情」（路径与 git 版本）默认收起 */
const showRepoInfo = ref(false);
/** 当前开着动作菜单的仓库行。菜单靠它控制自身可见 */
const repoMenu = ref<number | null>(null);
/** 引用面板默认收起：它占侧栅里最高的一块，而分支列表属于“要用时才看” */
const refsOpen = ref<string[]>([]);

/** 仓库列表超过这个数才给搜索框——三个仓库时它只是一条多余的输入框 */
const SEARCH_THRESHOLD = 8;

/**
 * 按名称或路径过滤。仓库名可能重名（同一个项目的两个副本），所以路径也要参与匹配：
 * 只按名称搜时两条都叫 demo-1，搜出来分不出哪个是哪个。
 */
const filteredRepos = computed(() => {
  const query = repoQuery.value.trim().toLowerCase();
  if (!query) return repos.repos;
  return repos.repos.filter(
    (repo) =>
      repo.name.toLowerCase().includes(query) || repo.path.toLowerCase().includes(query),
  );
});

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

function toggleRepoMenu(id: number) {
  repoMenu.value = repoMenu.value === id ? null : id;
}

function closeRepoMenu(id: number) {
  if (repoMenu.value === id) repoMenu.value = null;
}

/**
 * 仓库行的动作菜单。
 *
 * 「从列表移除」单独用分隔线隔开：它与「克隆 / 重命名」并排平铺时，
 * 手一滑点到的就是删除——虽然它只删注册记录、磁盘上的仓库不动（§6.1）。
 */
function repoActions(repo: Repo): DropdownOption[] {
  return [
    ...(repo.kind === "browse" ? [{ label: "克隆到本地", key: "clone", disabled: busy.value }] : []),
    { label: "重命名", key: "rename" },
    { label: "复制路径", key: "copy-path" },
    { label: "在资源管理器中显示", key: "reveal" },
    { label: "在终端中打开", key: "terminal" },
    { type: "divider", key: "divider" },
    { label: "从列表移除", key: "remove" },
  ];
}

async function copyRepoPath(repo: Repo) {
  try {
    await navigator.clipboard.writeText(repo.path);
  } catch {
    // WebView2 在非安全上下文里会拒绝。不弹提示打扰用户：
    // 路径本来就写在行的 title 上，复制失败也不是什么大事
  }
}

/** 在系统文件管理器里定位到这个仓库。走 opener 插件的系统 API，不经过 shell */
async function revealRepo(repo: Repo) {
  try {
    await revealItemInDir(repo.path);
  } catch (err) {
    repos.error = new GitTidyError("shell_reveal_failed", String(err));
  }
}

/**
 * 在终端里打开。跨平台没有统一 API，由 Rust 起进程；
 * 路径在那边是独立参数（优先设成子进程工作目录），不当 shell 代码执行。
 */
async function openRepoTerminal(repo: Repo) {
  try {
    await openTerminal(repo.id);
  } catch (err) {
    repos.error =
      err instanceof GitTidyError ? err : new GitTidyError("shell_failed", String(err));
  }
}

function onRepoAction(repo: Repo, key: string) {
  closeRepoMenu(repo.id);
  if (key === "clone") {
    void repos.materialize(repo.id);
  } else if (key === "rename") {
    startRename(repo);
  } else if (key === "copy-path") {
    void copyRepoPath(repo);
  } else if (key === "reveal") {
    void revealRepo(repo);
  } else if (key === "terminal") {
    void openRepoTerminal(repo);
  } else if (key === "remove") {
    removeRepo(repo);
  }
}

function kindTag(repo: Repo) {
  return repo.kind === "worktree"
    ? { text: "本地", type: "success" as const }
    : { text: "只读", type: "info" as const };
}

/** Ctrl+Shift+P 的落点。仓库少到不需要搜索框时它是 undefined，调用方自己兜底 */
const repoSearch = ref<InstanceType<typeof NInput> | null>(null);
let releaseSwitchRepo: (() => void) | undefined;

onMounted(async () => {
  if (repos.repos.length === 0) await repos.load();
  // 远程命令的进度行可能在 promise 落地前就到了，所以订阅必须先于任何远程动作（§6.7）
  writes.watchProgress();
  if (repos.currentId !== null) await writes.loadAll();
  releaseSwitchRepo = provideShell("switchRepo", () => {
    if (repoSearch.value) repoSearch.value.focus();
    else repoQuery.value = "";
  });
});
onBeforeUnmount(() => releaseSwitchRepo?.());

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

    <!--
      钉住区：分支、跟踪、HEAD、待提交条数。
      这些是每时每刻都要看的东西，原来它们混在下面的滚动流里——
      而那个流里有仓库列表、引用列表、stash、写操作日志，一屏装不下，
      于是「我现在在哪个分支」要靠向上滚才能找到。
    -->
    <div v-if="repos.info" class="pinned" :class="{ switching: repos.switching }">
      <div class="pinned-title">{{ repos.current?.name }}</div>

      <!-- 分支与跟踪合成一行：它们本来就是一个事实的两半，分两行反而难对 -->
      <div class="pinned-line">
        <span class="branch" :title="branchText">{{ branchText }}</span>
        <span v-if="trackText" class="track" :class="{ warn: trackStale }">{{ trackText }}</span>
      </div>

      <div class="pinned-line">
        <code v-if="repos.info.headCommit">{{ repos.info.headCommit.slice(0, 8) }}</code>
        <span v-else class="muted">空仓库</span>
        <span class="sep" aria-hidden="true">·</span>
        <span :class="{ strong: repos.workingFiles.length > 0 }">
          {{ repos.workingFiles.length }} 项待提交
        </span>
        <span v-if="repos.interrupted" class="warn">{{ interruptText }}</span>
      </div>

      <!--
        路径与 git 版本是诊断信息，不是日常看的——它们原来占两行常驻，
        而路径一长就换行，把下面整个列表往下推。收进一个可展开的行。
      -->
      <button class="pinned-more" type="button" @click="showRepoInfo = !showRepoInfo">
        <span class="chevron" :class="{ collapsed: !showRepoInfo }" aria-hidden="true" />
        仓库详情
      </button>

      <div v-if="showRepoInfo" class="pinned-more-body">
        <div :title="repos.info.workTree"><span class="key">工作区</span>{{ repos.info.workTree }}</div>
        <div :title="repos.info.gitDir"><span class="key">git 目录</span>{{ repos.info.gitDir }}</div>
        <div><span class="key">git</span>{{ repos.info.gitVersion }}</div>
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

    <!-- 只有这一段滚动：钉住区与品牌不跟着走 -->
    <div class="side-scroll">
    <div class="section">
      <div class="section-title">
        仓库
        <button class="section-action" type="button" @click="showAdd = !showAdd">
          {{ showAdd ? "收起" : "添加" }}
        </button>
      </div>

      <!--
        添加表单按需展开：路径输入框 + 三个按钮 + URL 区常年占着五六行，
        而大多数时候根本不用添加仓库。
      -->
      <div v-if="showAdd" class="add">
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
      </div>

      <n-input
        v-if="repos.repos.length > SEARCH_THRESHOLD"
        ref="repoSearch"
        v-model:value="repoQuery"
        size="small"
        clearable
        placeholder="按名称或路径过滤仓库"
      />

      <div class="repo-list">
        <div
          v-for="repo in filteredRepos"
          :key="repo.id"
          class="repo-row"
          :class="{ active: repo.id === repos.currentId }"
          @click="repos.select(repo.id)"
        >
          <template v-if="editing && editing.id === repo.id">
            <n-input
              v-model:value="editing.name"
              size="small"
              placeholder="留空回落到目录名"
              class="rename"
              @keyup.enter="saveRename"
              @click.stop
            />
            <n-button size="small" type="primary" @click.stop="saveRename">存</n-button>
            <n-button size="small" quaternary @click.stop="editing = null">取消</n-button>
          </template>

          <template v-else>
            <!--
              类型用一颗色点而不是文字标签：本地/只读只有两档，文字标签每行占二十几像素，
              而仓库名才是要读的东西。色点 + title 已经够。
            -->
            <span class="kind-dot" :class="repo.kind" :title="kindTag(repo).text" />
            <span class="repo-name" :title="repo.path">{{ repo.name }}</span>

            <!--
              动作收进一个菜单。原来每行三个按钮悬停展开，
              「移除」还和「克隆」并排——破坏性操作不该和常用操作一个待遇。
            -->
            <n-dropdown
              trigger="click"
              placement="bottom-start"
              :options="repoActions(repo)"
              @clickoutside="closeRepoMenu(repo.id)"
              @select="onRepoAction(repo, $event)"
            >
              <button class="row-menu" type="button" :class="{ open: repoMenu === repo.id }" @click.stop="toggleRepoMenu(repo.id)">
                ⋯
              </button>
            </n-dropdown>
          </template>
        </div>
        <n-empty v-if="!repos.repos.length" size="small" description="还没有仓库" />
        <n-empty v-else-if="filteredRepos.length === 0" size="small" description="没有匹配的仓库" />
      </div>
    </div>

    <!--
      引用列表读自 §7.3 那一次 for-each-ref，不另起进程。
      **写**操作拆到 RefPanel：读的这一列常驻摆几十个分支，写入口悬停才露出来，
      而且每一个都要自己的确认（§7.10）。

      默认收起：它是侧栅里最高的一块（内部已占 30vh），而分支列表属于“要用时才看”。
      标题上的计数让人知道里面有多少，不用展开就知道。
    -->
    <n-collapse v-if="repos.canCommit" v-model:value="refsOpen" arrow-placement="right">
      <n-collapse-item :title="`引用 ${repos.refs.length}`" name="refs">
        <ref-panel />
      </n-collapse-item>
    </n-collapse>

    <!-- 冲突解决器只在有未合并文件时占地方。§7.13：它在中断态里才有用 -->
    <div v-if="writes.hasConflicts" class="block">
      <conflict-resolver />
    </div>

    <div v-if="repos.canCommit" class="block">
      <stash-panel />
    </div>

    <!--
      改写面板（§7.14）只在 todo 非空时占地方：它由提交列表里的「从这里开始改写」打开，
      关掉之后侧栅就恢复原样。放这里是因为它的操作对象是整个区间，
      而区间不绑定于任何一条选中的提交。
    -->
    <div v-if="writes.rewriteTodo.length" class="block">
      <rewrite-panel />
    </div>

    <div v-if="repos.currentId !== null" class="block">
      <op-journal />
    </div>
    </div>

    <!--
      设置入口：钉在侧栅左下角（需求 7.23）。

      为什么不用页签：设置一年可能点不到两次，而它占的是主区导航位——
      页签宽度有限，每多一个标签就少一格留给真正天天用的东西。
      齿轮在左下角是固定位置：不占导航，但始终在，也符合“设置属于应用而不属于仓库”的直觉。

      图标内联而不引图标库：项目里其它记号（文件树、diff 记号）都是这么画的，
      为一个齿轮加一个依赖不值当。
    -->
    <div class="foot">
      <button
        type="button"
        class="gear"
        :class="{ active: settingsOpen }"
        :aria-current="settingsOpen ? 'page' : undefined"
        title="设置（Ctrl+,）"
        :style="{ borderRadius: RADIUS_CONTROL }"
        @click="toggleSettings"
      >
        <n-icon :component="Settings" size="14" />
        <span>设置</span>
      </button>
    </div>
  </aside>
</template>

<style scoped>
.side {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 14px;
  /* 不写 height:100%：项目没有 box-sizing 重置，100% 是内容高，再加上上下各 14px 内边距
     就变成 100vh + 28px，把文档撑出窗口，右侧因此多出一条整页滚动条。
     .shell 是 flex 行容器，交叉轴默认 stretch 已经把高度正好给到 100vh（含内边距） */
  /* 滚动只发生在下面那段里：品牌与钉住区不跟着滚 */
  overflow: hidden;
  border-right: 1px solid var(--border);
  background: var(--surface-app);
  font-size: 12px;
}

/**
 * 底部那条：只装一个设置齿轮。
 *
 * 钉在滚动区之外，所以不管上面的面板开了多少，它始终在左下角同一个位置——
 * 这正是把设置放这里而不是放进页签的理由。
 */
.foot {
  flex: none;
  border-top: 1px solid var(--border);
  padding-top: 8px;
}

.gear {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 5px 8px;
  border: none;
  background: none;
  color: var(--text-2);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
  /* 圆角用模板上绑的 RADIUS_CONTROL 常量：写成自定义属性又带兜底值的写法，
     正好是 check-tokens 要挑出来的那种“静默失效”写法，而它并不在颜色表里 */
}

.gear:hover {
  background: var(--surface-hover);
}

.gear.active {
  color: var(--accent);
  background: var(--surface-selected);
}

.side-scroll {
  display: flex;
  flex-direction: column;
  gap: 14px;
  flex: 1;
  min-height: 0;
  /* 表头 .head.toggle 用负 margin 让 hover 底色出血到文字边缘，会向右探出约 4px；
     只裁横向、保留纵向滚动，否则底部会多出一条横向滚动条 */
  overflow-x: hidden;
  overflow-y: auto;
  padding-right: 2px;
}

/*
 * 钉住区：侧栅里唯一不滚的一块。给它自己的底色，
 * 不然边界只靠一条线，而下面全是卡片，一条线不够。
 *
 * 只有三行：标题、分支+跟踪、HEAD+待提交。
 * 路径与 git 版本收进「仓库详情」那一行——它们是诊断信息，
 * 而路径一长就换行，会把下面整个列表往下推。
 */
.pinned {
  flex: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--surface);
}

/*
 * 切换仓库期间：内容还是上一个仓库的快照，置灰 + 挡住交互。
 * 这样整块留在原地（不清空、不塌），只是变灰，不会闪一下。
 */
.pinned.switching {
  opacity: 0.5;
  pointer-events: none;
}

.pinned-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pinned-line {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--text-2);
  min-width: 0;
}

.pinned-line .branch {
  font-weight: 600;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pinned-line .track {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pinned-line .sep {
  opacity: 0.4;
}

/* 有待提交时给点存在感：它意味着工作区不干净，而那是写操作的入口状态 */
.pinned-line .strong {
  font-weight: 600;
  color: var(--text-1);
}

.pinned-more {
  display: flex;
  align-items: center;
  gap: 5px;
  align-self: flex-start;
  margin-top: 2px;
  padding: 0;
  border: 0;
  background: none;
  color: var(--text-3);
  font-size: 11px;
  font-family: inherit;
  cursor: pointer;
}

.pinned-more:hover {
  color: var(--accent);
}

.pinned-more-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-top: 4px;
  padding-top: 5px;
  border-top: 1px solid var(--border-soft);
  font-size: 11px;
  color: var(--text-2);
}

.pinned-more-body > div {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pinned-more-body .key {
  display: inline-block;
  width: 56px;
  color: var(--text-3);
}

/* 箭头用 CSS 三角形画——只需要一个方向，不值得引一个图标库 */
.chevron {
  flex: none;
  width: 0;
  height: 0;
  border: 4px solid transparent;
  border-top-color: currentColor;
  margin-top: 3px;
  transition: transform 120ms ease;
}

.chevron.collapsed {
  transform: rotate(-90deg);
  margin-top: 0;
}

/**
 * 区块标题。
 *
 * 侧栅里三种标题样式（block-title / NCollapse / 面板自己的 head）并排时，
 * 看不出它们是平级的——所以统一成一套，折叠与否只由箭头区分。
 */
.section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.section-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
  padding: 0 2px;
}

.section-action {
  border: 0;
  background: none;
  padding: 0 2px;
  font-family: inherit;
  font-size: 11px;
  font-weight: 500;
  letter-spacing: 0;
  text-transform: none;
  color: var(--accent);
  cursor: pointer;
}

.section-action:hover {
  text-decoration: underline;
}

.add {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.block {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.brand {
  display: flex;
  align-items: baseline;
  gap: 6px;
}

.brand-title {
  font-size: 14px;
  font-weight: 700;
  color: var(--accent);
}

.brand-sub {
  font-size: 11px;
  color: var(--text-3);
}

.repo-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.repo-row {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 5px 8px;
  border-radius: 6px;
  cursor: pointer;
}

.repo-row:hover {
  background: var(--surface-hover);
}

.repo-row.active {
  background: var(--surface-selected);
}

/*
 * 本地 / 只读只有两档，用一颗色点而不是文字标签：
 * 标签每行占二十几像素，而仓库名才是要读的东西。
 */
.kind-dot {
  flex: none;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--text-3);
}

.kind-dot.worktree {
  background: var(--ok-text);
}

.kind-dot.browse {
  background: var(--text-3);
}

.repo-name {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/*
 * 动作入口平时不占视觉重量，悬停/打开时才显出来。
 * 三个按钮常驻时，一屏的仓库列表全是按钮，仓库名反而读不出来。
 */
.row-menu {
  flex: none;
  width: 22px;
  height: 22px;
  border: 0;
  border-radius: 4px;
  background: none;
  color: var(--text-3);
  font-size: 13px;
  line-height: 1;
  cursor: pointer;
  opacity: 0;
}

.repo-row:hover .row-menu,
.row-menu.open {
  opacity: 1;
}

.row-menu:hover {
  /* 行本身已经是 hover 底色，按钮这一格要再深一档才看得出边界 */
  background: var(--border);
  color: var(--text-1);
}

.rename {
  flex: 1;
  min-width: 0;
}

.meta {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.key {
  display: inline-block;
  width: 56px;
  color: var(--text-3);
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
