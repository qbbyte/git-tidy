<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import dayjs from "dayjs";
import { NAlert, NButton, NEmpty, NSelect, NSpin, NTab, NTabs } from "naive-ui";
import FileTree from "@/components/FileTree.vue";
import BlameView from "@/components/BlameView.vue";
import { useBrowseStore } from "@/stores/browse";
import { useDetailStore } from "@/stores/detail";
import { useReposStore } from "@/stores/repos";
import { formatBytes } from "@/format";
import type { TreeEntry } from "@/api/file";

/**
 * 「文件」页（§7.6）：左边是某个修订的文件树，右边是选中文件的三种看法。
 *
 * 整页只读浏览，不碰工作区也不碰索引——所以 browse（treeless）仓库同样能用，
 * 缺 blob 时 git 会按需向远程取（§4 的那条例外）。
 */
const repos = useReposStore();
const browse = useBrowseStore();
const detail = useDetailStore();
const router = useRouter();

const repoId = computed(() => repos.currentId);
const tab = ref<"content" | "blame" | "history">("content");

/** rev 补全：HEAD 加全部引用（分支、远程跟踪分支、标签） */
const revOptions = computed(() => [
  { label: "HEAD（当前提交）", value: "HEAD" },
  ...repos.refs.map((ref) => ({
    label: `${
      ref.kind === "tag" ? "标签" : ref.kind === "remote" ? "远程" : "分支"
    } ${ref.name}`,
    value: ref.name,
  })),
]);

const file = computed(() => browse.entry);

/** 子模块指针没有正文与归属，页签就别摆出能点的样子 */
const tabs = computed(() => {
  if (file.value === null) return [];
  if (file.value.kind === "commit") return [{ name: "history", label: "历史" }];
  return [
    { name: "content", label: "内容" },
    { name: "blame", label: "归属" },
    { name: "history", label: "历史" },
  ];
});

function pick(entry: TreeEntry) {
  if (repoId.value === null) return;
  // 再点一次同一个文件就收起来：右半边不该由一次误点长期占着
  if (browse.path === entry.path) {
    browse.closeFile();
    return;
  }
  void browse.openFile(repoId.value, entry);
}

function changeRev(value: string) {
  if (repoId.value === null) return;
  void browse.setRev(repoId.value, value);
}

function retryBlame() {
  if (repoId.value === null || file.value === null) return;
  void browse.openFile(repoId.value, file.value);
}

/**
 * 跳到那条提交的详情。
 *
 * 走详情状态而不是自己渲染：改动清单与逐行差异已经在那一栏里实现了（§7.4/§7.5），
 * 这一页再做一份就是两套渲染。行数据不在这页的列表里，所以详情那边会自己取一次。
 */
function jumpToCommit(sha: string) {
  const id = repoId.value;
  if (id === null) return;
  void detail.open(id, sha, null);
  void router.push({ name: "history" });
}

onMounted(async () => {
  if (repos.repos.length === 0) await repos.load();
  if (repoId.value !== null) await browse.loadTree(repoId.value);
});

watch(repoId, (id) => {
  browse.closeFile();
  if (id !== null) void browse.loadTree(id);
});

watch(
  () => file.value?.path,
  () => {
    tab.value = "content";
  },
);
</script>

<template>
  <div class="page">
    <n-empty v-if="repoId === null" description="在左侧添加或打开一个仓库" class="placeholder" />

    <template v-else>
      <div class="bar">
        <n-select
          :value="browse.rev"
          :options="revOptions"
          size="small"
          filterable
          tag
          class="rev"
          @update:value="changeRev"
        />
        <span class="muted">共 {{ browse.entries.length }} 个文件</span>
        <span v-if="browse.treeLoading" class="muted">读取中…</span>
      </div>

      <div class="split">
        <section class="pane">
          <n-alert v-if="browse.treeError" type="error" :title="browse.treeError.message">
            <div>错误码：{{ browse.treeError.code }}</div>
            <pre v-if="browse.treeError.detail" class="raw-output">{{ browse.treeError.detail }}</pre>
          </n-alert>
          <file-tree
            v-else
            :entries="browse.entries"
            :loading="browse.treeLoading"
            :selected="browse.path"
            @pick="pick"
          />
        </section>

        <section class="detail">
          <n-empty v-if="file === null" description="点左边的一个文件" class="placeholder" />

          <template v-else>
            <div class="head">
              <span class="path">{{ file.path }}</span>
              <span class="muted">{{ file.mode }}</span>
              <span v-if="file.size !== undefined" class="muted">{{ formatBytes(file.size) }}</span>
            </div>

            <n-tabs v-model:value="tab" type="line" size="small" class="tabs">
              <n-tab v-for="item in tabs" :key="item.name" :name="item.name">
                {{ item.label }}
              </n-tab>
            </n-tabs>

            <div class="body">
              <template v-if="tab === 'content'">
                <div v-if="browse.contentLoading" class="waiting">
                  <n-spin size="small" />
                  <span class="muted">正在读这个修订里的内容…</span>
                </div>
                <n-alert
                  v-else-if="browse.contentError"
                  type="error"
                  :title="browse.contentError.message"
                  class="block"
                >
                  <div>错误码：{{ browse.contentError.code }}</div>
                </n-alert>
                <div v-else-if="browse.content?.binary" class="notice muted">
                  二进制内容，不能按行看（{{ formatBytes(browse.content.size) }}）。
                </div>
                <template v-else-if="browse.content">
                  <div v-if="browse.content.truncated" class="notice">
                    文件超过一次显示的闸门，只给前一段（共 {{ formatBytes(browse.content.size) }}）。
                  </div>
                  <pre class="text">{{ browse.content.text }}</pre>
                </template>
              </template>

              <blame-view
                v-else-if="tab === 'blame'"
                :blame="browse.blame"
                :loading="browse.blameLoading"
                :error="browse.blameError"
                @pick="jumpToCommit"
                @retry="retryBlame"
              />

              <div v-else class="history">
                <div v-if="browse.historyLoading" class="waiting">
                  <n-spin size="small" />
                  <span class="muted">正在读这个文件的全部改动…</span>
                </div>
                <n-alert
                  v-else-if="browse.historyError"
                  type="error"
                  :title="browse.historyError.message"
                  class="block"
                >
                  <div>错误码：{{ browse.historyError.code }}</div>
                </n-alert>
                <n-empty v-else-if="browse.history.length === 0" description="没有改动记录" />
                <div v-for="commit in browse.history" :key="commit.id" class="commit">
                  <code class="sha">{{ commit.id.slice(0, 8) }}</code>
                  <span class="subject" :title="commit.subject">{{ commit.subject }}</span>
                  <span class="muted">{{ commit.authorName }}</span>
                  <span class="muted">{{ dayjs(commit.time * 1000).format("YYYY-MM-DD") }}</span>
                  <n-button size="small" quaternary @click="jumpToCommit(commit.id)">查看</n-button>
                </div>
                <div class="muted">历史跟着改名走（git log --follow），改名之前的改动也在其中。</div>
              </div>
            </div>
          </template>
        </section>
      </div>
    </template>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: 100%;
}

.bar {
  display: flex;
  align-items: center;
  gap: 12px;
  flex: none;
}

.rev {
  width: 240px;
}

.split {
  display: flex;
  gap: 12px;
  flex: 1;
  min-height: 0;
}

.pane {
  flex: 0 0 320px;
  min-width: 0;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.detail {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 10px 12px;
  overflow: hidden;
}

.head {
  display: flex;
  align-items: baseline;
  gap: 10px;
  font-size: 13px;
}

.path {
  font-family: Consolas, "Courier New", monospace;
  font-weight: 600;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.tabs {
  flex: none;
}

.body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  flex-direction: column;
}

.text {
  margin: 0;
  font-family: Consolas, "Courier New", monospace;
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-all;
  line-height: 18px;
}

.history {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 12px;
}

.commit {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 4px;
  border-bottom: 1px solid var(--surface-sunken);
}

.sha {
  flex: none;
  color: var(--text-3);
}

.subject {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.waiting,
.notice {
  display: flex;
  gap: 8px;
  padding: 12px 4px;
}

.placeholder {
  padding-top: 18vh;
}

.muted {
  font-size: 11px;
  opacity: 0.7;
}

.raw-output {
  margin: 8px 0 0;
  white-space: pre-wrap;
  font-size: 12px;
  opacity: 0.8;
}
</style>