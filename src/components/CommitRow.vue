<script setup lang="ts">
import { computed } from "vue";
import dayjs from "dayjs";
import { NTag } from "naive-ui";
import type { Commit, GraphRow, GraphSegment } from "@/api/commit";

/**
 * 只给常见规范 type 上色。type 是否在白名单里由仓库配置判定（需求 6.7），
 * 列表层不当裁判：解析不出 type 的显示"非规范"灰色，解析出但不认识的也用灰色。
 */
const TYPE_COLORS: Record<string, "success" | "error" | "warning" | "info"> = {
  feat: "success",
  fix: "error",
  perf: "warning",
  refactor: "warning",
  docs: "info",
  test: "info",
  build: "info",
  ci: "info",
};

const props = defineProps<{
  commit: Commit;
  /** 这一行的图元。undefined = 图还没落地（或这一行本来就没有历史关系），列留空 */
  row?: GraphRow;
  /** 整页统一列数：按行自定宽度会让不同行的同一条泳道落在不同像素上 */
  lanes?: number;
}>();

const ROW_HEIGHT = 44;
const LANE_WIDTH = 14;

/**
 * 调色板。下标由 Rust 侧按分支分配：0 永远是主线（HEAD 沿第一父那条链），每条支线另开一支、
 * 避开同时在用的那几支，所以这里第一支要挑"看着就是主干"的颜色，后面的依次区分度高的排。
 * 长度只要不小于同时存在的分支数就够用（超了会取模复用）。
 */
const PALETTE = [
  "#6fb3d1",
  "#d699b6",
  "#a5c965",
  "#dfaf7a",
  "#bd96d3",
  "#7fbbb3",
  "#e0797f",
  "#9aa5b8",
];

function colorOf() {
  const type = props.commit.commitType;
  return type ? (TYPE_COLORS[type] ?? "default") : "default";
}

function labelOf() {
  return props.commit.commitType ?? "非规范";
}

/** 页面先出列表、后出图：图那一层没有落地之前先按单列占位，不能让整行横跳 */
const columnCount = () => Math.max(props.lanes ?? 0, 1);
const graphWidth = () => columnCount() * LANE_WIDTH + 4;

function centerX(lane: number) {
  return lane * LANE_WIDTH + LANE_WIDTH / 2 + 2;
}

/** 竖直方向的 S 形连线：两个端点的横坐标不同时，就按 git 自己的画法弯过去 */
function bend(fromX: number, fromY: number, toX: number, toY: number) {
  const middle = (toY - fromY) / 2;
  return `M ${fromX} ${fromY} C ${fromX} ${fromY + middle}, ${toX} ${toY - middle}, ${toX} ${toY}`;
}

/**
 * 一段线在本行里怎么画，只看它两端和本行圆点的关系：
 * 从圆点出发的向下弯（分叉去某个父提交），汇进圆点的向上弯（支线在这里合并），
 * 其余一律整高竖线（别人的泳道从这一行穿过去）。
 */
function strokeOf(segment: GraphSegment) {
  const fromX = centerX(segment.from);
  const toX = centerX(segment.to);
  const middle = ROW_HEIGHT / 2;

  if (segment.from === segment.to) {
    return `M ${fromX} 0 L ${fromX} ${ROW_HEIGHT}`;
  }
  if (segment.from === props.row?.lane) {
    return bend(fromX, middle, toX, ROW_HEIGHT);
  }
  if (segment.to === props.row?.lane) {
    return bend(fromX, 0, toX, middle);
  }
  return bend(fromX, 0, toX, ROW_HEIGHT);
}

const strokeColor = (color: number) => PALETTE[color % PALETTE.length];
const dotColor = () => (props.row ? strokeColor(props.row.color) : "transparent");

/** 一行最多画几个徽标。一个合并点能挂十几条远程分支，全画出来这一行就溢出了 */
const MAX_BADGES = 4;

const badges = computed(() => props.commit.refs.slice(0, MAX_BADGES));
const hiddenCount = computed(() => props.commit.refs.length - badges.value.length);

/**
 * 游离 HEAD：这条提交是 HEAD，但 refs 里没有一个带"当前"标记
 * （游离时 git 的装饰就只有光秃秃一个 HEAD，实测）。没有它这行就看不出 HEAD 在哪。
 */
const detachedHead = computed(
  () => props.commit.head && !props.commit.refs.some((badge) => badge.head),
);

/**
 * 圆点上面那一小截。上一行的连线只画到它自己的底边，而圆点在本行行高的正中，
 * 中间这半截没人画就是一个断口——根提交和"第一父已经排在别的列"的那种行最容易看见。
 * 本行已经有线落到圆点这一列时不补：竖线穿过去（from 和 to 都是本列）或别人的线
 * 汇进圆点（to 是本列），那两种本来就接着。
 */
const needsStub = computed(() => {
  const row = props.row;
  if (!row || !row.incoming) return false;
  return !row.segments.some((segment) => segment.to === row.lane);
});

function stubOf() {
  const x = centerX(props.row?.lane ?? 0);
  return `M ${x} 0 L ${x} ${ROW_HEIGHT / 2}`;
}
</script>

<template>
  <div class="commit-row">
    <svg class="graph" :width="graphWidth()" :height="ROW_HEIGHT" aria-hidden="true">
      <template v-if="row">
        <path
          v-if="needsStub"
          :d="stubOf()"
          :stroke="dotColor()"
          stroke-width="2"
          fill="none"
          stroke-linecap="round"
        />
        <path
          v-for="(segment, index) in row.segments"
          :key="index"
          :d="strokeOf(segment)"
          :stroke="strokeColor(segment.color)"
          stroke-width="2"
          fill="none"
          stroke-linecap="round"
        />
        <circle
          :cx="centerX(row.lane)"
          :cy="ROW_HEIGHT / 2"
          :r="commit.merge ? 4.5 : 3.5"
          :fill="dotColor()"
        />
      </template>
    </svg>
    <code class="sha">{{ commit.id.slice(0, 8) }}</code>
    <n-tag :type="colorOf()" size="small" :bordered="false" class="type-tag">
      {{ labelOf() }}
    </n-tag>
    <n-tag v-if="commit.scope" size="small" :bordered="false">{{ commit.scope }}</n-tag>
    <n-tag v-if="commit.breaking" type="error" size="small" :bordered="false">BREAKING</n-tag>
    <n-tag v-if="commit.merge" type="warning" size="small" :bordered="false">merge</n-tag>
    <n-tag v-if="commit.revert" size="small" :bordered="false">revert</n-tag>
    <div v-if="badges.length || detachedHead" class="refs">
      <span v-if="detachedHead" class="badge current" title="游离 HEAD：当前提交不落在任何分支上">
        HEAD
      </span>
      <span
        v-for="badge in badges"
        :key="`${badge.kind}:${badge.name}`"
        class="badge"
        :class="[badge.kind, { head: badge.head }]"
        :title="badge.head ? `当前 ${badge.name}` : badge.name"
        >{{ badge.name }}</span
      >
      <span v-if="hiddenCount > 0" class="badge more" :title="`还有 ${hiddenCount} 个引用`">
        +{{ hiddenCount }}
      </span>
    </div>
    <span class="subject" :title="commit.subject">{{ commit.subject }}</span>
    <span class="author">{{ commit.authorName }}</span>
    <span class="time">{{ dayjs(commit.time * 1000).format("YYYY-MM-DD HH:mm") }}</span>
  </div>
</template>

<style scoped>
.commit-row {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 44px;
  padding: 0 12px;
  border-bottom: 1px solid #f0f2f5;
  font-size: 13px;
}

.graph {
  flex: none;
  overflow: visible;
}

.sha {
  flex: none;
  color: #8a94a6;
  font-size: 12px;
}

.type-tag {
  flex: none;
  min-width: 62px;
  justify-content: center;
}

.refs {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: none;
  /* 分支名可以很长，也能带斜杠；封顶让主体信息始终看得见 */
  max-width: 46%;
  overflow: hidden;
}

.badge {
  flex: none;
  max-width: 160px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  padding: 1px 7px;
  border-radius: 9px;
  font-size: 12px;
  line-height: 16px;
}

.badge.branch {
  color: #1b6ba8;
  background: #e2f0fa;
}

.badge.remote {
  color: #5a6472;
  background: #edf0f4;
}

.badge.tag {
  color: #8a5a00;
  background: #fdf1d8;
}

/* 游离 HEAD 用的那一个：它不是任何一种引用，所以不复用上面三色 */
.badge.current {
  color: #6d3bb5;
  background: #f1e9fb;
}

/* HEAD 所在的那个引用要一眼看出来，不然一条线上几个徽标得分开数 */
.badge.head {
  font-weight: 600;
  box-shadow: inset 0 0 0 1px currentColor;
}

.badge.more {
  color: #6b7484;
  background: #f0f2f5;
}

.subject {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.author,
.time {
  flex: none;
  font-size: 12px;
  opacity: 0.7;
}
</style>
