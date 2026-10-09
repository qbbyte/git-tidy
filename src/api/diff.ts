import { call } from "@/api/client";

/**
 * 提交表单「AI 生成」需要的整片暂存区 diff（§AI）。
 *
 * 只取已暂存的内容，与「提交」按钮要落盘的范围一致。返回 git 原始文本，
 * 每个文件自带 `diff --git a/x b/x` 段头，直接喂给 LLM。空串 = 没有暂存改动。
 */
export function commitDiff(repoId: number) {
  return call<string>("commit_diff", { id: repoId });
}
