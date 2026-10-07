import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { GitTidyError } from "@/api/client";
import { listCommits, type Commit } from "@/api/commit";

/** 与需求五第 9 条一致：一次 200 条，禁止全量拉取 */
const PAGE_SIZE = 200;

export const useCommitStore = defineStore("commits", () => {
  const commits = ref<Commit[]>([]);
  const total = ref(0);
  const loading = ref(false);
  const error = ref<GitTidyError | null>(null);

  const loadedAll = computed(() => commits.value.length >= total.value);

  async function fetchPage(repoId: number, skip: number, replace: boolean) {
    loading.value = true;
    error.value = null;
    try {
      const page = await listCommits(repoId, skip, PAGE_SIZE);
      commits.value = replace ? page.commits : commits.value.concat(page.commits);
      total.value = page.total;
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

  return { commits, total, loading, error, loadedAll, open, loadMore };
});
