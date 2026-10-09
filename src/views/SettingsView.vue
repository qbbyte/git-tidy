<script setup lang="ts">
import { computed, onMounted } from "vue";
import {
  NAlert,
  NButton,
  NCard,
  NInput,
  NSelect,
  NSpace,
  NSwitch,
  NTag,
} from "naive-ui";
import { PULL_STRATEGY_LABEL, type PullStrategy } from "@/api/prefs";
import { usePrefsStore } from "@/stores/prefs";
import { SHORTCUTS, TAB_SHORTCUTS, formatKeys } from "@/shortcuts";

/**
 * 设置页（需求 7.23）。
 *
 * 两块内容刻意放在一起：
 * - 偏好：真正会改行为的几项（拉取策略、Flow 前缀、列显示、自动更新），落盘到
 *   app config dir 的 `preferences.json`，文件路径显示在下面；
 * - 快捷键说明：**由 `SHORTCUTS` 数组渲染**，那份数组同时驱动按键绑定。
 *   所以这张表不可能与实际行为对不上——这是把说明书和实现绑成一个数据源的意义。
 */
const prefsStore = usePrefsStore();

const prefs = computed(() => prefsStore.prefs);

const pullOptions = (Object.keys(PULL_STRATEGY_LABEL) as PullStrategy[]).map((value) => ({
  label: PULL_STRATEGY_LABEL[value],
  value,
}));

const columnOptions = [
  { key: "refs" as const, label: "引用徽标（分支 / 标签）" },
  { key: "author" as const, label: "作者" },
  { key: "time" as const, label: "时间" },
  { key: "sha" as const, label: "提交号" },
];

/** 说明表按分组归拢，顺序跟数组里的顺序一致 */
const grouped = computed(() => {
  const groups: { group: string; rows: { keys: string; label: string; when: string }[] }[] = [];
  for (const shortcut of SHORTCUTS) {
    let bucket = groups.find((item) => item.group === shortcut.group);
    if (!bucket) {
      bucket = { group: shortcut.group, rows: [] };
      groups.push(bucket);
    }
    bucket.rows.push({
      keys: formatKeys(shortcut.keys),
      label: shortcut.label,
      when: shortcut.when,
    });
  }
  return groups;
});

function setPull(value: PullStrategy) {
  prefsStore.patch({ pullStrategy: value });
}

function setColumn(key: "refs" | "author" | "time" | "sha", value: boolean) {
  if (!prefs.value) return;
  prefsStore.patch({ columns: { ...prefs.value.columns, [key]: value } });
}

function setPrefix(key: "feature" | "hotfix" | "release", value: string) {
  if (!prefs.value) return;
  prefsStore.patch({ flowPrefixes: { ...prefs.value.flowPrefixes, [key]: value } });
}

onMounted(() => prefsStore.load());
</script>

<template>
  <div class="page">
    <n-alert v-if="prefsStore.error" type="error" :title="prefsStore.error.message">
      <div>错误码：{{ prefsStore.error.code }}</div>
    </n-alert>

    <n-card size="small" title="拉取">
      <div class="row">
        <span class="label">拉取策略</span>
        <n-select
          :value="prefs?.pullStrategy ?? 'ff_only'"
          :options="pullOptions"
          size="small"
          style="width: 260px"
          :disabled="!prefs"
          @update:value="setPull"
        />
        <span class="muted">
          默认是"只快进"：远端有本地没有的提交就停下来问，不自动制造合并提交
        </span>
      </div>
    </n-card>

    <n-card size="small" title="Git Flow 命名前缀">
      <n-space vertical size="small">
        <div v-for="key in (['feature', 'hotfix', 'release'] as const)" :key="key" class="row">
          <span class="label">{{ key }}</span>
          <n-input
            :value="prefs?.flowPrefixes[key] ?? ''"
            size="small"
            style="width: 220px"
            :placeholder="key"
            @update:value="(value: string) => setPrefix(key, value)"
          />
          <code v-if="prefs" class="muted">
            {{ prefs.flowPrefixes[key] }}{{ key === "release" ? "1.2.0" : "示例" }}
          </code>
        </div>
        <div class="muted">清空即不加前缀。Git Flow 是自组合的，不依赖系统里装了 git-flow。</div>
      </n-space>
    </n-card>

    <n-card size="small" title="提交列表显示的列">
      <n-space size="large">
        <n-space
          v-for="option in columnOptions"
          :key="option.key"
          align="center"
          size="small"
        >
          <span>{{ option.label }}</span>
          <n-switch
            :value="prefs?.columns[option.key] ?? true"
            size="small"
            :disabled="!prefs"
            @update:value="(value: boolean) => setColumn(option.key, value)"
          />
        </n-space>
      </n-space>
      <div class="muted">关掉只是不显示，数据还在——报告与 CHANGELOG 照样按它们统计。</div>
    </n-card>

    <n-card size="small" title="更新">
      <div class="row">
        <span class="label">启动时检查一次新版本</span>
        <n-switch
          :value="prefs?.autoUpdate ?? true"
          size="small"
          :disabled="!prefs"
          @update:value="(value: boolean) => prefsStore.patch({ autoUpdate: value })"
        />
        <span class="muted">关掉就完全不联网问版本；更新源是 GitHub Releases 的静态文件，不自建服务端。</span>
      </div>
    </n-card>

    <n-card size="small" title="快捷键">
      <n-space vertical size="medium">
        <div v-for="bucket in grouped" :key="bucket.group">
          <div class="group-title">{{ bucket.group }}</div>
          <table class="table">
            <tbody>
              <tr v-for="row in bucket.rows" :key="row.keys">
                <td class="keys">
                  <n-tag size="small" :bordered="false">{{ row.keys }}</n-tag>
                </td>
                <td>{{ row.label }}</td>
                <td class="when">{{ row.when }}</td>
              </tr>
            </tbody>
          </table>
        </div>

        <div>
          <div class="group-title">主区页签直达</div>
          <table class="table">
            <tbody>
              <tr v-for="tab in TAB_SHORTCUTS" :key="tab.key">
                <td class="keys">
                  <n-tag size="small" :bordered="false">{{ tab.key }}</n-tag>
                </td>
                <td>切到「{{ tab.label }}」</td>
                <td class="when"></td>
              </tr>
            </tbody>
          </table>
        </div>

        <n-space>
          <n-button size="small" quaternary @click="prefsStore.reset">恢复默认偏好</n-button>
        </n-space>
      </n-space>
    </n-card>

    <n-card size="small" title="这些设置存在哪">
      <code class="path">{{ prefsStore.path || "读取中" }}</code>
      <div class="muted">
        个人偏好跟着这台机器走，不进仓库；团队规范在仓库里的
        <code>git-tidy.config.json</code>（或 commitlint / versionrc / cliff）——两者不互相覆盖。
      </div>
    </n-card>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
  max-width: 760px;
}

.row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.label {
  font-size: 12px;
  color: var(--text-2);
  width: 150px;
}

.group-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-3);
  margin-bottom: 4px;
}

.table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}

.table td {
  padding: 3px 8px 3px 0;
  border-bottom: 1px solid var(--surface-sunken);
  vertical-align: top;
}

.keys {
  width: 130px;
}

.when {
  opacity: 0.65;
  width: 260px;
}

.path {
  display: block;
  margin-bottom: 6px;
  font-size: 12px;
  word-break: break-all;
}

.muted {
  font-size: 12px;
  opacity: 0.7;
}
</style>