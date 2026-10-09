<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { NAlert, NButton, NCard, NSpace, NTag } from "naive-ui";
import { GitTidyError } from "@/api/client";
import {
  hookInstall,
  hookScript,
  hookStatus,
  hookUninstall,
  type HookInstall,
  type HookStatus,
} from "@/api/spec";

/**
 * commit-msg hook 的安装面板（需求 7.20）。
 *
 * 三条规矩决定了这个面板的形状：
 * 1. 规则与提交表单同源，但写进磁盘的是字面量快照——不依赖本工具在 PATH 里；
 * 2. 位置被别人占着（`core.hooksPath` / 别人的 commit-msg）就不写文件，只给共存方案；
 * 3. 拦截不是闭环，被 `--no-verify` 绕过的提交由符合率报告兜住——所以这里必须把这句话说出来。
 */
const props = defineProps<{ repoId: number }>();

const status = ref<HookStatus | null>(null);
const result = ref<HookInstall | null>(null);
const script = ref<string | null>(null);
const busy = ref(false);
const error = ref<GitTidyError | null>(null);
const copied = ref(false);

const view = computed(() => {
  const current = result.value?.status ?? status.value;
  if (!current) return { text: "读取中", type: "default" as const };
  switch (current.state) {
    case "installed":
      return current.ruleSnapshotCurrent
        ? { text: "已安装", type: "success" as const }
        : { text: "已安装 · 规则已过期", type: "warning" as const };
    case "foreign":
      return { text: "被其他 hook 占用", type: "warning" as const };
    default:
      return { text: "未安装", type: "default" as const };
  }
});

/** 规范改过但脚本还是老的：装着的 hook 与表单已经不是同一把尺子，必须提示出来 */
const stale = computed(
  () => status.value?.state === "installed" && !status.value.ruleSnapshotCurrent,
);

async function load() {
  const repoId = props.repoId;
  status.value = null;
  result.value = null;
  script.value = null;
  copied.value = false;
  try {
    const loaded = await hookStatus(repoId);
    if (repoId === props.repoId) status.value = loaded;
  } catch (err) {
    if (repoId === props.repoId) error.value = asError(err);
  }
}

async function act(action: "install" | "uninstall" | "force") {
  busy.value = true;
  error.value = null;
  try {
    const outcome =
      action === "uninstall"
        ? await hookUninstall(props.repoId)
        : await hookInstall(props.repoId, action === "force");
    result.value = outcome;
    status.value = outcome.status;
  } catch (err) {
    error.value = asError(err);
  } finally {
    busy.value = false;
  }
}

async function showScript() {
  busy.value = true;
  error.value = null;
  try {
    script.value = await hookScript(props.repoId);
  } catch (err) {
    error.value = asError(err);
  } finally {
    busy.value = false;
  }
}

async function copyScript() {
  if (!script.value) return;
  try {
    await navigator.clipboard.writeText(script.value);
    copied.value = true;
  } catch (err) {
    error.value = asError(err);
  }
}

function asError(err: unknown) {
  return err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
}

onMounted(load);
watch(() => props.repoId, load);
</script>

<template>
  <n-card title="commit-msg hook" size="small">
    <template #header-extra>
      <n-space align="center" size="small">
        <n-tag size="small" :bordered="false" :type="view.type">{{ view.text }}</n-tag>
        <n-button size="small" :loading="busy" @click="load">重新探测</n-button>
      </n-space>
    </template>

    <n-space vertical size="small">
      <code v-if="status" class="path">{{ status.hookPath }}</code>

      <n-space align="center" size="small">
        <n-button
          v-if="status?.state !== 'installed'"
          size="small"
          type="primary"
          :loading="busy"
          @click="act('install')"
        >
          一键安装
        </n-button>
        <n-button v-else size="small" type="primary" :loading="busy" @click="act('install')">
          按当前规范更新
        </n-button>
        <n-button
          v-if="status?.state === 'installed'"
          size="small"
          :loading="busy"
          @click="act('uninstall')"
        >
          卸载
        </n-button>
        <n-button
          v-if="status?.state === 'foreign' && !status.hooksPath"
          size="small"
          :loading="busy"
          @click="act('force')"
        >
          覆盖安装（原文件会先备份）
        </n-button>
        <n-button size="small" :loading="busy" @click="showScript">查看规则快照</n-button>
      </n-space>

      <n-alert v-if="stale" type="warning" :bordered="false">
        仓库规范改过了，已装的 hook 还是旧的规则快照。终端里的提交会按旧规则判定，
        和这里的表单不是同一把尺子——点「按当前规范更新」把规则刷新一遍。
      </n-alert>

      <n-alert v-if="result && !result.installed" type="info" :bordered="false">
        <div>{{ result.message }}</div>
        <pre v-if="result.suggestion" class="snippet">{{ result.suggestion }}</pre>
      </n-alert>
      <n-alert v-else-if="result" type="success" :bordered="false">
        <div>{{ result.message }}</div>
        <div v-if="result.backup" class="muted">原文件备份：{{ result.backup }}</div>
      </n-alert>

      <div v-if="script" class="script">
        <div class="script-head">
          <span class="muted">
            这就是会写进磁盘的那份脚本：规则以字面量写在文件里，不依赖本工具
          </span>
          <n-button size="small" quaternary @click="copyScript">
            {{ copied ? "已复制" : "复制" }}
          </n-button>
        </div>
        <pre class="script-text">{{ script }}</pre>
      </div>

      <div class="muted">
        hook 只拦得住走这条路的提交：`git commit --no-verify` 能绕过它，
        被绕过的提交在 commit 对象里不留痕迹，只能由符合率报告记为「疑似绕过」——闭环靠审计，不靠拦截。
      </div>

      <n-alert v-if="error" type="error" :title="error.message">
        <div>错误码：{{ error.code }}</div>
      </n-alert>
    </n-space>
  </n-card>
</template>

<style scoped>
.path {
  font-size: 12px;
  opacity: 0.7;
  word-break: break-all;
}

.snippet,
.script-text {
  margin: 6px 0 0;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--surface-sunken);
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-all;
}

.script {
  max-height: 320px;
  overflow: auto;
}

.script-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.muted {
  font-size: 12px;
  opacity: 0.7;
}
</style>