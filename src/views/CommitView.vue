<script setup lang="ts">
import { computed } from "vue";
import { NButton, NCheckbox, NEmpty, NTag } from "naive-ui";
import CommitForm from "@/components/CommitForm.vue";
import { useReposStore } from "@/stores/repos";
import type { WorkingFile } from "@/api/status";

/**
 * 提交页（照 Fork 的 commit 面板）：左边两列勾选——已暂存 / 变更，右边规范表单。
 *
 * 一个文件可以同时出现在两列里（暂存了一半又改了），这时上列勾着、下列没勾：
 * 勾下列是把剩下那半也暂存，取消上列是把暂存那半退回工作区。勾选态一律以 git
 * 返回的整份列表为准，不做乐观更新。
 */
const repos = useReposStore();

const repoId = computed(() => repos.currentId);
const staged = computed(() => repos.stagedFiles);
const changed = computed(() => repos.changedFiles);
/** 一次 IPC 能带一串路径，"全部"就是把它当成一次多文件操作 */
const busy = computed(() => repos.staging);

function statusOf(file: WorkingFile) {
  if (file.conflict) return { text: "冲突", type: "error" as const };
  // 索引列是"这次要提交的内容"，工作区列是"还没提交的部分"，界面只说最要紧的那个
  const code = file.staged && file.worktreeStatus === " " ? file.indexStatus : file.worktreeStatus;
  const labels: Record<string, { text: string; type: "success" | "warning" | "info" | "default" }> =
    {
      A: { text: "新增", type: "success" },
      M: { text: "修改", type: "info" },
      D: { text: "删除", type: "warning" },
      R: { text: "重命名", type: "info" },
      C: { text: "复制", type: "info" },
      "?": { text: "未跟踪", type: "default" },
      " ": { text: "仅暂存", type: "success" },
    };
  return labels[code.trim() || " "] ?? { text: code, type: "default" as const };
}

function check(file: WorkingFile) {
  repos.stage([file.path]);
}

function uncheck(file: WorkingFile) {
  repos.unstage([file.path]);
}

function stageAll() {
  repos.stage(changed.value.map((file) => file.path));
}

function unstageAll() {
  repos.unstage(staged.value.map((file) => file.path));
}

function pathLabel(file: WorkingFile) {
  return file.fromPath ? `${file.fromPath} → ${file.path}` : file.path;
}
</script>

<template>
  <div v-if="repoId === null || !repos.canCommit" class="placeholder">
    <n-empty
      :description="
        repos.current && repos.current.kind === 'browse'
          ? '这是只读浏览的仓库，先「克隆到本地」才能暂存和提交'
          : '在左侧打开一个本地仓库'
      "
    />
  </div>

  <div v-else class="pane">
    <section class="column">
      <header class="column-head">
        <span>已暂存（{{ staged.length }}）</span>
        <n-button
          size="tiny"
          quaternary
          :disabled="!staged.length || busy"
          @click="unstageAll"
        >
          全部取消暂存
        </n-button>
      </header>
      <div class="file-list">
        <div v-for="file in staged" :key="`staged-${file.path}`" class="file-row">
          <n-checkbox :checked="true" :disabled="busy" @update:checked="uncheck(file)" />
          <n-tag :type="statusOf(file).type" size="tiny" :bordered="false">
            {{ statusOf(file).text }}
          </n-tag>
          <code class="file-path" :title="pathLabel(file)">{{ pathLabel(file) }}</code>
        </div>
        <n-empty v-if="!staged.length" size="small" description="暂存区是空的" />
      </div>

      <header class="column-head">
        <span>变更（{{ changed.length }}）</span>
        <n-button size="tiny" quaternary :disabled="!changed.length || busy" @click="stageAll">
          全部暂存
        </n-button>
      </header>
      <div class="file-list">
        <div v-for="file in changed" :key="`changed-${file.path}`" class="file-row">
          <n-checkbox :checked="false" :disabled="busy" @update:checked="check(file)" />
          <n-tag :type="statusOf(file).type" size="tiny" :bordered="false">
            {{ statusOf(file).text }}
          </n-tag>
          <code class="file-path" :title="pathLabel(file)">{{ pathLabel(file) }}</code>
        </div>
        <n-empty v-if="!changed.length" size="small" description="工作区干净" />
      </div>
    </section>

    <commit-form
      class="form"
      :repo-id="repoId"
      :locked="repos.interrupted"
      @committed="repos.refreshAll()"
    />
  </div>
</template>

<style scoped>
.pane {
  display: flex;
  gap: 14px;
  align-items: flex-start;
  height: 100%;
}

.column {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: 1;
  min-width: 0;
}

.form {
  flex: none;
  width: 430px;
}

.column-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  font-weight: 600;
  color: #8a94a6;
  padding: 2px 4px;
}

.file-list {
  background: #fff;
  border: 1px solid #e5e8ee;
  border-radius: 6px;
  padding: 6px 8px;
  max-height: 38vh;
  overflow: auto;
}

.file-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 0;
  font-size: 12px;
}

.file-path {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 60vh;
}
</style>
