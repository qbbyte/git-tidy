import { call } from "@/api/client";

/**
 * 检查更新（需求 7.24 的检查部分）。
 *
 * **只查不装**：这里拿到"有没有新版本、在哪下"，下载仍然是用户自己在 GitHub 上点。
 * 不做自动下载替换，是因为替换正在运行的程序是整件事里最脏的一段，
 * 而 Git 客户端不是天天更新的软件。
 *
 * 请求为什么在前端发：Rust 侧没有 HTTP 客户端依赖，为一次 GET 引入一个（含 TLS）
 * 不划算。这个 GET 与本应用其它"打开一个网址"是同一性质，出网面就一个 URL，
 * 而且能在设置里完全关掉。
 */
export interface UpdateEndpoint {
  latestReleaseUrl: string;
  releasesPageUrl: string;
  currentVersion: string;
}

export interface ReleaseInfo {
  version: string;
  url: string;
  name: string;
}

export interface CheckResult {
  current: string;
  repo: string;
  latest: ReleaseInfo | null;
  available: boolean;
}

export function updateEndpoint() {
  return call<UpdateEndpoint>("update_endpoint", {});
}

/** 启动时该不该查：偏好里关掉了就不发这个请求。决定只有一处知道（Rust 侧） */
export function updateCheckOnStartup() {
  return call<boolean>("update_check_on_startup", {});
}

/**
 * 查一次新版本。
 *
 * 超时是必须的：启动路径上的网络请求没有上限的话，断网能让应用在启动时干等几十秒。
 * 3 秒连接 / 8 秒读取足够一次普通的 API 请求。
 */
export async function checkForUpdate(): Promise<CheckResult> {
  const endpoint = await updateEndpoint();
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 8000);
  try {
    const response = await fetch(endpoint.latestReleaseUrl, {
      signal: controller.signal,
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) {
      // 403 多半是触到 GitHub 的匿名请求限额（60 次/小时/IP），说清是哪一类
      throw new Error(
        response.status === 403
          ? "GitHub 拒绝了这次请求（多半是匿名请求次数用满了，过一会儿再试）"
          : `GitHub 返回 ${response.status}`,
      );
    }
    // 判定在 Rust 侧：字段怎么读、版本怎么比都在那里，且有测试守着
    return await call<CheckResult>("update_compare", { payload: await response.text() });
  } catch (err) {
    if (err instanceof Error && err.name === "AbortError") {
      throw new Error("连不上 GitHub（8 秒没回），可能网络不通");
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}