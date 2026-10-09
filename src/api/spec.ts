import { call } from "@/api/client";
import type { Commit } from "@/api/commit";

/** 与 Rust 侧 config/spec.rs 的 Spec 一一对应。source 决定界面显示"规范从哪来"。 */
export type SpecSource = "repoconfig" | "commitlint" | "versionrc" | "cliff" | "fallback";

export interface TypeGroup {
  ty: string;
  section: string;
}

export interface Spec {
  types: string[];
  scopeRequired: boolean;
  subjectMaxLength: number;
  taskIdPattern: string | null;
  groups: TypeGroup[];
  source: SpecSource;
}

/** 与 Rust 侧 config/check.rs 的 Reason 一一对应（snake_case）。 */
export type Reason =
  | "empty_subject"
  | "missing_type"
  | "invalid_type"
  | "missing_scope"
  | "subject_too_long"
  | "missing_task_id"
  | "non_informative"
  | "malformed_header"
  | "fullwidth_colon"
  | "trailing_period";

export interface Violation {
  reason: Reason;
  blocking: boolean;
  title: string;
  hint: string;
}

export interface Outcome {
  violations: Violation[];
  conformant: boolean;
  commitType: string | null;
  scope: string | null;
  breaking: boolean;
}

/** 与 Rust 侧 git/commit.rs 的 Draft 一一对应；校验和提交都传这个结构。 */
export interface Draft {
  subject: string;
  body: string;
  footer: string;
}

/** 与 Rust 侧 git/hook.rs 的 HookState 一一对应（snake_case）。 */
export type HookState = "missing" | "installed" | "foreign";

export interface HookStatus {
  hookPath: string;
  state: HookState;
  /** `core.hooksPath` 原值；配了就是“这个位置被别的工具占着” */
  hooksPath: string | null;
  specSource: SpecSource;
  /** 已装脚本里的规则快照是否还等于当前规范（false = 规范改过，该更新） */
  ruleSnapshotCurrent: boolean;
}

/** installed=false 表示没写任何文件：message 是拒绝理由，suggestion 是共存方案。 */
export interface HookInstall {
  installed: boolean;
  hookPath: string;
  message: string;
  suggestion: string | null;
  backup: string | null;
  status: HookStatus;
}

export function hookStatus(id: number) {
  return call<HookStatus>("hook_status", { id });
}

export function hookInstall(id: number, force = false) {
  return call<HookInstall>("hook_install", { id, force });
}

export function hookUninstall(id: number) {
  return call<HookInstall>("hook_uninstall", { id });
}

/** 将要写入磁盘的那份脚本原文：界面用它做「查看规则快照」与共存时的复制内容。 */
export function hookScript(id: number) {
  return call<string>("hook_script", { id });
}

export function specFor(id: number) {
  return call<Spec>("spec_for", { id });
}

/** 实时校验：走的就是 commit_create 内部那次判定，所以预览不会骗人。 */
export function messageCheck(id: number, draft: Draft) {
  return call<Outcome>("message_check", { id, draft });
}

export function commitScopes(id: number) {
  return call<string[]>("commit_scopes", { id });
}

export function commitCreate(id: number, draft: Draft) {
  return call<Commit>("commit_create", { id, draft });
}

/** 界面文案：规范来源要说清是哪个文件，用户才知道该去改哪一份。 */
export const SOURCE_LABEL: Record<SpecSource, string> = {
  repoconfig: "git-tidy.config.json",
  commitlint: "commitlint 配置推导",
  versionrc: ".versionrc 推导",
  cliff: "cliff.toml 推导",
  fallback: "内置默认（仓库没配置）",
};
