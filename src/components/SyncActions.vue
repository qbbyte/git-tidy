<script setup lang="ts">
/**
 * 顶栏的拉取 / 推送按钮。
 *
 * 独立成一个组件是为了用 useMessage：它必须在 NMessageProvider 的子级里调用，
 * 而 App.vue 自身就是那个 provider 的宿主——在它的 setup 里 inject 拿不到实例。
 */
import { computed, ref } from "vue";
import { NButton, NDropdown, NIcon, useMessage, type DropdownOption } from "naive-ui";
import DownloadIcon from "@vicons/tabler/es/Download";
import UploadIcon from "@vicons/tabler/es/Upload";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";

const repos = useReposStore();
const writes = useWriteStore();
const message = useMessage();

const syncBranch = computed(() => repos.repoState?.branch ?? null);
const syncAhead = computed(() => repos.repoState?.ahead ?? 0);
const syncBehind = computed(() => repos.repoState?.behind ?? 0);
const syncUpstream = computed(() => repos.repoState?.upstream ?? null);
const syncRemote = computed(() => {
  const remote = repos.refs.filter((item) => item.kind === "remote")[0]?.name.split("/")[0];
  return remote ?? "origin";
});

const pullOptions: DropdownOption[] = [
  { label: "快进合并", key: "ff_only" },
  { label: "变基", key: "rebase" },
  { type: "divider", key: "d" },
  { label: "仅抓取", key: "fetch" },
];

const pushing = ref(false);

function onPullSelect(key: string) {
  if (key === "fetch") {
    void writes.fetch(syncRemote.value).then((report) => {
      if (report) message.success(report.summary.join("\n") || "抓取完成");
      else if (writes.error) message.error(writes.error.message);
    });
  } else {
    void writes
      .pull(syncRemote.value, key as "ff_only" | "rebase")
      .then((outcome) => {
        if (outcome) message.success("拉取完成");
        else if (writes.error) message.error(writes.error.message);
      });
  }
}

async function doPush() {
  const name = syncBranch.value;
  if (!name) return;
  pushing.value = true;
  try {
    const report = await writes.push(syncRemote.value, name, !syncUpstream.value, false);
    if (report) {
      message.success(report.summary.join("\n") || `已推送到 ${syncRemote.value}/${name}`);
    } else if (writes.error) {
      message.error(writes.error.message);
    }
  } finally {
    pushing.value = false;
  }
}
</script>

<template>
  <div class="actions">
    <n-dropdown trigger="click" :options="pullOptions" :disabled="!writes.canWrite" @select="onPullSelect">
      <n-button
        size="small"
        quaternary
        :disabled="!writes.canWrite"
        :loading="writes.syncAction === 'pull' || writes.syncAction === 'fetch'"
        :title="syncBehind > 0 ? `${syncBehind} 个提交待拉取` : '拉取远程更新'"
      >
        <template #icon><n-icon :component="DownloadIcon" size="16" /></template>
        拉取
        <span v-if="syncBehind > 0" class="sync-badge">{{ syncBehind }}</span>
      </n-button>
    </n-dropdown>
    <n-button
      size="small"
      quaternary
      :disabled="!writes.canWrite || !syncBranch"
      :loading="pushing"
      :title="syncAhead > 0 ? `${syncAhead} 个提交待推送` : '推送'"
      @click="doPush"
    >
      <template #icon><n-icon :component="UploadIcon" size="16" /></template>
      推送
      <span v-if="syncAhead > 0" class="sync-badge accent">{{ syncAhead }}</span>
    </n-button>
  </div>
</template>

<style scoped>
.actions {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: none;
}

.sync-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  margin-left: 2px;
  border-radius: 8px;
  font-size: 10px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: var(--text-3);
  background: var(--surface-hover);
}

.sync-badge.accent {
  color: var(--accent);
  background: var(--surface-selected);
}
</style>