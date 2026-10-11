import { ref, shallowRef } from "vue";
import { defineStore } from "pinia";
import { GitTidyError } from "@/api/client";
import type { Diff } from "@/api/detail";
import type { WorkingFile } from "@/api/status";
import { stagedFileDiff, worktreeFileDiff } from "@/api/write";

/**
 * 提交页中间那一栏的状态：左栏点中的文件，画它属于哪一列的那一半改动。
 *
 * 与 `useDetailStore` 是同一个形状（序号防串台、shallowRef 存大对象），但两者
 * **不能合并**：历史页看的是某条提交对父的差异，这里看的是当前工作区里一个文件的
 * 一半，仓库是同一个而"看的是哪一份"完全不同。合起来就是一个 store 里塞两套
 * sha / path / diff，互相把对方的在飞请求判成过期。
 */

/** 预览的是这个文件的哪一半，与提交页左栏那两列一一对应 */
export type PreviewSide = "staged" | "worktree";

/**
 * 「变更」列的判据，必须与 `repos.changedFiles` 逐字一致。
 *
 * 不一致的后果不是画错，而是**文件从预览里凭空消失**：左栏还列着，选中态却对不上。
 */
function onChangedSide(file: WorkingFile): boolean {
  return !file.staged || file.worktreeStatus !== " ";
}

function onSide(file: WorkingFile, side: PreviewSide): boolean {
  return side === "staged" ? file.staged : onChangedSide(file);
}

/**
 * 这个文件的差异为什么取不出来。**非空时不发 IPC**：宁可先把原因说清楚，
 * 也不要拿一次注定为空的 diff 去让人猜"是不是它没改动"。
 *
 * 两条都是 §7.8 那套硬边界在预览里的同一口径：
 * - 未跟踪文件不在索引里，git 两侧都给不出差异（`git diff -- 未跟踪文件` 是空的）；
 * - 冲突文件的差异是 git 的合并格式，逐行画出来不是"这次改了什么"。
 */
function blockedReason(file: WorkingFile): string | null {
  if (file.untracked) {
    return "未跟踪文件不在索引里，git 给不出差异。先勾选暂存，之后这里会显示它进索引的那一份。";
  }
  if (file.conflict) {
    return "这个文件正处在冲突中，逐行差异没有意义。请在冲突面板里逐块取舍。";
  }
  return null;
}

export const usePreviewStore = defineStore("preview", () => {
  /** 这些数据属于哪个仓库。换仓库后旧文件对不上新的列表，所以它也是失效判据之一 */
  const repoId = ref<number | null>(null);
  /** 正在看的文件路径。null = 没选 */
  const path = ref<string | null>(null);
  const side = ref<PreviewSide>("staged");
  const diff = shallowRef<Diff | null>(null);
  const loading = ref(false);
  const error = ref<GitTidyError | null>(null);
  /** 取不到差异的原因（未跟踪 / 冲突）。非空时 `diff` 一定是 null */
  const blocked = ref<string | null>(null);
  const ignoreWhiteSpace = ref(false);

  /**
   * 序号：连点两个文件时，先发的请求可能后落地。没有它，最后画出来的是哪一个
   * 取决于哪次 IPC 跑得慢，而慢的通常是文件更大的那一个。
   */
  let seq = 0;
  /** 上次取数时这个文件的状态两位（索引 + 工作区）。它变了才值得重取一次 */
  let signature = "";
  /** 上一次取数用的文件对象。重试与切换空白开关要拿它重发 */
  let last: WorkingFile | null = null;

  function wrap(err: unknown): GitTidyError {
    return err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
  }

  function statusPair(file: WorkingFile): string {
    return `${file.indexStatus}${file.worktreeStatus}`;
  }

  async function load(id: number, file: WorkingFile, which: PreviewSide, mine: number) {
    loading.value = true;
    try {
      // 改名要把来源路径一起给，否则 git 配不成一对，改了一行的文件会画成整个新增
      const got =
        which === "staged"
          ? await stagedFileDiff(id, file.path, file.fromPath, ignoreWhiteSpace.value)
          : await worktreeFileDiff(id, file.path, file.fromPath, ignoreWhiteSpace.value);
      if (mine !== seq) return;
      diff.value = got;
    } catch (err) {
      if (mine !== seq) return;
      error.value = wrap(err);
    } finally {
      if (mine === seq) loading.value = false;
    }
  }

  /** 选中一个文件的一侧。同一侧再点一次也重取：这中间工作区可能已经被别的工具改了 */
  function open(id: number, file: WorkingFile, which: PreviewSide) {
    const mine = ++seq;
    repoId.value = id;
    path.value = file.path;
    side.value = which;
    last = file;
    signature = statusPair(file);
    diff.value = null;
    error.value = null;
    blocked.value = blockedReason(file);
    if (blocked.value !== null) {
      loading.value = false;
      return Promise.resolve();
    }
    return load(id, file, which, mine);
  }

  /** 重取当前这一份。错误提示里的「重试」和空白开关都走它 */
  function reopen() {
    const id = repoId.value;
    if (id === null || last === null) return Promise.resolve();
    return open(id, last, side.value);
  }

  /**
   * 忽略空白开关：拿现在这个文件重取一次。
   *
   * 不重取的话点一下什么都不会变——这个开关改的是 git 的比较方式，
   * 前端手里那份 hunks 已经算完了，自己滤不出"忽略空白之后还剩什么"。
   */
  function setIgnoreWhiteSpace(value: boolean) {
    ignoreWhiteSpace.value = value;
    return reopen();
  }

  /**
   * 暂存 / 取消暂存之后对一次账。
   *
   * 选中那个文件的两位状态没变就什么都不做：勾一下别的文件不该让这边白跑一次 IPC，
   * 也不该让正在看的那份 diff 闪一下重建。它离开了当前这一列就跟着它去另一列——
   * 刚把最后几段暂存掉的人，正想看的就是刚进去的那一份。两列都不在了（被提交掉了）
   * 才收起预览：留着一份"已经不在任何一列里"的差异，比空着更容易让人按错提交。
   */
  function sync(id: number, files: WorkingFile[]) {
    if (repoId.value !== id) {
      close();
      return;
    }
    const current = path.value;
    if (current === null) return;
    const file = files.find((item) => item.path === current);
    if (file === undefined) {
      close();
      return;
    }
    if (!onSide(file, side.value)) {
      const other: PreviewSide = side.value === "staged" ? "worktree" : "staged";
      if (!onSide(file, other)) {
        close();
        return;
      }
      void open(id, file, other);
      return;
    }
    if (statusPair(file) === signature) return;
    void open(id, file, side.value);
  }

  function close() {
    ++seq;
    repoId.value = null;
    path.value = null;
    diff.value = null;
    error.value = null;
    blocked.value = null;
    loading.value = false;
    signature = "";
    last = null;
  }

  return {
    path,
    side,
    diff,
    loading,
    error,
    blocked,
    ignoreWhiteSpace,
    open,
    reopen,
    setIgnoreWhiteSpace,
    sync,
    close,
  };
});
