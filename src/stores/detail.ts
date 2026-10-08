import { ref, shallowRef } from "vue";
import { defineStore } from "pinia";
import { GitTidyError } from "@/api/client";
import { showCommit } from "@/api/commit";
import {
  fetchDetail,
  fetchDiff,
  type Change,
  type Detail,
  type Diff,
} from "@/api/detail";
import type { Commit } from "@/api/commit";

function wrap(err: unknown): GitTidyError {
  return err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
}

/**
 * 「历史」页右半边的状态：选中提交的改动清单，加上其中一个文件的差异。
 *
 * 两个请求各自一个序号。共用一个的话，点文件会把还在飞的清单请求判成过期，
 * 而清单正是那个文件的出处。
 */
export const useDetailStore = defineStore("detail", () => {
  const sha = ref<string | null>(null);
  /**
   * 这一条的行数据。列表页里选中的提交直接用它（列表里已经全有了）；
   * 从文件历史/blame 跳过来的提交不在列表里，那就单独取一次——没有它，
   * 那一栏只剩一个 sha，没有标题与作者。
   */
  const row = shallowRef<Commit | null>(null);
  const detail = shallowRef<Detail | null>(null);
  const loading = ref(false);
  const rowLoading = ref(false);
  const rowError = ref<GitTidyError | null>(null);
  const error = ref<GitTidyError | null>(null);

  const file = shallowRef<Change | null>(null);
  const diff = shallowRef<Diff | null>(null);
  const diffLoading = ref(false);
  const diffError = ref<GitTidyError | null>(null);

  /** 空白改动只看统计不看逐行：这是个显示开关，改它只是重取同一个文件 */
  const ignoreWhiteSpace = ref(false);

  let detailSeq = 0;
  let diffSeq = 0;
  let rowSeq = 0;

  /**
   * 选中的提交换了：清单和 diff 一起作废，等新的落地。
   *
   * `known` 是列表里那一条行数据。给了就用，不额外起进程；不给（比如从文件历史跳过来）
   * 才去取一条——那一次多出来的 IPC 只发生在跳转路径上，不在高频的列表点选上。
   */
  async function open(repoId: number, commitSha: string | null, known: Commit | null = null) {
    const mine = ++detailSeq;
    ++diffSeq;
    ++rowSeq;
    sha.value = commitSha;
    detail.value = null;
    error.value = null;
    file.value = null;
    diff.value = null;
    diffError.value = null;
    row.value = known;
    rowError.value = null;
    if (commitSha === null) {
      loading.value = false;
      return;
    }
    if (known === null) void loadRow(repoId, commitSha, mine);
    loading.value = true;
    try {
      const got = await fetchDetail(repoId, commitSha);
      // 点下一条时这条已经没人等了：落下来会把旧提交盖回去
      if (mine !== detailSeq) return;
      detail.value = got;
    } catch (err) {
      if (mine !== detailSeq) return;
      error.value = wrap(err);
    } finally {
      if (mine === detailSeq) loading.value = false;
    }
  }

  /** 单独取一条提交的行数据。拿不到不算详情失败：清单照样读得出来 */
  async function loadRow(repoId: number, commitSha: string, mine: number) {
    rowLoading.value = true;
    try {
      const got = await showCommit(repoId, commitSha);
      if (mine !== rowSeq) return;
      row.value = got;
    } catch (err) {
      if (mine !== rowSeq) return;
      rowError.value = wrap(err);
    } finally {
      if (mine === rowSeq) rowLoading.value = false;
    }
  }

  /**
   * 点开清单里的一个文件。子模块指针（gitlink）没有 diff 可读，界面也不该给这个入口，
   * 这里再挡一次：命令层收到的会是一个查不到的对象号。
   */
  async function pickFile(repoId: number, change: Change) {
    if (change.gitlink || sha.value === null) return;
    const mine = ++diffSeq;
    const whiteSpace = ignoreWhiteSpace.value;
    file.value = change;
    diff.value = null;
    diffError.value = null;
    diffLoading.value = true;
    try {
      const got = await fetchDiff(repoId, sha.value, change.path, change.oldPath, whiteSpace);
      if (mine !== diffSeq) return;
      diff.value = got;
    } catch (err) {
      if (mine !== diffSeq) return;
      diffError.value = wrap(err);
    } finally {
      if (mine === diffSeq) diffLoading.value = false;
    }
  }

  /** 换忽略空白开关：拿现在这个文件重取一次，不重取就点一下什么也不会变 */
  function setIgnoreWhiteSpace(repoId: number, value: boolean) {
    ignoreWhiteSpace.value = value;
    if (file.value !== null) return pickFile(repoId, file.value);
    return Promise.resolve();
  }

  function retryDiff(repoId: number) {
    if (file.value !== null) return pickFile(repoId, file.value);
    return Promise.resolve();
  }

  /** 换仓库、或列表里一条都选不上时用。三个序号都要推一次，让在飞的响应落地时对不上号 */
  function close() {
    ++detailSeq;
    ++diffSeq;
    ++rowSeq;
    sha.value = null;
    row.value = null;
    rowError.value = null;
    detail.value = null;
    error.value = null;
    file.value = null;
    diff.value = null;
    diffError.value = null;
    loading.value = false;
    diffLoading.value = false;
  }

  return {
    sha,
    row,
    rowLoading,
    rowError,
    detail,
    loading,
    error,
    file,
    diff,
    diffLoading,
    diffError,
    ignoreWhiteSpace,
    open,
    pickFile,
    setIgnoreWhiteSpace,
    retryDiff,
    close,
  };
});
