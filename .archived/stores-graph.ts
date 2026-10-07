import { computed, ref, shallowRef, triggerRef } from "vue";
import { defineStore } from "pinia";
import { GitTidyError } from "@/api/client";
import { fetchGraph, type GraphRow } from "@/api/graph";

/**
 * 提交历史的图列数据。
 *
 * 为什么不并进 commits store：列表那一层要守的是需求五第 9 条的"禁止全量拉取"，
 * 而泳道分配按定义要看整条历史（第 N 页长什么样取决于前面所有页），两边约束相反，
 * 混在一个 store 里迟早会为了图的方便把列表的规矩破掉。
 *
 * 行列下标是"当前这次 HEAD 遍历"里的位置，不是仓库全历史的绝对位置：提交、克隆之后
 * 老行可能整个从 HEAD 上消失，而 sha 不会重用，下标会撞车。所以每次清空都换一代。
 */
export const useGraphStore = defineStore("graph", () => {
  const index = shallowRef(new Map<string, GraphRow>());
  const generation = ref(0);
  /** 第一扇窗口是否已定案（拿到，或确认拿不到）。定案前列表先不画，避免列宽中途变化 */
  const ready = ref(false);
  /** 全历史最宽的泳道数，一次定死，翻页不会再变 */
  const widest = ref(0);
  const outstanding = ref(0);
  const error = ref<GitTidyError | null>(null);

  /** 还有图请求在飞。列表可以先画，晚到的行先空着图列 */
  const pending = computed(() => outstanding.value > 0);

  /** 整页统一列数。0 表示第一扇窗口还没定案，调用方该先不画列表 */
  const lanes = computed(() => (ready.value ? widest.value : 0));

  /** 渲染期读 index.value，所以整批 set 完要 triggerRef 通知一次 */
  function rowFor(sha: string): GraphRow | undefined {
    return index.value.get(sha);
  }

  /** 换仓库或重开列表：清空这一代，再读第一页。count 是列表本页实际给出的行数 */
  function open(repoId: number, count: number) {
    generation.value += 1;
    index.value = new Map();
    ready.value = false;
    widest.value = 0;
    error.value = null;
    return loadPage(repoId, 0, count);
  }

  /** skip/count 必须与同一次的 commit_list 取值一致，行才落在同一条水平线上 */
  async function loadPage(repoId: number, skip: number, count: number) {
    if (count <= 0) return;
    const myGeneration = generation.value;
    const isFirstWindow = skip === 0;
    outstanding.value += 1;
    try {
      const page = await fetchGraph(repoId, skip, count);
      if (myGeneration !== generation.value) return;
      for (const row of page.rows) index.value.set(row.sha, row);
      triggerRef(index);
      widest.value = Math.max(widest.value, page.lanes);
      if (isFirstWindow) ready.value = true;
    } catch (err) {
      // 换过仓库了就不该把上一个仓库的失败显示在新仓库的界面上
      if (myGeneration !== generation.value) return;
      error.value =
        err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
      // 失败也算定案：列表不该为了一个画不出来的图列一直等着
      if (isFirstWindow) ready.value = true;
    } finally {
      outstanding.value -= 1;
    }
  }

  return {
    index,
    generation,
    error,
    ready,
    pending,
    lanes,
    rowFor,
    open,
    loadPage,
  };
});
