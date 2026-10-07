<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { open } from "@tauri-apps/plugin-dialog";
import {
  NAlert,
  NButton,
  NCard,
  NEmpty,
  NInput,
  NProgress,
  NSpace,
  NTag,
} from "naive-ui";
import { useReposStore } from "@/stores/repos";
import CommitForm from "@/components/CommitForm.vue";
import type { WorkingFile } from "@/api/status";

const router = useRouter();
const repos = useReposStore();

const path = ref("");
const url = ref("");
const editing = ref<{ id: number; name: string } | null>(null);

const hasRepos = computed(() => repos.repos.length > 0);
const stagedCount = computed(() => repos.workingFiles.filter((file) => file.staged).length);
/** 克隆/补齐是长任务，期间所有写入口都要关掉，否则第二次点击会并发抢同一个目录 */
const busy = computed(() => repos.loading || repos.progress !== null);
/** 只有本地工作区仓库能提交：只读浏览仓库没有索引，Rust 侧同样会拒，界面先把入口收掉 */
const commitTargetId = computed(() => {
  const repo = repos.current;
  return repo && repo.kind === "worktree" ? repo.id : null;
});

/** 提交成功后重开一次这个仓库：待提交文件列表和 HEAD 都变了，本地猜不出来 */
function afterCommit() {
  const repo = repos.current;
  if (repo) repos.select(repo.id);
}

/** 一个文件可能同时出现在暂存区和工作区两侧，标签按优先级只说最要紧的那件事 */
function fileGroup(file: WorkingFile): { text: string; type: "success" | "warning" | "info" | "error" | "default" } {
  if (file.conflict) return { text: "冲突", type: "error" };
  if (file.untracked) return { text: "未跟踪", type: "default" };
  if (file.staged && file.worktreeStatus !== " ") return { text: "已暂存又改了", type: "warning" };
  if (file.staged) return { text: "已暂存", type: "success" };
  return { text: "未暂存", type: "info" };
}

function add() {
  const trimmed = path.value.trim();
  if (!trimmed) return;
  repos.add(trimmed).then(() => {
    if (!repos.error) path.value = "";
  });
}

function addByRemoteUrl() {
  const trimmed = url.value.trim();
  if (!trimmed) return;
  repos.addByUrl(trimmed).then(() => {
    // 失败时把地址留在框里，用户不必重新粘贴
    if (!repos.error) url.value = "";
  });
}

function cloneCurrent() {
  const id = repos.current?.id;
  if (id === undefined) return;
  repos.materialize(id);
}

// as const 是必须的：options 的字段类型被推宽成 boolean 时，
// open() 的条件返回类型会退化成 string[] | null
async function pickDirectory() {
  const picked = await open(
    { title: "选择 Git 仓库目录", directory: true, multiple: false } as const,
  );
  if (picked) {
    path.value = picked;
    add();
  }
}

function kindLabel(kind: "worktree" | "browse") {
  return kind === "worktree" ? "本地" : "只读浏览";
}

onMounted(() => repos.load());
</script>

<template>
  <n-space vertical size="large">
    <n-card title="添加仓库" size="small">
      <n-space vertical size="medium">
        <n-space align="center">
          <span class="add-kind">本地目录</span>
          <n-input
            v-model:value="path"
            style="width: 380px"
            placeholder="Git 仓库的本地目录，例如 D:\project\demo"
            @keyup.enter="add"
          />
          <n-button type="primary" :loading="busy" :disabled="!path.trim()" @click="add">
            添加到列表
          </n-button>
          <n-button :disabled="busy" @click="pickDirectory">选择目录</n-button>
        </n-space>
        <n-space align="center">
          <span class="add-kind">仓库地址</span>
          <n-input
            v-model:value="url"
            style="width: 380px"
            placeholder="https://github.com/org/repo.git 或 git@github.com:org/repo.git"
            @keyup.enter="addByRemoteUrl"
          />
          <n-button type="primary" :loading="busy" :disabled="!url.trim()" @click="addByRemoteUrl">
            按地址浏览
          </n-button>
        </n-space>
        <div class="muted">
          地址方式只下载提交对象，不建工作区：能读提交列表，但不能看待提交文件、不能提交、不能改写历史。
          想要完整能力就添加之后点「克隆到本地」。
        </div>
      </n-space>
    </n-card>

    <n-card v-if="repos.progress" :title="`下载中：${repos.progress.url}`" size="small">
      <n-progress
        type="line"
        :percentage="repos.progress.percent"
        :indeterminate="repos.progress.percent === 0"
        processing
        show-indicator
      />
      <div class="muted progress-phase">{{ repos.progress.phase }}</div>
    </n-card>

    <n-alert v-if="repos.error" type="error" :title="repos.error.message">
      <div>错误码：{{ repos.error.code }}</div>
      <pre v-if="repos.error.detail" class="raw-output">{{ repos.error.detail }}</pre>
    </n-alert>

    <n-card v-if="hasRepos" title="仓库列表" size="small">
      <div v-for="repo in repos.repos" :key="repo.id" class="repo-row">
        <div class="repo-main">
          <span class="repo-name">{{ repo.name }}</span>
          <n-tag :type="repo.kind === 'worktree' ? 'success' : 'info'" size="small">
            {{ kindLabel(repo.kind) }}
          </n-tag>
          <code class="repo-path">{{ repo.path }}</code>
        </div>
        <n-space size="small">
          <n-button
            v-if="repo.kind === 'browse'"
            size="small"
            type="primary"
            secondary
            :loading="busy"
            @click="repos.materialize(repo.id)"
          >
            克隆到本地
          </n-button>
          <n-button
            size="small"
            :type="repo.id === repos.currentId ? 'primary' : 'default'"
            @click="repos.select(repo.id)"
          >
            {{ repo.id === repos.currentId ? "已打开" : "打开" }}
          </n-button>
          <n-button size="small" @click="editing = { id: repo.id, name: repo.name }">
            重命名
          </n-button>
          <n-button size="small" quaternary type="error" @click="repos.remove(repo.id)">
            移除
          </n-button>
        </n-space>
      </div>
    </n-card>

    <n-card v-if="editing" title="重命名" size="small" class="rename-card">
      <n-space>
        <n-input
          v-model:value="editing.name"
          style="width: 280px"
          placeholder="留空则回落到目录名"
          @keyup.enter="repos.rename(editing.id, editing.name); editing = null"
        />
        <n-button
          type="primary"
          :loading="repos.loading"
          @click="
            repos.rename(editing.id, editing.name);
            editing = null;
          "
        >
          保存
        </n-button>
        <n-button @click="editing = null">取消</n-button>
      </n-space>
    </n-card>

    <n-empty v-if="!hasRepos && !repos.loading" description="列表里还没有仓库" />

    <n-card v-if="repos.info && repos.current" :title="repos.current.name" size="small">
      <n-alert v-if="repos.current.kind === 'browse'" type="info" title="只读浏览" class="browse-note">
        <div>这个仓库只下载了提交对象，没有工作区。以下能力在补齐之前不可用：查看待提交文件、提交、改写历史。</div>
        <div v-if="repos.current.remoteUrl" class="muted">来源：{{ repos.current.remoteUrl }}</div>
        <n-button type="primary" size="small" :loading="busy" @click="cloneCurrent">
          克隆到本地
        </n-button>
      </n-alert>
      <div class="repo-meta">
        <div><span class="meta-key">工作区</span>{{ repos.info.workTree }}</div>
        <div><span class="meta-key">Git 目录</span>{{ repos.info.gitDir }}</div>
        <div><span class="meta-key">Git 版本</span>{{ repos.info.gitVersion }}</div>
        <div>
          <span class="meta-key">当前分支</span>
          <n-tag v-if="repos.info.branch" type="info" size="small">{{ repos.info.branch }}</n-tag>
          <n-tag v-else type="warning" size="small">游离 HEAD</n-tag>
        </div>
        <div>
          <span class="meta-key">HEAD 提交</span>
          <code v-if="repos.info.headCommit">
            {{ repos.info.headCommit.slice(0, 8) }}
          </code>
          <n-tag v-else type="warning" size="small">空仓库，尚无提交</n-tag>
        </div>
      </div>
      <n-button
        v-if="repos.info.headCommit"
        type="primary"
        size="small"
        class="open-commits"
        @click="router.push('/commits')"
      >
        查看提交列表
      </n-button>
      <n-tag v-else type="warning" size="small" class="open-commits">空仓库没有提交可读</n-tag>
    </n-card>

    <n-card
      v-if="repos.current?.kind === 'worktree' && repos.info"
      title="待提交文件"
      size="small"
    >
      <template #header-extra>
        <n-space align="center" size="small">
          <span class="muted">
            {{ repos.workingFiles.length }} 项，已暂存 {{ stagedCount }}
          </span>
          <n-button
            size="tiny"
            :loading="repos.loading"
            @click="repos.current && repos.select(repos.current.id)"
          >
            重新读取
          </n-button>
        </n-space>
      </template>

      <n-empty v-if="!repos.workingFiles.length" size="small" description="工作区干净" />
      <div v-else class="file-list">
        <div v-for="file in repos.workingFiles" :key="file.path" class="file-row">
          <n-tag :type="fileGroup(file).type" size="small">{{ fileGroup(file).text }}</n-tag>
          <code class="file-path">
            {{ file.fromPath ? `${file.fromPath} → ${file.path}` : file.path }}
          </code>
        </div>
      </div>
    </n-card>

    <commit-form
      v-if="commitTargetId !== null"
      :repo-id="commitTargetId"
      @committed="afterCommit"
    />
  </n-space>
</template>

<style scoped>
.repo-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 8px 0;
  border-bottom: 1px solid #eef0f4;
}

.repo-row:last-child {
  border-bottom: none;
}

.repo-main {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.repo-name {
  font-weight: 600;
  font-size: 13px;
  white-space: nowrap;
}

.repo-path {
  font-size: 12px;
  opacity: 0.65;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rename-card {
  border-style: dashed;
}

.add-kind {
  font-size: 12px;
  opacity: 0.7;
  width: 56px;
}

.progress-phase {
  margin-top: 6px;
}

.browse-note {
  margin-bottom: 12px;
}

.repo-meta {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 13px;
}

.meta-key {
  display: inline-block;
  width: 76px;
  opacity: 0.65;
}

.open-commits {
  margin-top: 12px;
}

.file-list {
  max-height: 260px;
  overflow: auto;
}

.file-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 0;
}

.file-path {
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.muted {
  font-size: 12px;
  opacity: 0.7;
}

.raw-output {
  margin: 8px 0 0;
  white-space: pre-wrap;
  font-size: 12px;
  opacity: 0.8;
}
</style>
