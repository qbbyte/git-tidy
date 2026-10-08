<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { NButton, NDatePicker, NInput, NSelect, NSpace, NTag } from "naive-ui";
import { filterIsEmpty, type CommitFilter } from "@/api/commit";
import type { Ref } from "@/api/refs";

/**
 * 历史页的筛选条（§7.7）。
 *
 * 两类控件：git 认识的条件（rev 范围 / 作者 / 时间 / 关键词）走 Rust 侧映射成 `log` 参数；
 * git 不认识的（type / 是否合规）由 Rust 侧在解析层判，所以**列表与图要按同一个可见集合算**，
 * 换条件时两边一起重来。
 *
 * 这一条只改**输入草稿**，不回写 store——回写就意味着每敲一个字符重取一次历史。
 * 真正生效靠「筛选」按钮（或在关键词框里回车），「清空」一键回到无筛选。
 */
const props = defineProps<{
  filter: CommitFilter;
  /** 仓库的引用，用来给 rev 范围做补全：分支名手打容易打错 */
  refs: Ref[];
  /** 规范里的 type 白名单，从提交表单那份规范来（需求 6.7 同一份尺子） */
  types: string[];
  busy: boolean;
}>();

const emit = defineEmits<{ apply: [CommitFilter]; clear: [] }>();

const rev = ref<string | null>(props.filter.rev ?? null);
const author = ref<string>(props.filter.authors?.[0] ?? "");
const keyword = ref<string>(props.filter.grep?.[0] ?? "");
const path = ref<string>(props.filter.path ?? "");
const since = ref<number | null>(props.filter.since ? Date.parse(props.filter.since) : null);
const until = ref<number | null>(props.filter.until ? Date.parse(props.filter.until) : null);
const pickedTypes = ref<string[]>(props.filter.types ?? []);
const conformant = ref<"any" | "yes" | "no">(
  props.filter.conformant === undefined ? "any" : props.filter.conformant ? "yes" : "no",
);

/** store 里的条件被别处改掉（例如点了「清空」）时，草稿要跟上，不然会显示旧值 */
watch(
  () => props.filter,
  (next) => {
    rev.value = next.rev ?? null;
    author.value = next.authors?.[0] ?? "";
    keyword.value = next.grep?.[0] ?? "";
    path.value = next.path ?? "";
    since.value = next.since ? Date.parse(next.since) : null;
    until.value = next.until ? Date.parse(next.until) : null;
    pickedTypes.value = next.types ?? [];
    conformant.value = next.conformant === undefined ? "any" : next.conformant ? "yes" : "no";
  },
  { deep: true },
);

/** 分支/标签补全。ref 名允许重名（本地 feat/x 与 origin/feat/x），所以带上来源前缀 */
const revOptions = computed(() =>
  props.refs
    .filter((ref) => ref.kind !== "remote")
    .map((ref) => ({
      label: `${ref.kind === "tag" ? "标签" : "分支"} ${ref.name}`,
      value: ref.name,
    })),
);

const conformantOptions = [
  { label: "不限", value: "any" },
  { label: "只看合规", value: "yes" },
  { label: "只看不合规", value: "no" },
];

const dirty = computed(
  () => JSON.stringify(collect()) !== JSON.stringify(normalizeProps(props.filter)),
);

/** 筛选条的"回显"那一条要不要显示：条件不空才显示 */
const filtering = computed(() => !filterIsEmpty(props.filter));

/** 组一份条件出来。空值一律不传，Rust 侧 `Filter` 的 default 才是那个语义 */
function collect(): CommitFilter {
  const next: CommitFilter = {};
  if (rev.value !== null && rev.value.trim() !== "") next.rev = rev.value.trim();
  if (author.value.trim() !== "") next.authors = [author.value.trim()];
  if (keyword.value.trim() !== "") next.grep = [keyword.value.trim()];
  if (path.value.trim() !== "") next.path = path.value.trim();
  if (since.value !== null) next.since = new Date(since.value).toISOString().slice(0, 10);
  if (until.value !== null) next.until = new Date(until.value).toISOString().slice(0, 10);
  if (pickedTypes.value.length > 0) next.types = [...pickedTypes.value];
  if (conformant.value !== "any") next.conformant = conformant.value === "yes";
  return next;
}

/** 把条件摊平成固定形状再比：字段顺序、缺省空值都不该算成"改了" */
function normalizeProps(filter: CommitFilter): CommitFilter {
  const next: CommitFilter = {};
  if (filter.rev) next.rev = filter.rev;
  if (filter.authors && filter.authors.length > 0) next.authors = filter.authors;
  if (filter.grep && filter.grep.length > 0) next.grep = filter.grep;
  if (filter.path) next.path = filter.path;
  if (filter.since) next.since = filter.since;
  if (filter.until) next.until = filter.until;
  if (filter.types && filter.types.length > 0) next.types = filter.types;
  if (filter.conformant !== undefined && filter.conformant !== null) {
    next.conformant = filter.conformant;
  }
  return next;
}

function apply() {
  emit("apply", collect());
}

function clear() {
  rev.value = null;
  author.value = "";
  keyword.value = "";
  path.value = "";
  since.value = null;
  until.value = null;
  pickedTypes.value = [];
  conformant.value = "any";
  emit("clear");
}
</script>

<template>
  <div class="bar">
    <n-space align="center" size="small" :wrap="false">
      <n-select
        v-model:value="rev"
        :options="revOptions"
        filterable
        tag
        clearable
        size="small"
        placeholder="分支 / rev 范围"
        class="rev"
      />
      <n-input
        v-model:value="author"
        size="small"
        clearable
        placeholder="作者"
        class="narrow"
        @keyup.enter="apply"
      />
      <n-input
        v-model:value="keyword"
        size="small"
        clearable
        placeholder="关键词（标题/正文）"
        class="keyword"
        @keyup.enter="apply"
      />
      <n-input
        v-model:value="path"
        size="small"
        clearable
        placeholder="路径"
        class="narrow"
        @keyup.enter="apply"
      />
      <n-date-picker v-model:value="since" size="small" type="date" placeholder="起始日期" clearable />
      <n-date-picker v-model:value="until" size="small" type="date" placeholder="截止日期" clearable />
      <n-select
        v-model:value="pickedTypes"
        :options="types.map((type) => ({ label: type, value: type }))"
        multiple
        size="small"
        clearable
        placeholder="type"
        class="narrow"
      />
      <n-select
        v-model:value="conformant"
        :options="conformantOptions"
        size="small"
        class="narrow"
      />
      <n-button size="small" type="primary" :loading="busy" :disabled="!dirty" @click="apply">
        筛选
      </n-button>
      <n-button size="small" quaternary :disabled="!dirty" @click="clear">清空</n-button>
    </n-space>
    <!-- 有筛选时把条件原样列出来：图上那些截断端点就是这么来的，得让人看得见原因 -->
    <div v-if="filtering" class="echo">
      <n-tag size="tiny" :bordered="false" type="info">筛选中</n-tag>
      <span v-if="filter.rev">rev：{{ filter.rev }}</span>
      <span v-if="filter.authors?.length">作者：{{ filter.authors.join("、") }}</span>
      <span v-if="filter.grep?.length">关键词：{{ filter.grep.join("、") }}</span>
      <span v-if="filter.path">路径：{{ filter.path }}</span>
      <span v-if="filter.since">自 {{ filter.since }}</span>
      <span v-if="filter.until">至 {{ filter.until }}</span>
      <span v-if="filter.types?.length">type：{{ filter.types.join("、") }}</span>
      <span v-if="filter.conformant === true">只看合规</span>
      <span v-if="filter.conformant === false">只看不合规</span>
    </div>
  </div>
</template>

<style scoped>
.bar {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: none;
}

.rev {
  width: 190px;
}

.narrow {
  width: 130px;
}

.keyword {
  width: 190px;
}

.echo {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  font-size: 11px;
  opacity: 0.75;
}
</style>