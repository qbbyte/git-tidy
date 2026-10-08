import { call } from "@/api/client";
import type { Diff } from "@/api/detail";
import type { WorkingFile } from "@/api/status";

/** 写操作的结果，对应 Rust 侧 `write::guard::Outcome`。 */
export interface Outcome {
  /** 还原 ref 的名字。界面要一直显示它：这是"怎么回去"的答案（§4） */
  backupRef: string;
  headBefore: string | null;
  headAfter: string | null;
  journalId: number;
}

/** 一个分支/标签引用，对应 Rust 侧 `git::branch::Deletable`。 */
export interface Deletable {
  /** 还没并进基准分支的提交数。`-D` 之前界面要先把"会丢多少"摆出来 */
  unmerged: number | null;
  merged: boolean;
}

/** 一条 stash，对应 Rust 侧 `git::stash::StashEntry`。 */
export interface StashEntry {
  /** `stash@{0}` 这种引用名，界面上的动作直接用它，不自己数序号 */
  reference: string;
  message: string;
  branch: string;
  /** Unix 秒 */
  time: number;
}

/** 一次远程操作的结果，对应 Rust 侧 `git::sync::SyncReport`。 */
export interface SyncReport {
  action: string;
  summary: string[];
  updated: string[];
}

/** 写操作日志的一条，对应 Rust 侧 `commands::write::WriteOpView`。 */
export interface WriteOpEntry {
  id: number;
  repoId: number;
  /** Unix 秒 */
  ts: number;
  action: string;
  affectedFrom: string | null;
  affectedTo: string | null;
  backupRef: string;
  headBefore: string | null;
  headAfter: string | null;
  /** ok / rolled_back / interrupted，中文说法由 Rust 侧一并给出 */
  status: "ok" | "rolled_back" | "interrupted";
  statusLabel: string;
  detail: string | null;
}

/** 撤销的结果，对应 Rust 侧 `write::guard::UndoReport`。 */
export interface UndoReport {
  action: string;
  backupRef: string;
  headBefore: string;
  headAfter: string;
}

/** 一个还原点，对应 Rust 侧 `write::backup::Backup`。 */
export interface Backup {
  reference: string;
  sha: string;
}

/** 这个文件能不能行级暂存，以及不能做时的原因（§7.8 的硬边界）。 */
export interface PartialSupport {
  supported: boolean;
  reason: string;
}

/** 选中的一段改动：`lines` 为空表示整个 hunk */
export interface HunkSelection {
  hunk: number;
  lines: number[];
}

/** 整文件暂存之外，逐行/分块暂存用。返回最新的整份工作区状态 */
export interface StagedFiles extends Outcome {
  files: WorkingFile[];
}

/** 工作区里某个文件的未暂存改动（索引 → 工作区），逐行暂存的界面靠它 */
export function worktreeFileDiff(repoId: number, path: string, ignoreWhiteSpace = false) {
  return call<Diff>("worktree_file_diff", { id: repoId, path, ignoreWhiteSpace });
}

export function filePartialSupport(repoId: number, path: string) {
  return call<PartialSupport>("file_partial_support", { id: repoId, path });
}

export function filesStageHunks(repoId: number, path: string, hunks: HunkSelection[]) {
  return call<StagedFiles>("files_stage_hunks", { id: repoId, path, hunks });
}

// ---------------------------------------------------------------- 分支与标签（§7.10）

export function branchCreate(
  repoId: number,
  name: string,
  start: string | null,
  switchToIt: boolean,
  expectedHead: string | null,
) {
  return call<Outcome>("branch_create", {
    id: repoId,
    name,
    start,
    switchToIt,
    expectedHead,
  });
}

/** 删之前先问会丢多少。这条是读操作，不进 write_guard。 */
export function branchDeletable(repoId: number, name: string) {
  return call<Deletable>("branch_deletable", { id: repoId, name });
}

export function branchDelete(repoId: number, name: string, force: boolean) {
  return call<Outcome>("branch_delete", { id: repoId, name, force });
}

export function branchRename(repoId: number, from: string, to: string) {
  return call<Outcome>("branch_rename", { id: repoId, from, to });
}

export function branchSwitch(
  repoId: number,
  name: string,
  create: boolean,
  start: string | null,
  expectedHead: string | null,
) {
  return call<Outcome>("branch_switch", { id: repoId, name, create, start, expectedHead });
}

export function upstreamSet(repoId: number, name: string, upstream: string | null) {
  return call<Outcome>("upstream_set", { id: repoId, name, upstream });
}

export function tagCreate(repoId: number, name: string, message: string | null) {
  return call<Outcome>("tag_create", { id: repoId, name, message });
}

export function tagDelete(repoId: number, name: string) {
  return call<Outcome>("tag_delete", { id: repoId, name });
}

// ---------------------------------------------------------------- stash（§7.9）

export function stashList(repoId: number) {
  return call<StashEntry[]>("stash_list", { id: repoId });
}

export function stashPush(
  repoId: number,
  paths: string[] | null,
  includeUntracked: boolean,
  message: string | null,
) {
  return call<Outcome>("stash_push", { id: repoId, paths, includeUntracked, message });
}

export function stashApply(repoId: number, reference: string) {
  return call<Outcome>("stash_apply", { id: repoId, reference });
}

export function stashPop(repoId: number, reference: string) {
  return call<Outcome>("stash_pop", { id: repoId, reference });
}

export function stashDrop(repoId: number, reference: string) {
  return call<Outcome>("stash_drop", { id: repoId, reference });
}

export function stashBranch(repoId: number, reference: string, name: string) {
  return call<Outcome>("stash_branch", { id: repoId, reference, name });
}

// ---------------------------------------------------------------- 摘取 / 回滚 / 复位（§7.11）

export function cherryPick(
  repoId: number,
  shas: string[],
  recordSource: boolean,
  expectedHead: string | null,
) {
  return call<Outcome>("op_cherry_pick", { id: repoId, shas, recordSource, expectedHead });
}

export function revertCommit(
  repoId: number,
  sha: string,
  mainline: number | null,
  expectedHead: string | null,
) {
  return call<Outcome>("op_revert", { id: repoId, sha, mainline, expectedHead });
}

/** reset 三档。hard 会扔掉工作区里的东西，确认强度按 §7.11 走。 */
export function resetTo(
  repoId: number,
  mode: "soft" | "mixed" | "hard",
  target: string,
  expectedHead: string | null,
) {
  return call<Outcome>("op_reset", { id: repoId, mode, target, expectedHead });
}

/** 中断态的一键退回。M2 不给"逐块取舍"，只给这一条精确的退路。 */
export function abortOperation(repoId: number) {
  return call<{ aborted: string; branch: string | null }>("op_abort", { id: repoId });
}

// ---------------------------------------------------------------- 远程（§7.12）

export function remoteFetch(repoId: number, remote: string | null) {
  return call<SyncReport>("remote_fetch", { id: repoId, remote });
}

export function remotePull(
  repoId: number,
  remote: string | null,
  strategy: "ff_only" | "rebase",
  expectedHead: string | null,
) {
  return call<Outcome>("remote_pull", { id: repoId, remote, strategy, expectedHead });
}

export function remotePush(
  repoId: number,
  remote: string,
  branch: string,
  setUpstream: boolean,
  forceWithLease: boolean,
) {
  return call<SyncReport>("remote_push", {
    id: repoId,
    remote,
    branch,
    setUpstream,
    forceWithLease,
  });
}

export function remoteDeleteBranch(repoId: number, remote: string, branch: string) {
  return call<SyncReport>("remote_delete_branch", { id: repoId, remote, branch });
}

// ---------------------------------------------------------------- 冲突解决器（§7.13）

/** 一边的正文。某一栏不存在（一方删了文件）时是 null */
export interface StageContent {
  text: string | null;
  size: number;
  binary: boolean;
}

/**
 * 冲突类型。降级穷举在 §7.13：
 * - `content` 三方都在，可以逐块取舍；
 * - `binary` / `submodule` / `renameRename` / `modifyDelete` / `deleteModify` / `bothAdded`
 *   只能选一边，界面不给逐块合并的入口。
 */
export type ConflictKind =
  | "content"
  | "modifyDelete"
  | "deleteModify"
  | "bothAdded"
  | "renameRename"
  | "binary"
  | "submodule";

/** 三方内容。三栏都可能缺，界面按缺哪一栏决定画几栏 */
export interface ConflictSides {
  /** 共同祖先 */
  base?: StageContent;
  /** 我方 = HEAD */
  ours?: StageContent;
  /** 对方 = 被合进来的那一支 */
  theirs?: StageContent;
  binary: boolean;
}

/** 一个未合并条目 */
export interface Conflict {
  path: string;
  /** porcelain v1 的两位状态，`UU` / `UD` / `DU` / `AA` */
  status: string;
  kind: ConflictKind;
  /** `kind` 的中文说法，直接显示 */
  label: string;
  /**
   * 能不能逐块合并。**用这个，不要从 `sides` 反推**——Rust 侧是看过索引里三个
   * stage 才得出的结论，界面自己猜会在改删、子模块这类三栏不全的情形下猜错，
   * 然后开出一个拼不出正确结果的合并区。
   */
  threeWay: boolean;
  /** 只能选一边（`threeWay` 的反面，一起给是为了界面不必自己取反） */
  pickSideOnly: boolean;
  /** 双改名时对端的路径 */
  otherPath?: string;
  /** 工作区里那份带 `<<<<<<<` 标记的草稿 */
  worktreeText?: string;
  sides: ConflictSides;
}

/** 解决一个冲突文件的方式。`text` 就是用户看到的，写回去的就是它 */
export type Resolution = { how: "ours" } | { how: "theirs" } | { how: "text"; text: string };

/** 解决之后的结果。带上剩余卡片，省一次 IPC */
export interface ResolvedConflict {
  backupRef: string;
  headBefore: string | null;
  headAfter: string | null;
  journalId: number;
  /** 还剩几个没解决 */
  remaining: number;
  conflicts: Conflict[];
}

/** 续跑的结果。`finished` 为 false 时说明下一个提交又冲突了，中断态还在 */
export interface Continued {
  backupRef: string;
  headBefore: string | null;
  headAfter: string | null;
  journalId: number;
  finished: boolean;
  /** 还停在哪一种中断态上 */
  stillInterrupted: "none" | "merge" | "rebase" | "cherryPick" | "revert";
  /** 变基时被变基的分支 */
  branch: string | null;
}

export function conflictList(repoId: number) {
  return call<Conflict[]>("conflict_list", { id: repoId });
}

export function conflictResolve(repoId: number, path: string, how: Resolution) {
  return call<ResolvedConflict>("conflict_resolve", { id: repoId, path, how });
}

/** 「接受删除」：改删/删改冲突里有一方已经把这个文件删了 */
export function conflictAcceptDeletion(repoId: number, path: string) {
  return call<ResolvedConflict>("conflict_accept_deletion", { id: repoId, path });
}

/**
 * 全部标记完之后续跑。
 *
 * 不是 `finished` 不代表出错：下一个提交也可能冲突，那时要接着解。
 */
export function conflictContinue(repoId: number, message: string | null = null) {
  return call<Continued>("conflict_continue", { id: repoId, message });
}

// ---------------------------------------------------------------- 日志与撤销（§7.17）

export function writeJournal(repoId: number, limit = 50) {
  return call<WriteOpEntry[]>("write_journal", { id: repoId, limit });
}

export function writeBackups(repoId: number) {
  return call<Backup[]>("write_backups", { id: repoId });
}

export function writeBackupTarget(repoId: number, reference: string) {
  return call<string | null>("write_backup_target", { id: repoId, reference });
}

/** 撤销上一步。HEAD 不在日志记录的位置时会被拒，界面据此提示去 reflog 找 */
export function writeUndo(repoId: number) {
  return call<UndoReport>("write_undo", { id: repoId });
}

/** 远程命令的进度事件名，与 Rust 侧 `commands::write::SYNC_PROGRESS_EVENT` 一致 */
export const SYNC_PROGRESS_EVENT = "sync-progress";

export interface SyncProgress {
  repoId: number;
  action: string;
  line: string;
}