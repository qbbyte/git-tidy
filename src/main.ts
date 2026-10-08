import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import { router } from "@/router";
import { BASE_CSS } from "@/styles/base";

// 全局基线（字体继承、滚动条、焦点环）必须在 mount 之前注入：
// 晚一帧的话首屏会先按浏览器默认字体渲染一下再跳变，那一下闪动比不注入更难看。
// token 表由 BASE_CSS 同一模块里的 installTokens() 铺成 CSS 变量。
const baseStyle = document.createElement("style");
baseStyle.textContent = BASE_CSS;
document.head.appendChild(baseStyle);

createApp(App).use(createPinia()).use(router).mount("#app");
