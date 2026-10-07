import { defineStore } from "pinia";
import { ref } from "vue";
import { GitTidyError } from "@/api/client";
import { probeRepo, type RepoInfo } from "@/api/repo";

export const useRepoStore = defineStore("repo", () => {
  const current = ref<RepoInfo | null>(null);
  const loading = ref(false);
  const error = ref<GitTidyError | null>(null);

  async function probe(path: string) {
    loading.value = true;
    error.value = null;
    try {
      current.value = await probeRepo(path);
    } catch (err) {
      current.value = null;
      error.value =
        err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
    } finally {
      loading.value = false;
    }
  }

  return { current, loading, error, probe };
});
