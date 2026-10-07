import { call } from "@/api/client";
import type { RefBadge } from "@/api/refs";

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
  /** 这一条是不是 HEAD 所在的提交，游离 HEAD 时同样为 true */
  head: boolean;
  /** 指向这一条的分支 / 远程跟踪分支 / 标签 */
  refs: RefBadge[];
  /** Summary 在 Rust 侧是 serde(flatten)，到 JSON 里是平铺的三个字段 */
  commitType: string | null;
  scope: string | null;
  breaking: boolean;
}

export interface CommitPage {
  commits: Commit[];
  total: number;
}

/** 与 Rust 侧 git/graph.rs 的 Segment 一一对应。 */
export interface GraphSegment {
  /** 本行顶部的泳道下标 */
  from: number;
  /** 本行底部的泳道下标，等于 from 就是竖线 */
  to: number;
  color: number;
}

/** 与 Rust 侧 git/graph.rs 的 Row 一一对应。 */
export interface GraphRow {
  /** 用 sha 对齐：列表页和图页是两次 IPC，谁先到不一定 */
  sha: string;
  /** 这条提交的圆点落在哪条泳道 */
  lane: number;
  color: number;
  /** 上一行有没有线落进本行这一列。图的第一行和独立历史的起点是 false */
  incoming: boolean;
  segments: GraphSegment[];
}

/** 与 Rust 侧 git/graph.rs 的 Page 一一对应。 */
export interface CommitWindow {
  rows: GraphRow[];
  /** 图列有几条泳道宽。这是整条历史的最宽值，翻页不会变，列宽因此不会中途横移 */
  lanes: number;
}

/** 按注册仓库 id 分页读提交，路径由 Rust 侧从注册表解析（§7.1）。
 *  参数名必须与 Rust 形点一致（id 而不是 repoId），Tauri 是按名字匹配的。 */
export function listCommits(repoId: number, skip: number, limit: number) {
  return call<CommitPage>("commit_list", { id: repoId, skip, limit });
}

/**
 * 读同一扇窗口的图列数据。skip/count 必须与上面那次一致，count 是本页实际拿到的行数：
 * Rust 侧两边共用同一个 --topo-order，行号才落在同一条水平线上。
 */
export function fetchGraph(repoId: number, skip: number, count: number) {
  return call<CommitWindow>("commit_graph", { id: repoId, skip, count });
}
