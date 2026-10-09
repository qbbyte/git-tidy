<script setup lang="ts">
import { computed, ref } from "vue";
import {
  NAlert,
  NButton,
  NEmpty,
  NInput,
  NPopconfirm,
  NSelect,
  NSpace,
} from "naive-ui";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";
import type { Ref } from "@/api/refs";

/**
 * 分支与标签的写操作（§7.10）。
 *
 * 与 §7.3 那一批只读列表分开成另一个面板，是因为**写与读的确认强度完全不同**：
 * 读只影响摆什么，写会移动 HEAD、重写工作区、删掉本地引用，所以每一个写动作
 * 都要经过自己的确认，而读的那一列不给任何写入口（§3 的期边界）。
 *
 * 所有写操作都经 `write_guard`：工作区脏、中断态、HEAD 被别处挪动，都会在 Rust 侧被拒，
 * 面板只负责把拒绝的理由显示出来，不在这里另做一套判断。
 *
 * 「新建分支」的表单不在这里了：它连同 Git Flow 前缀按钮一起搬进了弹窗
 * （`NewBranchModal.vue`）。那排控件在栏内要折两行，而建分支是偶尔做一次的事。
 * 现在面板里只留一个「新建分支」的按钮（挂在「分支 N」标题上），点开那个弹窗。
 * 「推送」也搬走了，去了提交页的同步区（`SyncPanel.vue`）——提交完顺手就推，
 * 不必换一栏。这里只留：分支列表的写操作（切换 / 上游 / 强推 / 改名 / 删除）、
 * 抓取与拉取、以及标签。
 */
const repos = useReposStore();
const writes = useWriteStore();

/** 「新建分支」往上抛：面板不开弹窗，由宿主（RefsModal → App）去开 */
const emit = defineEmits<{ (e: "new-branch"): void }>();

const branches = computed(() => repos.refs.filter((item) => item.kind === "branch"));
const remoteBranches = computed(() => repos.refs.filter((item) => item.kind === "remote"));
const tags = computed(() => repos.refs.filter((item) => item.kind === "tag"));
const currentBranch = computed(() => repos.repoState?.branch ?? repos.info?.branch ?? null);

/** 远程名取自远程跟踪分支的前缀（origin/…）。没有远程跟踪分支时给 origin */
const remoteName = computed(() => {
  const remote = remoteBranches.value[0]?.name.split("/")[0];
  return remote ?? "origin";
});

const newTag = ref("");
const newTagMessage = ref("");
const renaming = ref<{ from: string; to: string } | null>(null);
const upstreamFor = ref<string | null>(null);
const upstreamChoice = ref<string | null>(null);
const confirmDelete = ref<{ name: string; typed: string; merged: boolean } | null>(null);

/**
 * 当前行内确认浮层开着的那一行（切换 / 强推 / 删标签）。
 *
 * 写入口是悬停才露出的：鼠标从按钮移到浮层面板上时已经离开了这一行，
 * 按钮会 display:none——浮层就失去了依附的锚点。所以浮层开着的时候把这一行的
 * 动作按钮钉住（`.pinned`），让它一直可见。
 */
const popRow = ref<string | null>(null);

function setPop(name: string, value: boolean) {
  if (value) popRow.value = name;
  else if (popRow.value === name) popRow.value = null;
}

const upstreamOptions = computed(() => [
  ...remoteBranches.value.map((item) => ({ label: item.name, value: item.name })),
]);

function trackOf(item: Ref) {
  if (item.upstreamGone) return "远程已删";
  if (item.ahead === null && item.behind === null) return item.upstream ? "已同步" : "";
  return `↑${item.ahead ?? 0} ↓${item.behind ?? 0}`;
}

function canWrite() {
  return writes.canWrite && !writes.busy;
}

/** 切换前先看一眼工作区：脏的时候切换会被 Rust 侧拒，不如自己先说清 */
const dirtyHint = computed(() =>
  repos.workingFiles.length === 0
    ? ""
    : `工作区还有 ${repos.workingFiles.length} 处改动，切换分支会被拒绝（先提交或存起来）`,
);

async function switchTo(name: string) {
  await writes.switchBranch(name, false);
}

async function startRename(item: Ref) {
  renaming.value = { from: item.name, to: item.name };
}

async function saveRename() {
  if (!renaming.value) return;
  const { from, to } = renaming.value;
  renaming.value = null;
  const trimmed = to.trim();
  if (!trimmed || trimmed === from) return;
  await writes.renameBranch(from, trimmed);
}

/**
 * 点「删除」先查"会丢多少"（`-D` 是真删，不可逆）。两条路都要二次确认，强度不同：
 * 已合并的确认一次即可（没丢提交，还能从 reflog 找回来）；
 * 未合并的还要手输分支名——它丢的是真正没进过别的分支的提交。
 * 所以这里只负责摆出确认，不再有一条"直接删掉"的路径。
 */
async function askDelete(item: Ref) {
  const probe = await writes.probeDeletable(item.name);
  if (probe === null) return;
  confirmDelete.value = { name: item.name, typed: "", merged: probe.merged };
}

const unmergedOf = (name: string) => writes.deletable[name]?.unmerged ?? null;

async function runDelete() {
  const target = confirmDelete.value;
  if (!target) return;
  if (!target.merged && target.typed !== target.name) return;
  confirmDelete.value = null;
  await writes.deleteBranch(target.name, !target.merged);
}

function openUpstream(item: Ref) {
  upstreamFor.value = upstreamFor.value === item.name ? null : item.name;
  upstreamChoice.value = item.upstream;
}

async function saveUpstream(name: string) {
  const upstream = upstreamChoice.value;
  upstreamFor.value = null;
  await writes.setUpstream(name, upstream);
}

async function createTag() {
  const name = newTag.value.trim();
  if (!name) return;
  const message = newTagMessage.value.trim();
  const done = await writes.createTag(name, message === "" ? null : message);
  if (done !== null) {
    newTag.value = "";
    newTagMessage.value = "";
  }
}

/** 删远程分支属远程写：要求手输分支名（§7.12 的确认强度） */
const remoteDeleteTyped = ref("");
const remoteDeleting = ref<string | null>(null);

async function deleteRemote(name: string) {
  if (remoteDeleteTyped.value !== name) return;
  const target = remoteDeleting.value;
  remoteDeleting.value = null;
  remoteDeleteTyped.value = "";
  if (target === null) return;
  await writes.deleteRemoteBranch(remoteName.value, target);
}

function shortSha(sha: string) {
  return sha.slice(0, 8);
}
</script>

<template>
  <div class="block">
    <div class="block-title with-action">
      <span>分支 {{ branches.length }}</span>
      <!--
        建分支的入口：只开弹窗，不在这里铺一排控件——它偶尔做一次，
        而面板里每一行高度都该留给分支列表。
      -->
      <n-button
        size="small"
        quaternary
        type="primary"
        :disabled="!canWrite()"
        @click="emit('new-branch')"
      >
        新建分支
      </n-button>
    </div>

    <n-alert v-if="writes.error" type="error" :title="writes.error.message" class="inline-alert">
      <div>错误码：{{ writes.error.code }}</div>
    </n-alert>

    <div class="ref-list">
      <div
        v-for="item in branches"
        :key="item.fullName"
        class="ref-row"
        :class="{ current: item.name === currentBranch, pinned: popRow === item.name }"
      >
        <span class="ref-dot" :class="{ on: item.name === currentBranch }"></span>
        <span class="ref-name" :title="item.fullName">
          <template v-if="renaming && renaming.from === item.name">
            <n-input
              v-model:value="renaming.to"
              size="small"
              @keyup.enter="saveRename"
              @click.stop
            />
            <n-button size="small" type="primary" @click.stop="saveRename">存</n-button>
          </template>
          <template v-else>{{ item.name }}</template>
        </span>
        <span class="ref-track" :class="{ warn: item.upstreamGone }">{{ trackOf(item) }}</span>

        <span class="ref-actions" @click.stop>
          <!--
            切换是高频动作，但切错了会带着工作区一起换上下文（脏工作区还会被拒），
            所以照样先问一句——确认浮层里的文案直接复用 dirtyHint，脏不脏点开就知道。
          -->
          <n-popconfirm
            v-if="item.name !== currentBranch"
            positive-text="切换"
            negative-text="算了"
            @update:show="(value: boolean) => setPop(item.name, value)"
            @positive-click="switchTo(item.name)"
          >
            <template #trigger>
              <n-button size="small" quaternary :disabled="!canWrite()" :title="dirtyHint">
                切换
              </n-button>
            </template>
            <div class="pop-text">
              切到 <b>{{ item.name }}</b>？
              <div class="pop-hint">{{ dirtyHint || "当前工作区没有未提交的改动。" }}</div>
            </div>
          </n-popconfirm>
          <n-button size="small" quaternary :disabled="!canWrite()" @click="openUpstream(item)">
            上游
          </n-button>
          <!--
            强推只给「配了上游」的分支，而且只用 lease：对方抢先推过时它会失败，
            而裸 `--force` 会把别人的提交抹掉（§7.12）
          -->
          <n-popconfirm
            v-if="item.upstream && !item.upstreamGone"
            positive-text="强推"
            negative-text="算了"
            @update:show="(value: boolean) => setPop(item.name, value)"
            @positive-click="writes.push(remoteName, item.name, false, true)"
          >
            <template #trigger>
              <n-button size="small" quaternary :disabled="!canWrite()">强推</n-button>
            </template>
            <div class="pop-text">
              用 lease 强推 <b>{{ item.name }}</b> → <b>{{ remoteName }}</b>？
              <div class="pop-hint">远程被别人抢先推过时这次会失败，不会抹掉别人的提交。</div>
            </div>
          </n-popconfirm>
          <n-button size="small" quaternary :disabled="!canWrite()" @click="startRename(item)">
            改名
          </n-button>
          <n-button
            size="small"
            quaternary
            type="error"
            :disabled="!canWrite() || item.name === currentBranch"
            @click="askDelete(item)"
          >
            删除
          </n-button>
        </span>

        <span v-if="unmergedOf(item.name) !== null" class="unmerged">
          未合并 {{ unmergedOf(item.name) }} 个提交
        </span>

        <!--
          删除的二次确认就地展开在行里（原来是挂在列表下方的一个浮层，分支一多就得
          滚到底才看得见）。未合并的还要手输分支名：那是真会丢提交的那一类。
        -->
        <div
          v-if="confirmDelete && confirmDelete.name === item.name"
          class="confirm"
          @click.stop
        >
          <span v-if="confirmDelete.merged">
            <b>{{ confirmDelete.name }}</b> 已合并进别的分支，删除不会丢提交（reflog 里还找得回来）。
          </span>
          <span v-else>
            删除 <b>{{ confirmDelete.name }}</b> 会丢掉
            <b>{{ unmergedOf(confirmDelete.name) ?? "?" }}</b> 个未合并的提交。输入分支名确认：
          </span>
          <n-input
            v-if="!confirmDelete.merged"
            v-model:value="confirmDelete.typed"
            size="small"
            placeholder="分支名"
            @keyup.enter="runDelete"
          />
          <n-space size="small" justify="end">
            <n-button size="small" @click="confirmDelete = null">算了</n-button>
            <n-button
              size="small"
              type="error"
              :disabled="!confirmDelete.merged && confirmDelete.typed !== confirmDelete.name"
              @click="runDelete"
            >
              {{ confirmDelete.merged ? "删除" : "强制删除" }}
            </n-button>
          </n-space>
        </div>

        <div v-if="upstreamFor === item.name" class="inline-form" @click.stop>
          <n-select
            v-model:value="upstreamChoice"
            :options="upstreamOptions"
            size="small"
            clearable
            filterable
            tag
            placeholder="选一个远程跟踪分支"
          />
          <n-button size="small" type="primary" @click="saveUpstream(item.name)">设定</n-button>
        </div>
      </div>
      <n-empty v-if="!branches.length" size="small" description="还没有本地分支" />
    </div>

    <!-- 工作区脏时切换会被拒：这条提示挂在分支列表下面，说明「切换」为什么点不动 -->
    <div v-if="dirtyHint" class="hint">{{ dirtyHint }}</div>


    <template v-if="remoteBranches.length">
      <div class="block-title">远程 {{ remoteName }}</div>
      <!--
        这里只放「开始干活前」的两个动作。推送搬去了提交页的同步区（SyncPanel）：
        它和提交是连着的两步，留在这里等于提交完还要换一栏去按。
        改写过历史要强推，用分支行悬停里的「强推（lease）」——那在分支列表，不在这一块。
      -->
      <n-space size="small" align="center">
        <n-button size="small" :loading="writes.syncAction === 'fetch'" @click="writes.fetch(remoteName)">
          抓取
        </n-button>
        <n-button
          size="small"
          :disabled="!currentBranch"
          @click="writes.pull(remoteName, 'ff_only')"
        >
          拉取（快进）
        </n-button>
        <n-button
          size="small"
          :disabled="!currentBranch"
          @click="writes.pull(remoteName, 'rebase')"
        >
          拉取（变基）
        </n-button>
      </n-space>
      <div class="hint">
        拉取默认只快进；偏好在设置页可改，这里两个按钮是本次临时换策略。
        推送在「提交」页的同步区。
      </div>

      <div class="remote-list">
        <div v-for="item in remoteBranches" :key="item.fullName" class="ref-row">
          <span class="ref-name" :title="item.fullName">{{ item.name }}</span>
          <span class="ref-actions" @click.stop>
            <n-button
              v-if="remoteDeleting === item.name"
              size="small"
              type="error"
              :disabled="remoteDeleteTyped !== item.name"
              @click="deleteRemote(item.name)"
            >
              确认删除
            </n-button>
            <n-button
              v-else
              size="small"
              quaternary
              type="error"
              @click="remoteDeleting = item.name; remoteDeleteTyped = ''"
            >
              删远程
            </n-button>
          </span>
          <div v-if="remoteDeleting === item.name" class="inline-form" @click.stop>
            <n-input v-model:value="remoteDeleteTyped" size="small" placeholder="输入分支名确认" />
          </div>
        </div>
      </div>
    </template>

    <div class="block-title">标签 {{ tags.length }}</div>
    <div class="ref-list">
      <div v-for="item in tags" :key="item.fullName" class="ref-row">
        <span class="ref-name" :title="item.fullName">{{ item.name }}</span>
        <code v-if="item.target" class="muted">{{ shortSha(item.target) }}</code>
        <!-- 标签删掉不影响提交本身（它只是一个指向提交的名字），但找回来要靠 reflog，
             所以照样先问一句 -->
        <n-popconfirm
          positive-text="删除标签"
          negative-text="算了"
          @positive-click="writes.deleteTag(item.name)"
        >
          <template #trigger>
            <n-button size="small" quaternary type="error" :disabled="!canWrite()">
              删除
            </n-button>
          </template>
          <div class="pop-text">
            删除标签 <b>{{ item.name }}</b>？
            <div class="pop-hint">提交记录不受影响，但这个名字就没了。</div>
          </div>
        </n-popconfirm>
      </div>
      <n-empty v-if="!tags.length" size="small" description="还没有标签" />
    </div>
    <n-space size="small" align="center">
      <n-input v-model:value="newTag" size="small" placeholder="标签名" />
      <n-input v-model:value="newTagMessage" size="small" placeholder="说明（留空＝轻量标签）" />
      <n-button size="small" :disabled="!canWrite() || !newTag.trim()" @click="createTag">
        打标签
      </n-button>
    </n-space>

    <div v-if="writes.syncLines.length" class="sync">
      <span class="muted">{{ writes.syncAction }}：</span>
      <code>{{ writes.syncLines[writes.syncLines.length - 1] }}</code>
    </div>

    <div v-if="writes.lastBackupRef" class="muted">
      上一步的还原点：<code>{{ writes.lastBackupRef }}</code>
    </div>
  </div>
</template>

<style scoped>
.block {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.block-title {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-3);
  margin-top: 4px;
}

/* 标题右侧挂一个动作（现在是「新建分支」）：计数与入口同一行，不用另起一排 */
.block-title.with-action {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.ref-list,
.remote-list {
  display: flex;
  flex-direction: column;
}

.ref-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  padding: 2px 0;
}

.ref-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--border);
  flex: none;
}

.ref-dot.on {
  background: var(--accent);
}

.ref-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ref-track,
.unmerged {
  flex: none;
  font-size: 11px;
  opacity: 0.7;
}

.ref-track.warn,
.unmerged {
  color: var(--warn-text);
  opacity: 1;
}

.ref-actions {
  display: none;
  flex: none;
  gap: 2px;
}

/* 悬停才露出写入口：这一列一屏要放几十个分支，常驻按钮会把它挤成按钮墙 */
.ref-row:hover .ref-actions,
/* 确认浮层开着时把它钉住：鼠标要移到浮层面板上，那时已经不在这一行里了 */
.ref-row.pinned .ref-actions {
  display: flex;
}

/* 确认浮层里的文案。内层元素在 body 上（popover 是 teleport 出去的），
   但编译期带着本组件的 scope 属性，所以 scoped 样式仍然命中 */
.pop-text {
  max-width: 260px;
  font-size: 12px;
  line-height: 1.5;
}

.pop-hint {
  margin-top: 2px;
  font-size: 11px;
  color: var(--text-3);
}

.inline-form {
  flex-basis: 100%;
  display: flex;
  gap: 4px;
  padding: 2px 0 4px 12px;
}

.confirm {
  flex-basis: 100%;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
  background: var(--error-bg);
  border: 1px solid var(--error-border);
  border-radius: 6px;
  font-size: 11px;
}

.hint {
  font-size: 11px;
  opacity: 0.7;
  line-height: 1.5;
}

.sync {
  font-size: 11px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.inline-alert {
  margin: 0;
}

.muted {
  font-size: 11px;
  opacity: 0.7;
}
</style>