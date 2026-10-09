<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  NAlert,
  NButton,
  NCard,
  NIcon,
  NInput,
  NMenu,
  NModal,
  NSelect,
  NSpace,
  NSwitch,
  NTag,
  type MenuOption,
} from "naive-ui";
import { PULL_STRATEGY_LABEL, type AiConfig, type PullStrategy } from "@/api/prefs";
import { validateConfig } from "@/lib/ai";
import { usePrefsStore } from "@/stores/prefs";
import { checkForUpdate, updateEndpoint, type CheckResult } from "@/api/update";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SHORTCUTS, TAB_SHORTCUTS, formatKeys } from "@/shortcuts";
import Settings from "@vicons/tabler/es/Settings";

/**
 * 设置弹窗（左侧菜单 + 右侧内容）。
 *
 * 入口仍是左下角齿轮与 Ctrl+,，但不再占一个主区页签。
 * 弹窗内按主题分组：通用 / 更新 / 快捷键 / 存储位置。
 */
const props = defineProps<{
  show: boolean;
}>();

const emit = defineEmits<{
  (e: "update:show", value: boolean): void;
}>();

const visible = computed({
  get: () => props.show,
  set: (value) => emit("update:show", value),
});

const prefsStore = usePrefsStore();
const prefs = computed(() => prefsStore.prefs);

const activeKey = ref("general");

const menuOptions: MenuOption[] = [
  { key: "general", label: "通用" },
  { key: "update", label: "更新" },
  { key: "shortcuts", label: "快捷键" },
  { key: "ai", label: "AI 模型" },
  { key: "storage", label: "存储位置" },
];

const pullOptions = (Object.keys(PULL_STRATEGY_LABEL) as PullStrategy[]).map((value) => ({
  label: PULL_STRATEGY_LABEL[value],
  value,
}));

const columnOptions = [
  { key: "refs" as const, label: "引用徽标（分支 / 标签）" },
  { key: "author" as const, label: "作者" },
  { key: "time" as const, label: "时间" },
  { key: "sha" as const, label: "提交号" },
];

/** 说明表按分组归拢，顺序跟数组里的顺序一致 */
const grouped = computed(() => {
  const groups: { group: string; rows: { keys: string; label: string; when: string }[] }[] = [];
  for (const shortcut of SHORTCUTS) {
    let bucket = groups.find((item) => item.group === shortcut.group);
    if (!bucket) {
      bucket = { group: shortcut.group, rows: [] };
      groups.push(bucket);
    }
    bucket.rows.push({
      keys: formatKeys(shortcut.keys),
      label: shortcut.label,
      when: shortcut.when,
    });
  }
  return groups;
});

function setPull(value: PullStrategy) {
  prefsStore.patch({ pullStrategy: value });
}

function setColumn(key: "refs" | "author" | "time" | "sha", value: boolean) {
  if (!prefs.value) return;
  prefsStore.patch({ columns: { ...prefs.value.columns, [key]: value } });
}

function setPrefix(key: "feature" | "hotfix" | "release", value: string) {
  if (!prefs.value) return;
  prefsStore.patch({ flowPrefixes: { ...prefs.value.flowPrefixes, [key]: value } });
}

/**
 * 检查更新（只查不装）。
 */
const updateResult = ref<CheckResult | null>(null);
const updateError = ref<string | null>(null);
const updateBusy = ref(false);

async function runUpdateCheck() {
  updateBusy.value = true;
  updateError.value = null;
  try {
    updateResult.value = await checkForUpdate();
  } catch (err) {
    updateResult.value = null;
    updateError.value = err instanceof Error ? err.message : String(err);
  } finally {
    updateBusy.value = false;
  }
}

async function openReleases(url?: string) {
  await openUrl(url && url !== "" ? url : releasesPage.value);
}

const releasesPage = ref("");
const currentVersion = ref("");

async function ensureReleasesPage() {
  if (releasesPage.value !== "") return;
  const endpoint = await updateEndpoint();
  releasesPage.value = endpoint.releasesPageUrl;
  currentVersion.value = endpoint.currentVersion;
}

/**
 * AI 配置（7.25）。
 *
 * 输入框先写本地草稿、再防抖落盘：这三栏是明文长串，逐字符 patch 就是逐字符往磁盘写
 * preferences.json，而且 API Key 每落一次盘就多一次暴露窗口。失焦立刻落盘，避免用户
 * 改完直接关窗丢配置。
 */
const aiDraft = ref<AiConfig>({ endpoint: "", apiKey: "", model: "" });
const AI_SAVE_DELAY_MS = 400;
let aiTimer: ReturnType<typeof setTimeout> | undefined;

watch(
  () => prefs.value?.ai,
  (value) => {
    // 落盘回来的值与草稿一致时才同步，避免自己 patch 回来的响应把光标位置冲掉
    if (value && value.endpoint !== aiDraft.value.endpoint && document.activeElement === null) {
      aiDraft.value = { ...value };
    }
  },
  { immediate: true },
);

function saveAi() {
  if (aiTimer !== undefined) clearTimeout(aiTimer);
  if (!prefs.value) return;
  prefsStore.patch({ ai: { ...aiDraft.value } });
}

function setAi(field: keyof AiConfig, value: string) {
  aiDraft.value = { ...aiDraft.value, [field]: value };
  if (aiTimer !== undefined) clearTimeout(aiTimer);
  aiTimer = setTimeout(saveAi, AI_SAVE_DELAY_MS);
}

/** 三栏齐了且协议合法才够一次调用；这里给出的是「还差什么」，不是一句「未配置」 */
const aiIssue = computed(() => (prefs.value ? validateConfig(aiDraft.value) : null));

onBeforeUnmount(() => {
  if (aiTimer !== undefined) clearTimeout(aiTimer);
});

onMounted(async () => {
  await prefsStore.load();
  void ensureReleasesPage();
});
</script>

<template>
  <n-modal
    v-model:show="visible"
    :mask-closable="true"
    :close-on-esc="true"
    preset="card"
    class="settings-modal"
    :class="{ 'with-icon': true }"
    :bordered="false"
    transform-origin="center"
  >
    <template #header>
      <div class="modal-header">
        <n-icon :component="Settings" size="16" />
        <span>设置</span>
      </div>
    </template>

    <div class="settings-body">
      <div class="settings-nav">
        <n-menu
          v-model:value="activeKey"
          :options="menuOptions"
          :indent="18"
          :root-indent="12"
        />
      </div>

      <div class="settings-content">
        <n-alert v-if="prefsStore.error" type="error" :title="prefsStore.error.message">
          <div>错误码：{{ prefsStore.error.code }}</div>
        </n-alert>

        <!-- 通用 -->
        <template v-if="activeKey === 'general'">
          <n-card size="small" title="拉取">
            <div class="row">
              <span class="label">拉取策略</span>
              <n-select
                :value="prefs?.pullStrategy ?? 'ff_only'"
                :options="pullOptions"
                size="small"
                style="width: 260px"
                :disabled="!prefs"
                @update:value="setPull"
              />
              <span class="muted">
                默认是"只快进"：远端有本地没有的提交就停下来问，不自动制造合并提交
              </span>
            </div>
          </n-card>

          <n-card size="small" title="Git Flow 命名前缀">
            <n-space vertical size="small">
              <div v-for="key in (['feature', 'hotfix', 'release'] as const)" :key="key" class="row">
                <span class="label">{{ key }}</span>
                <n-input
                  :value="prefs?.flowPrefixes[key] ?? ''"
                  size="small"
                  style="width: 220px"
                  :placeholder="key"
                  @update:value="(value: string) => setPrefix(key, value)"
                />
                <code v-if="prefs" class="muted">
                  {{ prefs.flowPrefixes[key] }}{{ key === "release" ? "1.2.0" : "示例" }}
                </code>
              </div>
              <div class="muted">清空即不加前缀。Git Flow 是自组合的，不依赖系统里装了 git-flow。</div>
            </n-space>
          </n-card>

          <n-card size="small" title="提交列表显示的列">
            <n-space size="large">
              <n-space
                v-for="option in columnOptions"
                :key="option.key"
                align="center"
                size="small"
              >
                <span>{{ option.label }}</span>
                <n-switch
                  :value="prefs?.columns[option.key] ?? true"
                  size="small"
                  :disabled="!prefs"
                  @update:value="(value: boolean) => setColumn(option.key, value)"
                />
              </n-space>
            </n-space>
            <div class="muted">关掉只是不显示，数据还在——报告与 CHANGELOG 照样按它们统计。</div>
          </n-card>
        </template>

        <!-- 更新 -->
        <template v-if="activeKey === 'update'">
          <n-card size="small" title="更新">
            <n-space vertical size="small">
              <div class="row">
                <span class="label">启动时检查一次新版本</span>
                <n-switch
                  :value="prefs?.autoUpdate ?? true"
                  size="small"
                  :disabled="!prefs"
                  @update:value="(value: boolean) => prefsStore.patch({ autoUpdate: value })"
                />
                <span class="muted">
                  关掉就完全不联网问版本。只查不装：发现新版只会告诉你，并把你领到 Releases 页。
                </span>
              </div>
              <div class="row">
                <n-button size="small" :loading="updateBusy" @click="runUpdateCheck">检查更新</n-button>
                <n-button size="small" quaternary :loading="updateBusy" @click="openReleases()">
                  打开发布页
                </n-button>
                <span v-if="currentVersion" class="muted">当前版本 v{{ currentVersion }}</span>
              </div>
              <n-alert v-if="updateError" type="warning" :bordered="false">{{ updateError }}</n-alert>
              <n-alert
                v-else-if="updateResult?.available"
                type="info"
                :bordered="false"
                :title="`发现新版本 v${updateResult.latest?.version}（当前 v${updateResult.current}）`"
              >
                <div>{{ updateResult.latest?.name }}</div>
                <n-space>
                  <n-button size="small" type="primary" @click="openReleases(updateResult.latest?.url)">
                    去看看
                  </n-button>
                </n-space>
              </n-alert>
              <div v-else-if="updateResult" class="muted">
                已是最新（v{{ updateResult.current }}）。
              </div>
            </n-space>
          </n-card>
        </template>

        <!-- 快捷键 -->
        <template v-if="activeKey === 'shortcuts'">
          <n-card size="small" title="快捷键">
            <n-space vertical size="medium">
              <div v-for="bucket in grouped" :key="bucket.group">
                <div class="group-title">{{ bucket.group }}</div>
                <table class="table">
                  <tbody>
                    <tr v-for="row in bucket.rows" :key="row.keys">
                      <td class="keys">
                        <n-tag size="small" :bordered="false">{{ row.keys }}</n-tag>
                      </td>
                      <td>{{ row.label }}</td>
                      <td class="when">{{ row.when }}</td>
                    </tr>
                  </tbody>
                </table>
              </div>

              <div>
                <div class="group-title">主区页签直达</div>
                <table class="table">
                  <tbody>
                    <tr v-for="tab in TAB_SHORTCUTS" :key="tab.key">
                      <td class="keys">
                        <n-tag size="small" :bordered="false">{{ tab.key }}</n-tag>
                      </td>
                      <td>切到「{{ tab.label }}」</td>
                      <td class="when"></td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </n-space>
          </n-card>
        </template>

        <!-- AI 模型 -->
        <template v-if="activeKey === 'ai'">
          <n-card size="small" title="AI 模型接入">
            <n-space vertical size="small">
              <div class="muted">
                填写 OpenAI 兼容接口（可接 OpenAI / DeepSeek / 通义 / 本地 Ollama 等任意
                <code>/chat/completions</code> 服务）。配置后在提交页点「AI 生成」，会依据
                <b>已暂存</b>的改动生成一条 Conventional Commits 风格的提交信息。
              </div>
              <div class="ai-field">
                <span class="ai-label">Base URL</span>
                <n-input
                  :value="aiDraft.endpoint"
                  size="small"
                  placeholder="https://api.openai.com/v1"
                  :disabled="!prefs"
                  @update:value="(value: string) => setAi('endpoint', value)"
                  @blur="saveAi"
                />
              </div>
              <div class="ai-field">
                <span class="ai-label">API Key</span>
                <n-input
                  :value="aiDraft.apiKey"
                  size="small"
                  type="password"
                  show-password-on="click"
                  placeholder="sk-..."
                  :disabled="!prefs"
                  @update:value="(value: string) => setAi('apiKey', value)"
                  @blur="saveAi"
                />
              </div>
              <div class="ai-field">
                <span class="ai-label">模型</span>
                <n-input
                  :value="aiDraft.model"
                  size="small"
                  placeholder="gpt-4o-mini / deepseek-chat / qwen-..."
                  :disabled="!prefs"
                  @update:value="(value: string) => setAi('model', value)"
                  @blur="saveAi"
                />
              </div>
              <div class="muted">
                Base URL 填到 <code>/v1</code> 即可，<code>/chat/completions</code> 会自动补上。
              </div>
              <div class="muted warn-line">
                API Key 由你自己提供：本应用不内置、不附带、不分发任何 key，只会用你填的这一个。
                它以明文保存在本机偏好文件（设置 → 存储位置可查看路径）中，请仅在可信设备填写。
              </div>
              <n-alert v-if="aiIssue" type="warning" :bordered="false">{{ aiIssue }}</n-alert>
              <n-tag v-else size="small" :bordered="false" type="success">
                已配置，可在提交页使用
              </n-tag>
              <div class="muted">
                启用后，提交页的「AI 生成」会把<b>已暂存</b>的改动发给这个端点起草提交信息，
                结果先进弹窗由你确认，不自动提交。未配置时一个请求都不发。
              </div>
            </n-space>
          </n-card>
        </template>

        <!-- 存储位置 -->
        <template v-if="activeKey === 'storage'">
          <n-card size="small" title="这些设置存在哪">
            <code class="path">{{ prefsStore.path || "读取中" }}</code>
            <div class="muted">
              个人偏好跟着这台机器走，不进仓库；团队规范在仓库里的
              <code>git-tidy.config.json</code>（或 commitlint / versionrc / cliff）——两者不互相覆盖。
            </div>
            <n-space style="margin-top: 12px">
              <n-button size="small" quaternary @click="prefsStore.reset">恢复默认偏好</n-button>
            </n-space>
          </n-card>
        </template>
      </div>
    </div>
  </n-modal>
</template>

<style scoped>
.modal-header {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
}

.settings-body {
  display: flex;
  height: 560px;
  margin: -16px -24px -20px;
}

.settings-nav {
  flex: none;
  width: 200px;
  padding: 12px 0;
  border-right: 1px solid var(--border);
  background: var(--surface-app);
}

.settings-content {
  flex: 1;
  min-width: 0;
  padding: 16px 24px 24px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.label {
  font-size: 12px;
  color: var(--text-2);
  width: 150px;
  flex: none;
}

.group-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-3);
  margin-bottom: 4px;
}

.table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}

.table td {
  padding: 3px 8px 3px 0;
  border-bottom: 1px solid var(--surface-sunken);
  vertical-align: top;
}

.keys {
  width: 130px;
}

.when {
  opacity: 0.65;
  width: 260px;
}

.path {
  display: block;
  margin-bottom: 6px;
  font-size: 12px;
  word-break: break-all;
}

.muted {
  font-size: 12px;
  opacity: 0.7;
}

.ai-field {
  display: flex;
  align-items: center;
  gap: 10px;
}

.ai-label {
  flex: none;
  width: 84px;
  font-size: 12px;
  color: var(--text-2);
}

.warn-line {
  color: var(--warn-text);
  opacity: 1;
}
</style>

<style>
/* Naive UI 弹窗卡片自身的样式需要在 :global 作用域下覆盖 */
.n-card.settings-modal {
  width: 760px;
  max-width: 90vw;
}

.n-card.settings-modal .n-card__content {
  padding: 16px 24px 20px;
}

.n-card.settings-modal .n-card__header {
  padding: 14px 24px;
  border-bottom: 1px solid var(--border);
}

.settings-nav .n-menu .n-menu-item-content {
  height: 34px;
  line-height: 34px;
  font-size: 13px;
}
</style>
