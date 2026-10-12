<script setup lang="ts">
/**
 * 提交历史分支图面板（连续 SVG 渲染）。
 *
 * 把整个可见区域当作一张连续的 SVG 来画：分支线条跨行平滑过渡，
 * 贝塞尔曲线在分叉/合入处自然弯转，一条分支全程同色。
 */
import { computed } from "vue";
import type { GraphRow } from "@/api/commit";
import { COMMIT_ROW_HEIGHT } from "@/styles/tokens";

const props = defineProps<{
  rows: GraphRow[];
  lanes: number;
  selectedSha?: string | null;
   /** 未推送的提交集合——它们整行改用 --outgoing 色，推送状态比分支身份更该被一眼看见 */
   unpushedShas?: Set<string>;
   /** 合并提交集合——画成空心圆环而非实心点 */
   mergeShas?: Set<string>;
}>();

const ROW_H = COMMIT_ROW_HEIGHT;
const LANE_W = 14;
const DOT_R = 4.5;
const MERGE_R = 3.5;
const STROKE_W = 2;
const PAD_LEFT = 6;

/** 未推送提交的线条色——从 CSS 变量取，与全局主题一致 */
const OUTGOING_COLOR = "var(--outgoing)";

const BRANCH_COLORS = [
  "#3b82f6", // 蓝——主线（HEAD 沿第一父的那条链）
  "#22c55e", // 绿
  "#eab308", // 黄
  "#ef4444", // 红
  "#8b5cf6", // 紫
  "#06b6d4", // 青
  "#f97316", // 橙
  "#ec4899", // 粉
];

function branchColor(c: number) {
  return BRANCH_COLORS[c % BRANCH_COLORS.length];
}

/** 未推送的行整行用 outgoing 色；其余按分支色号取 */
function strokeColor(c: number, sha: string) {
  if (props.unpushedShas?.has(sha)) return OUTGOING_COLOR;
  return branchColor(c);
}

/** 节点颜色同理 */
function dotColor(c: number, sha: string) {
  if (props.unpushedShas?.has(sha)) return OUTGOING_COLOR;
  return branchColor(c);
}

const graphWidth = computed(() =>
  Math.max(props.lanes, 1) * LANE_W + PAD_LEFT * 2,
);

function centerX(lane: number) {
  return lane * LANE_W + LANE_W / 2 + PAD_LEFT;
}

function bezier(fx: number, fy: number, tx: number, ty: number) {
  const dy = ty - fy;
  return `M ${fx} ${fy} C ${fx} ${fy + dy * 0.5}, ${tx} ${ty - dy * 0.5}, ${tx} ${ty}`;
}

interface RenderPath { d: string; color: string; key: string }

const renderPaths = computed<RenderPath[]>(() => {
  const result: RenderPath[] = [];
  props.rows.forEach((row, ri) => {
    const yT = ri * ROW_H;
    const yM = yT + ROW_H / 2;
    const yB = yT + ROW_H;

    if (row.incoming && !row.segments.some((s) => s.to === row.lane)) {
      const x = centerX(row.lane);
      result.push({ d: `M ${x} ${yT} L ${x} ${yM}`, color: strokeColor(row.color, row.sha), key: `${row.sha}-stub` });
    }

    for (const seg of row.segments) {
      const fx = centerX(seg.from);
      const tx = centerX(seg.to);
      const col = strokeColor(seg.color, row.sha);
      if (seg.dangling) {
        const endY = yB - ROW_H * 0.18;
        const gap = ROW_H * 0.18;
        result.push({
          d: `M ${fx} ${yM} C ${fx} ${yM + gap}, ${tx} ${endY - gap * 0.75}, ${tx} ${endY}`,
          color: col, key: `${row.sha}-d-${seg.from}-${seg.to}`,
        });
        // 截断端帽：小横杠表示"到此为止"
        result.push({
          d: `M ${tx - 3} ${endY} L ${tx + 3} ${endY}`,
          color: col, key: `${row.sha}-dc-${seg.from}-${seg.to}`,
        });
        continue;
      }
      if (seg.from === seg.to) {
        result.push({ d: `M ${fx} ${yT} L ${fx} ${yB}`, color: col, key: `${row.sha}-v-${seg.from}` });
      } else if (seg.from === row.lane) {
        result.push({ d: bezier(fx, yM, tx, yB), color: col, key: `${row.sha}-f-${seg.from}-${seg.to}` });
      } else if (seg.to === row.lane) {
        result.push({ d: bezier(fx, yT, tx, yM), color: col, key: `${row.sha}-m-${seg.from}-${seg.to}` });
      } else {
        result.push({ d: bezier(fx, yT, tx, yB), color: col, key: `${row.sha}-p-${seg.from}-${seg.to}` });
      }
    }
  });
  return result;
});

interface RenderDot { cx: number; cy: number; r: number; color: string; key: string; selected: boolean; isMerge: boolean }

const renderDots = computed<RenderDot[]>(() =>
  props.rows.map((row, ri) => ({
    cx: centerX(row.lane),
    cy: ri * ROW_H + ROW_H / 2,
    r: props.mergeShas?.has(row.sha) ? MERGE_R : DOT_R,
    color: dotColor(row.color, row.sha),
    key: `${row.sha}-dot`,
    selected: row.sha === props.selectedSha,
   isMerge: props.mergeShas?.has(row.sha) ?? false,
  })),
);

const svgHeight = computed(() => props.rows.length * ROW_H);
defineExpose({ graphWidth });
</script>

<template>
  <svg
    class="branch-graph"
    :width="graphWidth"
    :height="svgHeight"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <path
      v-for="p in renderPaths"
      :key="p.key"
      :d="p.d"
      :stroke="p.color"
      :stroke-width="STROKE_W"
      fill="none"
      stroke-linecap="round"
    />
    <circle
      v-for="dot in renderDots"
      :key="dot.key"
      :cx="dot.cx"
      :cy="dot.cy"
      :r="dot.selected ? dot.r + 2 : dot.r"
      :fill="dot.isMerge ? 'var(--surface-app)' : dot.color"
      :stroke="dot.isMerge ? dot.color : (dot.selected ? '#fff' : 'none')"
      :stroke-width="dot.isMerge ? 2 : (dot.selected ? 2 : 0)"
    />
  </svg>
</template>

<style scoped>
.branch-graph { display: block; flex: none; }
</style>
