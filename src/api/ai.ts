import { commitDiff } from "@/api/diff";
import {
  buildPrompt,
  normalizeEndpoint,
  normalizeType,
  parseCommitMessage,
  redactKey,
  validateConfig,
  type AiConfig,
  type GeneratedCommit,
} from "@/lib/ai";

export type { AiConfig, GeneratedCommit } from "@/lib/ai";

/**
 * AI 生成提交信息的网络出口。
 *
 * 请求在前端 `fetch`，不进 Rust 侧：为一个 POST 引入含 TLS 的 HTTP 客户端不划算，
 * 而且这个请求里没有我们必须保密的 git 数据——它本来就要把 diff 发给用户自己配的
 * 那个模型服务。代价是 API Key 过一道 WebView，所以 Key 存在哪、谁能看到它，
 * 在设置页与 README 里都写明了。
 *
 * 不依赖各家服务特有的 `response_format`（本地 Ollama 就不一定支持），改成要求模型
 * 按纯文本返回、前端用正则解析：任何 OpenAI 兼容服务都能用，代价是多一层解析逻辑
 * （`lib/ai.ts`，由 `npm run check:ai` 守着）。
 */

/** 一次请求的超时。取 60 秒：本地小模型慢，但让用户对着表单干等更久没有意义。 */
const REQUEST_TIMEOUT_MS = 60_000;

/** 真正发请求。带上 diff 与 type 白名单，返回可直接填进表单的一组字段。 */
export async function requestCommitMessage(
  config: AiConfig,
  diff: string,
  allowedTypes: string[],
): Promise<GeneratedCommit> {
  // 校验放在发请求的地方而不只在按钮上：任何调用方都必须自己拒「没填全」，
  // 否则换一个入口就会拿着空 key 出门。规则只有一份（lib/ai.ts）。
  const invalid = validateConfig(config);
  if (invalid) throw new Error(invalid);

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);

  let response: Response;
  try {
    response = await fetch(normalizeEndpoint(config.endpoint), {
      method: "POST",
      signal: controller.signal,
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${config.apiKey.trim()}`,
      },
      body: JSON.stringify({
        model: config.model.trim(),
        temperature: 0.2,
        messages: [{ role: "user", content: buildPrompt(diff, allowedTypes) }],
      }),
    });
  } catch (err) {
    if (err instanceof DOMException && err.name === "AbortError") {
      throw new Error(`AI 请求超时（${REQUEST_TIMEOUT_MS / 1000} 秒未返回）`);
    }
    const reason = err instanceof Error ? err.message : String(err);
    throw new Error(`调用 AI 失败：${redactKey(reason, config)}`);
  } finally {
    clearTimeout(timer);
  }

  if (!response.ok) {
    const detail = await response.text().catch(() => "");
    throw new Error(`AI 返回 ${response.status}：${redactKey(detail, config).slice(0, 200)}`);
  }

  const data = (await response.json().catch(() => null)) as {
    choices?: { message?: { content?: string } }[];
  } | null;
  const content = data?.choices?.[0]?.message?.content?.trim();
  if (!content) {
    throw new Error("AI 返回为空，或响应不是 OpenAI 兼容格式，请检查 Base URL 与模型名");
  }

  const parsed = parseCommitMessage(content);
  parsed.type = normalizeType(parsed.type, allowedTypes);
  return parsed;
}

/**
 * 取待生成用的暂存区 diff。
 *
 * 只取已暂存内容（`--cached`），与提交按钮要落盘的范围完全一致：模型不该根据还没
 * `git add` 的草稿编造提交说明。空暂存区在这里就报错，不去问模型。
 */
export async function stagedDiff(repoId: number): Promise<string> {
  const diff = (await commitDiff(repoId)).trim();
  if (!diff) {
    throw new Error("暂存区没有改动：先 git add 要提交的文件，再让 AI 生成");
  }
  return diff;
}