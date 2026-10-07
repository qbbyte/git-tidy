import { call } from "@/api/client";

/** 与 Rust 侧 git/log.rs 的 Commit 一一对应。 */
export interface Commit {
  id: string;
  authorName: string;
  authorEmail: string;
  /** 作者时间，Unix 秒，时区渲染归前端 */
  time: number;
  subject: string;
  body: string;
  merge: boolean;
  revert: boolean;
  /** Summary 在 Rust 侧是 serde(flatten)，到 JSON 里是平铺的三个字段 */
  commitType: string | null;
  scope: string | null;
  breaking: boolean;
}

export interface CommitPage {
  commits: Commit[];
  total: number;
}

/** 按注册仓库 id 分页读提交，路径由 Rust 侧从注册表解析（§7.1）。
 *  参数名必须与 Rust 形点一致（id 而不是 repoId），Tauri 是按名字匹配的。 */
export function listCommits(repoId: number, skip: number, limit: number) {
  return call<CommitPage>("commit_list", { id: repoId, skip, limit });
}
