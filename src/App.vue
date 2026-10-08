<script setup lang="ts">
import { computed, ref } from "vue";
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
import { FONT_UI, RADIUS_CONTROL, tokens } from "@/styles/tokens";
import { usePaneDivider } from "@/composables/usePaneDivider";

/**
 * 外壳照 Fork：左栏常驻仓库与只读信息，主区用页签切「历史 / 提交」。
 * 页签状态由路由决定，这样刷新和深链都能落回同一个页签。
 */
const route = useRoute();
const router = useRouter();
const repos = useReposStore();
const writes = useWriteStore();

/**
 * 侧栏宽度，像素单位：它是导航，带宽固定比跟着窗口一起长要好。
 *
 * 上限 420 是因为再宽就变成第二个主区了，而主区里还摆着列表与 diff 两栏。
 */
const shell = ref<HTMLElement | null>(null);
const sider = usePaneDivider({
  container: shell,
  storageKey: "git-tidy:sidebar:width",
  fallback: 280,
  min: 220,
  max: 420,
  unit: "px",
});

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

/**
 * Naive UI 的主题覆盖。
 *
 * **颜色全部从 `@/styles/tokens` 取**，不写字面色值：CSS 变量进不了 Naive 的
 * 派生逻辑（hover / pressed 是它自己按 primary 算的），所以这里是唯一需要
 * 「重复一遍」的地方——但它重复的是同一个常量，不是另一个手抄的色号。
 */
const themeOverrides: GlobalThemeOverrides = {
  common: {
    primaryColor: tokens.accent,
    primaryColorHover: tokens.accentHover,
    primaryColorPressed: tokens.accentPressed,
    fontFamily: FONT_UI,
    borderRadius: RADIUS_CONTROL,
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
      <div ref="shell" class="shell" :class="{ resizing: sider.dragging.value }">
        <repo-sidebar class="sider" :style="{ width: `${sider.size.value}px` }" />

        <div
          class="sider-divider"
          role="separator"
          aria-orientation="vertical"
          :aria-valuenow="Math.round(sider.size.value)"
          title="拖动调宽窄，双击恢复默认"
          @pointerdown="sider.onPointerDown"
          @dblclick="sider.reset"
        />

        <section class="main">
          <!--
            仓库名与页签：以前是 space-between 把页签甩到最右、标题缩成 12px 淡字，
            全屏最该被看见的上下文反而最弱。现在页签左对齐当主导航，标题当标题。
          -->
          <header class="bar">
            <n-tabs type="line" size="small" :value="activeTab" @update:value="go">
              <n-tab name="history">历史</n-tab>
              <!-- 文件页只读浏览，不碰工作区，所以 browse 仓库也摆出来 -->
              <n-tab name="files">文件</n-tab>
              <n-tab name="commit" :disabled="!commitTabEnabled">提交</n-tab>
            </n-tabs>
            <span class="headline" :title="headline">{{ headline }}</span>
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
              {{ writes.hasConflicts
                ? `左栏列出了 ${writes.conflicts.length} 个冲突文件，逐块取舍后可以续跑收尾。`
                : "没有待解决的冲突：要么已经全部标记完，可以直接续跑，要么去终端里处理完再回来。" }}
            </div>
            <n-space size="small" class="recheck">
              <n-button
                v-if="!writes.hasConflicts"
                size="small"
                type="warning"
                :loading="writes.busy"
                @click="writes.abort()"
              >
                一键退回
              </n-button>
              <n-button
                v-if="!writes.hasConflicts"
                size="small"
                type="primary"
                :loading="writes.busy"
                @click="writes.continueOperation(null)"
              >
                续跑收尾
              </n-button>
              <n-button
                size="small"
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

<style scoped>
.shell {
  display: flex;
  height: 100vh;
  background: var(--surface-app);
}

.shell.resizing {
  cursor: col-resize;
  user-select: none;
}

.sider {
  flex: none;
}

.sider-divider {
  position: relative;
  flex: none;
  width: 3px;
  margin: 0 2px;
  cursor: col-resize;
  border-radius: 2px;
  transition: background 120ms ease;
}

/* 热区扩到 15px，不占布局——理由见 CommitsView 里的同一条注释 */
.sider-divider::after {
  content: "";
  position: absolute;
  inset: 0 -6px;
}

.sider-divider:hover,
.shell.resizing .sider-divider {
  background: var(--accent);
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
  gap: 16px;
  padding: 6px 16px;
  background: var(--surface);
  border-bottom: 1px solid var(--border);
}

/*
 * 标题给足权重：仓库名、分支、待提交条数是每时每刻都要看的东西，
 * 原来压到 12px + 0.8 透明度和那排正文一样重，等于把它藏了。
 */
.headline {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-2);
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
