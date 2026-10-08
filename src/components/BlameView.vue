<script setup lang="ts">
import { computed } from "vue";
import dayjs from "dayjs";
import { NAlert, NButton, NEmpty, NSpin } from "naive-ui";
import type { Blame, BlameLine } from "@/api/file";
import type { GitTidyError } from "@/api/client";

/**
 * 逐行归属（§7.6）。左两列是行号与原文，右边是那次提交的短号、作者与时间。
 *
 * 同一提交占一整块：归属是按块来的，一行一个短号看着比一���一个块更难读。
 * 边界提交不给跳转入口——那一端的提交通常在仓库导入之前，本仓库里查不到。
 */
const props = defineProps<{
  blame: Blame | null;
  loading: boolean;
  error: GitTidyError | null;
}>();

const emit = defineEmits<{ pick: [sha: string]; retry: [] }>();

/**
 * 把行按归属分组：短号与作者相同的就是一块（git 的 porcelain 输出本来就是按组给的，
 * 这里只是把同一提交的连续行归到一起）。
 * 边界提交不给跳转入口——那一端的提交通常在仓库导入之前，本仓库里查不到。
 */
interface Block {
  sha: string;
  author: string;
  time: number;
  boundary: boolean;
  lines: BlameLine[];
}

const blocks = computed<Block[]>(() => {
  const out: Block[] = [];
  for (const line of props.blame?.lines ?? []) {
    const head = out[out.length - 1];
    if (head && head.sha === line.sha && head.author === line.author) {
      head.lines.push(line);
      continue;
    }
    out.push({
      sha: line.sha,
      author: line.author,
      time: line.time,
      boundary: line.boundary,
      lines: [line],
    });
  }
  return out;
});
</script>

<template>
  <div class="blame">
    <n-alert v-if="error" type="error" :title="error.message" class="block">
      <div>错误码：{{ error.code }}</div>
      <pre v-if="error.detail" class="raw-output">{{ error.detail }}</pre>
      <n-button size="tiny" @click="emit('retry')">重试</n-button>
    </n-alert>

    <div v-else-if="loading" class="waiting">
      <n-spin size="small" />
      <span class="muted">正在算逐行归属…</span>
    </div>

    <template v-else-if="blame">
      <div class="note muted">
        <span>{{ blame.lines.length }} 行</span>
        <span v-if="blame.truncated">文件较大，只算了开头一段</span>
        <span v-if="blame.ignored.length">
          已忽略 {{ blame.ignored.length }} 个修订（.blame-ignore-revs），归属与原始 blame 不同
        </span>
      </div>

      <n-empty v-if="blame.lines.length === 0" description="这个文件在当前修订里没有内容行" />

      <div v-else class="body">
        <div v-for="block in blocks" :key="`${block.sha}-${block.lines[0].line}`" class="block">
          <div class="owner">
            <code v-if="!block.boundary" class="sha" @click="emit('pick', block.sha)">
              {{ block.sha.slice(0, 8) }}
            </code>
            <code v-else class="sha boundary" title="边界提交：归属早于本仓库的历史">（边界）</code>
            <span class="who">{{ block.author }}</span>
            <span class="when">{{ dayjs(block.time * 1000).format("YYYY-MM-DD") }}</span>
            <span class="muted">{{ block.lines.length }} 行</span>
          </div>
          <div v-for="line in block.lines" :key="line.line" class="line">
            <span class="no">{{ line.line }}</span>
            <span class="text">{{ line.text }}</span>
          </div>
        </div>
      </div>
    </template>

    <div v-else class="waiting muted">点左边文件树里的一个文件看它的归属。</div>
  </div>
</template>

<style scoped>
.blame {
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: auto;
  font-size: 12px;
}

.block {
  margin-top: 6px;
  border: 1px solid #e5e8ee;
  border-radius: 6px;
  overflow: hidden;
}

.owner {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 8px;
  background: #f2f6fb;
  border-bottom: 1px solid #e5e8ee;
  position: sticky;
  top: 0;
}

.sha {
  color: #1f5aa8;
  cursor: pointer;
}

.sha.boundary {
  color: #6b7484;
  cursor: default;
}

.who,
.when {
  font-size: 11px;
  opacity: 0.85;
}

.line {
  display: flex;
  white-space: pre-wrap;
  word-break: break-all;
  line-height: 18px;
}

.no {
  flex: none;
  width: 44px;
  text-align: right;
  padding-right: 8px;
  color: #98a2b3;
  user-select: none;
  background: #fafbfc;
  border-right: 1px solid #eef1f5;
}

.text {
  padding-left: 8px;
  font-family: Consolas, "Courier New", monospace;
}

.waiting {
  display: flex;
  gap: 8px;
  padding: 16px;
}

.note {
  display: flex;
  gap: 10px;
  padding: 4px 2px;
  font-size: 11px;
}

.muted {
  font-size: 11px;
  opacity: 0.75;
}

.raw-output {
  margin: 8px 0;
  white-space: pre-wrap;
  font-size: 12px;
  opacity: 0.8;
}
</style>