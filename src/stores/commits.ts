import { computed, ref, shallowRef, triggerRef } from "vue";
import { defineStore } from "pinia";
import { GitTidyError } from "@/api/client";
import {
  fetchGraph,
  listCommits,
  type Commit,
  type GraphRow,
} from "@/api/commit";

/** 与需求五第 9 条一致：一次 200 条，禁止全量拉取 */
const PAGE_SIZE = 200;

export const useCommitStore = defineStore("commits", () => {
  const commits = ref<Commit[]>([]);
  const total = ref(0);
  const loading = ref(false);
  const error = ref<GitTidyError | null>(null);

  /**
   * 图列状态。这里的行号是"当前这次 HEAD 遍历"里的位置，不是仓库全历史的绝对位置：
   * 提交或克隆补全之后老行可能整个从 HEAD 上消失（被砍掉、或不再可达），而 sha 不会重用，
   * 旧行删掉后新行会从 0 开始重排，下标照样撞车。所以每次重新载入列表就整批作废，
   * 作废的判据是 generation 计数，不是 sha。
   */
  const graphRows = shallowRef(new Map<string, GraphRow>());
  const graphGeneration = ref(0);
  const graphLanes = ref(0);
  const graphReady = ref(false);
  const graphError = ref<GitTidyError | null>(null);
  const graphOutstanding = ref(0);

  const loadedAll = computed(() => commits.value.length >= total.value);
  const graphLoading = computed(() => graphOutstanding.value > 0);

  /** 渲染期读 graphRows.value，所以整批 set 完要 triggerRef 通知一次 */
  function rowFor(sha: string): GraphRow | undefined {
    return graphRows.value.get(sha);
  }

  async function fetchPage(repoId: number, skip: number, replace: boolean) {
    loading.value = true;
    error.value = null;
    if (replace) {
      graphRows.value = new Map();
      graphGeneration.value += 1;
      graphLanes.value = 0;
      graphReady.value = false;
      graphError.value = null;
    }
    try {
      const page = await listCommits(repoId, skip, PAGE_SIZE);
      commits.value = replace ? page.commits : commits.value.concat(page.commits);
      total.value = page.total;
      // 图列跟列表要同一扇窗口，所以放在列表落地之后发：本页实际有几行就取几行。
      // 这一步不 await：图那一层要读的比这一页多得多，让它自己到齐。
      loadGraphWindow(repoId, skip, page.commits.length);
    } catch (err) {
      error.value =
        err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
    } finally {
      loading.value = false;
    }
  }

  /** 换仓库时先清空，否则新仓库的第一页会追加到上一个仓库的记录后面 */
  function open(repoId: number) {
    commits.value = [];
    total.value = 0;
    return fetchPage(repoId, 0, true);
  }

  function loadMore(repoId: number) {
    if (loading.value || loadedAll.value) return Promise.resolve();
    return fetchPage(repoId, commits.value.length, false);
  }

  async function loadGraphWindow(repoId: number, skip: number, count: number) {
    if (count <= 0) return;
    const myGeneration = graphGeneration.value;
    graphOutstanding.value += 1;
    try {
      const window = await fetchGraph(repoId, skip, count);
      if (myGeneration !== graphGeneration.value) return;
      for (const row of window.rows) graphRows.value.set(row.sha, row);
      triggerRef(graphRows);
      graphLanes.value = Math.max(graphLanes.value, window.lanes);
      // 列宽只在第一扇窗口定案后才对外给：给早了，后面的行会整体横移
      if (skip === 0) graphReady.value = true;
    } catch (err) {
      if (myGeneration !== graphGeneration.value) return;
      graphError.value =
        err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
      // 失败也算定案：列表不该为了一个画不出来的图列一直等着
      if (skip === 0) graphReady.value = true;
    } finally {
      graphOutstanding.value -= 1;
    }
  }

  return {
    commits,
    total,
    loading,
    error,
    loadedAll,
    graphLanes,
    graphReady,
    graphLoading,
    graphError,
    rowFor,
    open,
    loadMore,
  };
});
