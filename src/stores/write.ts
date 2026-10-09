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
  rewritePlan as rewritePlanApi,
  rewritePlanSize as rewritePlanSizeApi,
  rewriteRun as rewriteRunApi,
  type PlanEntry,
  type PlanSize,
  type RewriteReport,
  type TodoItem,
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
import { usePrefsStore } from "@/stores/prefs";

function wrap(err: unknown): GitTidyError {
  return err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
}

/** 提交号在界面文案里只给前 8 位：两串 40 位并排没人读得下去 */
function shortSha(sha: string): string {
  return sha.slice(0, 8);
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
  /** 拉取策略的默认值来自个人偏好，所以这个 store 也读偏好 */
  const prefsStore = usePrefsStore();
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

  /** 改写面板的 todo（§7.14）。空数组 = 还没打开面板，不是"区间是空的" */
  const rewriteTodo = shallowRef<TodoItem[]>([]);
  /** 区间里每条提交的原始事实（标题、作者、日期）。todo 只存动作，事实查这里 */
  const rewriteEntries = shallowRef<PlanEntry[]>([]);
  /** todo 覆盖的区间的底端（`base`）。改写只发生在 base..HEAD 这段连续区间里 */
  const rewriteBase = ref<string | null>(null);
  /** 区间有多大。`large` 为真时界面要先告诉用户"会跑一阵子"再让他确认 */
  const rewriteSize = shallowRef<PlanSize | null>(null);
  /** 最近一次改写的结果。成功后界面上要一直显示还原 ref */
  const rewriteReport = shallowRef<RewriteReport | null>(null);

  /**
   * 中断态下能做的事只有一件：退回（§3 的 M2 边界）。
   * 切换仓库期间也全关：此刻界面挂的还是上一个仓库的快照，动了就会操作错仓库。
   */
  const canWrite = computed(() => repos.canCommit && !repos.interrupted && !repos.switching);

  /** 有没有没解决的冲突。界面据此把「续跑」以外的写入口全禁掉 */
  const hasConflicts = computed(() => conflicts.value.length > 0);

  /** 有没有可撤销的写操作：最后一条必须是成功的。切换仓库期间同样关掉 */
  const canUndo = computed(
    () => !repos.switching && journal.value.length > 0 && journal.value[0].status === "ok",
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
    // todo 里的提交号属于上一个仓库，留着就是一份指向别的仓库的操作清单
    closeRewrite();
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

  /**
   * 拉取。`strategy` 不传就用个人偏好里的拉取策略（设置页可改，默认只快进）。
   * 界面上仍然可以临时换一次——所以“默认”与“本次”得分得开，不能写成同一个字段。
   */
  async function pull(
    remote: string | null = null,
    strategy?: "ff_only" | "rebase" | "merge",
  ) {
    const id = requireId();
    if (id === null) return null;
    const chosen = strategy ?? prefsStore.prefs?.pullStrategy ?? "ff_only";
    syncLines.value = [];
    syncAction.value = "pull";
    try {
      const outcome = await run(() => remotePull(id, remote, chosen, expectedHead()));
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

  // ---------------------------------------------------------------- 交互式改写（§7.14）

  /**
   * 打开改写面板：以 `base` 为底端拉出 todo 初稿（§7.14 只支持从 HEAD 往回的连续区间）。
   *
   * todo 的内容归界面管：排序、选动作、改信息都在这里完成，执行时才整份传给 Rust。
   */
  async function openRewrite(base: string) {
    const id = requireId();
    if (id === null) return null;
    const head = repos.info?.headCommit;
    if (!head) return null;
    try {
      const [entries, size] = await Promise.all([
        rewritePlanApi(id, base, head),
        rewritePlanSizeApi(id, base, head),
      ]);
      rewriteBase.value = base;
      rewriteEntries.value = entries;
      rewriteTodo.value = entries.map((entry: PlanEntry) => ({
        sha: entry.sha,
        action: "pick" as const,
        message: null,
      }));
      rewriteSize.value = size;
      rewriteReport.value = null;
      return entries;
    } catch (err) {
      error.value = wrap(err);
      return null;
    }
  }

  function closeRewrite() {
    rewriteTodo.value = [];
    rewriteEntries.value = [];
    rewriteBase.value = null;
    rewriteSize.value = null;
    rewriteReport.value = null;
  }

  /**
   * todo 里的非法组合在后端也会被拒，但界面不留这种可能：squash/fixup 打头没有
   * 可并的对象，跑到一半才失败是最坏的时机（§7.14）。
   */
  const rewriteProblem = computed(() => {
    let hasLeader = false;
    for (const item of rewriteTodo.value) {
      if (item.action === "squash" || item.action === "fixup") {
        if (!hasLeader) return `${shortSha(item.sha)} 要并进前一条，但它前面没有可保留的提交`;
      }
      hasLeader = item.action !== "drop" && item.action !== "squash" && item.action !== "fixup";
    }
    if (rewriteTodo.value.length === 0) return "区间里没有可改写的提交";
    if (rewriteTodo.value.every((item) => item.action === "pick")) return null;
    return null;
  });

  /** 有没有真正要执行的改动。全是 pick 等于什么也没做，不必给入口 */
  const rewriteDirty = computed(() => rewriteTodo.value.some((item) => item.action !== "pick"));

  async function runRewrite() {
    const id = requireId();
    const base = rewriteBase.value;
    if (id === null || base === null) return null;
    const report = await run(async () => {
      const [outcome, detail] = await rewriteRunApi(id, base, rewriteTodo.value, expectedHead());
      lastBackupRef.value = outcome.backupRef;
      rewriteReport.value = detail;
      return detail;
    });
    // 成功之后 todo 就过期了：区间里的提交号全变了，留在面板上会诱导用户再跑一次
    if (report) closeRewrite();
    return report;
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
    rewriteTodo,
    rewriteEntries,
    rewriteBase,
    rewriteSize,
    rewriteReport,
    rewriteProblem,
    rewriteDirty,
    openRewrite,
    closeRewrite,
    runRewrite,
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