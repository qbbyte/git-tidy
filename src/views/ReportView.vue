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
  NTag,
} from "naive-ui";
import { GitTidyError } from "@/api/client";
import {
  complianceReport,
  complianceRevisions,
  type ComplianceReport,
} from "@/api/report";
import { SOURCE_LABEL } from "@/api/spec";
import { useReposStore } from "@/stores/repos";
import { useDetailStore } from "@/stores/detail";

/**
 * 符合率报告（需求 7.21）。
 *
 * 页面只做两件事：把 Rust 侧算好的数字摆出来，以及把不合规的提交送到能处理它的地方。
 * **所有判定都在 Rust 侧**——报告页不重写规则，所以表单、hook、报告永远是同一把尺子。
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

/** 按月倒序：最近的在最上面，趋势才看得出方向 */
const months = computed(() => [...(report.value?.byMonth ?? [])].reverse());
const authors = computed(() => report.value?.byAuthor ?? []);

function typeLabel(commitType: string | null) {
  return commitType ?? "非规范";
}

function percentOf(rate: number) {
  return Math.round(rate * 100);
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

watch(repoId, load);
watch(rev, load);
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
      <n-button size="small" :loading="loading" @click="load">重新统计</n-button>
      <span v-if="report" class="muted">规范来源：{{ SOURCE_LABEL[report.specSource] }}</span>
    </header>

    <n-alert v-if="error" type="error" :title="error.message">
      <div>错误码：{{ error.code }}</div>
    </n-alert>

    <template v-if="report">
      <n-card size="small">
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
      </n-card>

      <div class="grid">
        <n-card title="不合规原因分布" size="small">
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
        </n-card>

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
      </div>

      <div class="grid">
        <n-card title="按作者" size="small">
          <table class="table">
            <tbody>
              <tr v-for="row in authors" :key="row.key">
                <td class="name" :title="row.key">{{ row.key }}</td>
                <td class="rate">{{ percentOf(row.rate) }}%</td>
                <td class="count">{{ row.conformant }} / {{ row.total }}</td>
              </tr>
            </tbody>
          </table>
          <n-empty v-if="!authors.length" size="small" description="没有数据" />
        </n-card>

        <n-card title="按月趋势（最近在前）" size="small">
          <table class="table">
            <tbody>
              <tr v-for="row in months" :key="row.key">
                <td class="name">{{ row.key }}</td>
                <td class="rate">{{ percentOf(row.rate) }}%</td>
                <td class="count">{{ row.conformant }} / {{ row.total }}</td>
              </tr>
            </tbody>
          </table>
          <n-empty v-if="!months.length" size="small" description="没有数据" />
        </n-card>
      </div>

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

.grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
  align-items: start;
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