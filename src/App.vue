<script setup lang="ts">
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  NAlert,
  NButton,
  NConfigProvider,
  NMessageProvider,
  NProgress,
  NSpace,
  NTab,
  NTabs,
  dateZhCN,
  zhCN,
  type GlobalThemeOverrides,
} from "naive-ui";
import RepoSidebar from "@/components/RepoSidebar.vue";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";
import { INTERRUPT_LABEL, type Interrupt } from "@/api/refs";

/**
 * 外壳照 Fork：左栏常驻仓库与只读信息，主区用页签切「历史 / 提交」。
 * 页签状态由路由决定，这样刷新和深链都能落回同一个页签。
 */
const route = useRoute();
const router = useRouter();
const repos = useReposStore();
const writes = useWriteStore();

/**
 * 每种中断态的出口。这一批只给文字指引，等 M2/M3 有「继续 / 中止」按钮再换成按钮。
 * 状态名本身不在这里重复定义，用 INTERRUPT_LABEL——它与 Rust 侧 `Interrupt::label()`
 * 逐字对齐，所以提示条和提交被拒时的错误文案叫法一致。
 */
const INTERRUPT_EXIT: Record<Interrupt, string> = {
  none: "",
  merge: "解决冲突后用 git merge --continue 收尾，或 git merge --abort 放弃这次合并",
  rebase: "解决冲突后用 git rebase --continue 继续，或 git rebase --abort 回到变基之前",
  cherry_pick: "解决冲突后用 git cherry-pick --continue 继续，或 --abort 放弃这次摘取",
  revert: "解决冲突后用 git revert --continue 继续，或 --abort 放弃这次回滚",
};

const themeOverrides: GlobalThemeOverrides = {
  common: {
    primaryColor: "#1F5AA8",
    primaryColorHover: "#2C6FC4",
    primaryColorPressed: "#1A4C8F",
    borderRadius: "6px",
  },
};

const activeTab = computed(() => {
  if (route.name === "commit") return "commit";
  if (route.name === "files") return "files";
  return "history";
});
/** 只读浏览仓库没有索引，提交页整个不给进（Rust 侧同样会拒，这里只是不摆出能点的按钮） */
const commitTabEnabled = computed(() => repos.canCommit);

const headline = computed(() => {
  const repo = repos.current;
  if (!repo) return "还没有打开仓库";
  return `${repo.name} · ${branchPart()} · ${repos.workingFiles.length} 项待提交`;
});

/**
 * 中断时这里直接换成中断态：变基过程中 HEAD 是游离的，写"游离 HEAD"会让人以为
 * 分支丢了，而真正要回答的是"哪个分支正卡在半路"（§7.3）。
 */
function branchPart() {
  const state = repos.repoState;
  if (repos.interrupted) {
    const label = INTERRUPT_LABEL[repos.interrupt];
    return state?.interruptBranch ? `${label}：${state.interruptBranch}` : label;
  }
  return state?.branch ?? repos.info?.branch ?? "游离 HEAD";
}

const interruptTitle = computed(() => INTERRUPT_LABEL[repos.interrupt]);
const interruptExit = computed(() => INTERRUPT_EXIT[repos.interrupt]);

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
              <!-- 文件页只读浏览，不碰工作区，所以 browse 仓库也摆出来 -->
              <n-tab name="files">文件</n-tab>
              <n-tab name="commit" :disabled="!commitTabEnabled">提交</n-tab>
            </n-tabs>
          </header>

          <!-- 中断态常驻、关不掉：它不是一个可以"知道了"的提醒，而是写入口为什么灰着 -->
          <n-alert
            v-if="repos.interrupted"
            class="banner interrupt"
            type="warning"
            :title="interruptTitle"
            :closable="false"
          >
            <div>{{ interruptExit }}</div>
            <div class="muted">
              M2 不提供逐块取舍：冲突要么一键退回，要么在终端里处理完再回来。
            </div>
            <n-space size="small" class="recheck">
              <n-button
                size="tiny"
                type="warning"
                :loading="writes.busy"
                @click="writes.abort()"
              >
                一键退回
              </n-button>
              <n-button
                size="tiny"
                :loading="repos.loading"
                @click="repos.refreshAll()"
              >
                已在终端处理完，重读一次
              </n-button>
            </n-space>
          </n-alert>

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

/* 桌面外壳不该有整页滚动条：滚动只属于各个面板自己（.content 和左栏都各自 overflow）。
   关掉它，任何一个页面板写错高度都只会缩掉自己那一块，不会把整窗拖走。 */
html,
body {
  overflow: hidden;
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

/* 常驻提示条不能被主区挤压掉：整屏就数它最不能看不见 */
.interrupt {
  flex: none;
}

.recheck {
  margin-top: 8px;
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
