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

  function makeHandle(key: string) {
    return (event: PointerEvent) => {
      if (event.button !== 0) return;
      event.preventDefault();
      event.stopPropagation();
      const startX = event.clientX;
      const startWidth = widths[key] ?? 0;
      let moved = false;
      const prev = document.body.style.userSelect;
      document.body.style.userSelect = "none";
      const onMove = (m: PointerEvent) => {
        moved = true;
        const spec = specs.find((s) => s.key === key);
        if (!spec || spec.flex) return;
        widths[key] = Math.max(spec.minWidth, startWidth + (m.clientX - startX));
      };
      const onUp = () => {
        document.body.style.userSelect = prev;
        if (moved) persist(`${group}:${key}`, widths[key]);
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("blur", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("blur", onUp);
    };
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

  return { widths, resize, makeHandle, resetColumn, gridTemplate };
}
