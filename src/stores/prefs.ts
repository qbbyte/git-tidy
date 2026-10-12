import { ref } from "vue";
import { defineStore } from "pinia";
import { GitTidyError, wrapError } from "@/api/client";
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

  async function load() {
    loading.value = true;
    error.value = null;
    try {
      const [loaded, file] = await Promise.all([prefsGet(), prefsPath()]);
      prefs.value = loaded;
      path.value = file;
    } catch (err) {
      error.value = wrapError(err);
    } finally {
      loading.value = false;
    }
  }

  /**
   * 落盘队列。**这是 `patch` 必须串行的理由**：`prefsUpdate` 在 Rust 侧是整份替换
   * （`Prefs::replace`），两次改设置落在同一趟 IPC 里时，后一次若拿调用点当时读到的
   * `prefs.value` 去拼整份，前一次的改动就被静默覆盖掉了。
   */
  let chain: Promise<void> = Promise.resolve();

  /**
   * 改一项并立刻落盘。返回存进去的真值——窗口尺寸会被夹取，界面要显示它。
   *
   * `change` 收的是**函数**而不是一个对象：拼接要发生在"上一笔写完之后"。
   * 调用点（比如设置页的四个开关）没法知道前一秒那一笔写完了没有，让它去读
   * `prefs.value` 就等于让它赌一次时序。
   */
  function patch(change: (current: Preferences) => Partial<Preferences>) {
    const task = chain.then(async () => {
      if (!prefs.value) await load();
      const current = prefs.value;
      if (!current) return;
      error.value = null;
      try {
        prefs.value = await prefsUpdate({ ...current, ...change(current) });
      } catch (err) {
        error.value = wrapError(err);
      }
    });
    // 出错也往下走：`task` 自己把异常吞成了 `error`，链不能因此断掉
    chain = task;
    return task;
  }

  async function reset() {
    error.value = null;
    try {
      prefs.value = await prefsReset();
    } catch (err) {
      error.value = wrapError(err);
    }
  }

  /** 列显示的便捷读法。没读到偏好时全开：省的是眼睛，不是数据 */
  function column(name: keyof Preferences["columns"]): boolean {
    return prefs.value?.columns[name] ?? true;
  }

  return { prefs, path, loading, error, load, patch, reset, column };
});