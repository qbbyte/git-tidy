<script setup lang="ts">
import { NEmpty, NSwitch, NTag } from "naive-ui";
import DiffView from "@/components/DiffView.vue";
import { usePreviewStore } from "@/stores/preview";

/**
 * 提交页中间的差异栏：左栏点中的那个文件，画它属于哪一列的那一半改动。
 *
 * 取数是 `usePreviewStore` 按侧别分派的（已暂存 = HEAD → 索引，变更 = 索引 → 工作区），
 * 这里只负责画。标题上那句提示是这一栏存在的理由本身——提交页里最要紧的疑问就是
 * "我现在看的这一份，到底会不会被提交"。
 */
const preview = usePreviewStore();

const SIDE_LABEL = { staged: "已暂存", worktree: "变更" } as const;
const SIDE_HINT = {
  staged: "这一份就是「提交」要落盘的改动",
  worktree: "这一份还没 git add，不会被提交",
} as const;
</script>

<template>
  <div class="preview">
    <div v-if="preview.path === null" class="placeholder">
      <n-empty description="点左边的文件，看它改了什么" />
    </div>

    <template v-else>
      <header class="head">
        <code class="path" :title="preview.path">{{ preview.path }}</code>
        <n-tag
          size="small"
          :bordered="false"
          :type="preview.side === 'staged' ? 'success' : 'info'"
        >
          {{ SIDE_LABEL[preview.side] }}
        </n-tag>
        <span class="hint">{{ SIDE_HINT[preview.side] }}</span>
        <label class="ws">
          <n-switch
            size="small"
            :value="preview.ignoreWhiteSpace"
            @update:value="preview.setIgnoreWhiteSpace"
          />
          忽略空白
        </label>
      </header>

      <div class="body">
        <!--
          取不到差异时只说原因，不把空的 diff 交给 DiffView：那一栏会把"没有逐行差异"
          画成结论，而这里的情况是"根本没法比"，两件事不一样。
        -->
        <div v-if="preview.blocked" class="notice">{{ preview.blocked }}</div>
        <diff-view
          v-else
          :diff="preview.diff"
          :loading="preview.loading"
          :error="preview.error"
          @retry="preview.reopen"
        />
      </div>
    </template>
  </div>
</template>

<style scoped>
.preview {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  flex: 1;
}

/**
 * 允许折行：这一栏可以被拖到 200px 出头（最小窗口 + 表单占着 430px 时就是这样），
 * 一行摆不下就让提示掉到第二行，而不是把路径挤成一个字。
 */
.head {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px 8px;
  padding: 6px 8px;
  border-bottom: 1px solid var(--border);
  font-size: 12px;
}

.path {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hint {
  flex: 1;
  min-width: 80px;
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ws {
  flex: none;
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-2);
  cursor: pointer;
}

/* 正文自己滚：这一栏的高度跟着窗口走，不能让整页因此长出滚动条 */
.body {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.notice {
  padding: 12px 16px;
  font-size: 12px;
  line-height: 1.7;
  color: var(--text-2);
}
</style>
