import { ref } from "vue";
import { defineStore } from "pinia";
import { GitTidyError } from "@/api/client";
import {
  prefsGet,
  prefsPath,
  prefsReset,
  prefsUpdate,
  type Preferences,
} from "@/api/prefs";

/**
 * 个人偏好（需求 6.7 第二层）。
 *
 * 启动时就读好，之后各处（提交列表的列显示、Git Flow 前缀、拉取策略）都从这里取，
 * 不各自去调 `prefs_get`——那样同一项设置在两个界面里可能显示得不一样。
 */
export const usePrefsStore = defineStore("prefs", () => {
  const prefs = ref<Preferences | null>(null);
  const path = ref("");
  const loading = ref(false);
  const error = ref<GitTidyError | null>(null);

  function wrap(err: unknown) {
    return err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
  }

  async function load() {
    loading.value = true;
    error.value = null;
    try {
      const [loaded, file] = await Promise.all([prefsGet(), prefsPath()]);
      prefs.value = loaded;
      path.value = file;
    } catch (err) {
      error.value = wrap(err);
    } finally {
      loading.value = false;
    }
  }

  /** 改一项并立刻落盘。返回存进去的真值——窗口尺寸会被夹取，界面要显示它。 */
  async function patch(change: Partial<Preferences>) {
    if (!prefs.value) await load();
    if (!prefs.value) return;
    error.value = null;
    try {
      prefs.value = await prefsUpdate({ ...prefs.value, ...change });
    } catch (err) {
      error.value = wrap(err);
    }
  }

  async function reset() {
    error.value = null;
    try {
      prefs.value = await prefsReset();
    } catch (err) {
      error.value = wrap(err);
    }
  }

  /** 列显示的便捷读法。没读到偏好时全开：省的是眼睛，不是数据 */
  function column(name: keyof Preferences["columns"]): boolean {
    return prefs.value?.columns[name] ?? true;
  }

  return { prefs, path, loading, error, load, patch, reset, column };
});