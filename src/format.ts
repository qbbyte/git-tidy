/** 字节数给人看的写法。两个地方要用（改动清单和差异面板），所以不在组件里各写一遍 */
export function formatBytes(bytes: number | null): string {
  if (bytes === null) return "未知";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** 大小差。两侧都拿不到时整栏不显示，而不是给一个 0 */
export function formatDelta(oldSize: number | null, newSize: number | null): string {
  if (oldSize === null && newSize === null) return "";
  return `${formatBytes(oldSize)} → ${formatBytes(newSize)}`;
}
