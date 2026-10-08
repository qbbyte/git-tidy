import { onBeforeUnmount, ref, type Ref } from "vue";

/**
 * 可拖拽的分栏分隔条。
 *
 * 三处共用（历史页、文件页、侧栏），所以持久化与约束只在这里写一次。
 *
 * 存 `localStorage` 而不是需求文档 §7.19 说的 `preferences.json`：
 * 那一层还没实现（全仓 `preferences` 命中 0 次），为存两个数字提前把 M4 的活拉进来
 * 不划算。键名统一带 `git-tidy:` 前缀，将来迁走时按前缀筛出来就行。
 */

/** 存不进去不该拖垮界面：隐私模式与配额满都会抛 */
function readStored(key: string): number | null {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return null;
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function persist(key: string, value: number) {
  try {
    localStorage.setItem(key, String(value));
  } catch {
    // 存不下只是下次开窗口要重新拖，不值得提示用户
  }
}

/**
 * 把值夹到区间内。
 *
 * `min > max` 时返回中点：窗口太窄时，两个下限会互相越过（详情栏的下限反过来压过了
 * 列表栏的上限）。这时跳到 `min` 会把某一栏硬按到最小宽度，而它同时又超了另一栏的下限——
 * 都不满足。给中点是唯一两边都不越界的选法。
 */
export function clampValue(value: number, min: number, max: number): number {
  if (min > max) return (min + max) / 2;
  return Math.min(Math.max(value, min), max);
}

export interface PaneDividerOptions {
  /** 外层容器，分隔条按它算比例 */
  container: Ref<HTMLElement | null>;
  /** localStorage 键 */
  storageKey: string;
  /** 默认值 */
  fallback: number;
  min: number;
  max: number;
  /**
   * 值的单位。
   *
   * - `ratio`：占容器宽度的比例（0–1），容器一变宽绝对宽度跟着变——
   *   两栏都要看内容的历史页用这个；
   * - `px`：固定像素宽度，容器再宽也不变——侧栏与文件树用这个，
   *   它们是导航而不是内容，给它跟着窗口一起长会让它在宽屏上喧宾夺主。
   */
  unit: "ratio" | "px";
  /** 默认值单位下的宽度，拖到两个边界时至少留这么多 */
  containerMinWidth?: number;
}

export interface PaneLimit {
  /** 容器的下边界（比例 0–1 或像素） */
  lo: number;
  /** 容器的上边界 */
  hi: number;
}

/**
 * 根据容器宽度算这一栏能拖到的区间。
 *
 * 比例制下还要给**补集**留出 `containerMinWidth`：把列表栏拖到 90% 看着痛快，
 * 而详情栏只剩几十像素，diff 就没法看了。
 */
export function computePaneLimit(input: {
  containerWidth: number;
  unit: "ratio" | "px";
  min: number;
  max: number;
  containerMinWidth: number;
}): PaneLimit {
  // 像素制不随容器缩放，所以补集下限由调用方的 max 兜着，这里不参与
  if (input.unit === "px" || input.containerWidth <= 0) {
    return { lo: input.min, hi: input.max };
  }
  const share = input.containerMinWidth / input.containerWidth;
  return { lo: Math.max(input.min, share), hi: Math.min(input.max, 1 - share) };
}

export function usePaneDivider(options: PaneDividerOptions) {
  const { container, storageKey, min, max, unit } = options;
  const containerMinWidth = options.containerMinWidth ?? 360;

  const stored = readStored(storageKey);
  const size = ref(clampValue(stored ?? options.fallback, min, max));
  const dragging = ref(false);

  let detach: (() => void) | null = null;

  function onPointerDown(event: PointerEvent) {
    // 只认主键：右键 / 中键拖会变成别的交互，抢过来反而让人以为坏了
    if (event.button !== 0) return;
    const el = container.value;
    if (!el) return;

    dragging.value = true;
    // 拖的时候别选中文字，否则会连一片正文一起高亮
    const previousUserSelect = document.body.style.userSelect;
    document.body.style.userSelect = "none";

    const onMove = (move: PointerEvent) => {
      const rect = el.getBoundingClientRect();
      if (rect.width === 0) return;
      const offset = move.clientX - rect.left;
      const raw = unit === "px" ? offset : offset / rect.width;
      const { lo, hi } = computePaneLimit({
        containerWidth: rect.width,
        unit,
        min,
        max,
        containerMinWidth,
      });
      size.value = clampValue(raw, lo, hi);
    };

    const onUp = () => {
      dragging.value = false;
      persist(storageKey, size.value);
      document.body.style.userSelect = previousUserSelect;
      detach?.();
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    // 中途窗口失焦时 pointerup 可能收不到，不 detach 就会一直跟着鼠标变
    window.addEventListener("blur", onUp);

    detach = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("blur", onUp);
      detach = null;
    };
  }

  /** 双击回到默认——拖乱了之后总得有一条回去的路 */
  function reset() {
    size.value = clampValue(options.fallback, min, max);
    persist(storageKey, size.value);
  }

  onBeforeUnmount(() => detach?.());

  return { size, dragging, onPointerDown, reset };
}