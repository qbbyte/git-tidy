import { computed, onBeforeUnmount, ref, watch, type Ref } from "vue";
import type { ThemeMode } from "@/api/prefs";
import { toggleDarkTokens } from "@/styles/tokens";

/**
 * 主题管理：三档（跟随系统 / 月之亮面 / 月之暗面）。
 *
 * - `mode`：用户选择的主题偏好（由 prefs 持久化）
 * - `isDark`：最终解析出来是不是暗色——App.vue 拿它决定 Naive UI 的 theme
 *
 * 「跟随系统」时监听 `prefers-color-scheme`，系统切了暗色就跟着切。
 * 切换 CSS 变量在 `toggleDarkTokens()` 里做，Naive UI 的 darkTheme 在 App.vue 里给。
 */
export function useTheme(mode: Ref<ThemeMode>) {
  const systemDark = ref(false);

  function detectSystem() {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    systemDark.value = mq.matches;
    return mq;
  }

  const media = detectSystem();

  function onSystemChange(event: MediaQueryListEvent) {
    systemDark.value = event.matches;
  }

  const isDark = computed(() => {
    if (mode.value === "dark") return true;
    if (mode.value === "light") return false;
    return systemDark.value;
  });

  const effectiveTheme = computed<"light" | "dark">(() => (isDark.value ? "dark" : "light"));

  let started = false;

  function start() {
    if (started) return;
    started = true;
    media.addEventListener("change", onSystemChange);
  }

  function stop() {
    if (started) media.removeEventListener("change", onSystemChange);
    started = false;
  }

  function init() {
    start();
    toggleDarkTokens(isDark.value);
  }

  watch(isDark, (dark) => toggleDarkTokens(dark));
  onBeforeUnmount(stop);

  return { isDark, effectiveTheme, init };
}

/** 三个档位的展示文案 */
export const THEME_LABELS: Record<ThemeMode, string> = {
  system: "跟随系统",
  light: "月之亮面",
  dark: "月之暗面",
};
