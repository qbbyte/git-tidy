<script setup lang="ts">
import { computed, onErrorCaptured, onBeforeUnmount, onMounted, ref, watch, type ComponentPublicInstance } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  NAlert,
  NButton,
  NConfigProvider,
  NDropdown,
  NDialogProvider,
  NMessageProvider,
  NProgress,
  NSpace,
  NIcon,
  darkTheme as naiveDarkTheme,
  dateZhCN,
  zhCN,
  type GlobalThemeOverrides,
} from "naive-ui";
import RepoSidebar from "@/components/RepoSidebar.vue";
import { createDiscreteApi } from "naive-ui";

const { dialog } = createDiscreteApi(["dialog"]);
import SyncActions from "@/components/SyncActions.vue";
import SettingsModal from "@/components/SettingsModal.vue";
import AddRepoModal from "@/components/AddRepoModal.vue";
import NewBranchModal from "@/components/NewBranchModal.vue";
import { useReposStore } from "@/stores/repos";
import { useWriteStore } from "@/stores/write";
import { usePrefsStore as usePrefs } from "@/stores/prefs";
import { INTERRUPT_LABEL, type Interrupt } from "@/api/refs";
import { FONT_UI, RADIUS_CONTROL, tokens } from "@/styles/tokens";
import { useTheme, THEME_LABELS } from "@/composables/useTheme";
import type { ThemeMode } from "@/api/prefs";
import type { DropdownOption } from "naive-ui";
import {
  SHORTCUTS,
  TAB_SHORTCUTS,
  isTypingTarget,
  matches,
  type ShortcutContext,
} from "@/shortcuts";
import { runShell } from "@/shell";
import { checkForUpdate, updateCheckOnStartup, type CheckResult } from "@/api/update";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";

// 导航图标
import HistoryIcon from "@vicons/tabler/es/History";
import FileIcon from "@vicons/tabler/es/FileText";
import GitCommitIcon from "@vicons/tabler/es/GitCommit";
import ReportIcon from "@vicons/tabler/es/DeviceAnalytics";
import ChangelogIcon from "@vicons/tabler/es/ClipboardList";
import GitBranchIcon from "@vicons/tabler/es/GitBranch";
import BranchNavIcon from "@vicons/tabler/es/GitBranch";
import SunIcon from "@vicons/tabler/es/Sun";
import MoonIcon from "@vicons/tabler/es/Moon";
import DesktopIcon from "@vicons/tabler/es/DeviceDesktop";
import MinusIcon from "@vicons/tabler/es/Minus";
import MaxIcon from "@vicons/tabler/es/ArrowsDiagonal";
import RestoreIcon from "@vicons/tabler/es/ArrowsDiagonal2";
import CloseWinIcon from "@vicons/tabler/es/X";
const route = useRoute();
const router = useRouter();
const repos = useReposStore();
const writes = useWriteStore();
const settingsVisible = ref(false);
const addRepoVisible = ref(false);
const newBranchVisible = ref(false);

/** 自绘标题栏的窗口控制 */
const appWindow = getCurrentWindow();
const isMaximized = ref(false);

async function syncMaxState() {
  try { isMaximized.value = await appWindow.isMaximized(); } catch { /* 非 Tauri 环境 */ }
}

function winMinimize() { void appWindow.minimize(); }
function winToggleMax() { void appWindow.toggleMaximize(); void syncMaxState(); }
function winClose() { void appWindow.close(); }

/** 点击品牌标题：关闭当前仓库，回到空白首页 */
function onBrandClick() {
  if (!repos.current) return;
  dialog.warning({
    title: "关闭当前仓库？",
    content: "关闭后中间区域将不再显示内容，仓库记录仍保留在侧栏列表中。",
    positiveText: "关闭仓库",
    negativeText: "算了",
    onPositiveClick: () => {
      repos.closeCurrent();
      router.push({ name: "history" });
    },
  });
}

/** 主题模式：跟随 prefs 持久化，传给 useTheme 做解析 */
const themeMode = ref<ThemeMode>("system");
const { isDark, init: initTheme } = useTheme(themeMode);

/** 主题切换菜单选项 */
const themeOptions: DropdownOption[] = [
  { label: THEME_LABELS.system, key: "system" },
  { label: THEME_LABELS.light, key: "light" },
  { label: THEME_LABELS.dark, key: "dark" },
];

function onThemeSelect(key: string) {
  themeMode.value = key as ThemeMode;
  const prefs = usePrefs();
  void prefs.patch(() => ({ theme: key as ThemeMode }));
}

const themeLabel = computed(() => {
  const labels: Record<ThemeMode, string> = THEME_LABELS;
  return labels[themeMode.value];
});

const themeOverrides = computed<GlobalThemeOverrides>(() => {
  const base: GlobalThemeOverrides = {
    common: {
      primaryColor: tokens.accent,
      primaryColorHover: tokens.accentHover,
      primaryColorPressed: tokens.accentPressed,
      fontFamily: FONT_UI,
      borderRadius: RADIUS_CONTROL,
    },
  };
  if (isDark.value) {
    base.common!.primaryColor = "#3b8eff";
    base.common!.primaryColorHover = "#5ba2ff";
    base.common!.primaryColorPressed = "#2a7adf";
  }
  return base;
});

const activeTheme = computed(() => (isDark.value ? naiveDarkTheme : null));

watch(
  () => repos.canCommit,
  (value) => { if (!value) newBranchVisible.value = false; },
);


const INTERRUPT_EXIT: Record<Interrupt, string> = {
  none: "",
  merge: "解决冲突后用 git merge --continue 收尾，或 git merge --abort 放弃这次合并",
  rebase: "解决冲突后用 git rebase --continue 继续，或 git rebase --abort 回到变基之前",
  cherry_pick: "解决冲突后用 git cherry-pick --continue 继续，或 --abort 放弃这次摘取",
  revert: "解决冲突后用 git revert --continue 继续，或 --abort 放弃这次回滚",
};

const activeTab = computed(() => {
  if (route.name === "commit") return "commit";
  if (route.name === "files") return "files";
  if (route.name === "report") return "report";
  if (route.name === "changelog") return "changelog";
  if (route.name === "branches") return "branches";
  return "history";
});
const commitTabEnabled = computed(() => repos.canCommit);


/** 顶栏右半的仓库名 + 分支显示 */
const repoName = computed(() => repos.current?.name ?? null);
const branchName = computed(() => {
  const state = repos.repoState;
  if (repos.interrupted) {
    const label = INTERRUPT_LABEL[repos.interrupt];
    return state?.interruptBranch ? `${label}：${state.interruptBranch}` : label;
  }
  return state?.branch ?? repos.info?.branch ?? "游离 HEAD";
});

const interruptTitle = computed(() => INTERRUPT_LABEL[repos.interrupt]);
const interruptExit = computed(() => INTERRUPT_EXIT[repos.interrupt]);

function go(name: string | number) {
  router.push({ name: String(name) });
}

const shortcutContext: ShortcutContext = {
  refresh() { if (!runShell("refresh")) repos.refreshAll(); },
  focusSearch() { runShell("focusSearch"); },
  submit() { runShell("submit"); },
  switchRepo() { runShell("switchRepo"); },
  gotoTab(index: number) {
    const tab = TAB_SHORTCUTS[index];
    if (tab) go(TAB_ROUTE[tab.label] ?? "history");
  },
  openSettings() { settingsVisible.value = true; },
};

const TAB_ROUTE: Record<string, string> = {
  历史: "history", 文件: "files", 提交: "commit", 报告: "report", CHANGELOG: "changelog", 分支: "branches",
};

function onKeyDown(event: KeyboardEvent) {
  const typing = isTypingTarget(event.target);
  for (const shortcut of SHORTCUTS) {
    if (typing && !shortcut.keys.some((key) => /^F\d+$/.test(key))) continue;
    if (!shortcut.keys.some((key) => matches(event, key))) continue;
    event.preventDefault();
    shortcut.run(shortcutContext);
    return;
  }
  if (typing || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
  const index = TAB_SHORTCUTS.findIndex((tab) => tab.key === event.key);
  if (index >= 0) {
    event.preventDefault();
    shortcutContext.gotoTab(index);
  }
}

const updateNotice = ref<CheckResult | null>(null);

async function startupUpdateCheck() {
  try {
    if (!(await updateCheckOnStartup())) return;
    const result = await checkForUpdate();
    if (result.available) updateNotice.value = result;
  } catch { /* 静默 */ }
}

function dismissUpdate() { updateNotice.value = null; }

async function openUpdatePage() {
  const url = updateNotice.value?.latest?.url;
  if (url) await openUrl(url);
  dismissUpdate();
}

function closeError() { repos.error = null; }

onMounted(() => {
  window.addEventListener("keydown", onKeyDown);
  void syncMaxState();
  // 窗口最大化/还原时同步按钮图标
  appWindow.onResized(() => void syncMaxState());
  usePrefs().load();
  void repos.load();
  void startupUpdateCheck();
  initTheme();
});

// prefs 异步加载完成后，把存储的 theme 同步到 themeMode
const prefsStore = usePrefs();
watch(
  () => prefsStore.prefs?.theme,
  (stored) => { if (stored) themeMode.value = stored; },
);
onBeforeUnmount(() => window.removeEventListener("keydown", onKeyDown));

// 远程命令与改写的逐行进度：必须在命令发出之前挂上监听，
// 否则 fetch 的头几行进度可能在 promise 落地前就到了却无人接收
let cancelProgress: (() => void) | undefined;
onMounted(async () => {
  cancelProgress = await writes.watchProgress();
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeyDown);
  cancelProgress?.();
});

// 全局错误边界：捕获组件树里未被 try/catch 接住的异常，渲染兜底 UI 而非白屏。
// 已经被各 store 的 wrapError() 处理的错误不会走到这里——只有 Vue 渲染、
// 生命周期钩子或未经包裹的同步异常才会触发。
const fatalError = ref<string | null>(null);
onErrorCaptured((err: unknown, _instance: ComponentPublicInstance | null, info: string) => {
  console.error("[Git Tidy] 渲染异常：", err, info);
  fatalError.value = err instanceof Error ? err.message : String(err);
  return false;
});
</script>

<template>
  <n-config-provider :locale="zhCN" :date-locale="dateZhCN" :theme-overrides="themeOverrides" :theme="activeTheme">
    <n-dialog-provider>
      <n-message-provider>
        <!-- 全局错误兜底：errorCaptured 捕获异常后替换整个内容区 -->
        <div v-if="fatalError" class="fatal-error" role="alert" aria-live="assertive">
          <p class="fatal-error__title">页面渲染遇到意外错误</p>
          <p class="fatal-error__msg">{{ fatalError }}</p>
          <n-button size="small" @click="fatalError = null">重试</n-button>
        </div>
        <div class="app" v-else>
          <!-- 全宽顶栏：品牌 + 导航页签 + 仓库上下文 + 引用按钮 -->
          <header class="toolbar" data-tauri-drag-region>
            <!-- 品牌 -->
            <div class="brand">
              <span class="brand-name" title="点击关闭当前仓库" @click="onBrandClick">Git Tidy</span>
            </div>

            <!-- 导航页签（带图标） -->
            <nav class="nav" aria-label="主导航">
              <button
                class="nav-item"
                :class="{ active: activeTab === 'history' }"
                :aria-current="activeTab === 'history' ? 'page' : undefined"
                @click="go('history')"
              >
                <n-icon :component="HistoryIcon" size="16" />
                <span>历史</span>
              </button>
              <button
                class="nav-item"
                :class="{ active: activeTab === 'files' }"
                :aria-current="activeTab === 'files' ? 'page' : undefined"
                @click="go('files')"
              >
                <n-icon :component="FileIcon" size="16" />
                <span>文件</span>
              </button>
              <button
                class="nav-item"
                :class="{ active: activeTab === 'commit', disabled: !commitTabEnabled }"
                :disabled="!commitTabEnabled"
                :aria-current="activeTab === 'commit' ? 'page' : undefined"
                @click="go('commit')"
              >
                <n-icon :component="GitCommitIcon" size="16" />
                <span>提交</span>
              </button>
              <button
                class="nav-item"
                :class="{ active: activeTab === 'report' }"
                :aria-current="activeTab === 'report' ? 'page' : undefined"
                @click="go('report')"
              >
                <n-icon :component="ReportIcon" size="16" />
                <span>报告</span>
              </button>
              <button
                class="nav-item"
                :class="{ active: activeTab === 'changelog' }"
                :aria-current="activeTab === 'changelog' ? 'page' : undefined"
                @click="go('changelog')"
              >
                <n-icon :component="ChangelogIcon" size="16" />
                <span>CHANGELOG</span>
              </button>
              <button
                class="nav-item"
                :class="{ active: activeTab === 'branches' }"
                :aria-current="activeTab === 'branches' ? 'page' : undefined"
                @click="go('branches')"
              >
                <n-icon :component="BranchNavIcon" size="16" />
                <span>分支</span>
              </button>
            </nav>

            <!-- 仓库上下文 -->
            <div v-if="repoName" class="context">
              <span class="repo">{{ repoName }}</span>
              <n-icon :component="GitBranchIcon" size="14" class="branch-icon" />
              <span class="branch">{{ branchName }}</span>
              <span v-if="repos.workingFiles.length" class="changes">{{ repos.workingFiles.length }} 待提交</span>
            </div>
            <div v-else class="context empty">还没有打开仓库</div>

            <!-- 右侧动作 -->
            <n-dropdown :options="themeOptions" trigger="click" @select="onThemeSelect">
              <button class="theme-btn" :title="'主题：' + themeLabel" aria-label="切换主题">
                <n-icon :component="themeMode === 'system' ? DesktopIcon : themeMode === 'light' ? SunIcon : MoonIcon" size="16" />
              </button>
            </n-dropdown>
            <SyncActions />

            <!-- 自绘窗口控制按钮 -->
            <div class="win-controls">
              <button class="win-btn" title="最小化" aria-label="最小化" @click="winMinimize">
                <n-icon :component="MinusIcon" size="14" />
              </button>
              <button class="win-btn" :title="isMaximized ? '还原' : '最大化'" :aria-label="isMaximized ? '还原' : '最大化'" @click="winToggleMax">
                <n-icon :component="isMaximized ? RestoreIcon : MaxIcon" size="13" />
              </button>
              <button class="win-btn win-close" title="关闭" aria-label="关闭" @click="winClose">
                <n-icon :component="CloseWinIcon" size="14" />
              </button>
            </div>
          </header>

          <!-- 下方：侧栏 + 主区 -->
          <div class="body">
            <aside class="sider-wrap">
              <repo-sidebar @open-settings="settingsVisible = true" @open-add-repo="addRepoVisible = true" @open-new-branch="newBranchVisible = true" @open-branch-manager="$router.push({ name: 'branches' })" />
            </aside>

            <main class="main-area">
              <!-- 中断态提示 -->
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
                    ? `左栏列出了 ${writes.conflicts.length} 个冲突文件。`
                    : "没有待解决的冲突。" }}
                </div>
                <n-space size="small" style="margin-top: 6px">
                  <n-button v-if="!writes.hasConflicts" size="small" type="warning" :loading="writes.busy" @click="writes.abort()">一键退回</n-button>
                  <n-button v-if="!writes.hasConflicts" size="small" type="primary" :loading="writes.busy" @click="writes.continueOperation(null)">续跑收尾</n-button>
                  <n-button size="small" :loading="repos.loading" @click="repos.refreshAll()">重读一次</n-button>
                </n-space>
              </n-alert>

              <div v-if="repos.progress" class="banner">
                <div class="banner-title">下载中：{{ repos.progress.url }}</div>
                <n-progress type="line" :percentage="repos.progress.percent" :indeterminate="repos.progress.percent === 0" processing show-indicator />
                <div class="muted">{{ repos.progress.phase }}</div>
              </div>

              <div v-if="updateNotice" class="banner">
                <n-alert type="info" :closable="true" @close="dismissUpdate">
                  <div class="banner-title">发现新版本 v{{ updateNotice.latest?.version }}（当前 v{{ updateNotice.current }}）</div>
                  <n-space size="small">
                    <span class="muted">{{ updateNotice.latest?.name }}</span>
                    <n-button size="small" type="primary" @click="openUpdatePage">去看看</n-button>
                  </n-space>
                </n-alert>
              </div>

              <n-alert v-if="repos.error" class="banner" type="error" :title="repos.error.message" closable @close="closeError">
                <div>错误码：{{ repos.error.code }}</div>
                <pre v-if="repos.error.detail" class="raw-output">{{ repos.error.detail }}</pre>
              </n-alert>

              <div class="content">
                <router-view />
              </div>
            </main>
          </div>

        </div>

        <settings-modal v-model:show="settingsVisible" />
        <add-repo-modal v-model:show="addRepoVisible" />
        <new-branch-modal v-model:show="newBranchVisible" />
      </n-message-provider>
    </n-dialog-provider>
  </n-config-provider>
</template>

<style scoped>
.app {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: var(--surface-app);
  color: var(--text-1);
}

/* ---- 全宽顶栏 ---- */
.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
  height: 40px;
  padding: 0 12px;
  background: var(--surface);
  border-bottom: 1px solid var(--border);
  z-index: 10;
}

/* 品牌 */
.brand {
  display: flex;
  align-items: center;
  flex: none;
  gap: 6px;
}
.theme-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
  width: 28px;
  height: 28px;
  border: 0;
  border-radius: 6px;
  background: none;
  color: var(--text-2);
  cursor: pointer;
  transition: all 120ms ease;
}
.theme-btn:hover {
  background: var(--surface-hover);
  color: var(--text-1);
}

/* 自绘窗口控制按钮：Windows 风格的平铺按钮 */
.win-controls {
  display: flex;
  align-items: center;
  flex: none;
  margin-left: 4px;
}

.win-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 28px;
  border: 0;
  background: none;
  color: var(--text-2);
  cursor: pointer;
  transition: background 80ms ease, color 80ms ease;
}

.win-btn:hover {
  background: var(--surface-hover);
  color: var(--text-1);
}

.win-btn:active {
  background: var(--surface-sunken);
}

/* 关闭按钮 hover 变红：Windows 的通行做法 */
.win-close:hover {
  background: #e81123;
  color: #fff;
}

.win-close:active {
  background: #c50f1f;
  color: #fff;
}
.brand-name {
  font-size: 14px;
  font-weight: 700;
  color: var(--accent);
  white-space: nowrap;
}

/* 导航页签 */
.nav {
  display: flex;
  align-items: center;
  gap: 2px;
  flex: none;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 4px 10px;
  border: 0;
  border-radius: 6px;
  background: none;
  color: var(--text-2);
  font-family: inherit;
  font-size: 13px;
  cursor: pointer;
  transition: all 120ms ease;
}

.nav-item:hover {
  background: var(--surface-hover);
  color: var(--text-1);
}

.nav-item.active {
  color: var(--accent);
  font-weight: 600;
}

.nav-item.disabled,
.nav-item:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* 仓库上下文 */
.context {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
  padding-left: 16px;
  margin-left: 8px;
  border-left: 1px solid var(--border);
  font-size: 12px;
  color: var(--text-2);
}

.context .repo {
  font-weight: 600;
  color: var(--text-1);
  white-space: nowrap;
}

.context .branch-icon {
  opacity: 0.5;
}

.context .branch {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.context .changes {
  flex: none;
  white-space: nowrap;
  padding: 0 6px;
  border-radius: 9px;
  font-size: 11px;
  line-height: 16px;
  color: var(--warn-text);
  background: var(--warn-bg);
}

.context.empty {
  opacity: 0.5;
  font-style: italic;
}

/* ---- 窄窗口自适应 ---- */

/* 中等窗口：藏掉导航文字，只留图标，腾出空间给仓库上下文 */
@media (max-width: 960px) {
  .nav-item span {
    display: none;
  }
}

/* 更窄：仓库上下文里的"待提交"计数也藏掉，把空间留给仓库名 */
@media (max-width: 760px) {
  .context .changes {
    display: none;
  }
}

/* 下方布局 */
.body {
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.sider-wrap {
  flex: none;
  width: 280px;
  border-right: 1px solid var(--border);
  overflow: hidden;
}

.main-area {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* 提示横幅 */
.banner {
  margin: 12px 16px 0;
}
.banner.interrupt { flex: none; }
.banner-title { font-size: 12px; margin-bottom: 6px; }

.content {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 12px 16px 16px;
}

.muted { font-size: 11px; opacity: 0.7; }
.raw-output { margin: 8px 0 0; white-space: pre-wrap; font-size: 12px; opacity: 0.8; }

.fatal-error {
  max-width: 480px;
  margin: 80px auto;
  padding: 24px;
  text-align: center;
}
.fatal-error__title { font-size: 16px; font-weight: 600; margin: 0 0 8px; }
.fatal-error__msg { font-size: 13px; opacity: 0.7; margin: 0 0 16px; word-break: break-word; }
</style>
