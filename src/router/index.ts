import { createRouter, createWebHashHistory } from "vue-router";
import RepoView from "@/views/RepoView.vue";

/**
 * 用 hash 模式：Tauri 生产包走自定义协议加载本地文件，history 模式下深链刷新会命中
 * 不存在的资源路径。桌面端没有地址栏，用 hash 不损失任何体验。
 */
export const router = createRouter({
  history: createWebHashHistory(),
  routes: [{ path: "/", name: "repo", component: RepoView }],
});
