/**
 * 全局基线样式。
 *
 * 只放「不属于任何组件」的规则：字体继承、滚动条、选中文本。
 * 颜色一律走 tokens.ts 铺出来的 CSS 变量——**这里不许出现字面色值**。
 */
import { FONT_MONO, FONT_UI, installTokens } from "./tokens";

// 必须在 mount 之前铺好，见 installTokens 的注释
installTokens();

export { FONT_MONO, FONT_UI };

/**
 * 字体挂在这里，而不是只在 token 表里定义。
 *
 * 之前全仓只有代码区设了 font-family，正文（提交列表的 subject/author、
 * 顶栏、侧栏、所有徽标）全落在浏览器默认值上——Windows 上是 Times New Roman。
 * Naive UI 的组件有自己的字体栈，所以界面上是两种字体并存。
 */
export const BASE_CSS = `
:root {
  font-family: ${FONT_UI};
  /* 15px 是 macOS 桌面应用的基准字号。中文在小字号下发糊，14px 看着比数字大一号 */
  font-size: 15px;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  text-rendering: optimizeLegibility;
}

/* Naive UI 的 reset 不会碰到原生元素的字体，而页面上原生元素很多 */
body, button, input, select, textarea {
  font-family: inherit;
}

/* 桌面应用不该有整页滚动条：滚动只属于各个面板自己。
   关掉它，任何一个页面板写错高度都只会缩掉自己那一块，不会把整窗拖走。 */
html, body {
  overflow: hidden;
}

html, body, #app {
  height: 100%;
  margin: 0;
}

/* 滚动条：Windows 默认那根 17px 的宽条在密集列表里占掉一行宽度，
   而浅灰底的滚动条比内容还显眼。这里收窄成 mac 那种细条。 */
::-webkit-scrollbar {
  width: 10px;
  height: 10px;
}
::-webkit-scrollbar-track {
  background: transparent;
}
::-webkit-scrollbar-thumb {
  background: var(--border-strong);
  border: 2px solid transparent;
  background-clip: content-box;
  border-radius: 6px;
}
::-webkit-scrollbar-thumb:hover {
  background: var(--text-3);
  background-clip: content-box;
}

/* 选中态跟随强调色。用默认的蓝会和你选中的行（--surface-selected）打架 */
::selection {
  background: var(--surface-selected);
  color: var(--text-1);
}

/* 键盘可达性：Naive 的 focus 样式够用，但裸 div 上 tabindex 的焦点环没有，
   补一条全局兜底——没有焦点环等于键盘用户看不见自己在哪 */
:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
`;