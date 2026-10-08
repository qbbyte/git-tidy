<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { NButton, NCollapse, NCollapseItem, NEmpty, NInput, NPopconfirm } from "naive-ui";
import type { Conflict, Resolution } from "@/api/write";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";
import { assemble, parseConflict, type HunkChoice } from "@/lib/conflict";

/**
 * 合并冲突解决器（§7.13）。
 *
 * 视图按 §7.13 的降级穷举分三种，不是三套页面而是同一个组件的三种用法：
 * - `content`：三方并排 + 合并结果区，逐块「取我 / 取对 / 手改」
 * - 二进制 / 子模块 / 双改名 / 改删 / 删改 / 两边新增：只给「选一边」
 *   （二进制与子模块没有逐块合并的余地，给一个假的合并区只会让人写出坏文件）
 * - 工作区没有冲突标记（被手工改过）时不给合并区，让用户看着原文自己写一份
 *
 * 手改内容归用户负责：写回去的就是他在结果区里看到的那份，我们不生成、不修正。
 */
const repos = useReposStore();
const writes = useWriteStore();

/** 展开的卡片路径。同一时刻只开一张，避免并排三栏滚出屏幕 */
const openPath = ref<string | null>(null);
/** 结果区的正文。逐块选完之后实时更新，用户也可以直接改 */
const result = ref("");
/** 每一块选了哪一边。null = 还没决定 */
const choices = ref<(HunkChoice | null)[]>([]);
const showResult = ref(false);

const conflicts = computed(() => writes.conflicts);
/** 还有没决定的块。没定完不许续跑——否则写回去的是一份带着空洞的结果 */
const undecided = computed(() => choices.value.filter((choice) => choice === null).length);

function canResolve() {
  // 冲突解决器只能在中断态里干活，但 `canWrite` 恰好在中断态时是 false，所以单独算
  return repos.canCommit && !writes.busy;
}

/**
 * 三方是否都还在且都是文本。
 *
 * 直接用 Rust 给的 `threeWay` 标志，不从 `sides` 反推：那边是看过索引里三个 stage
 * 之后得出的结论。界面自己猜会在改删、子模块这些三栏不全的情形下猜错，
 * 然后给用户开一个拼不出正确结果的合并区（§7.13 的降级穷举）。
 */
function threeWay(conflict: Conflict) {
  return conflict.threeWay;
}

/** 这一栏在不在。改删/删改里有一方是空的，那一栏就不画 */
function side(conflict: Conflict, which: "base" | "ours" | "theirs") {
  const content = conflict.sides[which];
  if (content == null) return null;
  return content.binary ? null : content;
}

function open(conflict: Conflict) {
  openPath.value = conflict.path;
  showResult.value = false;
  // 用 git 自己标好的冲突块做初始值：每块默认取我方，用户逐块改成要的那一边。
  // 不预选 `null`——那样每个人都得点一遍才知道哪些块要改，而 git 已经告诉我们冲突在哪了
  const parsed = conflict.worktreeText ? parseConflict(conflict.worktreeText) : null;
  choices.value = parsed ? parsed.hunk.map(() => "ours" as HunkChoice) : [];
  result.value = conflict.worktreeText ?? "";
}

/** 折叠面板的展开项变化。收起时把编辑状态清掉，否则下次展开看到的是上一次的选择 */
function onExpand(value: string | string[] | null) {
  const path = typeof value === "string" ? value : (value?.[0] ?? null);
  if (path === null) {
    openPath.value = null;
    showResult.value = false;
    return;
  }
  const conflict = conflicts.value.find((item) => item.path === path);
  if (conflict && conflict.path !== openPath.value) open(conflict);
  else openPath.value = path;
}

function rebuild() {
  const conflict = current.value;
  if (!conflict?.worktreeText) return;
  const parsed = parseConflict(conflict.worktreeText);
  if (!parsed) return;
  result.value = assemble(parsed, choices.value);
  showResult.value = true;
}

function choose(index: number, choice: HunkChoice) {
  choices.value = choices.value.map((item, i) => (i === index ? choice : item));
  rebuild();
}

const current = computed(() => conflicts.value.find((item) => item.path === openPath.value) ?? null);

/** 当前卡片的冲突块。用户改坏标记时是空数组，此时不给逐块按钮 */
const hunks = computed(() => {
  const text = current.value?.worktreeText;
  return (text ? parseConflict(text)?.hunk : undefined) ?? [];
});

/** 工作区里的标记被改坏了：不给逐块合并，让用户对着原文写一份 */
const markersBroken = computed(() => {
  const conflict = current.value;
  if (!conflict?.worktreeText) return true;
  return parseConflict(conflict.worktreeText) === null;
});

async function resolve(conflict: Conflict, apply: Resolution | null) {
  const done = await writes.resolveConflict(conflict.path, apply);
  if (done === null) return;
  if (openPath.value === conflict.path) openPath.value = null;
}

/** 取一边：那一栏是二进制或不存在时按钮本来就是禁的，不会走到这里 */
async function takeSide(conflict: Conflict, which: "ours" | "theirs") {
  await resolve(conflict, { how: which });
}

/** 接受删除：改删/删改里有一方已经把文件删了 */
async function acceptDeletion(conflict: Conflict) {
  await resolve(conflict, null);
}

async function applyResult(conflict: Conflict) {
  if (undecided.value > 0) return;
  await resolve(conflict, { how: "text", text: result.value });
}

/** 续跑收尾。`finished` 为 false 时下一个提交又冲突了，列表会自己多出卡片 */
async function continueRunning() {
  const done = await writes.continueOperation(null);
  if (done?.finished) openPath.value = null;
}

// 换仓库时收起全部卡片：上一个仓库的文件名留在这里会误导
watch(
  () => repos.currentId,
  () => {
    openPath.value = null;
    showResult.value = false;
  },
);
</script>

<template>
  <div class="resolver">
    <div class="head">
      <span>冲突 {{ conflicts.length }}</span>
      <span class="muted">
        {{ repos.interrupt ? `停在「${repos.interrupt}」中` : "没有进行中的操作" }}
      </span>
    </div>

    <n-empty v-if="!conflicts.length" size="small" description="没有待解决的冲突" />

    <n-collapse v-else accordion @update:value="onExpand">
      <n-collapse-item v-for="conflict in conflicts" :key="conflict.path" :title="conflict.path">
        <div class="meta">
          <code class="status">{{ conflict.status }}</code>
          <span class="muted">{{ conflict.label }}</span>
        </div>

        <div v-if="conflict.otherPath" class="other muted">
          对端路径：<code>{{ conflict.otherPath }}</code>
        </div>

        <!-- 三方并排：只有三栏都在且都是文本才有意义 -->
        <div v-if="threeWay(conflict)" class="sides">
          <div v-for="which in (['base', 'ours', 'theirs'] as const)" :key="which" class="side">
            <div class="side-title muted">
              {{ which === "base" ? "共同祖先" : which === "ours" ? "我方（HEAD）" : "对方" }}
            </div>
            <pre v-if="side(conflict, which)" class="body">{{ side(conflict, which)?.text }}</pre>
            <div v-else class="body muted">（无）</div>
          </div>
        </div>

        <!-- 二进制与子模块只说选一边，不给正文。
             `pickSideOnly` 是 Rust 侧给的结论，不自己判断哪一栏缺。 -->
        <div v-else-if="conflict.sides.binary" class="muted">
          这个文件没有可合并的正文（{{ conflict.label }}）。
        </div>

        <!-- 逐块取舍 -->
        <div v-if="threeWay(conflict) && hunks.length && !markersBroken" class="hunks">
          <div class="muted">
            git 在工作区里标出了 {{ hunks.length }} 处冲突，逐块定夺：
          </div>
          <div v-for="(hunk, index) in hunks" :key="hunk.startLine" class="hunk">
            <div class="hunk-head muted">
              第 {{ index + 1 }} 处（{{ conflict.worktreeText?.split("\n").slice(hunk.startLine, hunk.endLine + 1).length }} 行）
            </div>
            <div class="hunk-body">
              <pre class="side-body">{{ hunk.ours.join("\n") }}</pre>
              <pre class="side-body alt">{{ hunk.theirs.join("\n") }}</pre>
            </div>
            <div class="hunk-actions">
              <n-button
                size="tiny"
                :type="choices[index] === 'ours' ? 'primary' : 'default'"
                :disabled="!canResolve()"
                @click="choose(index, 'ours')"
              >
                取我
              </n-button>
              <n-button
                size="tiny"
                :type="choices[index] === 'theirs' ? 'primary' : 'default'"
                :disabled="!canResolve()"
                @click="choose(index, 'theirs')"
              >
                取对
              </n-button>
              <n-button
                size="tiny"
                quaternary
                :disabled="!canResolve()"
                @click="choose(index, 'base')"
              >
                回到共同祖先
              </n-button>
            </div>
          </div>

          <div class="actions">
            <n-button size="small" :disabled="!canResolve()" @click="rebuild">看合并结果</n-button>
            <n-popconfirm
              :disabled="undecided > 0"
              positive-text="写回并标记已解决"
              negative-text="再想想"
              @positive-click="applyResult(conflict)"
            >
              <template #trigger>
                <n-button size="small" type="primary" :disabled="!canResolve() || undecided > 0">
                  用这个结果
                </n-button>
              </template>
              {{ undecided > 0 ? `还有 ${undecided} 处没定夺` : "写回的就是你在结果区看到的那份内容" }}
            </n-popconfirm>
          </div>
        </div>

        <!-- 工作区里的标记被改坏了：不给逐块合并，让人对着原文写 -->
        <div v-if="markersBroken" class="broken">
          <div class="muted">
            工作区里这份文件的冲突标记不完整，不能逐块合并。写一份完整内容，或选一边。
          </div>
          <n-input
            v-model:value="result"
            type="textarea"
            :autosize="{ minRows: 6, maxRows: 20 }"
            placeholder="写出合并后的完整内容"
          />
          <n-button size="small" type="primary" :disabled="!canResolve() || !result" @click="applyResult(conflict)">
            写回并标记已解决
          </n-button>
        </div>

        <div v-if="showResult" class="result">
          <div class="muted">合并结果（这就是写回磁盘的内容）：</div>
          <pre class="body">{{ result }}</pre>
        </div>

        <div class="actions">
          <n-button
            v-if="side(conflict, 'ours') || conflict.kind === 'content'"
            size="small"
            :disabled="!canResolve() || conflict.sides.ours == null"
            @click="takeSide(conflict, 'ours')"
          >
            整份取我
          </n-button>
          <n-button
            size="small"
            :disabled="!canResolve() || conflict.sides.theirs == null"
            @click="takeSide(conflict, 'theirs')"
          >
            整份取对方
          </n-button>
          <n-button
            v-if="conflict.pickSideOnly && (conflict.sides.ours == null || conflict.sides.theirs == null)"
            size="small"
            type="error"
            :disabled="!canResolve()"
            @click="acceptDeletion(conflict)"
          >
            接受删除
          </n-button>
        </div>
      </n-collapse-item>
    </n-collapse>

    <!-- 收尾：全部标记完之后才能续跑（§7.13）。
         一键退回始终给：解决器不该把用户堵在一个解不开的状态里。 -->
    <div v-if="conflicts.length" class="foot">
      <n-button
        size="small"
        type="primary"
        :loading="writes.busy"
        :disabled="!repos.canCommit"
        @click="continueRunning"
      >
        全部解决，续跑收尾
      </n-button>
      <n-button size="small" :disabled="writes.busy" @click="writes.abort()">
        一键退回
      </n-button>
      <span class="muted">
        续跑若在下一个提交上又冲突，会回到这里；改用默认提交信息，不弹编辑器。
      </span>
    </div>
  </div>
</template>

<style scoped>
.resolver {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.head {
  display: flex;
  justify-content: space-between;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
}

.meta {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
}

.status {
  flex: none;
  padding: 0 4px;
  border-radius: 4px;
  background: var(--warn-bg);
  color: var(--warn-text);
}

.other {
  font-size: 11px;
}

/* 三方并排等分：内容长短差很多，按内容宽度分会让某几栏窄到没法读 */
.sides {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr;
  gap: 6px;
  margin: 6px 0;
}

.side-title {
  font-size: 10px;
  margin-bottom: 2px;
}

.body,
.side-body {
  margin: 0;
  padding: 4px 6px;
  border: 1px solid var(--border-soft);
  border-radius: 6px;
  background: var(--surface-sunken);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 11px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 180px;
  overflow: auto;
}

.hunks {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 6px 0;
}

.hunk {
  border: 1px solid var(--border-soft);
  border-radius: 6px;
  padding: 4px 6px;
}

.hunk-head {
  font-size: 10px;
}

.hunk-body {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 6px;
  margin: 3px 0;
}

.hunk-body .side-body {
  max-height: 120px;
}

/* 对方那栏换个底色：并排两栏同色时容易看错行对应关系 */
.side-body.alt {
  background: var(--surface-alt);
}

.hunk-actions {
  display: flex;
  gap: 4px;
  flex-wrap: wrap;
}

.actions,
.foot {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.result {
  margin: 6px 0;
}

.broken {
  display: flex;
  flex-direction: column;
  gap: 4px;
  align-items: flex-start;
  margin: 6px 0;
}

.muted {
  font-size: 11px;
  opacity: 0.7;
}
</style>
