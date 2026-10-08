<script setup lang="ts">
import { computed } from "vue";
import dayjs from "dayjs";
import { NTag } from "naive-ui";
import type { Commit, GraphRow, GraphSegment } from "@/api/commit";
import { COMMIT_ROW_HEIGHT } from "@/styles/tokens";

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
  /** 右边那栏正在显示它 */
  selected?: boolean;
}>();

const emit = defineEmits<{ click: [] }>();

/** 行高从 tokens 取，虚拟列表的 item-size 从同一个值取——见 COMMIT_ROW_HEIGHT 的注释 */
const ROW_HEIGHT = COMMIT_ROW_HEIGHT;
const LANE_WIDTH = 14;

/**
 * 图列里所有纵向偏移都从 ROW_HEIGHT 推，**不写死像素**。
 *
 * 截断边（筛选态，§7.7）原本写的是 `ROW_HEIGHT/2 + 8` / `ROW_HEIGHT - 14` /
 * `ROW_HEIGHT - 8`：行高 44 时刚好是 22→30→30→36，一条顺的竖线；行高改成 30 就变成
 * 15→23→16→22，中间那个控制点跑到了起点上方，画出来是个回环。所以这三个数得跟着行高走。
 */
const STUB_GAP = ROW_HEIGHT * 0.18;

/**
 * 调色板。下标由 Rust 侧**按分支永久发号**（`graph.rs` 的 `take_color`）：0 永远是主线
 * （HEAD 沿第一父那条链），每开一条支线发一个新号，用过的不回收，所以同一个号在这张图上
 * 自始至终是同一条分支。
 *
 * 色相按 16 等分取点（相邻两个色相 22.5°），但**槽位顺序不是色相顺序**，而是色相步长的
 * 位反序（0,8,4,12,2,10,6,14,1,…）：这样前 8 个槽位互相隔开 45°，第 9 支起才插进它们中间
 * 的 22.5° 空位。同时先后两半各用一档明度（前 8 支深、后 8 支浅），挨得最近的那批
 * 靠明度差兜住。历史上这里是 8 色，第 9 条支线就绕回主线那支蓝，看着像同一条分支。
 *
 * 只有 `strokeColor` 一处消费。**取模的边界要说清**：号是按分支发的，一次全历史遍历能发几百号，
 * 所以撞色是常态而不是意外——撞上的两条分支要同时出现在屏幕上才刺眼，而屏幕上同时可见的泳道数
 * 由列宽封顶，远小于 16。真要无限区分只能靠徽标和筛选，不靠颜色。
 */
const PALETTE = [
  "hsl(198, 55%, 58%)", // 主线：原 #6fb3d1 那支蓝，深一档让 2px 线在白底上站得住
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

/**
 * 截断端点（筛选态，§7.7）：父提交被筛掉了，这段边下面没有可见的行可接。
 *
 * 画法是一段短竖线加一个端点帽，**不画到行底**——画到底就等于告诉用户
 * "下一行那条就是它的父"，而那一行根本不在可见集合里。宁可少画一段，
 * 也不能让用户顺着一条不存在的连线去找一条不存在的提交。
 */
function stubEndOf(segment: GraphSegment) {
  const fromX = centerX(segment.from);
  const x = centerX(segment.to);
  const startY = ROW_HEIGHT / 2;
  const endY = ROW_HEIGHT - STUB_GAP;
  return `M ${fromX} ${startY} C ${fromX} ${startY + STUB_GAP}, ${x} ${endY - STUB_GAP * 0.75}, ${x} ${endY}`;
}

/** 端点帽：一个小横杠，让这一段看着是"到头了"而不是"被裁了" */
function stubCapOf(segment: GraphSegment) {
  const x = centerX(segment.to);
  const y = ROW_HEIGHT - STUB_GAP;
  return `M ${x - 3} ${y} L ${x + 3} ${y}`;
}

/** 实线段。截断边另画，两者不能共用一条 path */
function solidOf(row?: GraphRow) {
  return row?.segments.filter((segment) => !segment.dangling) ?? [];
}

/** 截断边：它下面没有落点，所以没有"等在哪一列"这回事 */
function danglingOf(row?: GraphRow) {
  return row?.segments.filter((segment) => segment.dangling) ?? [];
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
  <div
    class="commit-row"
    :class="{ selected }"
    @click="emit('click')"
  >
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
          v-for="(segment, index) in solidOf(row)"
          :key="index"
          :d="strokeOf(segment)"
          :stroke="strokeColor(segment.color)"
          stroke-width="2"
          fill="none"
          stroke-linecap="round"
        />
        <!-- 截断边：短竖线 + 端点帽，不接到下一行 -->
        <g v-for="(segment, index) in danglingOf(row)" :key="`d${index}`">
          <path
            :d="stubEndOf(segment)"
            :stroke="strokeColor(segment.color)"
            stroke-width="2"
            fill="none"
            stroke-linecap="round"
          />
          <path
            :d="stubCapOf(segment)"
            :stroke="strokeColor(segment.color)"
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
  gap: 6px;
  /* 行高不能在这里另写一个数：虚拟列表的 item-size 从 tokens 取，
     两个值不一致时列表会重叠或留缝 */
  height: 30px;
  padding: 0 10px;
  border-bottom: 1px solid var(--border-soft);
  font-size: 12px;
  cursor: pointer;
}

/* 选中态是"右边那一栏正在显示它"，不是焦点态：颜色要够淡，一屏几十行同时亮着不能刺眼 */
.commit-row:hover {
  background: var(--surface-hover);
}

.commit-row.selected {
  background: var(--surface-selected);
  box-shadow: inset 2px 0 0 var(--accent);
}

.graph {
  flex: none;
  overflow: visible;
}

.sha {
  flex: none;
  color: var(--text-3);
  font-size: 11px;
}

.type-tag {
  flex: none;
  min-width: 52px;
  justify-content: center;
}

.refs {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: none;
  /* 分支名可以很长，也能带斜杠；封顶让主体信息始终看得见。
     原来给到 46%——一条提交挂十几个远程分支时 subject 只剩几十像素，
     而扫历史找的正是 subject */
  max-width: 28%;
  overflow: hidden;
}

.badge {
  flex: none;
  max-width: 140px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  padding: 0 6px;
  border-radius: 9px;
  font-size: 11px;
  line-height: 15px;
}

.badge.branch {
  color: var(--badge-branch-text);
  background: var(--badge-branch-bg);
}

.badge.remote {
  color: var(--badge-remote-text);
  background: var(--badge-remote-bg);
}

.badge.tag {
  color: var(--badge-tag-text);
  background: var(--badge-tag-bg);
}

/* 游离 HEAD 用的那一个：它不是任何一种引用，所以不复用上面三色 */
.badge.current {
  color: var(--badge-head-text);
  background: var(--badge-head-bg);
}

/* HEAD 所在的那个引用要一眼看出来，不然一条线上几个徽标得分开数 */
.badge.head {
  font-weight: 600;
  box-shadow: inset 0 0 0 1px currentColor;
}

.badge.more {
  color: var(--badge-more-text);
  background: var(--badge-more-bg);
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
  font-size: 11px;
  opacity: 0.7;
}
</style>
