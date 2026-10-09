<script setup lang="ts">
import { computed, ref } from "vue";
import { NButton, NEmpty, NInput, NSelect, useDialog, useMessage } from "naive-ui";
import { useWriteStore } from "@/stores/write";
import { useReposStore } from "@/stores/repos";
import type { RewriteAction, TodoItem } from "@/api/write";

/**
 * 交互式改写面板（§7.14）。
 *
 * todo 的顺序与动作全在这里定，定完整份交给 Rust 在临时分支上执行。本面板不做任何
 * git 操作，所以排错了不会有任何副作用——真正动手之前仓库一动不动。
 *
 * 三条界面上的硬边界：
 * - 只支持从 HEAD 往回的**连续区间**，底端由用户在提交列表上选定；
 * - squash / fixup 打头时不给执行入口（它们没有可并的对象），而不是点了再报错；
 * - 有 drop 时要显式确认：drop 是唯一会真的丢掉内容的动作。
 */
const writes = useWriteStore();
const repos = useReposStore();
const dialog = useDialog();
const message = useMessage();

const ACTION_OPTIONS: { label: string; value: RewriteAction }[] = [
  { label: "保留", value: "pick" },
  { label: "改信息", value: "reword" },
  { label: "并入前一条（信息追加）", value: "squash" },
  { label: "并入前一条（信息丢弃）", value: "fixup" },
  { label: "丢弃", value: "drop" },
];

const collapsed = ref(false);
/** 正在展开提交信息输入框的那一条 */
const editing = ref<string | null>(null);

const todo = computed(() => writes.rewriteTodo);
const entries = computed(() => writes.rewriteEntries);

const subjectOf = (sha: string) => entries.value.find((e) => e.sha === sha)?.subject ?? sha;
const authorOf = (sha: string) => {
  const entry = entries.value.find((e) => e.sha === sha);
  return entry ? `${entry.authorName} · ${entry.authoredAt.slice(0, 10)}` : "";
};

const dropRows = computed(() => todo.value.filter((item) => item.action === "drop"));
const squashed = computed(
  () => todo.value.filter((i) => i.action === "squash" || i.action === "fixup"),
);
const reworded = computed(() => todo.value.filter((i) => i.action === "reword"));

function canWrite() {
  return writes.canWrite && !writes.busy;
}

/** 选完动作之后顺手修正 message：改成 squash/fixup 就不该再留着一条独立的信息 */
function setAction(item: TodoItem, action: RewriteAction) {
  item.action = action;
  if (action !== "reword") item.message = null;
  if (action === "reword") {
    item.message = item.message ?? subjectOf(item.sha);
    editing.value = item.sha;
  }
}

function move(index: number, delta: number) {
  const target = index + delta;
  if (target < 0 || target >= todo.value.length) return;
  const next = [...todo.value];
  const [row] = next.splice(index, 1);
  next.splice(target, 0, row);
  writes.rewriteTodo = next;
}

function reset() {
  writes.rewriteTodo = todo.value.map((item) => ({ sha: item.sha, action: "pick" as const, message: null }));
  editing.value = null;
}

async function execute() {
  const problem = writes.rewriteProblem;
  if (problem) {
    message.warning(problem);
    return;
  }
  await writes.runRewrite();
}

function confirmExecute() {
  // drop 之外的情况也要拦一道：这段区间一旦重排，sha 与提交号就对不上了，
  // 用户回不到"刚才看到的那条提交"，只能靠还原点
  const note = [
    dropRows.value.length ? `丢弃 ${dropRows.value.length} 条提交` : null,
    squashed.value.length ? `合并 ${squashed.value.length} 条` : null,
    reworded.value.length ? `改写 ${reworded.value.length} 条信息` : null,
  ]
    .filter(Boolean)
    .join("，");

  dialog.warning({
    title: "执行改写",
    content: `${note}。改写完成后分支会指向新的提交，还原点记为 ${
      writes.lastBackupRef ?? "（执行时生成）"
    }。`,
    positiveText: "执行",
    negativeText: "再想想",
    onPositiveClick: execute,
  });
}
</script>

<template>
  <div class="rewrite">
    <div class="head toggle" :class="{ collapsed }" @click="collapsed = !collapsed">
      <span class="chevron" aria-hidden="true" />
      <span>改写 {{ todo.length }}</span>
      <span v-if="writes.rewriteBase" class="muted">
        底端 {{ writes.rewriteBase.slice(0, 8) }}
      </span>
    </div>

    <template v-if="!collapsed">
      <div v-if="writes.rewriteSize?.large" class="warn">
        区间里有 {{ writes.rewriteSize.count }} 条提交。每条是一次 git 进程，
        跑起来要等一会儿，中途不要关窗口。
      </div>

      <div v-if="repos.interrupted" class="warn">
        现在停在「{{ repos.interrupt }}」上，改写前先退回。
      </div>

      <div v-if="todo.length" class="list">
        <div v-for="(item, index) in todo" :key="item.sha" class="row" :class="item.action">
          <div class="row-main">
            <code class="sha">{{ item.sha.slice(0, 8) }}</code>
            <span class="subject" :title="subjectOf(item.sha)">{{ subjectOf(item.sha) }}</span>
            <span class="muted author">{{ authorOf(item.sha) }}</span>
          </div>
          <div class="row-side">
            <n-select
              size="tiny"
              :value="item.action"
              :options="ACTION_OPTIONS"
              :disabled="!canWrite()"
              class="action"
              @update:value="(value) => setAction(item, value as RewriteAction)"
            />
            <n-button size="tiny" quaternary :disabled="index === 0 || !canWrite()" @click="move(index, -1)">
              ↑
            </n-button>
            <n-button
              size="tiny"
              quaternary
              :disabled="index === todo.length - 1 || !canWrite()"
              @click="move(index, 1)"
            >
              ↓
            </n-button>
          </div>
          <div v-if="item.action === 'reword'" class="reword">
            <n-input
              v-model:value="item.message"
              type="textarea"
              size="small"
              :autosize="{ minRows: 2, maxRows: 8 }"
              placeholder="新的提交信息"
              :disabled="!canWrite()"
              @focus="editing = item.sha"
            />
          </div>
        </div>
      </div>
      <n-empty v-else size="small" description="在提交列表上右键某一条，选「从这里开始改写」" />

      <div v-if="writes.rewriteProblem" class="problem">{{ writes.rewriteProblem }}</div>

      <div class="actions">
        <n-button size="small" :disabled="!todo.length || !canWrite()" @click="reset">全部重置</n-button>
        <n-button size="small" @click="writes.closeRewrite()">关掉</n-button>
        <n-button
          size="small"
          type="primary"
          :loading="writes.busy"
          :disabled="!canWrite() || !todo.length || !writes.rewriteDirty || !!writes.rewriteProblem"
          @click="confirmExecute"
        >
          执行改写
        </n-button>
      </div>

      <div v-if="writes.syncAction === 'rewrite'" class="progress">
        <span v-for="(line, i) in writes.syncLines" :key="i" class="line">{{ line }}</span>
      </div>
    </template>
  </div>
</template>

<style scoped>
.rewrite {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.head {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
}

.head.toggle {
  cursor: pointer;
  user-select: none;
  padding: 2px 4px;
  margin: -2px -4px;
  border-radius: 4px;
}

.head.toggle:hover {
  background: var(--surface-hover);
}

.chevron {
  flex: none;
  width: 0;
  height: 0;
  border: 4px solid transparent;
  border-top-color: currentColor;
  margin-top: 3px;
  transition: transform 120ms ease;
}

.head.collapsed .chevron {
  transform: rotate(-90deg);
  margin-top: 0;
}

.list {
  display: flex;
  flex-direction: column;
  max-height: 320px;
  overflow: auto;
}

.row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 2px 6px;
  padding: 3px 0;
  border-bottom: 1px solid var(--surface-sunken);
}

/* 被丢掉的行压暗：它是唯一一个"点了就真的没了"的动作，得一眼看出来 */
.row.drop .subject,
.row.drop .sha {
  opacity: 0.45;
  text-decoration: line-through;
}

.row-main {
  display: flex;
  align-items: baseline;
  gap: 6px;
  flex: 1;
  min-width: 0;
}

.sha {
  flex: none;
  color: var(--text-3);
  font-size: 11px;
}

.subject {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.author {
  flex: none;
  font-size: 11px;
}

.row-side {
  display: flex;
  align-items: center;
  gap: 2px;
  flex: none;
}

.action {
  width: 168px;
}

.reword {
  flex-basis: 100%;
  padding: 2px 0 4px;
}

.actions {
  display: flex;
  gap: 4px;
}

.warn {
  padding: 6px 8px;
  background: var(--warn-bg);
  border: 1px solid var(--warn-border);
  border-radius: 6px;
  font-size: 11px;
}

.problem {
  padding: 4px 8px;
  border-left: 2px solid var(--warn-border);
  font-size: 11px;
  color: var(--text-2);
}

.progress {
  display: flex;
  flex-direction: column;
  font-size: 11px;
  /* 用等宽字体（tokens.ts 里的 FONT_MONO 那一档），不走自定义属性：
     `--font-mono` 不是颜色 token，check:tokens 会报它；而带兜底值的自定义属性
     正是会静默失效的那种写法 */
  font-family: Consolas, "Courier New", monospace;
  opacity: 0.75;
  max-height: 76px;
  overflow: hidden;
}

.muted {
  font-size: 11px;
  opacity: 0.7;
}
</style>