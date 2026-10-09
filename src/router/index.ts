import { createRouter, createWebHashHistory } from "vue-router";
import CommitsView from "@/views/CommitsView.vue";
import CommitView from "@/views/CommitView.vue";
import FilesView from "@/views/FilesView.vue";
import ReportView from "@/views/ReportView.vue";
import ChangelogView from "@/views/ChangelogView.vue";

/**
 * 用 hash 模式：Tauri 生产包走自定义协议加载本地文件，history 模式下深链刷新会命中
 * 不存在的资源路径。桌面端没有地址栏，用 hash 不损失任何体验。
 *
 * 仓库列表在常驻侧栏里，不再单独占一个路由——主区只保留 Fork 那两个页签。
 * 设置由左下角齿轮打开弹窗，也不占路由。
 */
export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: { name: "history" } },
    { path: "/history", name: "history", component: CommitsView },
    { path: "/files", name: "files", component: FilesView },
    { path: "/commit", name: "commit", component: CommitView },
    { path: "/report", name: "report", component: ReportView },
    { path: "/changelog", name: "changelog", component: ChangelogView },
  ],
});
