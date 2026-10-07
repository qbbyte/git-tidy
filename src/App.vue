<script setup lang="ts">
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  NAlert,
  NConfigProvider,
  NMessageProvider,
  NProgress,
  NTab,
  NTabs,
  dateZhCN,
  zhCN,
  type GlobalThemeOverrides,
} from "naive-ui";
import RepoSidebar from "@/components/RepoSidebar.vue";
import { useReposStore } from "@/stores/repos";

/**
 * 外壳照 Fork：左栏常驻仓库与只读信息，主区用页签切「历史 / 提交」。
 * 页签状态由路由决定，这样刷新和深链都能落回同一个页签。
 */
const route = useRoute();
const router = useRouter();
const repos = useReposStore();

const themeOverrides: GlobalThemeOverrides = {
  common: {
    primaryColor: "#1F5AA8",
    primaryColorHover: "#2C6FC4",
    primaryColorPressed: "#1A4C8F",
    borderRadius: "6px",
  },
};

const activeTab = computed(() => (route.name === "commit" ? "commit" : "history"));
/** 只读浏览仓库没有索引，提交页整个不给进（Rust 侧同样会拒，这里只是不摆出能点的按钮） */
const commitTabEnabled = computed(() => repos.canCommit);

const headline = computed(() => {
  const repo = repos.current;
  if (!repo) return "还没有打开仓库";
  const branch = repos.info?.branch ?? "游离 HEAD";
  const dirty = repos.workingFiles.length;
  return `${repo.name} · ${branch} · ${dirty} 项待提交`;
});

function go(name: string | number) {
  router.push({ name: String(name) });
}

function closeError() {
  repos.error = null;
}
</script>

<template>
  <n-config-provider :locale="zhCN" :date-locale="dateZhCN" :theme-overrides="themeOverrides">
    <n-message-provider>
      <div class="shell">
        <repo-sidebar class="sider" />

        <section class="main">
          <header class="bar">
            <span class="headline" :title="headline">{{ headline }}</span>
            <n-tabs type="line" size="small" :value="activeTab" @update:value="go">
              <n-tab name="history">历史</n-tab>
              <n-tab name="commit" :disabled="!commitTabEnabled">提交</n-tab>
            </n-tabs>
          </header>

          <div v-if="repos.progress" class="banner">
            <div class="banner-title">下载中：{{ repos.progress.url }}</div>
            <n-progress
              type="line"
              :percentage="repos.progress.percent"
              :indeterminate="repos.progress.percent === 0"
              processing
              show-indicator
            />
            <div class="muted">{{ repos.progress.phase }}</div>
          </div>

          <n-alert
            v-if="repos.error"
            class="banner"
            type="error"
            :title="repos.error.message"
            closable
            @close="closeError"
          >
            <div>错误码：{{ repos.error.code }}</div>
            <pre v-if="repos.error.detail" class="raw-output">{{ repos.error.detail }}</pre>
          </n-alert>

          <div class="content">
            <router-view />
          </div>
        </section>
      </div>
    </n-message-provider>
  </n-config-provider>
</template>

<style>
html,
body,
#app {
  height: 100%;
  margin: 0;
}
</style>

<style scoped>
.shell {
  display: flex;
  height: 100vh;
  background: #f5f7fa;
}

.sider {
  flex: none;
  width: 280px;
}

.main {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
}

.bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 8px 16px;
  background: #fff;
  border-bottom: 1px solid #e5e8ee;
}

.headline {
  font-size: 12px;
  opacity: 0.8;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.banner {
  margin: 12px 16px 0;
}

.banner-title {
  font-size: 12px;
  margin-bottom: 6px;
}

.content {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 12px 16px 16px;
}

.muted {
  font-size: 11px;
  opacity: 0.7;
}

.raw-output {
  margin: 8px 0 0;
  white-space: pre-wrap;
  font-size: 12px;
  opacity: 0.8;
}
</style>
