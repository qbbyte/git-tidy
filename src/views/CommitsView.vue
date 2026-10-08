<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { NAlert, NButton, NEmpty, NSpace, NSpin, NVirtualList } from "naive-ui";
import CommitRow from "@/components/CommitRow.vue";
import CommitDetail from "@/components/CommitDetail.vue";
import FilterBar from "@/components/FilterBar.vue";
import { useCommitStore } from "@/stores/commits";
import { useDetailStore } from "@/stores/detail";
import { useReposStore } from "@/stores/repos";
import { specFor } from "@/api/spec";
import type { CommitFilter } from "@/api/commit";

/**
 * 历史页：仓库由常驻侧栏选，这里左半是当前仓库的提交列表 + 图列，右半是选中提交的
 * 改动清单和单个文件的差异（§7.4、§7.5，照 Fork 的一屏两栏）。
 */
const repos = useReposStore();
const commitStore = useCommitStore();
const detail = useDetailStore();

const repoId = computed(() => repos.currentId);

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
</script>

<template>
  <div class="page">
    <n-empty
      v-if="repoId === null"
      description="在左侧添加或打开一个仓库"
      class="placeholder"
    />

    <template v-else>
      <div class="count-line">
        <span class="muted">共 {{ commitStore.total }} 条提交</span>
        <span v-if="commitStore.commits.length" class="muted">
          已读出 {{ commitStore.commits.length }} 条
        </span>
        <span v-if="commitStore.graphLoading" class="muted">历史走向读取中…</span>
        <!--
          解析层筛选（type / 合规）要分段扫历史，扫到上限时 Rust 会置位。
          这时候“共 N 条”与实得条数可能对不上，必须写明，不能让人以为那就是全部。
        -->
        <span v-if="commitStore.truncated" class="scanned muted">
          只扫了历史的前一段，下面可能还有
        </span>
      </div>

      <filter-bar
        v-if="repoId !== null"
        :filter="commitStore.filter"
        :refs="repos.refs"
        :types="typeOptions"
        :busy="commitStore.loading"
        @apply="applyFilter"
        @clear="clearFilter"
      />

      <div class="split">
        <section class="list-pane">
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
              :items="commitStore.commits"
              :item-size="44"
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
                  @click="pick(item.id)"
                />
              </template>
            </n-virtual-list>
          </template>

          <n-space align="center">
            <template v-if="commitStore.error && hasRows">
              <span class="muted">读取下一页失败：{{ commitStore.error.message }}</span>
              <n-button size="tiny" @click="loadMore">重试</n-button>
            </template>
            <n-spin v-else-if="commitStore.loading" size="small" />
            <span v-else-if="hasRows && commitStore.loadedAll" class="muted">已全部加载</span>
          </n-space>
        </section>

        <section class="detail-pane">
          <commit-detail :repo-id="repoId" :commit="selectedCommit" />
        </section>
      </div>
    </template>
  </div>
</template>

<style scoped>
.page {
  /* 外壳的 .content 已经把剩余高度给过来了，这里按列分给它，不再拿 100vh 去猜 */
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: 100%;
}

.count-line {
  display: flex;
  gap: 12px;
  flex: none;
}

.split {
  display: flex;
  gap: 12px;
  flex: 1;
  min-height: 0;
}

.list-pane {
  display: flex;
  flex-direction: column;
  gap: 12px;
  flex: 1.15;
  min-width: 0;
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
