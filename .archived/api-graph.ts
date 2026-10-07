import { call } from "@/api/client";

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
  /** 用 sha 对齐：列表和图是两次独立 IPC，谁先到不一定 */
  sha: string;
  /** 这条提交的圆点落在哪条泳道 */
  lane: number;
  color: number;
  segments: GraphSegment[];
}

export interface GraphPage {
  rows: GraphRow[];
  /** 图列有几条泳道宽。这是整条历史的最宽值，翻页不会变，所以列宽不会中途横移 */
  lanes: number;
}

/**
 * 读一页提交图。skip/limit 必须和同一次的 commit_list 完全一致，
 * 两边的行才落在同一条水平线上（Rust 侧共用同一个 --topo-order）。
 */
export function fetchGraph(repoId: number, skip: number, limit: number) {
  return call<GraphPage>("commit_graph", { id: repoId, skip, limit });
}
