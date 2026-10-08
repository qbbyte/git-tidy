import { listen } from "@tauri-apps/api/event";
import { computed, ref, shallowRef } from "vue";
import { defineStore } from "pinia";
import { GitTidyError } from "@/api/client";
import {
  abortOperation,
  branchCreate,
  branchDelete,
  branchDeletable,
  branchRename,
  branchSwitch,
  cherryPick,
  conflictAcceptDeletion,
  conflictContinue,
  conflictList,
  conflictResolve,
  remoteDeleteBranch,
  remoteFetch,
  remotePull,
  remotePush,
  resetTo,
  revertCommit,
  stashApply,
  stashBranch,
  stashDrop,
  stashList,
  stashPop,
  stashPush,
  SYNC_PROGRESS_EVENT,
  tagCreate,
  tagDelete,
  upstreamSet,
  writeBackups,
  writeJournal,
  writeUndo,
  type Backup,
  type Conflict,
  type Deletable,
  type Outcome,
  type Resolution,
  type StashEntry,
  type SyncProgress,
  type WriteOpEntry,
} from "@/api/write";
import { useReposStore } from "@/stores/repos";

function wrap(err: unknown): GitTidyError {
  return err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
}

/**
 * 写操作的状态层（M2）。
 *
 * 三件事都在这里收口，别处不直接调 IPC：
 * - 一次只跑一个写操作（`busy`）：并发点两次"删分支"的结果没人能预测；
 * - 每次写完重读仓库信息、引用与待提交文件（HEAD 与工作区都可能变了）；
 * - 写操作日志与撤销跟写操作一起刷新——"撤销上一步"必须在操作之后立刻可用。
 */
export const useWriteStore = defineStore("write", () => {
  const repos = useReposStore();

  const busy = ref(false);
  const error = ref<GitTidyError | null>(null);
  /** 最近一次写操作的还原 ref。界面上要一直显示：这是"怎么回去"的答案 */
  const lastBackupRef = ref<string | null>(null);

  const stashes = shallowRef<StashEntry[]>([]);
  /** 未合并文件。空数组 = 没有冲突，不是"没读"（§7.13） */
  const conflicts = shallowRef<Conflict[]>([]);
  const journal = shallowRef<WriteOpEntry[]>([]);
  const backups = shallowRef<Backup[]>([]);
  /** 远程命令的逐行输出，最近几条。界面上当进度条文本显示 */
  const syncLines = ref<string[]>([]);
  const syncAction = ref<string | null>(null);
  /** 删分支前查到的"会丢多少" */
  const deletable = shallowRef<Record<string, Deletable>>({});

  /** 中断态下能做的事只有一件：退回（§3 的 M2 边界） */
  const canWrite = computed(() => repos.canCommit && !repos.interrupted);

  /** 有没有没解决的冲突。界面据此把「续跑」以外的写入口全禁掉 */
  const hasConflicts = computed(() => conflicts.value.length > 0);

  /** 有没有可撤销的写操作：最后一条必须是成功的 */
  const canUndo = computed(
    () => journal.value.length > 0 && journal.value[0].status === "ok",
  );

  async function run<T>(task: () => Promise<T>): Promise<T | null> {
    busy.value = true;
    error.value = null;
    try {
      const done = await task();
      await refreshAfterWrite();
      return done;
    } catch (err) {
      error.value = wrap(err);
      return null;
    } finally {
      busy.value = false;
    }
  }

  /** 写操作之后：仓库信息、引用、中断态、日志都要跟上，否则界面摆的是过期状态 */
  async function refreshAfterWrite() {
    await repos.refreshAll();
    await Promise.all([loadJournal(), loadStashes(), loadBackups(), loadConflicts()]);
  }

  /** 乐观并发的判据：界面加载时的 HEAD。空仓库与没加载过时传 null（不校验）。 */
  function expectedHead(): string | null {
    return repos.info?.headCommit ?? null;
  }

  async function loadStashes() {
    const id = repos.currentId;
    if (id === null || !repos.canCommit) {
      stashes.value = [];
      return;
    }
    try {
      stashes.value = await stashList(id);
    } catch {
      // stash 读不到不是致命的：仓库可能没有 stash，也可能正在中断态里
      stashes.value = [];
    }
  }

  /** 读未合并文件。这条只在中断态里有东西，读不到就当没有——
   *  正常仓库里 `ls-files -u` 是空的，treeless 仓库上则是这条命令直接失败 */
  async function loadConflicts() {
    const id = repos.currentId;
    if (id === null || !repos.canCommit) {
      conflicts.value = [];
      return;
    }
    try {
      conflicts.value = await conflictList(id);
    } catch {
      conflicts.value = [];
    }
  }

  async function loadJournal() {
    const id = repos.currentId;
    if (id === null) {
      journal.value = [];
      return;
    }
    try {
      journal.value = await writeJournal(id);
    } catch {
      journal.value = [];
    }
  }

  async function loadBackups() {
    const id = repos.currentId;
    if (id === null || !repos.canCommit) {
      backups.value = [];
      return;
    }
    try {
      backups.value = await writeBackups(id);
    } catch {
      backups.value = [];
    }
  }

  /** 换仓库时把两个列表都清掉：上一个仓库的 stash 与日志挂在界面上会误导 */
  function clearForRepo() {
    stashes.value = [];
    journal.value = [];
    backups.value = [];
    conflicts.value = [];
    deletable.value = {};
    lastBackupRef.value = null;
    syncLines.value = [];
    syncAction.value = null;
  }

  async function loadAll() {
    await Promise.all([loadStashes(), loadJournal(), loadBackups(), loadConflicts()]);
  }

  /** 记住一次写操作的还原 ref */
  function remember(outcome: Outcome | null) {
    lastBackupRef.value = outcome?.backupRef ?? null;
  }

  function requireId(): number | null {
    const id = repos.currentId;
    if (id === null) {
      error.value = new GitTidyError("no_repo", "还没有打开仓库");
      return null;
    }
    return id;
  }

  // ---------------------------------------------------------------- 分支与标签

  async function createBranch(name: string, start: string | null, switchToIt: boolean) {
    const id = requireId();
    if (id === null) return null;
    const outcome = await run(() => branchCreate(id, name, start, switchToIt, expectedHead()));
    remember(outcome);
    return outcome;
  }

  async function switchBranch(name: string, create: boolean, start: string | null = null) {
    const id = requireId();
    if (id === null) return null;
    const outcome = await run(() => branchSwitch(id, name, create, start, expectedHead()));
    remember(outcome);
    return outcome;
  }

  async function renameBranch(from: string, to: string) {
    const id = requireId();
    if (id === null) return null;
    return run(() => branchRename(id, from, to));
  }

  /** 删分支。先查未合并提交数，界面据此决定要不要 `-D` 二次确认（§7.10）。 */
  async function deleteBranch(name: string, force: boolean) {
    const id = requireId();
    if (id === null) return null;
    const outcome = await run(() => branchDelete(id, name, force));
    if (outcome !== null) delete deletable.value[name];
    return outcome;
  }

  async function probeDeletable(name: string) {
    const id = requireId();
    if (id === null) return null;
    try {
      const probe = await branchDeletable(id, name);
      deletable.value = { ...deletable.value, [name]: probe };
      return probe;
    } catch (err) {
      error.value = wrap(err);
      return null;
    }
  }

  async function setUpstream(name: string, upstream: string | null) {
    const id = requireId();
    if (id === null) return null;
    return run(() => upstreamSet(id, name, upstream));
  }

  async function createTag(name: string, message: string | null) {
    const id = requireId();
    if (id === null) return null;
    return run(() => tagCreate(id, name, message));
  }

  async function deleteTag(name: string) {
    const id = requireId();
    if (id === null) return null;
    return run(() => tagDelete(id, name));
  }

  // ---------------------------------------------------------------- stash

  async function pushStash(paths: string[] | null, includeUntracked: boolean, message: string | null) {
    const id = requireId();
    if (id === null) return null;
    return run(() => stashPush(id, paths, includeUntracked, message));
  }

  async function applyStash(reference: string) {
    const id = requireId();
    if (id === null) return null;
    return run(() => stashApply(id, reference));
  }

  async function popStash(reference: string) {
    const id = requireId();
    if (id === null) return null;
    const outcome = await run(() => stashPop(id, reference));
    // 冲突时 Rust 侧回的是中断态：stash 条目仍在，日志会记成"中断待处理"
    return outcome;
  }

  async function dropStash(reference: string) {
    const id = requireId();
    if (id === null) return null;
    return run(() => stashDrop(id, reference));
  }

  async function branchFromStash(reference: string, name: string) {
    const id = requireId();
    if (id === null) return null;
    return run(() => stashBranch(id, reference, name));
  }

  // ---------------------------------------------------------------- 摘取 / 回滚 / 复位

  async function pick(sha: string, recordSource = true) {
    const id = requireId();
    if (id === null) return null;
    return run(() => cherryPick(id, [sha], recordSource, expectedHead()));
  }

  /** 回滚。合并提交必须显式给主线号（`mainline`），界面不给默认值（§7.11） */
  async function revert(sha: string, mainline: number | null) {
    const id = requireId();
    if (id === null) return null;
    return run(() => revertCommit(id, sha, mainline, expectedHead()));
  }

  async function reset(mode: "soft" | "mixed" | "hard", target: string) {
    const id = requireId();
    if (id === null) return null;
    return run(() => resetTo(id, mode, target, expectedHead()));
  }

  /** 中断态的一键退回。M2 只给这一条，不给逐块解决（§3） */
  async function abort() {
    const id = requireId();
    if (id === null) return null;
    return run(() => abortOperation(id));
  }

  // ---------------------------------------------------------------- 冲突解决器（§7.13）

  /**
   * 解决一个冲突文件。
   *
   * `apply` 传 null 表示「接受删除」：改删/删改里有一方已经把这个文件删了，
   * 界面单独给一个按钮，走的是另一条命令而不是一个假的解决方式。
   */
  async function resolveConflict(path: string, apply: Resolution | null) {
    const id = requireId();
    if (id === null) return null;
    const outcome = await run(() =>
      apply === null
        ? conflictAcceptDeletion(id, path)
        : conflictResolve(id, path, apply),
    );
    remember(outcome);
    return outcome;
  }

  /**
   * 续跑收尾。
   *
   * `finished` 为 false 不是失败：下一个提交也冲突了，中断态还在，界面要接着显示剩余卡片。
   */
  async function continueOperation(message: string | null = null) {
    const id = requireId();
    if (id === null) return null;
    const outcome = await run(() => conflictContinue(id, message));
    remember(outcome);
    return outcome;
  }

  // ---------------------------------------------------------------- 远程

  async function fetch(remote: string | null = null) {
    const id = requireId();
    if (id === null) return null;
    syncLines.value = [];
    syncAction.value = "fetch";
    try {
      return await remoteFetch(id, remote);
    } catch (err) {
      error.value = wrap(err);
      return null;
    } finally {
      syncAction.value = null;
      await repos.refreshRefs();
    }
  }

  async function pull(
    remote: string | null = null,
    strategy: "ff_only" | "rebase" = "ff_only",
  ) {
    const id = requireId();
    if (id === null) return null;
    syncLines.value = [];
    syncAction.value = "pull";
    try {
      const outcome = await run(() => remotePull(id, remote, strategy, expectedHead()));
      return outcome;
    } finally {
      syncAction.value = null;
    }
  }

  async function push(remote: string, branch: string, setUpstream: boolean, forceWithLease = false) {
    const id = requireId();
    if (id === null) return null;
    try {
      const report = await remotePush(id, remote, branch, setUpstream, forceWithLease);
      await repos.refreshRefs();
      return report;
    } catch (err) {
      error.value = wrap(err);
      return null;
    }
  }

  /** 删远程分支。属于远程写，确认强度按 §7.12：界面要求手输分支名 */
  async function deleteRemoteBranch(remote: string, branch: string) {
    const id = requireId();
    if (id === null) return null;
    try {
      const report = await remoteDeleteBranch(id, remote, branch);
      await repos.refreshRefs();
      return report;
    } catch (err) {
      error.value = wrap(err);
      return null;
    }
  }

  /**
   * 订阅远程命令的进度。返回取消订阅函数。
   *
   * 必须在命令发出**之前**挂上：fetch 的头几行进度可能在 promise 落地前就到了。
   */
  async function watchProgress() {
    return listen<SyncProgress>(SYNC_PROGRESS_EVENT, (event) => {
      const payload = event.payload;
      if (payload.repoId !== repos.currentId) return;
      syncAction.value = payload.action;
      syncLines.value = [...syncLines.value.slice(-4), payload.line];
    });
  }

  // ---------------------------------------------------------------- 日志与撤销

  async function undo() {
    const id = requireId();
    if (id === null) return null;
    const outcome = await run(() => writeUndo(id));
    lastBackupRef.value = null;
    return outcome;
  }

  return {
    busy,
    error,
    canWrite,
    canUndo,
    hasConflicts,
    lastBackupRef,
    stashes,
    conflicts,
    journal,
    backups,
    deletable,
    syncLines,
    syncAction,
    loadAll,
    loadStashes,
    loadJournal,
    loadBackups,
    loadConflicts,
    clearForRepo,
    watchProgress,
    createBranch,
    switchBranch,
    renameBranch,
    deleteBranch,
    probeDeletable,
    setUpstream,
    createTag,
    deleteTag,
    pushStash,
    applyStash,
    popStash,
    dropStash,
    branchFromStash,
    pick,
    revert,
    reset,
    abort,
    resolveConflict,
    continueOperation,
    fetch,
    pull,
    push,
    deleteRemoteBranch,
    undo,
  };
});