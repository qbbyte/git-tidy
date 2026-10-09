import { call } from "@/api/client";

/**
 * 活跃度统计（报告页的第二个视角）。
 *
 * 口径全部由 Rust 侧 `git::activity::build` 出，这里只负责摆：
 * 作者按 `.mailmap` 归并、merge / revert 被排除、行数是"git 记录的增删行"而非净产出。
 * 前端不重算——重算就等于两把尺子，日报和提交页迟早对不上。
 */
export interface AuthorStat {
  name: string;
  email: string;
  commits: number;
  insertions: number;
  deletions: number;
  /** 动过的文件数（纯改名算一个） */
  files: number;
  /** 有提交的自然天数，比"次数"更能看出节奏 */
  activeDays: number;
  /** 作者时间（Unix 秒），渲染成什么时区归界面管 */
  lastTime: number;
}

export interface DayStat {
  /** 本地日历日 `YYYY-MM-DD` */
  day: string;
  commits: number;
  insertions: number;
  deletions: number;
  authors: number;
}

export interface Activity {
  commits: number;
  insertions: number;
  deletions: number;
  files: number;
  /** numstat 对二进制文件给 `-\t-`，它们单独报个数，不混进行数 */
  binaryFiles: number;
  authorCount: number;
  activeDays: number;
  authors: AuthorStat[];
  /** 按本地日历日升序 */
  byDay: DayStat[];
  /** 被排除的 merge / revert 条数 */
  excluded: number;
  /** 扫描撞了上限：下面的数字只覆盖最近一批提交 */
  truncated: boolean;
  totalInRange: number;
  rev: string;
  since: string | null;
  until: string | null;
}

/**
 * 时间预设。
 *
 * 取值是 **git 自己的时间口语**（`today`、`1 week ago`），不是前端算的日期串：
 * `--since` 由 git 解析，两边必须是同一个裁判，否则"今日"在边界上会对不上。
 */
export type SincePreset = "today" | "week" | "month" | "all";

export const SINCE_PRESET: Record<SincePreset, string> = {
  today: "today",
  week: "1 week ago",
  month: "1 month ago",
  all: "",
};

export function commitActivity(id: number, rev?: string, since?: string) {
  return call<Activity>("commit_activity", {
    id,
    rev: rev ?? null,
    since: since ?? null,
    until: null,
  });
}