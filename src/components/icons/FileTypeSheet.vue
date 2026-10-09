<!--
  文件类型记号（自绘）。

  **为什么不从图标库取**：这一族是刻意设计的，不是通用字形——
  `JS`/`TS`/`RS` 是彩色字母（字母本身就说清了类型），其余是矢量图形
  （`<>` / `{}` / `#` / `_>` / 相框 / 文档页）。九种记号按色相排开、互不重样，
  配色按 WCAG 小字标准（9px 粗体算小字，要 4.5:1）逐个量过，最差的也有 4.57:1。

  换成通用图标库等于把这套推导丢掉：形状不再是"类型的一部分"，颜色也得改成单色。

  形状集中在这里定义一次（`<symbol>` + `<use>`），使用方 FileTree 只引用。
  **标准 UI 字形（齿轮、刷新、折叠……）不走这里**，走 `@vicons/tabler` + `NIcon`：
  那是通用字形，手画只会不准、不一致。
-->
<template>
    <svg class="sheet" aria-hidden="true">
      <defs>
        <symbol id="fi-none" viewBox="0 0 16 16">
          <path
            d="M4.6 2.4h4.3l2.6 2.6v7.7c0 .5-.4.9-.9.9H4.6c-.5 0-.9-.4-.9-.9V3.3c0-.5.4-.9.9-.9Z"
            fill="none"
            stroke="currentColor"
            stroke-width="1.15"
            stroke-linejoin="round"
          />
          <path
            d="M8.9 2.4v2.6h2.6"
            fill="none"
            stroke="currentColor"
            stroke-width="1.15"
            stroke-linejoin="round"
          />
        </symbol>

        <!-- Vue 的 V：外层实心三角、内层按 evenodd 挖空，不依赖行底色 -->
        <symbol id="fi-vue" viewBox="0 0 16 16">
          <path
            fill="currentColor"
            fill-rule="evenodd"
            d="M1.3 3.1h3.5L8 9.5l3.2-6.4h3.5L8 14.1zM4.8 3.1h2.1L8 5.9l1.1-2.8h2.1L8 9.5z"
          />
        </symbol>

        <symbol id="fi-markup" viewBox="0 0 16 16">
          <g
            fill="none"
            stroke="currentColor"
            stroke-width="1.35"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M5.7 4.9 2.6 8l3.1 3.1" />
            <path d="M10.3 4.9 13.4 8l-3.1 3.1" />
          </g>
        </symbol>

        <symbol id="fi-data" viewBox="0 0 16 16">
          <g
            fill="none"
            stroke="currentColor"
            stroke-width="1.3"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path
              d="M6.8 3.7c-1.2 0-1.9.7-1.9 1.9v1.2c0 1.1-.6 1.6-1.6 1.6 1 0 1.6.5 1.6 1.6v1.2c0 1.2.7 1.9 1.9 1.9"
            />
            <path
              d="M9.2 3.7c1.2 0 1.9.7 1.9 1.9v1.2c0 1.1.6 1.6 1.6 1.6-1 0-1.6.5-1.6 1.6v1.2c0 1.2-.7 1.9-1.9 1.9"
            />
          </g>
        </symbol>

        <symbol id="fi-doc" viewBox="0 0 16 16">
          <path
            d="M4.1 1.9h4.5l2.9 2.9v8.2c0 .6-.4 1-1 1H4.1c-.6 0-1-.4-1-1V2.9c0-.6.4-1 1-1Z"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linejoin="round"
          />
          <path
            d="M8.4 1.9v2.9h2.9"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linejoin="round"
          />
          <g fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round">
            <path d="M5.2 8.5h5.4" />
            <path d="M5.2 10.7h3.3" />
          </g>
        </symbol>

        <symbol id="fi-style" viewBox="0 0 16 16">
          <g fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round">
            <path d="M6.3 3.4 5.2 12.6" />
            <path d="M10.8 3.4 9.7 12.6" />
            <path d="M3.6 6.2h9" />
            <path d="M3.1 9.8h9" />
          </g>
        </symbol>

        <symbol id="fi-shell" viewBox="0 0 16 16">
          <g
            fill="none"
            stroke="currentColor"
            stroke-width="1.35"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M3.4 4.9 6.5 8l-3.1 3.1" />
            <path d="M7.9 11.1h4.7" />
          </g>
        </symbol>

        <symbol id="fi-image" viewBox="0 0 16 16">
          <rect
            x="2.2"
            y="3.1"
            width="11.6"
            height="9.8"
            rx="1.7"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
          />
          <path
            d="M3.4 11.5 6.5 8.4l1.9 1.9 1.5-1.5 2.7 2.7"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
          <circle cx="6" cy="6.4" r="0.95" fill="currentColor" />
        </symbol>
      </defs>
    </svg>
</template>

<style scoped>
/* 零尺寸的隐藏 sprite：不占布局，只作为 <use> 的定义处 */
.sheet {
  position: absolute;
  width: 0;
  height: 0;
}
</style>
