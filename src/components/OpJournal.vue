<script setup lang="ts">
import dayjs from "dayjs";
import { NButton, NEmpty, NPopconfirm, NTag } from "naive-ui";
import { useWriteStore } from "@/stores/write";

/**
 * 写操作日志与「撤销上一步」（§7.17）。
 *
 * 日志每条都带着还原 ref：这是"怎么回到那一次操作之前"的答案，比任何一句"已撤销"都硬。
 * 撤销的判据在 Rust 侧——当前 HEAD 不等于日志里的 `head_after` 就拒绝自动撤销，
 * 因为那说明中间有人动过（终端、IDE、另一个实例），自动撤销会把别人的操作一起抹掉。
 */
const writes = useWriteStore();

const TYPE = "success" as const;
const TYPE_WARN = "warning" as const;
const TYPE_ERROR = "error" as const;
const TYPE_INFO = "info" as const;

function typeOf(status: string) {
  switch (status) {
    case "ok":
      return TYPE;
    case "rolled_back":
      return TYPE_WARN;
    case "interrupted":
      return TYPE_ERROR;
    default:
      return TYPE_INFO;
  }
}

/** 动作名的中文说法。日志里的 action 是命令名，界面上不必让用户去猜 */
const ACTION_LABELS: Record<string, string> = {
  branch_create: "建分支",
  branch_create_switch: "建分支并切换",
  branch_delete: "删分支",
  branch_delete_force: "强制删分支",
  branch_rename: "分支改名",
  branch_switch: "切换分支",
  upstream_set: "设置上游",
  tag_create: "打标签",
  tag_delete: "删标签",
  stash_push: "存 stash",
  stash_apply: "取 stash",
  stash_pop: "取并删 stash",
  stash_drop: "删 stash",
  stash_branch: "从 stash 建分支",
  cherry_pick: "摘取提交",
  revert: "回滚提交",
  reset_soft: "reset --soft",
  reset_mixed: "reset --mixed",
  reset_hard: "reset --hard",
  pull: "拉取",
  operation_abort: "退回中断操作",
  files_stage_hunks: "分段暂存",
};

function label(action: string) {
  return ACTION_LABELS[action] ?? action;
}

function short(sha: string | null) {
  return sha === null ? "" : sha.slice(0, 8);
}
</script>

<template>
  <div class="journal">
    <div class="head">
      <span>写操作 {{ writes.journal.length }}</span>
      <n-popconfirm
        :disabled="!writes.canUndo || writes.busy"
        positive-text="撤销"
        negative-text="算了"
        @positive-click="writes.undo()"
      >
        <template #trigger>
          <n-button size="tiny" :disabled="!writes.canUndo || writes.busy" :loading="writes.busy">
            撤销上一步
          </n-button>
        </template>
        撤销会把 HEAD 移回上一次操作之前。如果这中间你在终端或 IDE 里提交过，
        这一步会被拒绝——那时请用下面的还原 ref 手工恢复。
      </n-popconfirm>
    </div>

    <n-empty v-if="!writes.journal.length" size="small" description="还没有写操作" />

    <div v-else class="list">
      <div v-for="entry in writes.journal" :key="entry.id" class="row">
        <n-tag :type="typeOf(entry.status)" size="tiny" :bordered="false">
          {{ entry.statusLabel }}
        </n-tag>
        <span class="what">{{ label(entry.action) }}</span>
        <span v-if="entry.affectedFrom || entry.affectedTo" class="range muted">
          {{ entry.affectedFrom ?? "" }}<template v-if="entry.affectedTo"> → {{ entry.affectedTo }}</template>
        </span>
        <span class="when muted">{{ dayjs(entry.ts * 1000).format("MM-DD HH:mm:ss") }}</span>
        <div class="detail muted">
          <span v-if="entry.headBefore">
            {{ short(entry.headBefore) }} → {{ short(entry.headAfter) }}
          </span>
          <code class="ref" :title="'用 git update-ref refs/heads/<分支> ' + entry.backupRef + ' 可以回到这一步之前'">
            {{ entry.backupRef }}
          </code>
          <pre v-if="entry.detail" class="raw">{{ entry.detail }}</pre>
        </div>
      </div>
    </div>

    <div v-if="writes.backups.length" class="backups">
      <div class="muted">还原点（可直接 update-ref 回去）：</div>
      <code v-for="item in writes.backups.slice(0, 5)" :key="item.reference" class="ref">
        {{ item.sha.slice(0, 8) }} {{ item.reference }}
      </code>
    </div>
  </div>
</template>

<style scoped>
.journal {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: #8a94a6;
}

.list {
  display: flex;
  flex-direction: column;
  max-height: 220px;
  overflow: auto;
}

.row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 6px;
  padding: 3px 0;
  border-bottom: 1px solid #f4f6f8;
}

.what {
  font-size: 12px;
}

.range,
.when {
  font-size: 11px;
}

.detail {
  flex-basis: 100%;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding-left: 2px;
}

.ref {
  font-size: 10px;
  opacity: 0.75;
  word-break: break-all;
}

.raw {
  margin: 0;
  font-size: 10px;
  white-space: pre-wrap;
  word-break: break-all;
}

.backups {
  display: flex;
  flex-direction: column;
}

.muted {
  font-size: 11px;
  opacity: 0.7;
}
</style>