import { call } from "@/api/client";
import type { Reason, SpecSource } from "@/api/spec";

/**
 * 符合率报告（需求 7.21）。所有数字都由 Rust 侧的 `config::check::evaluate` 出，
 * 前端不重算、不改口径——表单、hook、报告必须是同一把尺子。
 */
export interface ReasonLabel {
  reason: Reason;
  title: string;
}

export interface Offender {
  id: string;
  authorName: string;
  time: number;
  subject: string;
  commitType: string | null;
  scope: string | null;
  breaking: boolean;
  reasons: ReasonLabel[];
  /**
   * 疑似 `--no-verify` 绕过。commit 对象里没有任何"被绕过"的痕迹，
   * 这是「装了 hook + 提交晚于安装 + 不合规」推出来的间接判断，所以叫疑似。
   */
  suspectedBypass: boolean;
}

export interface ReasonCount extends ReasonLabel {
  count: number;
  /** 占不合规提交数的比例，不是占全量 */
  share: number;
}

export interface TypeCount {
  commitType: string | null;
  total: number;
  conformant: number;
}

export interface TrendRow {
  /** 作者名或 `YYYY-MM` */
  key: string;
  total: number;
  conformant: number;
  rate: number;
}

export interface ComplianceReport {
  scanned: number;
  conformant: number;
  rate: number;
  byReason: ReasonCount[];
  byType: TypeCount[];
  byAuthor: TrendRow[];
  byMonth: TrendRow[];
  offenders: Offender[];
  offendersTruncated: boolean;
  /** 统计被上限截断：下面的百分比只覆盖最近这 N 条 */
  truncated: boolean;
  /** 不计入分母的条数（合并 / revert：信息是 git 生成的） */
  excluded: number;
  suspectedBypass: number;
  hookInstalled: boolean;
  specSource: SpecSource;
  totalInRange: number;
  rev: string;
}

export function complianceReport(id: number, rev?: string, author?: string) {
  return call<ComplianceReport>("compliance_report", { id, rev: rev ?? null, author: author ?? null });
}

export function complianceRevisions(id: number) {
  return call<string[]>("compliance_revisions", { id });
}