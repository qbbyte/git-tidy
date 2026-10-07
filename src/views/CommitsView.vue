<script setup lang="ts">
import { computed, onMounted, watch } from "vue";
import { NAlert, NButton, NEmpty, NSpace, NSpin, NVirtualList } from "naive-ui";
import CommitRow from "@/components/CommitRow.vue";
import { useCommitStore } from "@/stores/commits";
import { useReposStore } from "@/stores/repos";

/** 历史页：仓库由常驻侧栏选，这里只负责当前仓库的提交列表 + 左侧图列。 */
const repos = useReposStore();
const commitStore = useCommitStore();

const repoId = computed(() => repos.currentId);

function loadMore() {
  if (repoId.value !== null) commitStore.loadMore(repoId.value);
}

onMounted(async () => {
  // 直接命中 hash 路由时注册表可能还没取回来
  if (repos.repos.length === 0) await repos.load();
  if (repoId.value !== null) commitStore.open(repoId.value);
});

/** 页签常驻，切仓库不会重新挂载：不盯住 currentId 的话列表会停在旧仓库上 */
watch(repoId, (id) => {
  if (id !== null) commitStore.open(id);
});
</script>

<template>
  <n-space vertical size="medium">
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
      </div>

      <n-alert v-if="commitStore.error" type="error" :title="commitStore.error.message">
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
        >
          <template #default="{ item }">
            <commit-row
              :commit="item"
              :row="commitStore.rowFor(item.id)"
              :lanes="commitStore.graphLanes"
            />
          </template>
        </n-virtual-list>
      </template>

      <n-space align="center">
        <n-spin v-if="commitStore.loading" size="small" />
        <n-button
          v-else-if="!commitStore.loadedAll && commitStore.total > 0"
          size="small"
          @click="loadMore"
        >
          加载更多（已加载 {{ commitStore.commits.length }} / {{ commitStore.total }}）
        </n-button>
        <span v-else-if="commitStore.total > 0" class="muted">已全部加载</span>
      </n-space>
    </template>
  </n-space>
</template>

<style scoped>
.count-line {
  display: flex;
  gap: 12px;
}

.muted {
  font-size: 12px;
  opacity: 0.7;
}

.graph-waiting {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 120px;
  padding: 0 16px;
  background: #fff;
  border-radius: 6px;
  border: 1px solid #e5e8ee;
}

.commit-list {
  /* 外壳把标题栏和页签的高度拿走了，这里按剩余高度铺满 */
  height: calc(100vh - 190px);
  background: #fff;
  border-radius: 6px;
  border: 1px solid #e5e8ee;
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
