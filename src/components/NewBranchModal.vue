<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { NAlert, NButton, NInput, NModal, NSelect, NSpace } from "naive-ui";
import { usePrefsStore } from "@/stores/prefs";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";

/**
 * 新建分支弹窗。
 *
 * 原先它是引用栏里挤出来的一个 inline 表单：三个前缀按钮 + 名字 + 起点 + 两个动作
 * 按钮，在两百多像素的栏里折成两行，还和「切换 / 上游 / 强推 / 改名 / 删除」共享
 * 同一屏高度。建分支是一件「偶尔做一次、但必须一次问清」的事，不是要常驻的一排控件，
 * 所以搬进弹窗：打开时全屏可用，关掉一分高度不占。
 *
 * 入口在引用弹窗（`RefsModal.vue`）里「分支 N」标题的右侧——建分支与分支列表同处，
 * 页头只保留一个「引用 N」入口，不再两个像是一回事的按钮并排。
 * 逻辑照抄原表单，一行没改：起点 `HEAD` / 空都归到 `null`（＝从当前 HEAD 建），
 * 建完即关，失败不关、输入留着让人直接改。
 */
const props = defineProps<{ show: boolean }>();

const emit = defineEmits<{ (e: "update:show", value: boolean): void }>();

const repos = useReposStore();
const writes = useWriteStore();
const prefsStore = usePrefsStore();

const visible = computed({
  get: () => props.show,
  set: (value: boolean) => emit("update:show", value),
});

/** Git Flow 的三类分支（需求 7.16：自组合，不依赖系统里装了 git-flow） */
const FLOW_KINDS = ["feature", "hotfix", "release"] as const;
type FlowKind = (typeof FLOW_KINDS)[number];

function flowPrefix(kind: FlowKind): string {
  return prefsStore.prefs?.flowPrefixes[kind] ?? "";
}

const branches = computed(() => repos.refs.filter((item) => item.kind === "branch"));

const newBranch = ref("");
const newBranchStart = ref<string | null>(null);

const startOptions = computed(() => [
  { label: "当前 HEAD", value: "HEAD" },
  ...branches.value.map((item) => ({ label: item.name, value: item.name })),
]);

const canCreate = computed(() => !!newBranch.value.trim() && writes.canWrite && !writes.busy);

/** 切换分支前先说清工作区脏不脏：脏的时候 Rust 侧会拒，不如提前讲 */
const dirtyHint = computed(() =>
  repos.workingFiles.length === 0 || !newBranch.value.trim()
    ? ""
    : `工作区还有 ${repos.workingFiles.length} 处改动，「创建并切换」会被拒绝（先提交或存起来）`,
);

function reset() {
  newBranch.value = "";
  newBranchStart.value = null;
}

async function create(switchToIt: boolean) {
  const name = newBranch.value.trim();
  if (!name || !canCreate.value) return;
  const start = newBranchStart.value === "HEAD" ? null : newBranchStart.value || null;
  const outcome = await writes.createBranch(name, start, switchToIt);
  if (outcome !== null) {
    reset();
    visible.value = false;
  }
}

/** 每次打开都从空白开始：上次可能是点遮罩关掉的，草稿没清 */
watch(
  () => props.show,
  (value) => {
    if (value) reset();
  },
);
</script>

<template>
  <n-modal
    v-model:show="visible"
    preset="card"
    title="新建分支"
    class="new-branch-modal"
    :bordered="false"
  >
    <n-space vertical size="medium">
      <div class="field">
        <div class="label">常用前缀</div>
        <n-space size="small" :wrap="false">
          <!--
            前缀来自个人偏好（设置页可改）。点一下只是把前缀填进名字框，
            不自动建分支——名字后面还要接用户自己写的内容，自动建会多出一堆
            `feature/feature-x`。
          -->
          <n-button
            v-for="flow in FLOW_KINDS"
            :key="flow"
            size="small"
            quaternary
            :disabled="!writes.canWrite"
            @click="newBranch = flowPrefix(flow)"
          >
            {{ flowPrefix(flow) || flow }}
          </n-button>
        </n-space>
      </div>

      <div class="field">
        <div class="label">分支名</div>
        <n-input
          v-model:value="newBranch"
          placeholder="例如 feature/login-page"
          @keyup.enter="create(true)"
        />
      </div>

      <div class="field">
        <div class="label">起点</div>
        <n-select
          v-model:value="newBranchStart"
          :options="startOptions"
          placeholder="当前 HEAD"
          clearable
        />
      </div>

      <n-alert v-if="dirtyHint" type="warning" :bordered="false">{{ dirtyHint }}</n-alert>
      <n-alert
        v-if="writes.error"
        type="error"
        :title="writes.error.message"
        :bordered="false"
      >
        <div>错误码：{{ writes.error.code }}</div>
      </n-alert>

      <n-space justify="end" size="small">
        <n-button size="small" @click="visible = false">取消</n-button>
        <n-button size="small" :disabled="!canCreate" @click="create(false)">只创建</n-button>
        <n-button
          size="small"
          type="primary"
          :disabled="!canCreate"
          :loading="writes.busy"
          @click="create(true)"
        >
          创建并切换
        </n-button>
      </n-space>
    </n-space>
  </n-modal>
</template>

<style scoped>
.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.label {
  font-size: 12px;
  color: var(--text-3);
}
</style>

<style>
/* 弹窗卡片在 body 里，scoped 样式盖不到，宽度在全局里定 */
.n-card.new-branch-modal {
  width: 460px;
  max-width: 90vw;
}
</style>
