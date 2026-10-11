<script setup lang="ts">
import { computed } from "vue";
import { NAlert, NButton, NEmpty, NSpin, NSwitch, NTag } from "naive-ui";
import DiffView from "@/components/DiffView.vue";
import { formatBytes } from "@/format";
import { usePreviewStore } from "@/stores/preview";

/**
 * 提交页中间的差异栏：左栏点中的那个文件，画它属于哪一列的那一半改动。
 *
 * 取数是 `usePreviewStore` 按侧别分派的（已暂存 = HEAD → 索引，变更 = 索引 → 工作区），
 * 这里只负责画。标题上那句提示是这一栏存在的理由本身——提交页里最要紧的疑问就是
 * "我现在看的这一份，到底会不会被提交"。
 *
 * 「分段暂存」也挂在这一栏的标题上：它的输入正是这里画着的这份未暂存差异，
 * 摆在旁边比挤在文件名后面更说得通，顺带把列表行整整齐齐地只剩勾选框、标签和路径。
 */
defineProps<{
  /** 当前这个文件能不能分段（看的必须是未暂存那一半，且不是未跟踪 / 冲突） */
  canPartial: boolean;
  /** 分段面板是不是正开着 */
  partialOpen: boolean;
}>();

const emit = defineEmits<{ togglePartial: [] }>();

const preview = usePreviewStore();

const SIDE_LABEL = { staged: "已暂存", worktree: "变更" } as const;

/**
 * 标题上那句提示。
 *
 * 按侧别说清"这一份算不算数"；而未跟踪文件走的是正文预览（没有可比的另一侧），
 * 那时该说的是"为什么这里不是差异"。
 */
const hint = computed(() => {
  if (preview.kind === "content") return "还没 git add：下面是它现在的全文";
  return preview.side === "staged"
    ? "这一份就是「提交」要落盘的改动"
    : "这一份还没 git add，不会被提交";
});

/**
 * 正文那一档要画的东西，**在脚本里算好**。
 *
 * 不在模板里逐项写 `preview.content?.x`：prop 是 `FileContent | null`，而"这一段只在
 * 非 null 时才渲染"这件事模板条件表达不出来（DiffView 顶部那条注释讲的是同一件事）。
 */
const contentView = computed(() => {
  const got = preview.content;
  if (got === null) return null;
  return {
    text: got.text ?? "",
    hasText: got.text !== undefined,
    notice: got.binary
      ? `二进制内容，不能按行看（${formatBytes(got.size)}）。`
      : got.truncated
        ? `文件超过一次显示的闸门，只给前一段（共 ${formatBytes(got.size)}）。`
        : "",
  };
});
</script>

<template>
  <div class="preview">
    <div v-if="preview.path === null" class="placeholder">
      <n-empty description="点左边的文件，看它改了什么" />
    </div>

    <template v-else>
      <header class="head">
        <!-- 左边是"在看什么"，右边是"能对它做什么"：窄下来时右边整组换行，不会把路径挤碎 -->
        <div class="id">
          <code class="path" :title="preview.path">{{ preview.path }}</code>
          <n-tag
            size="small"
            :bordered="false"
            :type="preview.side === 'staged' ? 'success' : 'info'"
          >
            {{ SIDE_LABEL[preview.side] }}
          </n-tag>
          <span class="hint">{{ hint }}</span>
        </div>

        <div class="ops">
          <n-button v-if="canPartial" size="small" quaternary @click="emit('togglePartial')">
            {{ partialOpen ? "收起分段" : "分段暂存" }}
          </n-button>
          <!-- 忽略空白改的是 git 的比较方式，对"整份正文"没有意义，所以那一档不摆它 -->
          <label v-if="preview.kind === 'diff'" class="ws">
            <n-switch
              size="small"
              :value="preview.ignoreWhiteSpace"
              @update:value="preview.setIgnoreWhiteSpace"
            />
            忽略空白
          </label>
        </div>
      </header>

      <div class="body">
        <!--
          取不到差异时只说原因，不把空的 diff 交给 DiffView：那一栏会把"没有逐行差异"
          画成结论，而这里的情况是"根本没法比"，两件事不一样。
        -->
        <div v-if="preview.blocked" class="notice">{{ preview.blocked }}</div>

        <!--
          未跟踪文件：没有可比的另一侧，所以给正文而不是差异。勾选暂存之后它会走回
          下面那条正常的差异预览，所以这里只说清"现在为什么不是差异"。
        -->
        <template v-else-if="preview.kind === 'content'">
          <div v-if="preview.loading" class="waiting">
            <n-spin size="small" />
            <span>正在读这个文件的正文…</span>
          </div>
          <n-alert
            v-else-if="preview.error"
            type="error"
            :title="preview.error.message"
            class="block"
          >
            <div>错误码：{{ preview.error.code }}</div>
            <pre v-if="preview.error.detail" class="raw-output">{{ preview.error.detail }}</pre>
            <n-button size="small" @click="preview.reopen">重试</n-button>
          </n-alert>
          <template v-else-if="contentView">
            <div class="notice">
              未跟踪文件没有可比的另一侧，所以给的是正文。勾选暂存之后，这里会换成「新增这一份」的逐行差异。
            </div>
            <div v-if="contentView.notice" class="notice">{{ contentView.notice }}</div>
            <pre v-if="contentView.hasText" class="content">{{ contentView.text }}</pre>
          </template>
        </template>

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

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 4px 8px;
  padding: 6px 8px;
  border-bottom: 1px solid var(--border);
  font-size: 12px;
}

.id {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  min-width: 0;
}

.ops {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: none;
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
  min-width: 90px;
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

.block {
  margin: 8px;
}

.waiting {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 16px;
  font-size: 12px;
  color: var(--text-2);
}

/* 正文与 diff 用同一套等宽字体与行高：两者在同一个位置切换，字形跳一下很显眼 */
.content {
  margin: 0;
  padding: 4px 16px 12px;
  font-family: Consolas, "Courier New", monospace;
  font-size: 12px;
  line-height: 18px;
  white-space: pre-wrap;
  word-break: break-all;
}

.raw-output {
  margin: 8px 0;
  white-space: pre-wrap;
  font-size: 12px;
  opacity: 0.8;
}
</style>
