import { call } from "@/api/client";

/** 与 Rust 侧 store/prefs.rs 的 Preferences 一一对应（camelCase）。 */
export type PullStrategy = "ff_only" | "rebase" | "merge";

export interface FlowPrefixes {
  feature: string;
  hotfix: string;
  release: string;
}

export interface Columns {
  refs: boolean;
  author: boolean;
  time: boolean;
  sha: boolean;
}

export interface WindowState {
  width: number;
  height: number;
}

export interface Preferences {
  pullStrategy: PullStrategy;
  flowPrefixes: FlowPrefixes;
  autoUpdate: boolean;
  columns: Columns;
  window: WindowState;
}

export function prefsGet() {
  return call<Preferences>("prefs_get", {});
}

/** 整份覆盖。返回的是实际存进去的值（含被夹取的窗口尺寸） */
export function prefsUpdate(next: Preferences) {
  return call<Preferences>("prefs_update", { next });
}

export function prefsReset() {
  return call<Preferences>("prefs_reset", {});
}

/** 设置文件在哪：设置页要把它显示出来 */
export function prefsPath() {
  return call<string>("prefs_path", {});
}

export function openTerminal(repoId: number) {
  return call<string>("shell_open_terminal", { id: repoId });
}

export const PULL_STRATEGY_LABEL: Record<PullStrategy, string> = {
  ff_only: "只快进（合不进去就停）",
  rebase: "变基",
  merge: "合并（自动造合并提交）",
};