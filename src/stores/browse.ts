import { ref, shallowRef } from "vue";
import { defineStore } from "pinia";
import { GitTidyError } from "@/api/client";
import {
  fileBlame,
  fileContent,
  fileHistory,
  fileTree,
  type Blame,
  type FileContent,
  type TreeEntry,
} from "@/api/file";
import type { Commit } from "@/api/commit";

function wrap(err: unknown): GitTidyError {
  return err instanceof GitTidyError ? err : new GitTidyError("unknown", String(err));
}

/** 历史最多列多少条文件改动。再多就翻页，这一版先只给一页 */
const HISTORY_LIMIT = 200;

/**
 * 「文件」页的状态：某个修订的文件树，选中一个文件后的三种看法（内容 / 归属 / 历史）。
 *
 * 三个请求各有一个序号：连用同一个的话，点「归属」会把还在飞的「内容」判成过期，
 * 而内容正是那个人点的文件在另一个视图里的东西。
 */
export const useBrowseStore = defineStore("browse", () => {
  const rev = ref("HEAD");
  const entries = shallowRef<TreeEntry[]>([]);
  const treeLoading = ref(false);
  const treeError = ref<GitTidyError | null>(null);

  const path = ref<string | null>(null);
  /** 选中的条目。子模块指针没有正文也没有归属，所以界面不给它开这两扇页签 */
  const entry = shallowRef<TreeEntry | null>(null);

  const content = shallowRef<FileContent | null>(null);
  const contentLoading = ref(false);
  const contentError = ref<GitTidyError | null>(null);

  const blame = shallowRef<Blame | null>(null);
  const blameLoading = ref(false);
  const blameError = ref<GitTidyError | null>(null);

  const history = shallowRef<Commit[]>([]);
  const historyLoading = ref(false);
  const historyError = ref<GitTidyError | null>(null);

  let treeSeq = 0;
  let contentSeq = 0;
  let blameSeq = 0;
  let historySeq = 0;

  async function loadTree(repoId: number) {
    const mine = ++treeSeq;
    treeLoading.value = true;
    treeError.value = null;
    try {
      const got = await fileTree(repoId, rev.value);
      if (mine !== treeSeq) return;
      entries.value = got;
      // 换修订后原来选中的文件可能不存在了：留着会让人在另一个修订里看同一个名字
      if (path.value !== null && !got.some((item) => item.path === path.value)) closeFile();
    } catch (err) {
      if (mine !== treeSeq) return;
      treeError.value = wrap(err);
    } finally {
      if (mine === treeSeq) treeLoading.value = false;
    }
  }

  function setRev(repoId: number, next: string) {
    rev.value = next;
    closeFile();
    return loadTree(repoId);
  }

  /**
   * 选一个文件。三种看法都取，但**只把用户正在看的那一种显示出来**：
   * 归属与历史各自可能几百 KB，全留在内存里不划算，而按需取又会让切页签等一次 IPC。
   * 取齐的代价换来的是切页签零等待——文件视图的切页签是高频动作。
   */
  async function openFile(repoId: number, item: TreeEntry) {
    ++contentSeq;
    ++blameSeq;
    ++historySeq;
    path.value = item.path;
    entry.value = item;
    content.value = null;
    contentError.value = null;
    blame.value = null;
    blameError.value = null;
    history.value = [];
    historyError.value = null;

    if (item.kind === "commit") {
      // 子模块指针：那一端的提交在本仓库里查不到，正文与归属都没有意义
      contentError.value = new GitTidyError("gitlink", "子模块指针没有可读的内容");
      return;
    }

    const mineContent = contentSeq;
    const mineBlame = blameSeq;
    const mineHistory = historySeq;
    contentLoading.value = true;
    blameLoading.value = true;
    historyLoading.value = true;
    const target = item.path;

    void fileContent(repoId, rev.value, target)
      .then((got) => {
        if (mineContent === contentSeq) content.value = got;
      })
      .catch((err: unknown) => {
        if (mineContent === contentSeq) contentError.value = wrap(err);
      })
      .finally(() => {
        if (mineContent === contentSeq) contentLoading.value = false;
      });

    void fileBlame(repoId, rev.value, target)
      .then((got) => {
        if (mineBlame === blameSeq) blame.value = got;
      })
      .catch((err: unknown) => {
        if (mineBlame === blameSeq) blameError.value = wrap(err);
      })
      .finally(() => {
        if (mineBlame === blameSeq) blameLoading.value = false;
      });

    void fileHistory(repoId, target, 0, HISTORY_LIMIT)
      .then((page) => {
        if (mineHistory === historySeq) history.value = page.commits;
      })
      .catch((err: unknown) => {
        if (mineHistory === historySeq) historyError.value = wrap(err);
      })
      .finally(() => {
        if (mineHistory === historySeq) historyLoading.value = false;
      });
  }

  function closeFile() {
    ++contentSeq;
    ++blameSeq;
    ++historySeq;
    path.value = null;
    entry.value = null;
    content.value = null;
    contentError.value = null;
    blame.value = null;
    blameError.value = null;
    history.value = [];
    historyError.value = null;
    contentLoading.value = false;
    blameLoading.value = false;
    historyLoading.value = false;
  }

  return {
    rev,
    entries,
    treeLoading,
    treeError,
    path,
    entry,
    content,
    contentLoading,
    contentError,
    blame,
    blameLoading,
    blameError,
    history,
    historyLoading,
    historyError,
    loadTree,
    setRev,
    openFile,
    closeFile,
  };
});