import { listen } from "@tauri-apps/api/event";
import { call } from "./client";

/** 与 Rust 侧 store/repos.rs 的 RepoKind 对应 */
export type RepoKind = "worktree" | "browse";

/** 注册表里的一行。git 命令一律用它带的 id，前端不再传路径（§7.1） */
export interface Repo {
  id: number;
  name: string;
  path: string;
  kind: RepoKind;
  remoteUrl: string | null;
}

/** 与 Rust 侧 git/repo.rs 的 RepoInfo 对应。这里没有 dirty：工作区状态归 status 那一层 */
export interface RepoInfo {
  workTree: string;
  gitDir: string;
  gitVersion: string;
  branch: string | null;
  headCommit: string | null;
}

export function addLocalRepo(path: string) {
  return call<Repo>("repo_add", { path });
}

export function listRepos() {
  return call<Repo[]>("repo_list");
}

export function refreshRepo(id: number) {
  return call<RepoInfo>("repo_refresh", { id });
}

export function renameRepo(id: number, name: string) {
  return call<Repo>("repo_rename", { id, name });
}

export function removeRepo(id: number) {
  return call<void>("repo_remove", { id });
}

/** 贴地址添加：Rust 侧做 treeless 克隆，成功后 kind 是 browse（只读） */
export function addRemoteRepo(url: string) {
  return call<Repo>("repo_add_remote", { url });
}

/** 「克隆」：把只读浏览仓库补齐成完整本地仓库，kind 变 worktree */
export function materializeRepo(id: number) {
  return call<Repo>("repo_materialize", { id });
}

/** 与 Rust 侧 commands/remote.rs 的 ProgressPayload 对应 */
export interface CloneProgress {
  url: string;
  phase: string;
  percent: number;
}

/** 订阅克隆/补齐进度。并发时靠 url 认领自己那一条 */
export function onRepoProgress(handler: (progress: CloneProgress) => void) {
  return listen<CloneProgress>("repo-progress", (event) => handler(event.payload));
}
