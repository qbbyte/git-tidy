import { reactive } from "vue";

export interface ColumnSpec {
  key: string;
  defaultWidth: number;
  minWidth: number;
  flex?: boolean;
}

const STORAGE_PREFIX = "git-tidy:col:";

function readStored(key: string): number | null {
  try {
    const raw = localStorage.getItem(STORAGE_PREFIX + key);
    if (raw === null) return null;
    const n = Number(raw);
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

function persist(key: string, value: number) {
  try {
    localStorage.setItem(STORAGE_PREFIX + key, String(Math.round(value)));
  } catch {
    // 存不下只是下次要重新拖
  }
}

/** 列宽拖拽调整。宽度持久化到 localStorage。 */
export function useColumnResize(group: string, specs: ColumnSpec[]) {
  const widths = reactive<Record<string, number>>({});
  for (const spec of specs) {
    if (spec.flex) continue;
    widths[spec.key] = readStored(`${group}:${spec.key}`) ?? spec.defaultWidth;
  }

  function resize(key: string, delta: number) {
    const spec = specs.find((s) => s.key === key);
    if (!spec || spec.flex) return;
    widths[key] = Math.max(spec.minWidth, widths[key] + delta);
  }

  /**
   * 直接作为模板的 pointerdown 处理器使用。
   *
   * 不能再返回一个函数让模板写成 `@pointerdown="makeHandle('refs')"`：
   * Vue 只会执行等号右边的表达式，不会自动调用它返回的函数，结果就是拖拽从未开始。
   */
  function startResize(event: PointerEvent, key: string, direction = 1) {
    if (event.button !== 0) return;
    const spec = specs.find((s) => s.key === key);
    if (!spec || spec.flex) return;

    event.preventDefault();
    event.stopPropagation();

    const startX = event.clientX;
    const startWidth = widths[key] ?? spec.defaultWidth;
    const pointerId = event.pointerId;
    const target = event.currentTarget as HTMLElement | null;
    const previousUserSelect = document.body.style.userSelect;
    const previousCursor = document.body.style.cursor;
    let moved = false;
    let finished = false;

    document.body.style.userSelect = "none";
    document.body.style.cursor = "col-resize";

    // 指针捕获负责把后续事件继续送到手柄；window 监听则覆盖元素被重渲染替换的情况。
    try {
      target?.setPointerCapture(pointerId);
    } catch {
      // 某些 WebView 在元素刚挂载时可能拒绝捕获，window 监听仍然可以完成拖拽
    }

    const onMove = (move: PointerEvent) => {
      moved = true;
      widths[key] = Math.max(
        spec.minWidth,
        startWidth + direction * (move.clientX - startX),
      );
    };

    const finish = () => {
      if (finished) return;
      finished = true;
      document.body.style.userSelect = previousUserSelect;
      document.body.style.cursor = previousCursor;
      try {
        if (target?.hasPointerCapture(pointerId)) target.releasePointerCapture(pointerId);
      } catch {
        // 捕获可能已经被浏览器自动释放，不影响结束逻辑
      }
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
      window.removeEventListener("blur", finish);
      target?.removeEventListener("lostpointercapture", finish);
      if (moved) persist(`${group}:${key}`, widths[key]);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    window.addEventListener("blur", finish);
    target?.addEventListener("lostpointercapture", finish);
  }

  function resetColumn(key: string) {
    const spec = specs.find((s) => s.key === key);
    if (!spec || spec.flex) return;
    widths[key] = spec.defaultWidth;
    persist(`${group}:${key}`, spec.defaultWidth);
  }

  function gridTemplate(): string {
    return specs.map((s) => (s.flex ? "1fr" : `${widths[s.key] ?? s.defaultWidth}px`)).join(" ");
  }

  return { widths, resize, startResize, resetColumn, gridTemplate };
}
