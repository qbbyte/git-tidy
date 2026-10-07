import { call } from "@/api/client";

/** 与 Rust 侧 git/detail.rs 的 ChangeStatus 一一对应（serde 是 lowercase，不是 snake） */
export type ChangeStatus = "add" | "modify" | "delete" | "rename" | "copy" | "typechange";

/** 一个文件在这条提交里的改动。行数和大小差都只在对应的读法下有值 */
export interface Change {
  path: string;
  /** rename / copy 的来源路径；点开 diff 时要和新路径一起传，才配得成一段 */
  oldPath: string | null;
  status: ChangeStatus;
  /** rename / copy 的相似度，`R100` 的那个 100 */
  score: number | null;
  oldMode: string | null;
  newMode: string | null;
  /** 只改权限位，内容一字节没动 */
  modeOnly: boolean;
  /** 子模块指针：那两个 sha 是提交号，不是 blob，点开没有 diff */
  gitlink: boolean;
  /** 子模块指针的两端提交号，只有 gitlink 才有值（§7.4 要摆出来给人对比） */
  oldOid: string | null;
  newOid: string | null;
  added: number | null;
  deleted: number | null;
  /** numstat 给 `-`：二进制没有行数差 */
  binary: boolean;
  oldSize: number | null;
  newSize: number | null;
}

/** 与 Rust 侧 git/detail.rs 的 Detail 对应 */
export interface Detail {
  sha: string;
  parents: string[];
  /** 合并提交：下面这份清单是**对第一父**的差集，界面必须写出来（§7.4） */
  merge: boolean;
  changes: Change[];
}

/** 与 Rust 侧 git/diff.rs 的 Render 对应。降级路径全在这里枚举，前端不再自己判长度 */
export type Render = "text" | "empty" | "binary" | "image" | "toolarge";

export type LineKind = "context" | "add" | "delete" | "meta";

export interface Line {
  kind: LineKind;
  /** 新增行没有旧行号，删除行没有新行号；`\` 那行两个都没有 */
  oldNo: number | null;
  newNo: number | null;
  /** 已去掉行首标记符。行尾的 `\r` 原样留着，CRLF 改动靠它才看得见 */
  text: string;
}

export interface Hunk {
  oldStart: number;
  oldCount: number;
  newStart: number;
  newCount: number;
  /** `@@ … @@` 后面的段落名，git 给什么显示什么 */
  header: string;
  lines: Line[];
}

export interface Blob {
  mime: string;
  base64: string;
  bytes: number;
}

export interface Images {
  /** 纯新增（或删除）时对应一侧是 null */
  old: Blob | null;
  new: Blob | null;
}

/** 与 Rust 侧 git/diff.rs 的 Diff 对应 */
export interface Diff {
  path: string;
  render: Render;
  /** render 为 toolarge 时是空的：原文不回传 */
  hunks: Hunk[];
  added: number;
  deleted: number;
  lineCount: number;
  byteCount: number;
  oldSize: number | null;
  newSize: number | null;
  images: Images | null;
}

/** 一条提交的改动清单 + 行数差。路径由 Rust 侧从注册表按 id 解析（§7.1） */
export function fetchDetail(repoId: number, sha: string) {
  return call<Detail>("commit_detail", { id: repoId, sha });
}

/**
 * 单个文件的差异。一次点一个：整条提交一起回，大提交要点开才传几十 MB，
 * 而人一次只看一个文件（§7.5）。
 *
 * 参数名必须与 Rust 形参一致：`ignoreWhiteSpace` 对应 Rust 的 `ignore_white_space`，
 * Tauri 是按名字匹配的，名字错了是"参数缺失"而不是编译错误。
 */
export function fetchDiff(
  repoId: number,
  sha: string,
  path: string,
  oldPath: string | null,
  ignoreWhiteSpace: boolean,
) {
  return call<Diff>("commit_file_diff", {
    id: repoId,
    sha,
    path,
    oldPath,
    ignoreWhiteSpace,
  });
}
