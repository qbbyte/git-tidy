import { call } from "@/api/client";

/** 与 Rust 侧 git/refs.rs 的 RefKind 一一对应（serde 小写）。 */
export type RefKind = "branch" | "remote" | "tag";

/** 与 Rust 侧 git/refs.rs 的 Interrupt 一一对应（serde snake_case）。 */
export type Interrupt = "none" | "merge" | "rebase" | "cherry_pick" | "revert";

/** 提交行上的引用徽标，对应 Rust 侧 git/refs.rs 的 Badge。 */
export interface RefBadge {
  /** 已去掉 refs/heads/ 这类命名空间前缀 */
  name: string;
  kind: RefKind;
  /** HEAD 正指在这里。游离 HEAD 时这条提交的 head 同样是 true，但没有带名字的徽标 */
  head: boolean;
}

/** 注册表里的一个引用，对应 Rust 侧 git/refs.rs 的 Ref。 */
export interface Ref {
  name: string;
  kind: RefKind;
  /** 原始完整名字：写操作和跨命令比对都认它，短名会被两个命名空间共用 */
  fullName: string;
  target: string;
  upstream: string | null;
  /** null = 没有这个数字（未设置跟踪分支 / 已同步 / git 换了格式），不等于 0 */
  ahead: number | null;
  behind: number | null;
  upstreamGone: boolean;
}

/** 仓库级状态摘要，对应 Rust 侧 git/refs.rs 的 RepoState。 */
export interface RepoState {
  /** null = 游离 HEAD */
  branch: string | null;
  upstream: string | null;
  ahead: number | null;
  behind: number | null;
  upstreamGone: boolean;
  interrupt: Interrupt;
  /** 只在变基时有值：变基过程中 HEAD 是游离的，靠 rebase-merge/head-name 才认得出分支名 */
  interruptBranch: string | null;
}

/** 引用与状态一次读回，对应 Rust 侧 commands/refs.rs 的 Scan。 */
export interface RefScan {
  refs: Ref[];
  state: RepoState;
}

/**
 * 中断态的中文说法，与 Rust 侧 `Interrupt::label()` 逐字对齐：
 * 提交被拒时的错误文案由 Rust 出，界面上的提示条由这里出，两边叫法必须一致。
 */
export const INTERRUPT_LABEL: Record<Interrupt, string> = {
  none: "没有中断操作",
  merge: "合并进行中",
  rebase: "变基进行中",
  cherry_pick: "摘取进行中",
  revert: "回滚进行中",
};

/** 引用扫描。ahead/behind 由 Rust 侧从这次扫描里直接取，不会为它单起进程（§7.3）。 */
export function scanRefs(repoId: number) {
  return call<RefScan>("refs_scan", { id: repoId });
}
