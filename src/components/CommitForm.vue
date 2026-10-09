<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  NAlert,
  NAutoComplete,
  NButton,
  NCard,
  NInput,
  NSelect,
  NSpace,
  NTag,
} from "naive-ui";
import { GitTidyError } from "@/api/client";
import { provideShell } from "@/shell";
import {
  commitCreate,
  commitScopes,
  messageCheck,
  specFor,
  SOURCE_LABEL,
  type Draft,
  type Outcome,
  type Spec,
} from "@/api/spec";

/**
 * 规范可视化提交表单（需求 6.2）。
 *
 * 关键约束：合规判定一律由 Rust 侧 `message_check` 给出，前端不自己写规则。
 * 表单里看到的"能不能提交"、commit-msg hook 的拦截、符合率报告的计数，
 * 走的是同一个内核，所以不会出现"表单放过、hook 拦下"这种解释不了的报错。
 */
/**
 * locked = 仓库正卡在某个中断态里（合并/变基/摘取/回滚未收尾）。
 * 这时候按钮必须灰：Rust 侧的 `commit_create` 同样会拒，前端先挡一层是为了
 * 让人在按下之前就看见原因，而不是收到一条错误码。
 */
const props = defineProps<{ repoId: number; locked: boolean }>();
const emit = defineEmits<{ (event: "committed"): void }>();

/** 输入停顿后再发 IPC：逐字符发会把每次按键变成一次磁盘读配置 */
const CHECK_DELAY_MS = 200;

const spec = ref<Spec | null>(null);
const scopes = ref<string[]>([]);

const type = ref<string | null>(null);
const scope = ref("");
const description = ref("");
const body = ref("");
const footer = ref("");
const taskId = ref("");

const outcome = ref<Outcome | null>(null);
const checking = ref(false);
const submitting = ref(false);
const error = ref<GitTidyError | null>(null);

const typeOptions = computed(() =>
  (spec.value?.types ?? []).map((value) => ({ label: value, value })),
);
const scopeOptions = computed(() => scopes.value);

/** 头部由三段拼出来，用户不必记得标点在哪儿 */
const subject = computed(() => {
  if (!type.value && !description.value.trim()) return "";
  const scopePart = scope.value.trim() ? `(${scope.value.trim()})` : "";
  return `${type.value ?? ""}${scopePart}: ${description.value.trim()}`;
});

/** 任务 ID 单独填，拼成脚注的一行；正则匹配的正是这段文本里的内容 */
const composedFooter = computed(() => {
  const id = taskId.value.trim();
  if (!id) return footer.value;
  const rest = footer.value.trim();
  return rest ? `${rest}\nRefs: ${id}` : `Refs: ${id}`;
});

const draft = computed<Draft>(() => ({
  subject: subject.value,
  body: body.value,
  footer: composedFooter.value,
}));

/** 预览必须和真正落库的那条一致：这段与 Rust 侧 `Draft::compose` 同形
 *  （三段各 trim、空段丢掉、空行分隔、末尾一个换行）。改那边要改这里。 */
const preview = computed(() => {
  const parts = [
    draft.value.subject.trim(),
    draft.value.body.trim(),
    draft.value.footer.trim(),
  ].filter((part) => part.length > 0);
  if (!parts.length) return "";
  return `${parts.join("\n\n")}\n`;
});

const subjectLength = computed(() => [...subject.value].length);
const overLong = computed(
  () => !!spec.value && subjectLength.value > spec.value.subjectMaxLength,
);

const blockers = computed(() => outcome.value?.violations.filter((item) => item.blocking) ?? []);
const warnings = computed(() => outcome.value?.violations.filter((item) => !item.blocking) ?? []);

const canSubmit = computed(
  () =>
    !!outcome.value?.conformant && !checking.value && !submitting.value && !props.locked,
);

let timer: ReturnType<typeof setTimeout> | undefined;
/** 只认最后一次请求的结果，否则慢的那次会把新草稿的判定覆盖掉 */
let checkSeq = 0;

function scheduleCheck() {
  if (timer !== undefined) clearTimeout(timer);
  timer = setTimeout(runCheck, CHECK_DELAY_MS);
}

async function runCheck() {
  const seq = ++checkSeq;
  checking.value = true;
  try {
    const result = await messageCheck(props.repoId, draft.value);
    if (seq === checkSeq) {
      outcome.value = result;
      error.value = null;
    }
  } catch (err) {
    if (seq === checkSeq) error.value = asError(err);
  } finally {
    if (seq === checkSeq) checking.value = false;
  }
}

function asError(err: unknown) {
  return err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
}

function clearDraft() {
  type.value = null;
  scope.value = "";
  description.value = "";
  body.value = "";
  footer.value = "";
  taskId.value = "";
  outcome.value = null;
}

async function open() {
  if (timer !== undefined) clearTimeout(timer);
  // 作废所有在途校验：不然上一个仓库的判定会写进新仓库刚清空的表单
  checkSeq++;
  const repoId = props.repoId;
  spec.value = null;
  scopes.value = [];
  clearDraft();
  error.value = null;
  try {
    const [loaded, historical] = await Promise.all([
      specFor(repoId),
      commitScopes(repoId),
    ]);
    if (repoId !== props.repoId) return;
    spec.value = loaded;
    scopes.value = historical;
    await runCheck();
  } catch (err) {
    if (repoId === props.repoId) error.value = asError(err);
  }
}

async function submit() {
  // 回车也走这里：禁用态只挡得住按钮，挡不住 keyup
  if (!canSubmit.value) return;
  submitting.value = true;
  error.value = null;
  try {
    await commitCreate(props.repoId, draft.value);
    clearDraft();
    emit("committed");
  } catch (err) {
    // Rust 侧兜底拦下的（§7.5）：文案里带的是同一条内核给出的原因，和表单提示一致
    error.value = asError(err);
  } finally {
    submitting.value = false;
  }
}

watch(draft, scheduleCheck);
watch(() => props.repoId, open);

onMounted(open);
onUnmounted(() => {
  if (timer !== undefined) clearTimeout(timer);
});

/**
 * Ctrl+Enter 提交（需求 7.23）与按钮、回车走的是同一个 `submit`。
 *
 * 写在这里而不是只写在 App.vue：能不能提交取决于本地草稿判定（`canSubmit`），
 * 外壳拿不到这个状态，硬接就变成“绕过表单校验直接提交”。
 */
let release: (() => void) | undefined;
onMounted(() => {
  release = provideShell("submit", () => {
    void submit();
  });
});
onUnmounted(() => release?.());
</script>

<template>
  <n-card title="提交" size="small">
    <template #header-extra>
      <n-space align="center" size="small">
        <n-tag size="small" :bordered="false" type="info">
          规范来源：{{ spec ? SOURCE_LABEL[spec.source] : "读取中" }}
        </n-tag>
        <n-button size="small" :loading="checking" @click="open">重新读取</n-button>
      </n-space>
    </template>

    <n-space vertical size="medium">
      <n-space align="center" size="small">
        <!-- 仓库没给白名单（"任何 `word: ` 都算有 type"）时放开手输，否则下拉是空的 -->
        <n-select
          v-model:value="type"
          :options="typeOptions"
          placeholder="type"
          style="width: 150px"
          :loading="!spec"
          filterable
          :tag="typeOptions.length === 0"
        />
        <n-auto-complete
          v-model:value="scope"
          :options="scopeOptions"
          :placeholder="spec?.scopeRequired ? 'scope（本仓库必填）' : 'scope（可选）'"
          style="width: 190px"
          clearable
        />
      </n-space>

      <n-input
        v-model:value="description"
        placeholder="描述这次改动，例如：修复导出 CSV 时列错位"
        @keyup.enter="submit"
      />

      <div class="counter" :class="{ over: overLong }">
        标题 {{ subjectLength }} / {{ spec?.subjectMaxLength ?? 72 }} 字符
        <span v-if="overLong" class="muted">超长只告警，不拦截</span>
      </div>

      <n-input
        v-model:value="body"
        type="textarea"
        placeholder="正文（可选）：为什么这么改、影响范围"
        :autosize="{ minRows: 3, maxRows: 8 }"
      />

      <n-input
        v-model:value="footer"
        type="textarea"
        placeholder="脚注（可选）：Refs: #123 / Closes: #456"
        :autosize="{ minRows: 2, maxRows: 4 }"
      />

      <n-input
        v-if="spec?.taskIdPattern"
        v-model:value="taskId"
        :placeholder="`任务 ID，本仓库要求匹配 ${spec?.taskIdPattern ?? ''}`"
        style="width: 320px"
      />

      <div v-if="preview" class="preview">
        <div class="preview-label">最终提交信息</div>
        <pre class="preview-text">{{ preview }}</pre>
      </div>

      <div v-if="blockers.length || warnings.length" class="violations">
        <div v-for="item in [...blockers, ...warnings]" :key="item.reason" class="violation">
          <n-tag :type="item.blocking ? 'error' : 'warning'" size="small" :bordered="false">
            {{ item.blocking ? "拦截" : "告警" }}
          </n-tag>
          <div class="violation-text">
            <span class="violation-title">{{ item.title }}</span>
            <span class="muted">{{ item.hint }}</span>
          </div>
        </div>
      </div>
      <div v-else-if="outcome?.conformant" class="muted">这条信息符合本仓库规范</div>

      <n-alert v-if="error" type="error" :title="error.message">
        <div>错误码：{{ error.code }}</div>
        <pre v-if="error.detail" class="raw-output">{{ error.detail }}</pre>
      </n-alert>

      <n-space align="center">
        <n-button type="primary" :disabled="!canSubmit" :loading="submitting" @click="submit">
          提交
        </n-button>
        <span v-if="locked" class="muted">
          有操作卡在半路，先完成或中止（见顶部提示条），期间不能提交
        </span>
        <span v-else-if="!outcome?.conformant" class="muted">
          {{ checking ? "校验中" : "有拦截级问题，改完再提交" }}
        </span>
      </n-space>
    </n-space>
  </n-card>
</template>

<style scoped>
.counter {
  font-size: 12px;
  opacity: 0.75;
}

.counter.over {
  opacity: 1;
  color: var(--warn-text);
}

.preview {
  border: 1px dashed var(--border);
  border-radius: 6px;
  padding: 8px 10px;
}

.preview-label {
  font-size: 12px;
  opacity: 0.65;
  margin-bottom: 4px;
}

.preview-text {
  margin: 0;
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-word;
}

.violations {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.violation {
  display: flex;
  align-items: flex-start;
  gap: 8px;
}

.violation-text {
  display: flex;
  flex-direction: column;
  font-size: 12px;
  line-height: 1.5;
}

.violation-title {
  font-weight: 600;
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
