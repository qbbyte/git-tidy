/**
 * 提交页三栏的宽度预算，以及外壳（侧栏 + 主区）为它留出来的位置。
 *
 * **这个模块存在的唯一理由，是让"阈值"和"CSS 里那几个宽度"不会各走各的。**
 * CSS 没法 import 常量，所以那几个数必然在 `CommitView.vue` / `App.vue` 的
 * `<style>` 里也各写一份；而它们一旦走偏，症状是"某个窗口宽度下刚好溢出、出现一条
 * 横向滚动条"——几乎不会有人联想到是这里。`npm run check:layout` 会把两份逐个比对。
 *
 * 单位都是 CSS 像素。标了「脚本比对」的那些，值必须与对应的 CSS 声明一致。
 */

export const COMMIT_LAYOUT = {
  /** 清单栏的默认宽度。不参与任何算式，只是"打开时多宽" */
  listDefault: 320,
  /** 清单栏能拖到的最窄。**不参与**最小宽度算式（算式取的是上限那一侧） */
  listMin: 200,
  /** 清单栏能拖到的最宽。脚本比对：`usePaneDivider` 的 max 直接用它，CSS 里没有对应值 */
  listMax: 380,
  /** 分隔条本体。脚本比对 `.divider { width }` */
  dividerWidth: 3,
  /** 分隔条单侧外边距。脚本比对 `.divider { margin }` */
  dividerMargin: 7,
  /** 三栏并排时差异栏的下限。脚本比对 `.pane:not(.narrow) .preview-pane { min-width }` */
  diffMin: 240,
  /** 窄布局下差异栏独占左区时的下限。脚本比对 `.pane.narrow .preview-pane { min-width }` */
  diffMinNarrow: 300,
  /** 表单栏下限：type / scope 两个下拉并排还铺得开。脚本比对 `.form-column { min-width }` */
  formMin: 290,
  /** 表单栏的首选宽度。不参与下限算式，脚本比对 `.form-column { flex }` 里的 basis */
  formBasis: 430,
} as const;

/** 分隔条连带两侧外边距占掉的宽度 */
export const DIVIDER_TOTAL = COMMIT_LAYOUT.dividerWidth + COMMIT_LAYOUT.dividerMargin * 2;

/**
 * 三栏并排放得下所需的最小容器宽度。
 *
 * 取**上限那一侧**算：清单栏拖到最宽时最容易溢出，所以用 `listMax` 而不是 `listMin`。
 * 380 + 17 + 240 + 290 = 927 —— 容器窄于它就是窄布局。
 */
export const COMMIT_THREE_PANE_MIN =
  COMMIT_LAYOUT.listMax + DIVIDER_TOTAL + COMMIT_LAYOUT.diffMin + COMMIT_LAYOUT.formMin;
