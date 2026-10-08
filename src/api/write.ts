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
