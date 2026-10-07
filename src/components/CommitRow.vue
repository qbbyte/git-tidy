<script setup lang="ts">
import dayjs from "dayjs";
import { NTag } from "naive-ui";
import type { Commit } from "@/api/commit";

/**
 * 只给常见规范 type 上色。type 是否在白名单里由仓库配置判定（需求 6.7），
 * 列表层不当裁判：解析不出 type 的显示"非规范"灰色，解析出但不认识的也用灰色。
 */
const TYPE_COLORS: Record<string, "success" | "error" | "warning" | "info"> = {
  feat: "success",
  fix: "error",
  perf: "warning",
  refactor: "warning",
  docs: "info",
  test: "info",
  build: "info",
  ci: "info",
};

const props = defineProps<{ commit: Commit }>();

function colorOf() {
  const type = props.commit.commitType;
  return type ? (TYPE_COLORS[type] ?? "default") : "default";
}

function labelOf() {
  return props.commit.commitType ?? "非规范";
}
</script>

<template>
  <div class="commit-row">
    <code class="sha">{{ commit.id.slice(0, 8) }}</code>
    <n-tag :type="colorOf()" size="small" :bordered="false" class="type-tag">
      {{ labelOf() }}
    </n-tag>
    <n-tag v-if="commit.scope" size="small" :bordered="false">{{ commit.scope }}</n-tag>
    <n-tag v-if="commit.breaking" type="error" size="small" :bordered="false">BREAKING</n-tag>
    <n-tag v-if="commit.merge" type="warning" size="small" :bordered="false">merge</n-tag>
    <n-tag v-if="commit.revert" size="small" :bordered="false">revert</n-tag>
    <span class="subject" :title="commit.subject">{{ commit.subject }}</span>
    <span class="author">{{ commit.authorName }}</span>
    <span class="time">{{ dayjs(commit.time * 1000).format("YYYY-MM-DD HH:mm") }}</span>
  </div>
</template>

<style scoped>
.commit-row {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 44px;
  padding: 0 12px;
  border-bottom: 1px solid #f0f2f5;
  font-size: 13px;
}

.sha {
  flex: none;
  color: #8a94a6;
  font-size: 12px;
}

.type-tag {
  flex: none;
  min-width: 62px;
  justify-content: center;
}

.subject {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.author,
.time {
  flex: none;
  font-size: 12px;
  opacity: 0.7;
}
</style>
