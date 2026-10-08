/**
 * 冲突块的解析与拼装。
 *
 * 刻意**不自己算三方 diff**：git 已经在工作区里那份草稿上标好了
 * `<<<<<<< / ||||||| / ======= / >>>>>>>`，那些标记就是 git 自己算出来的冲突块边界。
 * 我们照着标记解析，比重新实现一遍 diff3 可靠得多——边界算错时用户看到的是一个
 * 看起来合理、其实内容错了的合并结果，那比报错糟糕得多。
 *
 * 两种标记形态都吃：`merge` 风格（默认）没有 `|||||||` 那一段，
 * `diff3` 风格多一段共同祖先。
 */

export interface ConflictHunk {
  /** 在原文里的起止（含标记行），用于定位 */
  startLine: number;
  endLine: number;
  /** 共同祖先那一段。`merge` 风格下为空 */
  base: string[];
  ours: string[];
  theirs: string[];
}

const MARK_OURS = /^<{7}(?:\s|$)/;
const MARK_BASE = /^\|{7}(?:\s|$)/;
const MARK_SPLIT = /^={7}(?:\s|$)/;
const MARK_THEIRS = /^>{7}(?:\s|$)/;

/**
 * 把带冲突标记的正文切成「干净片段」与「冲突块」的交替序列。
 * 返回的 `clean[i]` 夹在 `hunks[i-1]` 与 `hunks[i]` 之间。
 */
export interface ParsedConflict {
  /** 冲突块前面的干净内容，第一个元素是标记之前的那一段 */
  clean: string[][];
  hunk: ConflictHunk[];
}

/**
 * 解析冲突标记。遇到不成对的标记就返回 `null`——
 * 那说明这份草稿已经被手工改过（比如用户删了一行标记），界面上要给原始内容让用户自己处理，
 * 不能按半截标记硬拼一个结果出来。
 */
export function parseConflict(text: string): ParsedConflict | null {
  const lines = text.split("\n");
  const clean: string[][] = [[]];
  const hunk: ConflictHunk[] = [];

  let i = 0;
  while (i < lines.length) {
    if (!MARK_OURS.test(lines[i])) {
      clean[clean.length - 1].push(lines[i]);
      i += 1;
      continue;
    }

    const startLine = i;
    let section: "ours" | "base" | "theirs" = "ours";
    const ours: string[] = [];
    const base: string[] = [];
    const theirs: string[] = [];
    i += 1;

    for (; i < lines.length; i += 1) {
      const line = lines[i];
      if (MARK_BASE.test(line)) {
        section = "base";
        continue;
      }
      if (MARK_SPLIT.test(line)) {
        section = "theirs";
        continue;
      }
      if (MARK_THEIRS.test(line)) {
        break;
      }
      if (MARK_OURS.test(line)) {
        // 嵌套的开标记：外层没关上，解析不了
        return null;
      }
      if (section === "ours") ours.push(line);
      else if (section === "base") base.push(line);
      else theirs.push(line);
    }

    if (i >= lines.length) {
      // 没有闭合的 >>>>>>>：标记被手工改坏了
      return null;
    }
    i += 1;
    hunk.push({ startLine, endLine: i - 1, base, ours, theirs });
    clean.push([]);
  }

  // 末尾那个干净片段是空的（最后一个冲突块之后没有内容），去掉一个空壳更省心
  if (clean.length > 1 && clean[clean.length - 1].length === 0) {
    clean.pop();
  }
  return { clean, hunk };
}

/** 一个冲突块怎么解决 */
export type HunkChoice = "ours" | "theirs" | "base" | "custom";

/**
 * 按选择拼出合并结果。`custom` 时用 `customText` 原样替换整个块。
 *
 * 拼出来的结果就是用户看到、也将写回磁盘的那一份，不做行尾规范化——
 * 用户在编辑器里怎么看到的就怎么存。
 */
export function assemble(parsed: ParsedConflict, choices: (HunkChoice | null)[], customText: string[] = []): string {
  const out: string[] = [...parsed.clean[0]];
  parsed.hunk.forEach((hunk, index) => {
    const choice = choices[index];
    if (choice === "ours") out.push(...hunk.ours);
    else if (choice === "theirs") out.push(...hunk.theirs);
    else if (choice === "base") out.push(...hunk.base);
    else if (choice === "custom") out.push(...customText);
    // null：还没决定，这一块留空。界面上不会走到这一步，续跑前会被挡住
    out.push(...[]);
    const next = parsed.clean[index + 1];
    if (next) out.push(...next);
  });
  return out.join("\n");
}

/**
 * 草稿里有多少冲突块。零块说明标记已经被人清干净了，界面上要问一句
 * 而不是直接给一个空结果。
 */
export function countHunks(text: string): number {
  return parseConflict(text)?.hunk.length ?? 0;
}
