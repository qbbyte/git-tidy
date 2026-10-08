import { call } from "@/api/client";
import type { CommitPage } from "@/api/commit";

/** 与 Rust 侧 git/tree.rs 的 EntryKind 一一对应（serde 小写）。 */
export type EntryKind = "blob" | "commit";

/** 某个修订里的一个文件，对应 Rust 侧 git/tree.rs 的 Entry。 */
export interface TreeEntry {
  path: string;
  /** 树里的 mode（100644 / 100755 / 120000 / 160000），可执行位变了要能看出来 */
  mode: string;
  kind: EntryKind;
  oid: string;
  /** blob 字节数。子模块指针没有（Rust 侧省略这个字段） */
  size?: number;
}

/** 某个修订里一个文件的正文，对应 Rust 侧 git/tree.rs 的 Content。 */
export interface FileContent {
  path: string;
  /** 二进制或超阈值时 Rust 侧不给这个字段 */
  text?: string;
  size: number;
  binary: boolean;
  truncated: boolean;
}

/** 一行的归属，对应 Rust 侧 git/blame.rs 的 BlameLine。 */
export interface BlameLine {
  line: number;
  sha: string;
  author: string;
  /** 作者时间，Unix 秒 */
  time: number;
  text: string;
  /** 边界提交：那一端的提交在本仓库里可能查不到，界面上不给它开跳转 */
  boundary: boolean;
}

/** 逐行归属，对应 Rust 侧 git/blame.rs 的 Blame。 */
export interface Blame {
  path: string;
  lines: BlameLine[];
  /** 文件过大，只 blame 了开头一段 */
  truncated: boolean;
  /** 这次忽略了哪些修订（`.blame-ignore-revs` + 勾选的）。归属因此与原始 blame 不同 */
  ignored: string[];
}

/** 逐行归属（§7.6）。rev 可以是提交号，也可以是 `HEAD~2` 这类表达式 */
export function fileBlame(repoId: number, rev: string, path: string, ignoreRevs?: string[]) {
  return call<Blame>("file_blame", { id: repoId, rev, path, ignoreRevs: ignoreRevs ?? null });
}

/** 一个文件的全部改动记录。Rust 侧走 `--follow`，历史跟着改名走 */
export function fileHistory(repoId: number, path: string, skip = 0, limit = 200) {
  return call<CommitPage>("file_history", { id: repoId, path, skip, limit });
}

/** 某个修订的文件树。扁平列表，按路径排序，前端自己缩进 */
export function fileTree(repoId: number, rev: string) {
  return call<TreeEntry[]>("file_tree", { id: repoId, rev });
}

/** 某个修订里一个文件的正文 */
export function fileContent(repoId: number, rev: string, path: string) {
  return call<FileContent>("file_content", { id: repoId, rev, path });
}