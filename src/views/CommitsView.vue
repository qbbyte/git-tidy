<script setup lang="ts">
import { computed, onMounted, watch } from "vue";
import { NAlert, NButton, NEmpty, NSpace, NSpin, NVirtualList } from "naive-ui";
import CommitRow from "@/components/CommitRow.vue";
import { useCommitStore } from "@/stores/commits";
import { useReposStore } from "@/stores/repos";

/** 历史页：仓库由常驻侧栏选，这里只负责当前仓库的提交列表。 */
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
      </div>

      <n-alert v-if="commitStore.error" type="error" :title="commitStore.error.message">
        <div>错误码：{{ commitStore.error.code }}</div>
        <pre v-if="commitStore.error.detail" class="raw-output">{{ commitStore.error.detail }}</pre>
      </n-alert>

      <n-empty
        v-else-if="!commitStore.loading && commitStore.total === 0"
        description="空仓库，尚无提交"
      />

      <n-virtual-list
        v-else
        :items="commitStore.commits"
        :item-size="44"
        key-field="id"
        class="commit-list"
      >
        <template #default="{ item }">
          <commit-row :commit="item" />
        </template>
      </n-virtual-list>

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
