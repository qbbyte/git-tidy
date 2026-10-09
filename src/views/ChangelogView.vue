<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useDialog, useMessage } from "naive-ui";
import { save } from "@tauri-apps/plugin-dialog";
import { GitTidyError } from "@/api/client";
import {
  changelogBuild,
  changelogPreviousTag,
  changelogReadTarget,
  changelogWrite,
  type AppendResult,
  type Changelog,
  type TargetPreview,
} from "@/api/changelog";
import { useReposStore } from "@/stores/repos";

/**
 * CHANGELOG 生成与导出（需求 6.5）。
 *
 * 三条硬约束决定了这个页面的形状：
 * 1. 区间默认 `<上一个 tag>..HEAD`，没有 tag 才让人手填起点；
 * 2. 不合规提交不进日志，但必须单独计数并给出跳符合率报告的入口；
 * 3. 写文件前先展示"上面已有 / 下面新增"，路径走系统 dialog 授权。
 */
const repos = useReposStore();
const message = useMessage();
const dialog = useDialog();

const repoId = computed(() => repos.currentId);
/** 写文件要落进工作区，只读浏览仓库没有工作区，写了也无处可循 */
const canWrite = computed(() => repos.canCommit);

const changelog = ref<Changelog | null>(null);
const from = ref<string | null>(null);
const previousTag = ref<string | null>(null);
const manualFrom = ref("");
const loading = ref(false);
const error = ref<GitTidyError | null>(null);
const copied = ref(false);
/** 写入前的目标文件预览；null 表示还没选路径 */
const preview = ref<TargetPreview | null>(null);
const written = ref<AppendResult | null>(null);

const stats = computed(() => {
  const current = changelog.value;
  if (!current) return "";
  const parts = [`区间 ${current.scanned} 条`, `收录 ${current.included} 条`];
  if (current.skipped.nonConformant > 0) {
    parts.push(`不合规未收录 ${current.skipped.nonConformant} 条`);
  }
  if (current.skipped.merges > 0) {
    parts.push(`排除合并 ${current.skipped.merges} 条`);
  }
  return parts.join(" · ");
});

async function load() {
  if (repoId.value === null) return;
  loading.value = true;
  error.value = null;
  copied.value = false;
  preview.value = null;
  written.value = null;
  try {
    const tag = await changelogPreviousTag(repoId.value);
    previousTag.value = tag;
    // 有上一个 tag 就默认用它；没有就把手填框打开，让人自己挑起点
    from.value = tag;
    await build();
  } catch (err) {
    error.value = asError(err);
  } finally {
    loading.value = false;
  }
}

async function build() {
  if (repoId.value === null) return;
  loading.value = true;
  error.value = null;
  try {
    const start = manualFrom.value.trim() || from.value || undefined;
    changelog.value = await changelogBuild(repoId.value, start ?? undefined);
  } catch (err) {
    error.value = asError(err);
  } finally {
    loading.value = false;
  }
}

async function copy() {
  if (!changelog.value) return;
  try {
    await navigator.clipboard.writeText(changelog.value.markdown);
    copied.value = true;
  } catch (err) {
    error.value = asError(err);
  }
}

/** 导出：同样是写文件，所以走同一条"先预览再落盘"的流程 */
async function chooseTarget(defaultName: string) {
  if (repoId.value === null) return;
  error.value = null;
  const picked = await save({
    defaultPath: defaultName,
    filters: [{ name: "Markdown", extensions: ["md"] }],
  });
  if (!picked) return;
  try {
    preview.value = await changelogReadTarget(picked);
  } catch (err) {
    error.value = asError(err);
  }
}

function confirmWrite() {
  const current = changelog.value;
  const target = preview.value;
  if (!current || !target) return;
  dialog.warning({
    title: "确认追加写入",
    content: `将把本次生成的 ${current.included} 条追加到：\n${target.path}\n\n${
      target.exists ? "文件已有内容会保留在上面。" : "文件不存在，将新建。"
    }`,
    positiveText: "写入",
    negativeText: "取消",
    onPositiveClick: () => write(target),
  });
}

async function write(target: TargetPreview) {
  if (repoId.value === null || !changelog.value) return;
  loading.value = true;
  error.value = null;
  try {
    written.value = await changelogWrite(
      repoId.value,
      target.path,
      changelog.value.markdown,
      // 指纹取自选路径那一刻的预览内容：中途被别人改过就拒写
      target.digest,
    );
    preview.value = null;
    message.success("已写入 " + written.value.path);
  } catch (err) {
    error.value = asError(err);
  } finally {
    loading.value = false;
  }
}

function asError(err: unknown) {
  return err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
}

onMounted(load);
watch(repoId, load);
</script>

<template>
  <div v-if="repoId === null" class="placeholder">
    <n-empty description="在左侧打开一个仓库" />
  </div>

  <div v-else class="page">
    <header class="head">
      <n-input
        v-model:value="manualFrom"
        size="small"
        style="width: 240px"
        placeholder="起点（tag / rev），留空用上一个 tag"
        clearable
      />
      <span v-if="previousTag" class="muted">上一个 tag：{{ previousTag }}</span>
      <span v-else class="muted">这个仓库还没有 tag，起点要自己填</span>
      <n-button size="small" type="primary" :loading="loading" @click="build">生成</n-button>
      <n-button size="small" :loading="loading" @click="load">按上一个 tag 重来</n-button>
    </header>

    <n-alert v-if="error" type="error" :title="error.message">
      <div>错误码：{{ error.code }}</div>
      <pre v-if="error.detail" class="raw">{{ error.detail }}</pre>
    </n-alert>

    <template v-if="changelog">
      <n-card size="small">
        <div class="head-line">
          <span class="stats">{{ stats }}</span>
          <n-space align="center" size="small">
            <n-button size="small" :disabled="changelog.included === 0" @click="copy">
              {{ copied ? "已复制" : "复制 Markdown" }}
            </n-button>
            <n-button
              size="small"
              :disabled="changelog.included === 0 || !canWrite"
              @click="chooseTarget('CHANGELOG.md')"
            >
              追加写入文件…
            </n-button>
          </n-space>
        </div>

        <n-alert v-if="changelog.truncated" type="warning" :bordered="false">
          区间历史超过统计上限，本次只覆盖最近 {{ changelog.scanned }} 条。想覆盖全量请缩小区间。
        </n-alert>

        <n-alert
          v-if="changelog.skipped.nonConformant > 0"
          type="info"
          :bordered="false"
        >
          有 {{ changelog.skipped.nonConformant }} 条提交不符合本仓库规范，没有进这份日志。
          规范提交才写得成发布说明——去「报告」页看它们卡在哪一条。
        </n-alert>

        <n-alert v-if="!canWrite" type="info" :bordered="false">
          只读浏览的仓库没有工作区，可以复制或导出，但不能写回文件。
        </n-alert>
      </n-card>

      <n-card v-if="preview" title="写入前预览" size="small">
        <n-space vertical size="small">
          <div class="muted">
            目标：{{ preview.path }}
            {{ preview.exists ? `（已存在 ${preview.bytes} 字节）` : "（不存在，将新建）" }}
          </div>
          <div class="split">
            <div class="pane old">
              <div class="pane-label">文件末尾（保留）</div>
              <pre class="code">{{ preview.tail || "（空文件）" }}</pre>
            </div>
            <div class="pane added">
              <div class="pane-label">本次追加</div>
              <pre class="code">{{ changelog.markdown }}</pre>
            </div>
          </div>
          <n-space>
            <n-button size="small" type="primary" :loading="loading" @click="confirmWrite">
              确认写入
            </n-button>
            <n-button size="small" quaternary @click="preview = null">取消</n-button>
          </n-space>
        </n-space>
      </n-card>

      <n-card v-if="written" size="small">
        <n-alert type="success" :bordered="false" :title="`已写入 ${written.path}`">
          <div>{{ written.undoHint }}</div>
          <div class="muted">还原点：{{ written.backupRef }}</div>
        </n-alert>
      </n-card>

      <n-card title="预览" size="small">
        <pre class="markdown">{{ changelog.markdown }}</pre>
      </n-card>

      <n-card v-if="changelog.skipped.samples.length" title="未收录的提交（前若干条）" size="small">
        <table class="table">
          <tbody>
            <tr v-for="item in changelog.skipped.samples" :key="item.sha">
              <td class="subject" :title="item.subject">{{ item.subject }}</td>
              <td class="author">{{ item.authorName }}</td>
            </tr>
          </tbody>
        </table>
      </n-card>
    </template>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.head,
.head-line {
  display: flex;
  align-items: center;
  gap: 10px;
}

.head-line {
  justify-content: space-between;
  margin-bottom: 8px;
}

.stats {
  font-size: 12px;
  color: var(--text-2);
}

.split {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
}

.pane-label {
  font-size: 11px;
  opacity: 0.65;
  margin-bottom: 4px;
}

.old {
  opacity: 0.8;
}

.code,
.markdown {
  margin: 0;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--surface-sunken);
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 320px;
  overflow: auto;
}

.markdown {
  max-height: none;
}

.raw {
  margin: 8px 0 0;
  white-space: pre-wrap;
  font-size: 12px;
  opacity: 0.8;
}

.table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}

.table td {
  padding: 3px 6px;
  border-bottom: 1px solid var(--surface-sunken);
}

.subject {
  max-width: 460px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.author {
  width: 140px;
  opacity: 0.75;
}

.muted {
  font-size: 12px;
  opacity: 0.7;
}

.placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 60vh;
}
</style>