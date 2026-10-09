/**
 * AI 提交信息生成的纯逻辑：配置校验、endpoint 归一、提示词拼装、返回文本解析、type 收敛。
 *
 * 放在 `lib/` 而不是 `api/` 是为了能脱离应用单测——`scripts/check-ai.mjs` 直接 import
 * 这个文件，网络与 IPC 都不沾。提示词和解析是整个功能里最容易悄悄坏掉的两段
 * （模型改口风就解析不出来），必须能脱离界面验证。
 */

/** 用户可填的 OpenAI 兼容接入配置，落在 preferences.json（见 `store/prefs.rs`）。 */
export interface AiConfig {
  /** `/chat/completions` 的基址。可填 `https://host/v1`，也可直接粘完整 URL */
  endpoint: string;
  apiKey: string;
  model: string;
}

/** 一次生成的结果。字段与提交表单一一对应，填回时不需要再翻译。 */
export interface GeneratedCommit {
  /** 不在仓库白名单内（或模型没给）时留 null，由用户手选，不硬塞一个不合规的 type */
  type: string | null;
  scope: string;
  description: string;
  body: string;
  /** 模型返回的原文，用于在弹窗里原样展示与"保留以便排查" */
  raw: string;
}

/**
 * 送进提示词的 diff 上限。
 *
 * 超了不是静默截断：截断会在 `buildPrompt` 里明说，界面也会显示"已截断"，因为
 * 模型只看到前 N 字这件事必须让人看见——否则它会对着半张 diff 编一个看起来很自信的
 * 提交说明。
 */
export const MAX_DIFF_CHARS = 12000;

/** 校验三件套是否填全。返回错误文案，OK 时返回 null。 */
export function validateConfig(config: AiConfig): string | null {
  if (!config.endpoint.trim()) return "AI 未配置：请先在设置里填写 Base URL";
  // 协议只放行 http/https：本地 Ollama 是 http，云端服务是 https，都在里面；
  // 而 `file://` 这类不该出现在一个"往哪发请求"的输入框里
  if (!/^https?:\/\//i.test(config.endpoint.trim())) {
    return "Base URL 必须以 http:// 或 https:// 开头";
  }
  if (!config.apiKey.trim()) return "AI 未配置：请先在设置里填写 API Key";
  if (!config.model.trim()) return "AI 未配置：请先在设置里填写模型名";
  return null;
}

/**
 * 把用户填的 endpoint 归一成"实际请求的 URL"。
 *
 * 两种填法都得认：填到 `/v1` 就补 `/chat/completions`；整条 URL 粘进来就原样用。
 * 用户从各家文档里复制粘贴，粘到哪一步全凭运气，让他自己判断后缀属于我们的错。
 */
export function normalizeEndpoint(raw: string): string {
  const endpoint = raw.trim().replace(/\/+$/, "");
  if (/\/chat\/completions$/i.test(endpoint)) return endpoint;
  return `${endpoint}/chat/completions`;
}

/**
 * 抹掉错误文本里可能出现的 key。
 *
 * 有些网关在 4xx 的响应体里原样回显请求头，那段文本会一路走到界面上，而界面上还带着
 * 用户刚填的明文 key。报错文案会被截图、被粘进 issue，所以这里默认抹掉。
 */
export function redactKey(text: string, config: AiConfig): string {
  const key = config.apiKey.trim();
  if (!key) return text;
  return text.split(key).join("***");
}

/** 截断 diff，并如实报告截了多少——不报告就会让人以为模型看全了。 */
export function truncateDiff(diff: string): { text: string; truncated: boolean } {
  if (diff.length <= MAX_DIFF_CHARS) return { text: diff, truncated: false };
  return { text: diff.slice(0, MAX_DIFF_CHARS), truncated: true };
}

/**
 * 拼提示词。
 *
 * 约束写进提示词而不是指望前端修：type 白名单来自仓库规范，模型不按它来就会给出
 * 一个表单当场拦下的 type，那次往返就白费了。
 */
export function buildPrompt(diff: string, allowedTypes: string[]): string {
  const allowed =
    allowedTypes.length > 0 ? allowedTypes.join("、") : "（本仓库未限制，按惯例选）";
  const { text, truncated } = truncateDiff(diff);
  return [
    "你是一个 Git 提交信息助手。根据下面的 git 暂存区 diff，生成一条 Conventional Commits 风格的提交信息。",
    "",
    "要求：",
    "- 第一行是 header，形如 `<type>(<scope 可选>): <描述>`，整行尽量不超过 72 字符；",
    `- type 必须且只能从下面列表里选一个（不要自创）：${allowed}`,
    "- scope 可选；只填单个模块名，否则连括号都别写；",
    "- 描述用简体中文，只讲「做了什么」，不要写原因、背景或影响；越简短越好；",
    "- 需要正文时用有序列表分条：每行以 `1. ` `2. ` …依次编号，一条一个要点，每条一行且不超过 72 字符；",
    "- 正文只在有多个独立改动时才写，最多 3 条；只有一个改动就完全省略正文；",
    "- 只输出提交信息本身，不要加引号、不要解释、不要 markdown 代码块；",
    ...(truncated ? ["", "注意：diff 过长已被截断，只依据以上部分描述，勿臆造未见过的改动。"] : []),
    "",
    "以下是 git diff：",
    "```",
    text,
    "```",
  ].join("\n");
}

/**
 * 解析模型返回的纯文本提交信息。
 *
 * 约定第一行是 header，其余是正文。解析不出的退化策略：整段当 description、type 留空，
 * 让用户手选——宁可少给一个字段，也不要把模型已经写好的内容丢掉重来。
 */
export function parseCommitMessage(text: string): GeneratedCommit {
  const raw = text.trim();
  if (!raw) return { type: null, scope: "", description: "", body: "", raw };

  // 模型很爱把内容包在 ``` 里，先剥掉，否则整段会被当成 description
  const lines = stripFence(raw).split("\n");
  const first = (lines.shift() ?? "").trim();
  const header = first.match(/^([\w-]+)(?:\(([^)]*)\))?:\s*(.+)$/);
  if (!header) return { type: null, scope: "", description: raw, body: "", raw };

  return {
    type: header[1].toLowerCase(),
    scope: (header[2] ?? "").trim(),
    description: header[3].trim(),
    body: lines.join("\n").trim(),
    raw,
  };
}

function stripFence(text: string): string {
  const fenced = text.match(/^```[^\n]*\n([\s\S]*?)\n?```$/);
  return fenced ? fenced[1] : text;
}

/**
 * 把模型给的 type 收敛到仓库白名单内。匹配不上返回 null，不猜。
 *
 * 前缀匹配是为了兜住白名单与模型词表的常见错位（`feat` ↔ `feature`、`fix` ↔ `bugfix`）。
 * 真有歧义时（白名单同时含 `feat` 和 `feature`）第一个命中即止，宁可让用户手选。
 */
export function normalizeType(type: string | null, allowed: string[]): string | null {
  if (!type) return null;
  // 统一小写再比：模型爱写 `Feat`，白名单里是 `feat`
  const lower = type.trim().toLowerCase();
  if (allowed.length === 0) return lower;
  if (allowed.includes(lower)) return lower;
  const byPrefix = allowed.find((value) => value.startsWith(lower) || lower.startsWith(value));
  return byPrefix ?? null;
}