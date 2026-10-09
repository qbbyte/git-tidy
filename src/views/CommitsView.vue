<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, reactive, ref, watch } from "vue";
import {
  NAlert,
  NButton,
  NDropdown,
  NEmpty,
  NSpace,
  NSpin,
  NVirtualList,
  useDialog,
  useMessage,
  type DropdownOption,
  type VirtualListInst,
} from "naive-ui";
import CommitRow from "@/components/CommitRow.vue";
import CommitDetail from "@/components/CommitDetail.vue";
import FilterBar from "@/components/FilterBar.vue";
import { useCommitStore } from "@/stores/commits";
import { useDetailStore } from "@/stores/detail";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";
import { specFor } from "@/api/spec";
import { COMMIT_ROW_HEIGHT } from "@/styles/tokens";
import { usePaneDivider } from "@/composables/usePaneDivider";
import { provideShell } from "@/shell";
import { usePrefsStore } from "@/stores/prefs";
import type { CommitFilter } from "@/api/commit";

/**
 * 历史页：仓库由常驻侧栏选，这里左半是当前仓库的提交列表 + 图列，右半是选中提交的
 * 改动清单和单个文件的差异（§7.4、§7.5，照 Fork 的一屏两栏）。
 */
const repos = useReposStore();
const prefsStore = usePrefsStore();
const commitStore = useCommitStore();
const detail = useDetailStore();
const writes = useWriteStore();
const dialog = useDialog();
const message = useMessage();

const repoId = computed(() => repos.currentId);

/**
 * 左右两栏的比例。
 *
 * 默认给详情（右边）六成：diff 的横向空间比提交列表值钱得多，1.15:1 时两边各半，
 * 看一个改了二十个文件的提交就得横向滚。
 *
 * `containerMinWidth` 是「另一栏至少留这么宽」——拖到详情只剩 200px 时
 * 代码会挤成一条，先卡住而不是等用户自己发现。
 */
const split = ref<HTMLElement | null>(null);
/** Ctrl+F 的落点：筛选条里那个关键词框由 FilterBar 自己拿 ref 并抛出聚焦动作 */
const filterBar = ref<InstanceType<typeof FilterBar> | null>(null);
/** 列显示来自个人偏好（设置页里改）。偏好没读到时 store 给的是全开 */
const prefs = computed(
  () =>
    prefsStore.prefs?.columns ?? {
      refs: true,
      author: true,
      time: true,
      sha: true,
    },
);
const divider = usePaneDivider({
  container: split,
  storageKey: "git-tidy:history:split",
  fallback: 0.4,
  min: 0.2,
  max: 0.7,
  unit: "ratio",
  // 详情栏（补集）至少留这么宽：diff 的横向空间比提交列表值钱。
  // 取得太大会有副作用——窗口一窄两个下限就互相打架，见 usePaneDivider 的 clamp
  containerMinWidth: 380,
});

/** 列表栏的宽度。分隔条那 3px ＋ 两侧 4px 边距不从这里扣——
 *  详情栏是 flex:1，会自然吃掉剩下的，列表栏只要占准自己的那一份 */
const listStyle = computed(() => ({
  width: `${divider.size.value * 100}%`,
}));

/**
 * 右半边的标题用列表里那条提交本身，不再为它单取一次（行数据已经全在这儿了）。
 * 列表里没有它（从文件历史/blame 跳过来的提交）时用详情状态里单取的那条。
 */
const selectedCommit = computed(() => {
  const sha = detail.sha;
  if (sha === null) return null;
  return commitStore.commits.find((commit) => commit.id === sha) ?? detail.row;
});

/** 筛选条的 type 候选。与提交表单读的是同一份规范（需求 6.7） */
const typeOptions = ref<string[]>([]);

/** 规范读不到时给 Conventional Commits 的常规几类：下拉空着比给一份无关的清单更糟 */
const FALLBACK_TYPES = [
  "feat",
  "fix",
  "docs",
  "refactor",
  "style",
  "test",
  "perf",
  "build",
  "ci",
  "chore",
  "revert",
];

/** 已经画出来的行。翻页失败时用它决定"整页报错"还是"列表照留、只在底部报错" */
const hasRows = computed(() => commitStore.commits.length > 0);

/** 离底还有这么多像素就去取下一页：约 13 行，够把一次请求的等待盖掉 */
const LOAD_AHEAD_PX = 600;

function loadMore() {
  if (repoId.value !== null) commitStore.loadMore(repoId.value);
}

/** 滚动事件由虚拟列表自己的滚动容器发出，target 就是那个容器 */
function onScroll(event: Event) {
  const el = event.target as HTMLElement;
  if (el.scrollHeight - el.scrollTop - el.clientHeight > LOAD_AHEAD_PX) return;
  loadMore();
}

/** 再点一次同一条就收起右半边：一屏两栏时，右半占的地方不该由一次误点长期占着 */
function pick(sha: string) {
  const commit = commitStore.commits.find((item) => item.id === sha) ?? null;
  if (detail.sha === sha) {
    detail.close();
    return;
  }
  if (repoId.value === null) return;
  void detail.open(repoId.value, sha, commit);
}

// ---------------------------------------------------------------- 键盘导航

const listRef = ref<VirtualListInst | null>(null);

/** 当前选中项在可见列表里的下标；-1 表示还没选 */
const selectedIndex = computed(() => {
  const sha = detail.sha;
  if (sha === null) return -1;
  return commitStore.commits.findIndex((item) => item.id === sha);
});

/**
 * 焦点在输入框里就不抢键。
 *
 * 不加这一条的话，在筛选栏的关键词框里按 ↓ 会把选中提交往下移，
 * 而用户以为自己是在选下拉建议——这是最容易被投诉的一类「快捷键抽风」。
 */
function isTypingTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el || typeof el.tagName !== "string") return false;
  if (el.isContentEditable) return true;
  return ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName);
}

function selectAt(index: number) {
  const commits = commitStore.commits;
  const commit = commits[index];
  if (!commit || repoId.value === null) return;
  void detail.open(repoId.value, commit.id, commit);
  // 键盘往下走时视口不会自己跟，得手动滚过去
  listRef.value?.scrollTo({ key: commit.id });
}

function onKeyDown(event: KeyboardEvent) {
  // 带修饰键的组合留给浏览器（复制粘贴、刷新等）
  if (event.ctrlKey || event.metaKey || event.altKey) return;
  if (isTypingTarget(event.target)) return;

  if (event.key === "Escape") {
    // 只关详情。弹层自己也会吃掉 Esc——它先关自己，这一条是余下的那一次；
    // 用户在弹层里按 Esc 时会看到“弹层关了、详情也关了”，属于可接受的粗糙，
    // 比去侦测“当前有没有弹层开着”可靠
    if (detail.sha === null) return;
    detail.close();
    event.preventDefault();
    return;
  }

  const commits = commitStore.commits;
  if (!commits.length) return;
  const at = selectedIndex.value;

  if (event.key === "ArrowDown" || event.key === "j") {
    // 没选过时往下 = 选第一条，而不是跳过第一条
    selectAt(at === -1 ? 0 : Math.min(at + 1, commits.length - 1));
    event.preventDefault();
  } else if (event.key === "ArrowUp" || event.key === "k") {
    selectAt(at === -1 ? commits.length - 1 : Math.max(at - 1, 0));
    event.preventDefault();
  }
}

// ---------------------------------------------------------------- 右键菜单

const menu = reactive({ show: false, x: 0, y: 0, sha: "" });

function openMenu(event: MouseEvent, sha: string) {
  // 右键也选中那一行：菜单里的“摘取 / 回滚 / 重置”都是针对这条的，
  // 而详情面板停在另一条上会让人以为菜单点错了
  const commit = commitStore.commits.find((item) => item.id === sha) ?? null;
  if (repoId.value !== null && detail.sha !== sha) {
    void detail.open(repoId.value, sha, commit);
  }
  menu.sha = sha;
  menu.x = event.clientX;
  menu.y = event.clientY;
  menu.show = true;
}

function closeMenu() {
  menu.show = false;
}

const menuOptions = computed<DropdownOption[]>(() => {
  const commit = commitStore.commits.find((item) => item.id === menu.sha);
  if (!commit) return [];
  const writable = writes.canWrite;
  return [
    { label: "摘取到当前分支", key: "pick", disabled: !writable },
    {
      // 合并提交的回滚必须选主线（§7.11），那个选择不能由工具替用户做，
      // 所以菜单里禁用它，让用户去详情面板那边选
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
      // 改写区间是「这条之后到 HEAD」，含不含这条由面板里的 todo 决定
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
    // WebView2 在非安全上下文里会拒绝；告知用户而不是静默失败
    message.warning(`复制失败，${what}：${text}`);
  }
}

function onMenuSelect(key: string) {
  const commit = commitStore.commits.find((item) => item.id === menu.sha);
  closeMenu();
  if (!commit) return;
  if (key === "copy-sha") {
    void copyText(commit.id, "提交号");
    return;
  }
  if (key === "copy-subject") {
    void copyText(commit.subject, "标题");
    return;
  }
  if (key === "rewrite") {
    void writes.openRewrite(commit.id);
    return;
  }

  const run = async () => {
    if (key === "pick") {
      await writes.pick(commit.id);
    } else if (key === "revert") {
      await writes.revert(commit.id, null);
    } else if (key.startsWith("reset:")) {
      await writes.reset(key.slice("reset:".length) as "soft" | "mixed" | "hard", commit.id);
    }
  };

  if (key === "reset:hard") {
    // 丢数据的操作不给一键就走的路
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

/** 换筛选：列表与图一起按新的可见集合重算（§7.7） */
function applyFilter(next: CommitFilter) {
  if (repoId.value === null) return;
  // 换集合后原来选中的那条很可能不在里面了，右半边一起收掉
  detail.close();
  void commitStore.applyFilter(repoId.value, next);
}

function clearFilter() {
  if (repoId.value === null) return;
  detail.close();
  void commitStore.clearFilter(repoId.value);
}

/** 筛选项里的 type 候选直接来自仓库规范，与提交表单、报告共用同一份尺子 */
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
  // 直接命中 hash 路由时注册表可能还没取回来
  if (repos.repos.length === 0) await repos.load();
  // 上一次离开这一页时选中的那条不该跟着过来：清单和 diff 都按提交号取
  detail.close();
  if (repoId.value !== null) {
    void commitStore.open(repoId.value);
    void loadTypes();
  }
});

/** 页签常驻，切仓库不会重新挂载：不盯住 currentId 的话列表会停在旧仓库上 */
watch(repoId, (id) => {
  detail.close();
  if (id === null) return;
  void commitStore.open(id);
  void loadTypes();
});

// 键盘监听挂在 window 上：焦点在列表里的哪一行都不影响它能收到。
// 只在这一页存活时绑定，否则在「提交」页按 j 也会改历史页的选中项。
onMounted(() => window.addEventListener("keydown", onKeyDown));
onBeforeUnmount(() => window.removeEventListener("keydown", onKeyDown));

/**
 * 外壳快捷键（Ctrl+F / F5）登记在这一页名下。
 *
 * `refresh` 要走「按当前筛选重开列表」而不是简单重读仓库：这一页的筛选条件在
 * `commitStore` 里，只有重新 `open` 才会带着筛选跑一遍图计算。
 */
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
    <n-empty
      v-if="repoId === null"
      description="在左侧添加或打开一个仓库"
      class="placeholder"
    />

    <template v-else>
      <filter-bar
        v-if="repoId !== null"
        ref="filterBar"
        :filter="commitStore.filter"
        :refs="repos.refs"
        :types="typeOptions"
        :busy="commitStore.loading"
        @apply="applyFilter"
        @clear="clearFilter"
      />

      <div ref="split" class="split" :class="{ resizing: divider.dragging.value }">
        <section class="list-pane" :style="listStyle">
          <n-alert
            v-if="commitStore.error && !hasRows"
            type="error"
            :title="commitStore.error.message"
          >
            <div>错误码：{{ commitStore.error.code }}</div>
            <pre v-if="commitStore.error.detail" class="raw-output">{{ commitStore.error.detail }}</pre>
          </n-alert>

          <template v-else>
            <!--
              图列失败不该把列表一起拖没：这条提示用独立的 v-if，不参与下面那串
              "空仓库 / 等图 / 列表"的分支，列表照画。
            -->
            <n-alert
              v-if="commitStore.graphError"
              type="warning"
              :title="`图列读取失败：${commitStore.graphError.message}`"
            >
              <div>错误码：{{ commitStore.graphError.code }}</div>
              <pre v-if="commitStore.graphError.detail" class="raw-output">{{ commitStore.graphError.detail }}</pre>
            </n-alert>

            <n-empty
              v-if="!commitStore.loading && commitStore.total === 0"
              description="空仓库，尚无提交"
            />

            <!--
              列宽一次定死后不再随翻页变，但第一页拿到图数据之前只能先空着。
              这时候先占位而不是画窄列再撑宽：已经画出去的行横向跳动比等一下更糟。
            -->
            <div v-else-if="!commitStore.graphReady" class="graph-waiting">
              <n-spin size="small" />
              <span class="muted">正在读这条分支的历史走向，第一次要把父子关系整条走一遍…</span>
            </div>

            <n-virtual-list
              v-else
              ref="listRef"
              :items="commitStore.commits"
              :item-size="COMMIT_ROW_HEIGHT"
              key-field="id"
              class="commit-list"
              @scroll="onScroll"
            >
              <template #default="{ item }">
                <commit-row
                  :commit="item"
                  :row="commitStore.rowFor(item.id)"
                  :lanes="commitStore.graphLanes"
                  :selected="item.id === detail.sha"
                  :columns="prefs"
                  @click="pick(item.id)"
                  @contextmenu="openMenu($event, item.id)"
                />
              </template>
            </n-virtual-list>
          </template>

          <!--
            计数行沉到这里，与「已全部加载 / 下一段读取中」同属“列表现在到哪了”。
            原来它单独占一行在列表上方，而下面已经有了一行状态——两处说同一件事。
          -->
          <n-space align="center" class="status-line">
            <span class="muted">共 {{ commitStore.total }} 条</span>
            <span v-if="commitStore.commits.length" class="muted">
              已读出 {{ commitStore.commits.length }} 条
            </span>
            <span v-if="commitStore.graphLoading" class="muted">历史走向读取中…</span>
            <!--
              解析层筛选（type / 合规）要分段扫历史，扫到上限时 Rust 会置位。
              这时候“共 N 条”与实得条数可能对不上，必须写明，不能让人以为那就是全部。
            -->
            <span v-if="commitStore.truncated" class="scanned">
              只扫了历史的前一段，下面可能还有
            </span>

            <template v-if="commitStore.error && hasRows">
              <span class="muted">读取下一页失败：{{ commitStore.error.message }}</span>
              <n-button size="small" @click="loadMore">重试</n-button>
            </template>
            <n-spin v-else-if="commitStore.loading" size="small" />
            <span v-else-if="hasRows && commitStore.loadedAll" class="muted">已全部加载</span>
          </n-space>
        </section>

        <!-- 分隔条：3px 透明热区，悬停/拖动时才显出那条 1px 线。给双击回默认 -->
        <div
          class="divider"
          role="separator"
          aria-orientation="vertical"
          :aria-valuenow="Math.round(divider.size.value * 100)"
          :title="'拖动调宽窄，双击恢复默认'"
          @pointerdown="divider.onPointerDown"
          @dblclick="divider.reset"
        />

        <section class="detail-pane">
          <commit-detail :repo-id="repoId" :commit="selectedCommit" />
        </section>
      </div>

      <!--
        右键菜单挂在页面层而不是每一行：虚拟列表的每一行都挂一个菜单的话，
        翻页时成百个实例跟着建拆，而同一时刻只有一个是开的。
      -->
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
  /* 外壳的 .content 已经把剩余高度给过来了，这里按列分给它，不再拿 100vh 去猜 */
  display: flex;
  flex-direction: column;
  gap: 8px;
  height: 100%;
}

.split {
  display: flex;
  gap: 0;
  flex: 1;
  min-height: 0;
}

/* 拖的时候整栏不要有选择高亮跟着跑，也不要让鼠标变成箭头 */
.split.resizing {
  cursor: col-resize;
  user-select: none;
}

/*
 * 分隔条本体只占 3px 布局宽度——两栏本来就贴着，常驻一条宽条会把它们切成两块
 * 「各自独立」的区域，而它们其实是一件事的两面。
 *
 * 热区用伪元素向两侧扩到 15px：3px 拖起来是拿不住的（WCAG 2.5.8 的下限是 24px，
 * 分隔条按惯例可以例外，但 3px 真的抓不住）。伪元素不吃布局，所以宽度不占地方。
 */
.divider {
  position: relative;
  flex: none;
  width: 3px;
  margin: 0 4px;
  cursor: col-resize;
  border-radius: 2px;
  transition: background 120ms ease;
}

.divider::after {
  content: "";
  position: absolute;
  inset: 0 -6px;
}

.divider:hover,
.split.resizing .divider {
  background: var(--accent);
}

.list-pane {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

.status-line {
  flex: none;
  min-height: 24px;
}

.detail-pane {
  flex: 1;
  min-width: 0;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  /* 里面自己滚动：清单和差异各占一段，不能让这一栏整体撑高把窗口拖出滚动条 */
  overflow: hidden;
}

.muted {
  font-size: 12px;
  opacity: 0.7;
}

.scanned {
  color: var(--warn-text);
  opacity: 1;
}

.graph-waiting {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 120px;
  padding: 0 16px;
  background: var(--surface);
  border-radius: 6px;
  border: 1px solid var(--border);
}

.commit-list {
  /* 吃掉本栏剩下的全部高度。以前写 calc(100vh - 190px)，那个 190 是手调的，
     上方行数一变（计数行、中断提示条、底部状态行）就差出几十像素的空白 */
  flex: 1;
  min-height: 0;
  background: var(--surface);
  border-radius: 6px;
  border: 1px solid var(--border);
}

.placeholder {
  padding-top: 20vh;
}

.raw-output {
  margin: 8px 0 0;
  white-space: pre-wrap;
  font-size: 12px;
  opacity: 0.8;
}
</style>
