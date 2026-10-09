<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import {
  NAlert,
  NButton,
  NIcon,
  NInput,
  NModal,
  NProgress,
  NRadioButton,
  NRadioGroup,
  type InputInst,
} from "naive-ui";
import FolderPlus from "@vicons/tabler/es/FolderPlus";
import { FONT_MONO } from "@/styles/tokens";
import { useReposStore } from "@/stores/repos";

/**
 * 添加仓库弹窗（需求：点击「添加」用弹窗而不是展开侧栏里的表单）。
 *
 * 版面按「一次只问一件事」排：来源开关 → 一个输入行 → 说明 → 底部动作。
 *
 * 两处刻意的取舍：
 *
 * 1. 目录与地址是互斥的两种来源，用分段开关（而不是同屏两块）——
 *    同屏摆着会让人以为可以填两个，分档之后「填什么、点哪个」是唯一确定的。
 *    分段开关而非页签：这里只是在两个档位之间切换，没有并列的「页面」要翻。
 *
 * 2. 「浏览…」贴在路径输入框右边，而不是丢到左下角——
 *    它是这一个字段的取值方式，离得远就成了一个不知道作用于谁的按钮。
 *
 * 成功即关：添加完仓库已经自动切过去（store 里的 add 都会 select），
 * 弹窗留着只会挡住刚打开的那个仓库。失败不关，并把输入留着让人直接改。
 */
const props = defineProps<{ show: boolean }>();

const emit = defineEmits<{ (e: "update:show", value: boolean): void }>();

const visible = computed({
  get: () => props.show,
  set: (value) => emit("update:show", value),
});

const repos = useReposStore();

type Mode = "path" | "url";
const mode = ref<Mode>("path");
const path = ref("");
const url = ref("");
const pathInput = ref<InputInst | null>(null);

/** 克隆/补齐是长任务，期间提交入口要关掉，否则第二次点击会并发抢同一个目录 */
const busy = computed(() => repos.loading || repos.progress !== null);

/** 分段开关收的是 `string | number`，这里收口成 Mode，免得类型一路漏出去 */
function setMode(value: string | number) {
  mode.value = value === "url" ? "url" : "path";
}

const actionLabel = computed(() => (mode.value === "path" ? "添加目录" : "以只读方式添加"));
const canSubmit = computed(() => !busy.value && (mode.value === "path" ? path.value : url.value).trim() !== "");

/**
 * 长任务的进度。
 *
 * Rust 侧一直在推 `repo-progress`（阶段 + 百分比），但弹窗原先一个字都不显示：
 * 贴地址添加时界面就是一个转圈的按钮，转多久、在干什么都看不出来。
 */
const progress = computed(() => repos.progress);
const percent = computed(() => {
  const value = progress.value?.percent ?? 0;
  return Math.min(100, Math.max(0, Math.round(value)));
});

/**
 * 归一化后比对注册表，提前告诉用户「这个已经在列表里了」。
 *
 * 路径统一分隔符、去尾斜杠并小写（Windows 路径大小写不敏感）；
 * 地址额外去掉 `.git` 后缀——`…/repo` 与 `…/repo.git` 是同一个仓库。
 *
 * 只是提示，不禁用按钮：后端 `repo_add` 走 `ON CONFLICT(path) DO NOTHING` 后重读，
 * 重复添加不会造出第二行，而是切到已有那条，没有需要拦住的后果。
 */
function normalize(value: string, isPath: boolean) {
  const trimmed = value.trim();
  if (!isPath) return trimmed.replace(/\/+$/, "").replace(/\.git$/i, "");
  return trimmed.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
}

const duplicate = computed(() => {
  const isPath = mode.value === "path";
  const target = normalize(isPath ? path.value : url.value, isPath);
  if (target === "") return null;
  return (
    repos.repos.find((repo) => {
      const candidate = normalize(isPath ? repo.path : (repo.remoteUrl ?? ""), isPath);
      return candidate !== "" && candidate === target;
    }) ?? null
  );
});

/** 打开时清掉上一次的内容与错误：弹窗每次都是一次新的开始 */
watch(
  () => props.show,
  async (show) => {
    if (!show) return;
    mode.value = "path";
    path.value = "";
    url.value = "";
    repos.error = null;
    // 分段开关还没渲染出来就要聚焦，所以让出一帧
    await nextTick();
    pathInput.value?.focus();
  },
);

async function submit() {
  if (!canSubmit.value) return;
  if (mode.value === "path") {
    await repos.add(path.value.trim());
    if (repos.error) return; // 失败时把路径留着，用户不必重新粘贴
    path.value = "";
  } else {
    await repos.addByUrl(url.value.trim());
    if (repos.error) return;
    url.value = "";
  }
  visible.value = false;
}

// as const 是必须的：options 的字段类型被推宽成 boolean 时，
// open() 的条件返回类型会退化成 string[] | null
async function pickDirectory() {
  const picked = await open(
    { title: "选择 Git 仓库目录", directory: true, multiple: false } as const,
  );
  if (!picked) return;
  path.value = picked;
  await submit();
}
</script>

<template>
  <n-modal
    v-model:show="visible"
    preset="card"
    :mask-closable="!busy"
    :close-on-esc="!busy"
    :bordered="false"
    class="add-modal"
    transform-origin="center"
  >
    <template #header>
      <div class="modal-header">
        <n-icon :component="FolderPlus" size="16" />
        <div class="header-text">
          <span class="header-title">添加仓库</span>
          <span class="header-sub">把磁盘上已有的仓库、或一个远程地址加入左侧列表</span>
        </div>
      </div>
    </template>

    <div class="body">
      <n-radio-group :value="mode" :disabled="busy" class="segmented" @update:value="setMode">
        <n-radio-button value="path">本地目录</n-radio-button>
        <n-radio-button value="url">远程地址</n-radio-button>
      </n-radio-group>

      <div v-if="mode === 'path'" class="field">
        <span class="field-label">目录路径</span>
        <div class="input-row">
          <n-input
            ref="pathInput"
            v-model:value="path"
            placeholder="例如 D:\project\demo 或 ~/code/demo"
            :disabled="busy"
            @keyup.enter="submit"
          />
          <n-button class="browse" :disabled="busy" @click="pickDirectory">浏览…</n-button>
        </div>
        <p class="field-hint">
          已经在磁盘上的仓库：粘贴路径或点「浏览…」都行，填子目录也可以（会归一到仓库根）。
          添加后立刻打开它。
        </p>
      </div>

      <div v-else class="field">
        <span class="field-label">仓库地址</span>
        <n-input
          v-model:value="url"
          placeholder="https://github.com/org/repo.git 或 git@host:org/repo.git"
          :disabled="busy"
          @keyup.enter="submit"
        />
        <p class="field-hint">
          只下载提交对象，不建工作区：能看提交历史、图形与报告，不能暂存、不能提交。
          想要完整能力，添加后点「克隆」。
        </p>
      </div>

      <p v-if="duplicate" class="field-note">
        已在列表里：{{ duplicate.name }}。再添加一次不会出现两条，会直接切到它。
      </p>

      <div v-if="progress" class="progress">
        <div class="progress-head">
          <span class="progress-phase">{{ progress.phase }}</span>
          <span class="progress-pct">{{ percent }}%</span>
        </div>
        <n-progress
          type="line"
          :percentage="percent"
          :show-indicator="false"
          :height="4"
          :border-radius="2"
          color="var(--accent)"
          rail-color="var(--border-soft)"
        />
      </div>

      <n-alert v-if="repos.error" class="failed" type="error" :title="repos.error.message" :bordered="false">
        <div class="failed-code">错误码：{{ repos.error.code }}</div>
        <pre v-if="repos.error.detail" class="raw-output" :style="{ fontFamily: FONT_MONO }">{{
          repos.error.detail
        }}</pre>
      </n-alert>
    </div>

    <template #footer>
      <div class="footer">
        <span v-if="!busy" class="footer-hint">回车即可添加</span>
        <span v-else />
        <div class="footer-actions">
          <n-button :disabled="busy" @click="visible = false">取消</n-button>
          <n-button type="primary" :loading="busy" :disabled="!canSubmit" @click="submit">
            {{ actionLabel }}
          </n-button>
        </div>
      </div>
    </template>
  </n-modal>
</template>

<style scoped>
.modal-header {
  display: flex;
  align-items: center;
  gap: 10px;
}

.header-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.header-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-1);
}

.header-sub {
  font-size: 12px;
  font-weight: 400;
  color: var(--text-3);
}

.body {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

/* 两个档位等宽：不等的宽度会让人以为其中一个是「主」的 */
.segmented {
  display: flex;
  width: 100%;
}

.segmented :deep(.n-radio-button) {
  flex: 1 1 0;
  display: flex;
  align-items: center;
  justify-content: center;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field-label {
  font-size: 12px;
  color: var(--text-2);
}

.input-row {
  display: flex;
  gap: 8px;
}

.input-row :deep(.n-input) {
  flex: 1 1 auto;
  min-width: 0;
}

.browse {
  flex: none;
}

.field-hint {
  margin: 0;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-3);
}

.field-note {
  margin: 0;
  font-size: 12px;
  line-height: 1.6;
  color: var(--warn-text);
}

.progress {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.progress-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  color: var(--text-2);
}

.progress-phase {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.progress-pct {
  flex: none;
  /* 数字等宽，百分比跳动时右边的框不会跟着抖 */
  font-variant-numeric: tabular-nums;
  color: var(--text-3);
}

.failed {
  margin: 0;
}

.failed-code {
  margin-top: 2px;
  font-size: 12px;
}

.raw-output {
  margin: 8px 0 0;
  max-height: 140px;
  overflow: auto;
  white-space: pre-wrap;
  font-size: 11px;
  line-height: 1.5;
}

.footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.footer-hint {
  font-size: 12px;
  color: var(--text-3);
}

.footer-actions {
  display: flex;
  gap: 8px;
}
</style>

<style>
/* Naive UI 弹窗卡片自身的宽度只能在 :global 作用域下覆盖。
   注意这里的选择器必须用 Naive 真实渲染出来的类名：头部与内容区是
   `n-card-header` / `n-card-content`（block 名），只有 footer 是元素名 `n-card__footer`——
   写成 `n-card__content` 会静默失效（不报错，只是那一处不生效）。 */
.n-card.add-modal {
  width: 520px;
  max-width: 92vw;
}

.n-card.add-modal .n-card-header {
  padding: 14px 20px;
  border-bottom: 1px solid var(--border);
}

.n-card.add-modal .n-card-content {
  padding: 16px 20px;
}

.n-card.add-modal .n-card__footer {
  padding: 12px 20px;
  border-top: 1px solid var(--border);
}
</style>
