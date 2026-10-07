import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { GitTidyError } from "@/api/client";
import {
  addLocalRepo,
  addRemoteRepo,
  listRepos,
  materializeRepo,
  onRepoProgress,
  refreshRepo,
  removeRepo,
  renameRepo,
  type CloneProgress,
  type Repo,
  type RepoInfo,
} from "@/api/repo";
import {
  stageFiles,
  unstageFiles,
  worktreeStatus,
  type WorkingFile,
} from "@/api/status";

/**
 * 多仓库注册表。所有 git 读取都用仓库 id 触发，界面同时只"打开"一个仓库，
 * 但列表里的任何一个都能一键切过去。
 */
export const useReposStore = defineStore("repos", () => {
  const repos = ref<Repo[]>([]);
  const currentId = ref<number | null>(null);
  const info = ref<RepoInfo | null>(null);
  const workingFiles = ref<WorkingFile[]>([]);
  const loading = ref(false);
  /** 暂存/取消暂存单独一个开关：它只该锁住勾选区，不该让侧栏的按钮一起变灰 */
  const staging = ref(false);
  const error = ref<GitTidyError | null>(null);
  /** 正在进行的克隆/补齐进度；null 表示没有长任务在跑 */
  const progress = ref<CloneProgress | null>(null);

  const current = computed(() => repos.value.find((repo) => repo.id === currentId.value) ?? null);
  const canCommit = computed(() => current.value?.kind === "worktree" && info.value !== null);
  const stagedFiles = computed(() => workingFiles.value.filter((file) => file.staged));
  const changedFiles = computed(() =>
    workingFiles.value.filter((file) => !file.staged || file.worktreeStatus !== " "),
  );

  function clearOpened() {
    info.value = null;
    workingFiles.value = [];
  }

  async function run<T>(task: () => Promise<T>): Promise<T | null> {
    loading.value = true;
    error.value = null;
    try {
      return await task();
    } catch (err) {
      error.value =
        err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
      return null;
    } finally {
      loading.value = false;
    }
  }

  async function load() {
    const listed = await run(listRepos);
    if (listed) repos.value = listed;
    if (currentId.value !== null && !repos.value.some((repo) => repo.id === currentId.value)) {
      currentId.value = null;
      clearOpened();
    }
  }

  /**
   * 长任务期间订阅进度。监听必须在命令发出之前挂上——克隆的头几行进度
   * 可能在命令 promise 落地之前就到达了。
   */
  async function withProgress<T>(url: string, task: () => Promise<T>): Promise<T | null> {
    progress.value = { url, phase: "准备中", percent: 0 };
    const unlisten = await onRepoProgress((event) => {
      if (event.url === url) progress.value = event;
    });
    try {
      return await run(task);
    } finally {
      unlisten();
      progress.value = null;
    }
  }

  async function add(path: string) {
    const repo = await run(() => addLocalRepo(path));
    if (!repo) return;
    await load();
    await select(repo.id);
  }

  /** 贴地址添加：走的是只读浏览，进度按地址认领 */
  async function addByUrl(url: string) {
    const repo = await withProgress(url, () => addRemoteRepo(url));
    if (!repo) return;
    await load();
    await select(repo.id);
  }

  /** 「克隆」：补齐成功后重新打开这个仓库，这时候才有工作区状态可读 */
  async function materialize(id: number) {
    const repo = repos.value.find((item) => item.id === id);
    const updated = await withProgress(repo?.remoteUrl ?? "", () => materializeRepo(id));
    if (!updated) return;
    await load();
    if (currentId.value === id) await select(id);
  }

  async function select(id: number) {
    currentId.value = id;
    // 先清空：读回来之前如果还挂着上一个仓库的文件列表，界面就会拿它去操作错的仓库
    clearOpened();
    const fetched = await run(() => refreshRepo(id));
    // 取回来的可能已经是另一个仓库的了（用户又点了一次），比对后再写
    if (!fetched || currentId.value !== id) return;
    info.value = fetched;
    await refreshStatus();
  }

  /** 只重读待提交文件。提交完、暂存完都走这里，不必把仓库信息再探测一遍。 */
  async function refreshStatus() {
    const id = currentId.value;
    if (id === null || current.value?.kind !== "worktree") return;
    const files = await run(() => worktreeStatus(id));
    if (files && currentId.value === id) workingFiles.value = files;
  }

  /** 提交之后走这里：HEAD 和待提交文件都变了，但不先清空，免得勾选区闪一下全没。 */
  async function refreshAll() {
    const id = currentId.value;
    if (id === null) return;
    const fetched = await run(() => refreshRepo(id));
    if (fetched && currentId.value === id) info.value = fetched;
    await refreshStatus();
  }

  /** 勾选一个文件：git 是唯一裁判，所以拿它返回的整份列表覆盖本地，不做乐观更新 */
  async function stage(paths: string[]) {
    const done = await runStaging(() => stageFiles(requiredId(), paths));
    if (done) workingFiles.value = done;
  }

  async function unstage(paths: string[]) {
    const done = await runStaging(() => unstageFiles(requiredId(), paths));
    if (done) workingFiles.value = done;
  }

  async function runStaging(task: () => Promise<WorkingFile[]>): Promise<WorkingFile[] | null> {
    staging.value = true;
    error.value = null;
    const id = currentId.value;
    try {
      const files = await task();
      // 期间切了仓库就把结果丢掉，否则会写进另一个仓库的勾选区
      return currentId.value === id ? files : null;
    } catch (err) {
      error.value =
        err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
      return null;
    } finally {
      staging.value = false;
    }
  }

  function requiredId() {
    const id = currentId.value;
    if (id === null) throw new GitTidyError("no_repo", "还没有打开仓库");
    return id;
  }

  async function rename(id: number, name: string) {
    const updated = await run(() => renameRepo(id, name));
    if (!updated) return;
    // 列表按名字排序，改完直接重取，不在本地猜位置
    await load();
  }

  /** 只移除记录，磁盘上的仓库文件不动（§6.1） */
  async function remove(id: number) {
    const done = await run(() => removeRepo(id).then(() => true));
    if (!done) return;
    if (currentId.value === id) {
      currentId.value = null;
      clearOpened();
    }
    await load();
  }

  return {
    repos,
    current,
    currentId,
    info,
    workingFiles,
    stagedFiles,
    changedFiles,
    canCommit,
    loading,
    staging,
    error,
    progress,
    load,
    add,
    addByUrl,
    materialize,
    select,
    refreshStatus,
    refreshAll,
    stage,
    unstage,
    rename,
    remove,
  };
});
