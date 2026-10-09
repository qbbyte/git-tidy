<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import dayjs from "dayjs";
import {
  NAlert,
  NButton,
  NCard,
  NEmpty,
  NProgress,
  NSelect,
  NSpace,
  NTabPane,
  NTabs,
  NTag,
} from "naive-ui";
import { GitTidyError } from "@/api/client";
import {
  complianceReport,
  complianceRevisions,
  type ComplianceReport,
} from "@/api/report";
import {
  commitActivity,
  SINCE_PRESET,
  type Activity,
  type DayStat,
  type SincePreset,
} from "@/api/activity";
import { SOURCE_LABEL } from "@/api/spec";
import { useReposStore } from "@/stores/repos";
import { useDetailStore } from "@/stores/detail";

/**
 * 报告页（需求 7.21）。
 *
 * 页面只做两件事：把 Rust 侧算好的数字摆出来，以及把不合规的提交送到能处理它的地方。
 * **所有判定都在 Rust 侧**——报告页不重写规则，所以表单、hook、报告永远是同一把尺子。
 *
 * 两个视角（`n-tabs`）共用同一份区间选择器：
 * - **符合率**：提交信息守没守规范；
 * - **活跃度**：这段时间里谁提交了多少、动了多少行。
 *
 * 为什么不另开一个页签：它们是同一件事的两个问法——"对这个仓库的提交历史怎么看"，
 * 而 `TAB_SHORTCUTS` 的 1–5 是写死的，另开一页要连带改导航、快捷键与设置页的说明表。
 */
const repos = useReposStore();
const detail = useDetailStore();
const router = useRouter();

const repoId = computed(() => repos.currentId);

const report = ref<ComplianceReport | null>(null);
const revisions = ref<string[]>(["HEAD"]);
const rev = ref<string>("HEAD");
const loading = ref(false);
const error = ref<GitTidyError | null>(null);
/** 明细按原因筛：点某个原因桶就只看它那几条 */
const reasonFilter = ref<string | null>(null);

/** 当前视角。两个视角共用 `rev`，只是换统计口径 */
type View = "compliance" | "activity";
const view = ref<View>("compliance");

/** 时间预设的显示名。值是 git 的口语（见 `SINCE_PRESET`），界面上说人话 */
const PRESET_LABEL: Record<SincePreset, string> = {
  today: "今日",
  week: "本周",
  month: "本月",
  all: "全部",
};
const presetOptions = (Object.keys(PRESET_LABEL) as SincePreset[]).map((value) => ({
  label: PRESET_LABEL[value],
  value,
}));

const activity = ref<Activity | null>(null);
const activityPreset = ref<SincePreset>("week");
const activityLoading = ref(false);
const activityError = ref<GitTidyError | null>(null);

// ---- 提交热力图（近一年，GitHub 贡献图那种） ----
//
// 与左侧时间范围**解耦**：热力图永远看最近一年。理由很直接——热力图的价值就在长区间的
// 节奏（连击、空档），跟着"今日/本周"走只会剩几个格子，反而更难读。所以它单独拉一次
// 数据，左侧预设仍只管上面的总数与按作者表。

/** 列数 = 周数。53 周恰好覆盖一年（含首尾边角），与 GitHub 一致 */
const HEAT_WEEKS = 53;
/** 星期标签：只标 Mon/Wed/Fri（行是周日→周六），其余留空，避免七行挤满字 */
const HEAT_WEEKDAYS = ["", "Mon", "", "Wed", "", "Fri", ""];
/** 月标签。11px 下中文短写正好放得下 */
const HEAT_MONTHS = [
  "1月", "2月", "3月", "4月", "5月", "6月",
  "7月", "8月", "9月", "10月", "11月", "12月",
];

interface HeatCell {
  key: string;
  /** 0 = 无提交（灰格），1–4 由浅到深；档位基准是区间内单日最高提交数 */
  level: number;
  title: string;
  /** 未来日期：占位但不显示 */
  future: boolean;
}

interface HeatCol {
  key: string;
  /** 这一列要显示的月份标签；不换月就是 null */
  month: string | null;
  cells: HeatCell[];
}

const heat = ref<Activity | null>(null);
const heatLoading = ref(false);
const heatError = ref<GitTidyError | null>(null);

/** 日期 → 当日统计。后端只给"有提交的日子"，空格子由 `heatCols` 按日历补出来 */
const heatByDay = computed(() => {
  const map = new Map<string, DayStat>();
  for (const day of heat.value?.byDay ?? []) map.set(day.day, day);
  return map;
});

/**
 * 53 列 × 7 行（列优先）的日历格，最右一列是本周。
 *
 * 颜色分四档，基准取区间内单日最高提交数——这样每一档都对应"当天算不算多"，
 * 而不是随最大值漂移的绝对数。空白的日子是实实在在的灰格子，不再被柱状图撑成大色块。
 */
const heatCols = computed<HeatCol[]>(() => {
  const map = heatByDay.value;
  const peak = Math.max(1, ...[...map.values()].map((day) => day.commits));
  const today = dayjs().startOf("day");
  // 从本周的周日往回退 52 周，得到第一列的起点
  const gridStart = today
    .subtract(today.day(), "day")
    .subtract((HEAT_WEEKS - 1) * 7, "day");

  const cols: HeatCol[] = [];
  let prevMonth = -1;
  for (let c = 0; c < HEAT_WEEKS; c += 1) {
    const colStart = gridStart.add(c * 7, "day");
    const cells: HeatCell[] = [];
    for (let row = 0; row < 7; row += 1) {
      const date = colStart.add(row, "day");
      const key = date.format("YYYY-MM-DD");
      const future = date.isAfter(today);
      const stat = map.get(key);
      const count = stat?.commits ?? 0;
      const level = count === 0 ? 0 : Math.min(4, Math.max(1, Math.ceil((count / peak) * 4)));
      cells.push({
        key,
        level,
        future,
        title: future ? "" : `${key}　${count} 次提交 · ${stat?.authors ?? 0} 人`,
      });
    }
    const month = colStart.month();
    const label = month === prevMonth ? null : HEAT_MONTHS[month];
    prevMonth = month;
    cols.push({ key: colStart.format("YYYY-MM-DD"), month: label, cells });
  }
  return cols;
});

/** 热力图这一年自己的口径（与按作者表不同：这里固定近一年，不受左侧范围影响） */
const heatNote = computed(() => {
  const current = heat.value;
  if (!current) return "";
  const parts = [`近一年 ${current.commits} 次提交`, `${current.authorCount} 位作者`];
  if (current.truncated) parts.push("仓库过大，只统计了最近一批，靠左的格子可能不全");
  return parts.join(" · ");
});

/** 区间里到底排除了什么、有没有被截断——这两个不说清楚，表格就没法当账看 */
const activityNote = computed(() => {
  const current = activity.value;
  if (!current) return "";
  const parts = [`区间 ${current.rev}`];
  if (current.since) parts.push(`自「${PRESET_LABEL[activityPreset.value]}」起`);
  parts.push(`${current.commits} 次提交`);
  if (current.excluded > 0) {
    parts.push(`已排除 ${current.excluded} 条合并 / revert（信息由 git 生成，不算谁的产出）`);
  }
  if (current.binaryFiles > 0) {
    parts.push(`其中 ${current.binaryFiles} 个二进制文件只计文件数，不计行数`);
  }
  if (current.truncated) {
    parts.push(`区间共 ${current.totalInRange} 条，只统计了最近一批`);
  }
  return parts.join(" · ");
});

const revOptions = computed(() =>
  revisions.value.map((value) => ({ label: value, value })),
);

const percent = computed(() => Math.round((report.value?.rate ?? 0) * 100));

/** 分母里排除了什么必须能说出来，否则这个百分比没法当账看 */
const denominatorNote = computed(() => {
  const current = report.value;
  if (!current) return "";
  const parts = [`统计 ${current.scanned} 条`];
  if (current.excluded > 0) {
    parts.push(`已排除 ${current.excluded} 条合并 / revert（信息由 git 生成，不计入分母）`);
  }
  if (current.truncated) {
    parts.push(`区间共 ${current.totalInRange} 条，只统计了最近 ${current.scanned} 条`);
  }
  return parts.join(" · ");
});

const reasons = computed(() => report.value?.byReason ?? []);
const offenders = computed(() => {
  const all = report.value?.offenders ?? [];
  return reasonFilter.value === null
    ? all
    : all.filter((item) => item.reasons.some((r) => r.reason === reasonFilter.value));
});

function typeLabel(commitType: string | null) {
  return commitType ?? "非规范";
}

function when(unixSeconds: number) {
  return dayjs(unixSeconds * 1000).format("YYYY-MM-DD HH:mm");
}

async function load() {
  if (repoId.value === null) return;
  loading.value = true;
  error.value = null;
  try {
    const got = await complianceReport(repoId.value, rev.value);
    if (repoId.value === null) return;
    report.value = got;
    reasonFilter.value = null;
  } catch (err) {
    error.value = asError(err);
  } finally {
    loading.value = false;
  }
}

async function loadActivity() {
  if (repoId.value === null) return;
  activityLoading.value = true;
  activityError.value = null;
  const since = SINCE_PRESET[activityPreset.value];
  try {
    const got = await commitActivity(repoId.value, rev.value, since || undefined);
    // 期间切了仓库就把结果丢掉，否则统计会挂在错的仓库上
    if (repoId.value === null) return;
    activity.value = got;
  } catch (err) {
    activityError.value = asError(err);
  } finally {
    activityLoading.value = false;
  }
}

/**
 * 热力图单独拉一次"近一年"的数据，和左侧预设解耦（见上方注释）。
 * `"1 year ago"` 是 git 自己的口语，与历史页的 `--since` 同源，两边不会各算各的。
 */
async function loadHeat() {
  if (repoId.value === null) return;
  heatLoading.value = true;
  heatError.value = null;
  try {
    const got = await commitActivity(repoId.value, rev.value, "1 year ago");
    if (repoId.value === null) return;
    heat.value = got;
  } catch (err) {
    heatError.value = asError(err);
  } finally {
    heatLoading.value = false;
  }
}

function refreshCurrent() {
  if (view.value === "compliance") {
    void load();
    return;
  }
  void loadActivity();
  void loadHeat();
}

/**
 * 点明细跳到「历史」页的详情 + diff（需求 7.21 的第一条跳转路径）。
 * 改写入口（§7.14）在历史页那边，所以这里不重复造一个：
 * 报告的职责是指出"这条有问题"，处理交给能处理的地方。
 */
async function openDetail(sha: string) {
  if (repoId.value === null) return;
  await detail.open(repoId.value, sha);
  await router.push({ name: "history" });
}

function asError(err: unknown) {
  return err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
}

onMounted(async () => {
  if (repoId.value === null) return;
  try {
    revisions.value = await complianceRevisions(repoId.value);
  } catch {
    // 候选区间取不到不是错误：默认 HEAD 照样能统计
  }
  await load();
});

watch(repoId, refreshCurrent);
watch(rev, refreshCurrent);
watch(activityPreset, () => {
  if (view.value === "activity") void loadActivity();
});
watch(view, refreshCurrent);
</script>

<template>
  <div v-if="repoId === null" class="placeholder">
    <n-empty description="在左侧打开一个仓库" />
  </div>

  <div v-else class="page">
    <header class="head">
      <n-select
        v-model:value="rev"
        :options="revOptions"
        size="small"
        style="width: 220px"
        :loading="loading"
        tag
        filterable
      />
      <n-button
        size="small"
        :loading="loading || activityLoading || heatLoading"
        @click="refreshCurrent"
      >
        重新统计
      </n-button>
      <span v-if="view === 'compliance' && report" class="muted">
        规范来源：{{ SOURCE_LABEL[report.specSource] }}
      </span>
    </header>

    <n-tabs v-model:value="view" type="line" size="small" class="views">
      <n-tab-pane name="compliance" tab="符合率">
    <n-alert v-if="error" type="error" :title="error.message">
      <div>错误码：{{ error.code }}</div>
    </n-alert>

      <template v-if="report">
        <!-- 结论仪表盘：总符合率（多少）与不合规原因（为什么）同框，一眼看清 -->
        <n-card size="small">
          <div class="dashboard">
            <div class="dashboard-score">
              <div class="score">
                <div class="score-number">{{ percent }}<span class="percent">%</span></div>
                <div class="score-body">
                  <n-progress
                    type="line"
                    :percentage="percent"
                    :status="percent >= 90 ? 'success' : percent >= 70 ? 'warning' : 'error'"
                    :show-indicator="false"
                  />
                  <div class="muted">{{ denominatorNote }}</div>
                </div>
              </div>

              <div class="dashboard-alerts">
                <n-alert v-if="report.truncated" type="warning" :bordered="false">
                  这个区间的历史比统计上限还长，下面所有数字只覆盖最近 {{ report.scanned }} 条，
                  不是全量结论。想看全量请缩小区间（例如按 tag 切段）。
                </n-alert>

                <n-alert v-if="report.suspectedBypass > 0" type="warning" :bordered="false">
                  有 {{ report.suspectedBypass }} 条提交「疑似」用
                  <code>--no-verify</code> 绕过了 hook。
                  commit 对象里不会留下任何被绕过的痕迹，这是「该仓库已装 hook + 提交晚于安装时间 + 不合规」
                  推出来的间接判断，所以只能叫疑似。
                </n-alert>
                <n-alert v-else-if="!report.hookInstalled" type="info" :bordered="false">
                  这个仓库还没装 commit-msg hook，所以无法推断有没有人绕过规范——
                  在「提交」页一键安装之后，新提交的拦截与审计才闭环。
                </n-alert>
              </div>
            </div>

            <div class="dashboard-reasons">
              <div class="panel-title">
                不合规原因分布
                <span class="muted">（点原因只看对应提交）</span>
              </div>
              <n-empty v-if="!reasons.length" size="small" description="这个区间没有不合规提交" />
              <div v-for="item in reasons" :key="item.reason" class="bar-row">
                <button
                  class="bar-label"
                  :class="{ active: reasonFilter === item.reason }"
                  :title="`只看「${item.title}」的提交`"
                  @click="reasonFilter = reasonFilter === item.reason ? null : item.reason"
                >
                  {{ item.title }}
                </button>
                <div class="bar-track">
                  <div class="bar-fill" :style="{ width: `${Math.round(item.share * 100)}%` }" />
                </div>
                <span class="bar-value">{{ item.count }} 条</span>
              </div>
            </div>
          </div>
        </n-card>

        <!-- 按 type 分布：维度拆解的主轴，单独成卡铺满宽度，避免与矮卡配对产生空档 -->
        <n-card title="按 type 分布" size="small">
          <n-space vertical size="small">
            <div v-for="item in report.byType" :key="typeLabel(item.commitType)" class="type-row">
              <n-tag size="small" :bordered="false" :type="item.commitType ? 'info' : 'warning'">
                {{ typeLabel(item.commitType) }}
              </n-tag>
              <span class="muted">{{ item.conformant }} / {{ item.total }} 合规</span>
            </div>
            <n-empty v-if="!report.byType.length" size="small" description="没有可统计的提交" />
          </n-space>
        </n-card>

        <!-- 不合规明细：可点进「历史」页看改动与 diff -->
        <n-card size="small">
          <template #header>
            <span>不合规明细{{ reasonFilter ? "（已按原因筛选）" : "" }}</span>
          </template>
          <template #header-extra>
            <n-space align="center" size="small">
              <span v-if="report.offendersTruncated" class="muted">
                明细只列前若干条，下面总计为准
              </span>
              <n-button v-if="reasonFilter" size="small" quaternary @click="reasonFilter = null">
                清除筛选
              </n-button>
            </n-space>
          </template>

          <n-empty v-if="!offenders.length" size="small" description="没有不合规提交" />
          <table v-else class="table offenders">
            <thead>
              <tr>
                <th>提交</th>
                <th>作者</th>
                <th>时间</th>
                <th>原因</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="item in offenders" :key="item.id" class="offender" @click="openDetail(item.id)">
                <td class="subject" :title="item.subject">
                  {{ item.subject }}
                  <n-tag v-if="item.suspectedBypass" size="small" :bordered="false" type="warning">
                    疑似绕过
                  </n-tag>
                </td>
                <td class="author">{{ item.authorName }}</td>
                <td class="time">{{ when(item.time) }}</td>
                <td>
                  <n-space size="small">
                    <n-tag
                      v-for="reason in item.reasons"
                      :key="reason.reason"
                      size="small"
                      :bordered="false"
                      :type="reason.reason === 'subject_too_long' ? 'warning' : 'error'"
                    >
                      {{ reason.title }}
                    </n-tag>
                  </n-space>
                </td>
              </tr>
            </tbody>
          </table>

          <div v-if="offenders.length" class="muted foot">
            点任意一行跳到「历史」页看改动与 diff；落在可改写区间内的提交，可以在那边直接改写
            （§7.14），区间外的仅可审计，需要人工处理。
          </div>
        </n-card>
      </template>

    <n-empty v-else-if="!loading" description="还没有报告" />
      </n-tab-pane>

      <n-tab-pane name="activity" tab="活跃度">
        <div class="activity-head">
          <n-select
            v-model:value="activityPreset"
            :options="presetOptions"
            size="small"
            style="width: 140px"
            :loading="activityLoading"
          />
          <span class="muted">
            时间范围走 git 自己的口语（今日 = 今日零点起），所以与历史页的筛选是同一把尺
          </span>
        </div>

        <n-alert v-if="activityError" type="error" :title="activityError.message">
          <div>错误码：{{ activityError.code }}</div>
        </n-alert>

        <template v-if="activity">
          <n-card size="small">
            <div class="totals">
              <div class="total">
                <span class="total-number">{{ activity.commits }}</span>
                <span class="total-label">次提交</span>
              </div>
              <div class="total">
                <span class="total-number plus">+{{ activity.insertions }}</span>
                <span class="total-label">新增行</span>
              </div>
              <div class="total">
                <span class="total-number minus">−{{ activity.deletions }}</span>
                <span class="total-label">删除行</span>
              </div>
              <div class="total">
                <span class="total-number">{{ activity.authorCount }}</span>
                <span class="total-label">位作者</span>
              </div>
              <div class="total">
                <span class="total-number">{{ activity.activeDays }}</span>
                <span class="total-label">个活跃日</span>
              </div>
            </div>
            <div class="muted">{{ activityNote }}</div>
          </n-card>

          <n-alert v-if="activity.truncated" type="warning" :bordered="false">
            这个区间的历史比统计上限还长，下面只覆盖最近一批提交，不是全量结论。
            想看全量请缩短区间。
          </n-alert>

          <n-alert type="info" :bordered="false">
            行数是 git 记录的增删行，不是净产出：rebase / amend 会把同一次改动数两遍。
            它用来回答“最近谁在动这个仓库”，不回答“谁的代码多”。
          </n-alert>

          <!-- 提交热力图：固定近一年，与左侧范围解耦——热力图看的是长期节奏，
               跟着"今日/本周"只会剩几个格子 -->
          <n-card title="提交热力图（近一年）" size="small">
            <n-alert v-if="heatError" type="error" :title="heatError.message">
              <div>错误码：{{ heatError.code }}</div>
            </n-alert>
            <template v-else-if="heat">
              <div class="heat">
                <div class="heat-weekdays">
                  <span v-for="(label, row) in HEAT_WEEKDAYS" :key="row">{{ label }}</span>
                </div>
                <div class="heat-body">
                  <div class="heat-months">
                    <div v-for="col in heatCols" :key="col.key" class="heat-month">
                      {{ col.month ?? "" }}
                    </div>
                  </div>
                  <div class="heat-grid">
                    <template v-for="col in heatCols" :key="col.key">
                      <div
                        v-for="cell in col.cells"
                        :key="cell.key"
                        class="heat-cell"
                        :class="{ future: cell.future }"
                        :data-level="cell.level"
                        :title="cell.title"
                      />
                    </template>
                  </div>
                </div>
              </div>
              <div class="heat-foot">
                <span class="muted">{{ heatNote }}</span>
                <span class="heat-legend">
                  <span class="muted">少</span>
                  <i class="heat-cell" data-level="0" />
                  <i class="heat-cell" data-level="1" />
                  <i class="heat-cell" data-level="2" />
                  <i class="heat-cell" data-level="3" />
                  <i class="heat-cell" data-level="4" />
                  <span class="muted">多</span>
                </span>
              </div>
            </template>
            <n-empty v-else-if="!heatLoading" size="small" description="近一年没有提交" />
          </n-card>

          <n-card title="按作者（提交多的在前）" size="small">
            <table class="table">
              <thead>
                <tr>
                  <th>作者</th>
                  <th>提交</th>
                  <th>+/−</th>
                  <th>文件</th>
                  <th>活跃天</th>
                  <th>最近</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="row in activity.authors" :key="row.email + row.name">
                  <td class="name" :title="row.email">{{ row.name }}</td>
                  <td class="count">{{ row.commits }}</td>
                  <td class="lines">
                    <span class="plus">+{{ row.insertions }}</span>
                    <span class="minus">−{{ row.deletions }}</span>
                  </td>
                  <td class="count">{{ row.files }}</td>
                  <td class="count">{{ row.activeDays }}</td>
                  <td class="when">{{ dayjs(row.lastTime * 1000).format("MM-DD") }}</td>
                </tr>
              </tbody>
            </table>
            <n-empty v-if="!activity.authors.length" size="small" description="这个区间没有提交" />
            <div v-if="activity.authors.length" class="muted foot">
              作者按仓库里的 <code>.mailmap</code> 归并：同一个人换了邮箱或名字只占一行。
              没有 mailmap 时同名不同邮箱仍算两个人。
            </div>
          </n-card>
        </template>

        <n-empty v-else-if="!activityLoading" description="还没有统计" />
      </n-tab-pane>
    </n-tabs>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.head {
  display: flex;
  align-items: center;
  gap: 10px;
}

/* 页签贴着区间选择器：区间是共用的，两个视角都吃它，不该看上去像各管各的 */
.views {
  margin-top: -6px;
}

.activity-head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
}

/* 汇总条：五个数字一排。这是"先看结论"的一层，下面才是表格 */
.totals {
  display: flex;
  gap: 28px;
  margin-bottom: 8px;
}

.total {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.total-number {
  font-size: 22px;
  font-weight: 700;
  line-height: 1.1;
  color: var(--text-1);
}

.total-label {
  font-size: 11px;
  color: var(--text-3);
}

.plus {
  color: var(--ok-text);
}

.minus {
  color: var(--warn-text);
  margin-left: 6px;
}

/* 提交热力图：53 列 × 7 行（列优先），自带星期/月坐标与图例。
   不为它引图表库——格子是 div、颜色走 token，跟整页同一套色 */
.heat {
  display: flex;
  gap: 6px;
}

/* 星期标签：与网格 7 行对齐。行用 1fr 随网格高度走，padding-top 让出上面月标签那一行 */
.heat-weekdays {
  display: grid;
  grid-template-rows: repeat(7, 1fr);
  gap: 2px;
  padding-top: 18px;
  font-size: 11px;
  line-height: 1;
  color: var(--text-3);
}

/* 自适应铺满卡片宽度：窗口越宽格子越大，不会在右侧留空。
   max-width 是上限，免得超宽窗口里格子大到离谱 */
.heat-body {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  min-width: 0;
  max-width: 1160px;
}

.heat-months {
  display: grid;
  grid-auto-flow: column;
  grid-auto-columns: 1fr;
  gap: 2px;
  height: 14px;
  font-size: 11px;
  line-height: 14px;
  color: var(--text-3);
}

/* 标签比格子宽，让它自然往右溢出到空格子上，不挤压网格 */
.heat-month {
  white-space: nowrap;
}

/* 53 列 × 7 行都用 1fr，格子随卡片宽度等比放大；
   aspect-ratio 让每格接近正方形（53:7 是整块网格的长宽比） */
.heat-grid {
  display: grid;
  grid-auto-flow: column;
  grid-template-rows: repeat(7, 1fr);
  grid-auto-columns: 1fr;
  gap: 2px;
  aspect-ratio: 53 / 7;
}

.heat-cell {
  display: block;
  width: 100%;
  height: 100%;
  border-radius: 3px;
  background: var(--surface-sunken);
}

/* 未来日期只占位、不显示，保证网格对齐 */
.heat-cell.future {
  visibility: hidden;
}

.heat-cell[data-level="1"] {
  background: var(--accent);
  opacity: 0.3;
}

.heat-cell[data-level="2"] {
  background: var(--accent);
  opacity: 0.5;
}

.heat-cell[data-level="3"] {
  background: var(--accent);
  opacity: 0.72;
}

.heat-cell[data-level="4"] {
  background: var(--accent);
  opacity: 1;
}

.heat-grid .heat-cell:hover {
  outline: 1px solid var(--border-strong);
  outline-offset: 1px;
}

.heat-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 10px;
}

.heat-legend {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

/* 图例里的格子要固定尺寸，否则会被 .heat-cell 的 width:100% 拉变形 */
.heat-legend .heat-cell {
  flex: none;
  width: 13px;
  height: 13px;
}

.lines {
  white-space: nowrap;
}

.when {
  opacity: 0.7;
}

/* 结论仪表盘：总符合率（多少）与不合规原因（为什么）同框，中间细分隔线提示两个不同维度 */
.dashboard {
  display: grid;
  grid-template-columns: 280px 1fr;
  gap: 24px;
}

.dashboard-score {
  min-width: 0;
}

.dashboard-alerts {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 12px;
}

.dashboard-reasons {
  min-width: 0;
  padding-left: 24px;
  border-left: 1px solid var(--border);
}

.panel-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-1);
  margin-bottom: 12px;
}

.score {
  display: flex;
  align-items: center;
  gap: 16px;
  margin-bottom: 8px;
}

.score-number {
  font-size: 34px;
  font-weight: 700;
  line-height: 1;
  color: var(--text-1);
}

.percent {
  font-size: 16px;
  margin-left: 2px;
  opacity: 0.6;
}

.score-body {
  flex: 1;
  min-width: 0;
}

.bar-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 2px 0;
}

.bar-label {
  font: inherit;
  font-size: 12px;
  background: none;
  border: none;
  padding: 0;
  cursor: pointer;
  color: var(--text-2);
  width: 140px;
  text-align: left;
}

.bar-label.active {
  color: var(--accent);
  font-weight: 600;
}

.bar-track {
  flex: 1;
  height: 6px;
  border-radius: 3px;
  background: var(--surface-sunken);
  overflow: hidden;
}

.bar-fill {
  height: 100%;
  background: var(--accent);
}

.bar-value {
  font-size: 12px;
  opacity: 0.7;
  width: 52px;
  text-align: right;
}

.type-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
}

.table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}

.table td,
.table th {
  text-align: left;
  padding: 3px 6px;
  border-bottom: 1px solid var(--surface-sunken);
}

.table .name {
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.table .rate {
  width: 56px;
  font-weight: 600;
}

.table .count {
  width: 84px;
  opacity: 0.65;
}

.offenders td {
  vertical-align: top;
}

.offender {
  cursor: pointer;
}

.offender:hover td {
  background: var(--surface-sunken);
}

.offenders .subject {
  max-width: 380px;
}

.offenders .author,
.offenders .time {
  width: 120px;
  opacity: 0.75;
}

.muted {
  font-size: 12px;
  opacity: 0.7;
}

.foot {
  margin-top: 8px;
}

.placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 60vh;
}
</style>