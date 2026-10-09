import { call } from "@/api/client";

/** CHANGELOG 生成与导出（需求 6.5）。判定与分组都在 Rust 侧，前端只负责摆结果与去处。 */

export interface ChangelogEntry {
  sha: string;
  /** 去掉 `type(scope): ` 之后的描述 */
  description: string;
  commitType: string;
  scope: string | null;
  breaking: boolean;
  authorName: string;
}

export interface ScopedGroup {
  scope: string;
  entries: ChangelogEntry[];
}

export interface ChangelogSection {
  /** 章节名来自仓库配置（`Spec.groups`），可被 commitlint / versionrc / cliff 覆盖 */
  heading: string;
  entries: ChangelogEntry[];
  scoped: ScopedGroup[];
}

export interface SkippedSample {
  sha: string;
  subject: string;
  authorName: string;
}

/** 没进日志的提交必须带计数回来——静默丢掉等于骗人 */
export interface Skipped {
  nonConformant: number;
  merges: number;
  samples: SkippedSample[];
}

export interface Changelog {
  from: string | null;
  to: string;
  scanned: number;
  included: number;
  breaking: ChangelogEntry[];
  sections: ChangelogSection[];
  skipped: Skipped;
  truncated: boolean;
  markdown: string;
  /** 目标文件当前内容的指纹，追加写入时做乐观并发校验 */
  targetDigest: string;
}

export interface AppendResult {
  path: string;
  bytes: number;
  journalId: number;
  backupRef: string;
  /** 撤销办法：写的是工作区文件，所以要给出确切命令 */
  undoHint: string;
}

/** 写入前的目标文件预览：只给末尾一段，够看清接缝就行 */
export interface TargetPreview {
  path: string;
  exists: boolean;
  bytes: number;
  digest: string;
  tail: string;
}

export function changelogBuild(id: number, from?: string, to?: string) {
  return call<Changelog>("changelog_build", {
    id,
    from: from ?? null,
    to: to ?? null,
  });
}

export function changelogPreviousTag(id: number, to?: string) {
  return call<string | null>("changelog_previous_tag", { id, to: to ?? null });
}

export function changelogWrite(
  id: number,
  path: string,
  markdown: string,
  expectedDigest: string,
) {
  return call<AppendResult>("changelog_write", { id, path, markdown, expectedDigest });
}

export function changelogReadTarget(path: string) {
  return call<TargetPreview>("changelog_read_target", { path });
}