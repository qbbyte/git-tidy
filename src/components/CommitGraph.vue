<script setup lang="ts">
/**
 * 提交图的 SVG 渲染——从 CommitRow 拆出来的纯展示组件。
 *
 * 逻辑与原来 CommitRow 里逐字一致，只是模板只剩 SVG，不带徽标和文字。
 * 供 n-table 的自定义渲染列使用（CommitsView 把它放进表格的第一列）。
 */
import { computed } from "vue";
import type { Commit, GraphRow, GraphSegment } from "@/api/commit";
import { COMMIT_ROW_HEIGHT, tokens } from "@/styles/tokens";

const props = defineProps<{
  commit: Commit;
  row?: GraphRow;
  lanes?: number;
}>();

const ROW_HEIGHT = COMMIT_ROW_HEIGHT;
const LANE_WIDTH = 14;
const STUB_GAP = ROW_HEIGHT * 0.18;

const PALETTE = [
  "hsl(198, 55%, 58%)",
  "hsl(18, 55%, 58%)",
  "hsl(288, 55%, 58%)",
  "hsl(108, 55%, 58%)",
  "hsl(243, 55%, 58%)",
  "hsl(63, 55%, 58%)",
  "hsl(333, 55%, 58%)",
  "hsl(153, 55%, 58%)",
  "hsl(220, 70%, 72%)",
  "hsl(40, 70%, 72%)",
  "hsl(310, 70%, 72%)",
  "hsl(130, 70%, 72%)",
  "hsl(266, 70%, 72%)",
  "hsl(85, 70%, 72%)",
  "hsl(355, 70%, 72%)",
  "hsl(175, 70%, 72%)",
];

const columnCount = () => Math.max(props.lanes ?? 0, 1);
const graphWidth = () => columnCount() * LANE_WIDTH + 4;

function centerX(lane: number) {
  return lane * LANE_WIDTH + LANE_WIDTH / 2 + 2;
}

function bend(fromX: number, fromY: number, toX: number, toY: number) {
  const middle = (toY - fromY) / 2;
  return `M ${fromX} ${fromY} C ${fromX} ${fromY + middle}, ${toX} ${toY - middle}, ${toX} ${toY}`;
}

function strokeOf(segment: GraphSegment) {
  const fromX = centerX(segment.from);
  const toX = centerX(segment.to);
  const middle = ROW_HEIGHT / 2;
  if (segment.from === segment.to) return `M ${fromX} 0 L ${fromX} ${ROW_HEIGHT}`;
  if (segment.from === props.row?.lane) return bend(fromX, middle, toX, ROW_HEIGHT);
  if (segment.to === props.row?.lane) return bend(fromX, 0, toX, middle);
  return bend(fromX, 0, toX, ROW_HEIGHT);
}

function stubEndOf(segment: GraphSegment) {
  const fromX = centerX(segment.from);
  const x = centerX(segment.to);
  const startY = ROW_HEIGHT / 2;
  const endY = ROW_HEIGHT - STUB_GAP;
  return `M ${fromX} ${startY} C ${fromX} ${startY + STUB_GAP}, ${x} ${endY - STUB_GAP * 0.75}, ${x} ${endY}`;
}

function stubCapOf(segment: GraphSegment) {
  const x = centerX(segment.to);
  const y = ROW_HEIGHT - STUB_GAP;
  return `M ${x - 3} ${y} L ${x + 3} ${y}`;
}

function solidOf(row?: GraphRow) {
  return row?.segments.filter((s) => !s.dangling) ?? [];
}
function danglingOf(row?: GraphRow) {
  return row?.segments.filter((s) => s.dangling) ?? [];
}

const graphColor = (color: number) =>
  props.commit.unpushed ? tokens.outgoing : PALETTE[color % PALETTE.length];
const dotColor = () => (props.row ? graphColor(props.row.color) : "transparent");

const needsStub = computed(() => {
  const row = props.row;
  if (!row || !row.incoming) return false;
  return !row.segments.some((s) => s.to === row.lane);
});

function stubOf() {
  const x = centerX(props.row?.lane ?? 0);
  return `M ${x} 0 L ${x} ${ROW_HEIGHT / 2}`;
}
</script>

<template>
  <svg class="commit-graph" :width="graphWidth()" :height="ROW_HEIGHT" aria-hidden="true">
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
        v-for="(segment, index) in solidOf(row)"
        :key="index"
        :d="strokeOf(segment)"
        :stroke="graphColor(segment.color)"
        stroke-width="2"
        fill="none"
        stroke-linecap="round"
      />
      <g v-for="(segment, index) in danglingOf(row)" :key="`d${index}`">
        <path
          :d="stubEndOf(segment)"
          :stroke="graphColor(segment.color)"
          stroke-width="2"
          fill="none"
          stroke-linecap="round"
        />
        <path
          :d="stubCapOf(segment)"
          :stroke="graphColor(segment.color)"
          stroke-width="2"
          fill="none"
          stroke-linecap="round"
        />
      </g>
      <circle
        :cx="centerX(row.lane)"
        :cy="ROW_HEIGHT / 2"
        :r="commit.merge ? 4.5 : 3.5"
        :fill="dotColor()"
      />
    </template>
  </svg>
</template>

<style scoped>
.commit-graph {
  display: block;
  overflow: visible;
}
</style>
