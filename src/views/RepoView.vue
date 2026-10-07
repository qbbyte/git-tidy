<script setup lang="ts">
import { ref } from "vue";
import {
  NAlert,
  NButton,
  NCard,
  NDescriptions,
  NDescriptionsItem,
  NInput,
  NSpace,
  NTag,
} from "naive-ui";
import { useRepoStore } from "@/stores/repo";

const repoStore = useRepoStore();
const path = ref("");

function probe() {
  const trimmed = path.value.trim();
  if (trimmed) repoStore.probe(trimmed);
}
</script>

<template>
  <n-space vertical size="large">
    <n-card title="添加仓库" size="small">
      <n-space>
        <n-input
          v-model:value="path"
          style="width: 420px"
          placeholder="Git 仓库的本地目录，例如 D:\project\demo"
          @keyup.enter="probe"
        />
        <n-button type="primary" :loading="repoStore.loading" :disabled="!path.trim()" @click="probe">
          探测
        </n-button>
      </n-space>
    </n-card>

    <n-alert v-if="repoStore.error" type="error" :title="repoStore.error.message">
      <div>错误码：{{ repoStore.error.code }}</div>
      <pre v-if="repoStore.error.detail" class="raw-output">{{ repoStore.error.detail }}</pre>
    </n-alert>

    <n-card v-if="repoStore.current" title="仓库信息" size="small">
      <n-descriptions label-placement="left" :column="1" bordered size="small">
        <n-descriptions-item label="工作区">{{ repoStore.current.workTree }}</n-descriptions-item>
        <n-descriptions-item label="Git 目录">{{ repoStore.current.gitDir }}</n-descriptions-item>
        <n-descriptions-item label="Git 版本">{{ repoStore.current.gitVersion }}</n-descriptions-item>
        <n-descriptions-item label="当前分支">
          <n-tag v-if="repoStore.current.branch" type="info" size="small">{{ repoStore.current.branch }}</n-tag>
          <n-tag v-else type="warning" size="small">游离 HEAD</n-tag>
        </n-descriptions-item>
        <n-descriptions-item label="HEAD 提交">
          <code v-if="repoStore.current.headCommit">{{ repoStore.current.headCommit.slice(0, 8) }}</code>
          <n-tag v-else type="warning" size="small">空仓库，尚无提交</n-tag>
        </n-descriptions-item>
        <n-descriptions-item label="工作区状态">
          <n-tag :type="repoStore.current.dirty ? 'warning' : 'success'" size="small">
            {{ repoStore.current.dirty ? "有未提交改动" : "干净" }}
          </n-tag>
        </n-descriptions-item>
      </n-descriptions>
    </n-card>
  </n-space>
</template>

<style scoped>
.raw-output {
  margin: 8px 0 0;
  white-space: pre-wrap;
  font-size: 12px;
  opacity: 0.8;
}
</style>
