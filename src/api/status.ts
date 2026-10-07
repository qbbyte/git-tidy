import { call } from "./client";

/** 与 Rust 侧 git/status.rs 的 WorkingFile 一一对应 */
export interface WorkingFile {
  path: string;
  /** 重命名/复制的来源路径 */
  fromPath: string | null;
  /** 索引列（相对 HEAD）：M A D R C U ? 或空格 */
  indexStatus: string;
  /** 工作区列（相对索引） */
  worktreeStatus: string;
  staged: boolean;
  untracked: boolean;
  conflict: boolean;
}

/** 只读浏览的仓库没有工作区，Rust 侧会直接报 read_only_repo */
export function worktreeStatus(id: number) {
  return call<WorkingFile[]>("worktree_status", { id });
}

/** 暂存/取消暂存都返回整份最新状态：勾选态以 git 算出来的为准，前端不自己猜。 */
export function stageFiles(id: number, paths: string[]) {
  return call<WorkingFile[]>("files_stage", { id, paths });
}

export function unstageFiles(id: number, paths: string[]) {
  return call<WorkingFile[]>("files_unstage", { id, paths });
}
