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
  /** 父提交号。Rust 侧在父集合为空时省掉这个字段 */
  parents?: string[];
}

export interface CommitPage {
  commits: Commit[];
  /** 总数。带筛选时是 git 按同一组条件数出来的（§7.7） */
  total: number;
  /**
   * 只按合规/type 筛时解析层要分段扫历史，扫到上限就会置位。
   * 置位时"共 N 条"与实得条数可能不一致，界面上要写明"只扫了前一段"。
   */
  truncated?: boolean;
}

/**
 * 筛选条件（§7.7）。两类能力分开：前五个是 git 认识的条件，直接映射成 `log` 的
 * 参数；后两个 git 不认识（Conventional Commits 是我们的规矩），由 Rust 侧在解析层判。
 * 字段名必须与 Rust 侧 `log::Filter` 的 serde 名一致。
 */
export interface CommitFilter {
  /** rev 范围：分支名、`a..b`、`HEAD~3` */
  rev?: string | null;
  authors?: string[];
  grep?: string[];
  /** 只看动过某个路径的提交 */
  path?: string | null;
  since?: string | null;
  until?: string | null;
  /** Conventional type 白名单（小写） */
  types?: string[];
  /** true 只留合规，false 只留不合规 */
  conformant?: boolean | null;
}

/** 与 Rust 侧 git/graph.rs 的 Segment 一一对应。 */
export interface GraphSegment {
  /** 本行顶部的泳道下标 */
  from: number;
  /** 本行底部的泳道下标，等于 from 就是竖线 */
  to: number;
  color: number;
  /**
   * 这段线下面没有可见的落点：父提交被筛选挡掉了。
   * 前端画成一段短截断线加端点，绝不硬连到下一行——连错一行比画不出来严重得多。
   */
  dangling?: boolean;
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
export function listCommits(
  repoId: number,
  skip: number,
  limit: number,
  filter?: CommitFilter,
) {
  return call<CommitPage>("commit_list", { id: repoId, skip, limit, filter: filter ?? null });
}

/**
 * 读一条提交本身。从文件历史、blame 跳到一条**不在当前列表页里**的提交时用它：
 * 没有它，那种跳转只能拿到一个 sha，没有标题、作者与父。
 */
export function showCommit(repoId: number, sha: string) {
  return call<Commit>("commit_show", { id: repoId, sha });
}

/**
 * 读同一扇窗口的图列数据。skip/count 必须与上面那次一致，count 是本页实际拿到的行数：
 * Rust 侧两边共用同一个 --topo-order，行号才落在同一条水平线上。
 * filter 也要一致：筛选改变可见集合，图是按可见集合算的。
 */
export function fetchGraph(repoId: number, skip: number, count: number, filter?: CommitFilter) {
  return call<CommitWindow>("commit_graph", { id: repoId, skip, count, filter: filter ?? null });
}

/** 有没有任何筛选条件。空条件与"没传"在 Rust 侧是同一条代码路径 */
export function filterIsEmpty(filter: CommitFilter): boolean {
  return (
    !filter.rev &&
    !filter.path &&
    !filter.since &&
    !filter.until &&
    (filter.authors?.length ?? 0) === 0 &&
    (filter.grep?.length ?? 0) === 0 &&
    (filter.types?.length ?? 0) === 0 &&
    filter.conformant === undefined
  );
}