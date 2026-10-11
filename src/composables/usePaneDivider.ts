import { computed, onBeforeUnmount, ref, watch, type Ref } from "vue";

/**
 * 可拖拽的分栏分隔条。
 *
 * 四处共用（侧栏、提交页、历史页、文件页），所以持久化与约束只在这里写一次。
 *
 * 存 `localStorage` 而不是 `preferences.json`：
 * 那一层现在是实现好的，但它管的是拉取策略与列表列显示这类个人偏好，分栏宽度不算；
 * 键名统一带 `git-tidy:` 前缀，将来要迁走时按前缀筛出来就行。
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
  /**
   * 用户自己拖出来的宽度。
   *
   * **它是"偏好"，不是"当前生效值"**（见 `size`）：拖拽与 `reset` 只改它，也只把它落盘。
   */
  const preferred = ref(clampValue(stored ?? options.fallback, min, max));
  const dragging = ref(false);
  /** 容器当前的宽度。0 = 还没量到（挂载之前） */
  const containerWidth = ref(0);

  /**
   * 这一栏当前实际占多宽：把 `preferred` 按容器算出来的区间夹一次。
   *
   * 与 `preferred` 分开是必须的。把夹取结果写回偏好，窗口一缩就永久改掉了用户拖出来的
   * 宽度，窗口恢复之后也回不去。分开之后：缩小时自动让位，放大时自动还原。
   *
   * 注意**像素制在这里并不会随容器缩小**（`computePaneLimit` 对 px 返回静态区间）：
   * 那几个栏的补集都是 `flex: 1`，拥挤时是该由补集让位，而不是把导航栏压窄。
   */
  const size = computed(() => {
    const { lo, hi } = computePaneLimit({
      containerWidth: containerWidth.value,
      unit,
      min,
      max,
      containerMinWidth,
    });
    return clampValue(preferred.value, lo, hi);
  });

  let detach: (() => void) | null = null;
  let observer: ResizeObserver | null = null;

  /**
   * 盯着容器尺寸。
   *
   * 原来只在拖拽时按容器算区间，于是有一个很难联想到的现象：在大窗口下拖出来的宽度是
   * 合法的，把窗口缩小之后同一个宽度就越界了——比例制会压垮补集（历史页的详情栏被挤到
   * 200px 以下），而没有任何东西会把它拉回来，除非用户想起来再拖一次。
   *
   * 顺带把量到的宽度暴露出去：有的页面要靠它决定"这个宽度还摆不摆得下三栏"。
   */
  watch(
    container,
    (el) => {
      observer?.disconnect();
      observer = null;
      if (el === null) return;
      const measure = () => {
        // 与拖拽那条路取同一个量（border box）：两处差一个 padding 就会互相打架
        containerWidth.value = el.getBoundingClientRect().width;
      };
      observer = new ResizeObserver(measure);
      observer.observe(el);
      measure();
    },
    { immediate: true },
  );

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
      preferred.value = clampValue(raw, lo, hi);
    };

    const onUp = () => {
      dragging.value = false;
      persist(storageKey, preferred.value);
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
    preferred.value = clampValue(options.fallback, min, max);
    persist(storageKey, preferred.value);
  }

  onBeforeUnmount(() => {
    detach?.();
    observer?.disconnect();
  });

  return { size, dragging, containerWidth, onPointerDown, reset };
}