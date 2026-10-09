<script setup lang="ts">
import { computed, ref } from "vue";
import { NAlert, NButton, NCard } from "naive-ui";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";

/**
 * 提交页的同步区：把「推出去」这一步挪到提交旁边。
 *
 * 提交和推送是同一条流水线上相邻的两步，而推送原来藏在右侧引用栏的远程区块里，
 * 和抓取 / 拉取挤成一排——提交完还得换一栏去找。现在它就跟在提交表单下面：
 * 提交成功、`ahead` 变成正值，这里就显示「↑ N 个提交待推送」，按一下就推。
 *
 * 只放推送、不放抓取 / 拉取：那两个是「开始干活之前」的动作，和「收尾提交」不在
 * 同一时刻发生，留在引用栏的远程区块里更合适。
 */
const repos = useReposStore();
const writes = useWriteStore();

const branch = computed(() => repos.repoState?.branch ?? repos.info?.branch ?? null);
const upstream = computed(() => repos.repoState?.upstream ?? null);
const ahead = computed(() => repos.repoState?.ahead ?? null);
const upstreamGone = computed(() => repos.repoState?.upstreamGone ?? false);

/** 远程名取自远程跟踪分支的前缀（origin/…）。没有远程跟踪分支时给 origin */
const remoteName = computed(() => {
  const remote = repos.refs.filter((item) => item.kind === "remote")[0]?.name.split("/")[0];
  return remote ?? "origin";
});

const canPush = computed(() => writes.canWrite && !!branch.value);
/** push 不走 store 的 `run`，没有 busy 可借，自己管一个 */
const pushing = ref(false);

async function push() {
  const name = branch.value;
  if (!name || !canPush.value) return;
  pushing.value = true;
  try {
    await writes.push(remoteName.value, name, !upstream.value, false);
  } finally {
    pushing.value = false;
  }
}
</script>

<template>
  <n-card size="small" title="同步">
    <template #header-extra>
      <span class="ref">{{ branch ?? "游离 HEAD" }}</span>
    </template>

    <div class="row">
      <div class="state">
        <template v-if="!branch">
          <span class="muted">当前是游离 HEAD，先切到一个分支再推</span>
        </template>
        <template v-else-if="upstreamGone">
          <span class="warn">上游 {{ upstream }} 在远程已被删除，推送会重新建立</span>
        </template>
        <template v-else-if="!upstream">
          <span class="muted">还没有上游，推送时会自动建立跟踪</span>
        </template>
        <template v-else-if="ahead && ahead > 0">
          <span class="count">↑ {{ ahead }}</span>
          <span class="muted">个提交待推送</span>
        </template>
        <template v-else-if="ahead === 0">
          <span class="muted">已经和 {{ upstream }} 同步</span>
        </template>
        <template v-else>
          <span class="muted">跟踪状态还没读出来</span>
        </template>
      </div>

      <n-button
        size="small"
        type="primary"
        :disabled="!canPush || pushing"
        :loading="pushing"
        :title="
          !branch
            ? '游离 HEAD 上没有分支可推'
            : !writes.canWrite
              ? '当前仓库没有写权限（只读浏览，或有操作卡在半路）'
              : `推送到 ${remoteName}/${branch}`
        "
        @click="push"
      >
        推送
      </n-button>
    </div>

    <n-alert v-if="writes.error" class="sync-error" type="error" :bordered="false">
      {{ writes.error.message }}
    </n-alert>

    <div class="target muted">
      目标 <code>{{ remoteName }}/{{ branch ?? "…" }}</code>
      <span v-if="!upstream && branch"> · 首次推送会建立跟踪（-u）</span>
    </div>
  </n-card>
</template>

<style scoped>
.ref {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-2);
}

.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.state {
  display: flex;
  align-items: baseline;
  gap: 6px;
  font-size: 12px;
  min-width: 0;
}

.count {
  font-weight: 600;
  color: var(--accent);
  font-variant-numeric: tabular-nums;
}

.warn {
  color: var(--warn-text);
  font-size: 12px;
}

.muted {
  font-size: 12px;
  opacity: 0.7;
}

.sync-error {
  margin-top: 8px;
}

.target {
  margin-top: 6px;
  font-size: 11px;
}
</style>
