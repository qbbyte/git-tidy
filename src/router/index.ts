import { createRouter, createWebHashHistory } from "vue-router";

/**
 * 用 hash 模式：Tauri 生产包走自定义协议加载本地文件，history 模式下深链刷新会命中
 * 不存在的资源路径。桌面端没有地址栏，用 hash 不损失任何体验。
 *
 * 仓库列表在常驻侧栏里，不再单独占一个路由——主区只保留 Fork 那两个页签。
 * 设置由左下角齿轮打开弹窗，也不占路由。
 */
// 路由懒加载：每个页面打成独立 chunk，首屏只下载当前页的代码。
// 994KB 单包拆成 6 块后，初始加载量降到 ~400KB，其余页面按需加载。
export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: { name: "history" } },
    { path: "/history", name: "history", component: () => import("@/views/CommitsView.vue") },
    { path: "/files", name: "files", component: () => import("@/views/FilesView.vue") },
    { path: "/commit", name: "commit", component: () => import("@/views/CommitView.vue") },
    { path: "/report", name: "report", component: () => import("@/views/ReportView.vue") },
    { path: "/changelog", name: "changelog", component: () => import("@/views/ChangelogView.vue") },
    { path: "/branches", name: "branches", component: () => import("@/views/BranchesView.vue") },
  ],
});
