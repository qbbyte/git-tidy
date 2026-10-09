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

/**
 * 送进提示词的 diff 总预算（字符数）。
 *
 * 超了不是静默截断：压缩过程会在 `buildPrompt` 里明说，界面也会显示"已压缩"，
 * 因为模型只看到一部分这件事必须让人看见——否则它会对着半张 diff 编一个看起来很
 * 自信的提交说明。
 *
 * 注意它现在是**总预算**而不是"前 N 个字符"：`shrinkDiff` 会先把这笔钱按文件分，
 * 保证每个改了文件的都露头（以前的写法是硬切，第 3 个文件之后连文件名都进不了提示词）。
 */
export const MAX_DIFF_CHARS = 12000;

/**
 * 这一类文件的逐行内容对"写提交信息"没有价值，只会把额度吃光：
 * 锁文件是包管理器算出来的，产物是编译器吐出来的。它们仍然要出现在提示词里
 * （否则模型不知道这次还改了依赖），但只给文件名 + 增删统计。
 */
const NOISE_FILE = [
  /(^|\/)(package-lock\.json|npm-shrinkwrap\.json|yarn\.lock|pnpm-lock\.yaml|bun\.lockb?)$/,
  /(^|\/)(Cargo\.lock|Gemfile\.lock|composer\.lock|poetry\.lock|Pipfile\.lock|go\.sum|flake\.lock)$/,
  /\.min\.(js|css)$/,
  /\.(map|snap)$/,
  /(^|\/)(dist|build|out|coverage|target)\/.+$/,
];

/** 一次压缩的结果。字段是给界面用的：它得说清楚"有多少文件没给全、为什么"。 */
export interface ShrunkDiff {
  text: string;
  /** 送进模型的不是完整 diff（无论是因为切了正文，还是折叠了锁文件） */
  truncated: boolean;
  /** 正文被额度切短的文件数 */
  clippedFiles: number;
  /** 锁文件 / 生成物：正文整块折叠掉的文件数 */
  foldedFiles: number;
  /** 连文件头和统计都没放进去的文件数（几百个文件的极端情况） */
  skippedFiles: number;
}

/** 一个文件的段落：头部元信息（diff --git / index / --- / +++）+ hunk 正文 */
interface DiffChunk {
  path: string;
  head: string;
  body: string;
  noise: boolean;
}

function isNoiseFile(path: string): boolean {
  return NOISE_FILE.some((pattern) => pattern.test(path));
}

/**
 * 按 `diff --git` 把整片 diff 切成一个文件一段。
 *
 * 头部取到第一个 `@@` 之前而不是"前 4 行"：新增文件有 `new file mode`、重命名有
 * `rename from/to`、二进制只有一句 `Binary files differ`——行数都不一样，按行数切会
 * 把 `+++` 行切进正文里，统计就错了。
 */
function splitDiff(diff: string): DiffChunk[] {
  return diff
    .split(/^(?=diff --git )/m)
    .filter((block) => block.trim() !== "")
    .map((block) => {
      const lines = block.split("\n");
      const at = lines.findIndex((line) => line.startsWith("@@"));
      const head = (at === -1 ? lines : lines.slice(0, at)).join("\n");
      const body = at === -1 ? "" : lines.slice(at).join("\n");
      const named = block.match(/^diff --git a\/(.+?) b\/(.+)$/m);
      const path = named?.[2] ?? "(未知文件)";
      return { path, head, body, noise: isNoiseFile(path) };
    });
}

function countChanges(body: string) {
  let added = 0;
  let deleted = 0;
  let hunks = 0;
  for (const line of body.split("\n")) {
    if (line.startsWith("@@")) hunks += 1;
    else if (line.startsWith("+")) added += 1;
    else if (line.startsWith("-")) deleted += 1;
  }
  return { added, deleted, hunks };
}

/** 挂在每个文件头后面的一行说明：即使正文一个字都没给，模型也知道这个文件改了多少 */
function summaryLine(
  stat: { added: number; deleted: number; hunks: number },
  folded: boolean,
  clipped: boolean,
): string {
  const scale = `+${stat.added} −${stat.deleted}`;
  if (folded) return `（锁文件 / 生成物，${stat.hunks} 个 hunk 的正文已折叠：${scale}）`;
  if (clipped) return `（${stat.hunks} 个 hunk，${scale}；以下只保留了开头部分）`;
  return `（${stat.hunks} 个 hunk，${scale}）`;
}

/**
 * 把额度按"最大最小公平"分给各个文件：每人先按人头均分，吃饱的人把余量让出来。
 *
 * 这样做的理由是**覆盖优先于深度**：模型知道"这次还改了 RefPanel"远比"App.vue 的
 * 每一行都看到了"重要——前者决定 header 该不该带 scope、正文要不要分条。
 */
function fairShare(demand: number[], room: number): number[] {
  const grants = demand.map(() => 0);
  let left = room;
  let pending = demand.map((need, index) => (need > 0 ? index : -1)).filter((index) => index >= 0);
  while (pending.length > 0) {
    const share = Math.floor(left / pending.length);
    if (share <= 0) break;
    const hungry: number[] = [];
    for (const index of pending) {
      const give = Math.min(demand[index] - grants[index], share);
      grants[index] += give;
      left -= give;
      if (grants[index] < demand[index]) hungry.push(index);
    }
    pending = hungry;
  }
  return grants;
}

/**
 * 压缩送进模型的 diff：给每个文件留头 + 统计，正文按额度公平分配。
 *
 * 旧的写法是 `diff.slice(0, MAX_DIFF_CHARS)`——一刀切在第 N 个字符上，切点之后的文件
 * 连名字都进不了提示词，模型于是对着一小半改动写说明（实测一次 6 文件的提交里，
 * 后 4 个文件全部消失）。现在的顺序是：
 *
 * 1. 没超预算就原样返回（大多数提交走这条，提示词里不会有任何额外说明）；
 * 2. 超了：锁文件 / 生成物先折叠成"文件名 + 增删统计"；
 * 3. 剩下的额度按文件均分，饿的让给没吃饱的（`fairShare`）；
 * 4. 仍有文件放不下就连头部一起丢，并计入 `skippedFiles`。
 *
 * 任何一步动了内容，`truncated` 都为 true——界面据此提示用户，不静默。
 */
export function shrinkDiff(diff: string): ShrunkDiff {
  if (diff.length <= MAX_DIFF_CHARS) {
    return { text: diff, truncated: false, clippedFiles: 0, foldedFiles: 0, skippedFiles: 0 };
  }

  const chunks = splitDiff(diff);
  if (chunks.length === 0) {
    return {
      text: diff.slice(0, MAX_DIFF_CHARS),
      truncated: true,
      clippedFiles: 0,
      foldedFiles: 0,
      skippedFiles: 0,
    };
  }

  const stats = chunks.map((chunk) => countChanges(chunk.body));
  const demand = chunks.map((chunk) => (chunk.noise ? 0 : chunk.body.length));
  // 固定开销：每块的文件头 + 摘要行 + 3 个换行（块内两个、块间一个），且摘要行按**最长**
  // 的那种估（正文被切过 / 被折叠的版本）。估宽不估窄——窄了就会在组装时触发兜底截断，
  // 白白把一个正文还放得下的文件切掉。
  const fixedCost = chunks.reduce(
    (sum, chunk, index) =>
      sum + chunk.head.length + summaryLine(stats[index], chunk.noise, !chunk.noise).length + 3,
    0,
  );
  const grants = fairShare(demand, Math.max(0, MAX_DIFF_CHARS - fixedCost));

  const parts: string[] = [];
  let used = 0;
  let clippedFiles = 0;
  let foldedFiles = 0;
  let skippedFiles = 0;

  for (let index = 0; index < chunks.length; index++) {
    const chunk = chunks[index];
    const clipped = grants[index] < chunk.body.length;
    let piece = [
      chunk.head,
      summaryLine(stats[index], chunk.noise, clipped),
      grants[index] > 0 ? chunk.body.slice(0, grants[index]) : "",
    ]
      .filter((value) => value !== "")
      .join("\n");

    // 固定开销已经算进预算，正常路径不会走到下面这两个分支：只有病态输入才会——
    // 整片当成一个文件（没有 `diff --git` 段头）、或者文件多到光文件头就超预算。
    const room = MAX_DIFF_CHARS - used - 1;
    if (room <= 0) {
      skippedFiles += 1;
      continue;
    }
    let cut = clipped;
    if (piece.length > room) {
      piece = `${piece.slice(0, Math.max(0, room - 20))}\n…（已省略）`;
      cut = true;
    }

    parts.push(piece);
    used += piece.length + 1;
    if (chunk.noise) foldedFiles += 1;
    else if (cut) clippedFiles += 1;
  }

  if (skippedFiles > 0) {
    parts.push(`（另有 ${skippedFiles} 个文件的 diff 未纳入：整片改动超过预算太多）`);
  }

  return { text: parts.join("\n"), truncated: true, clippedFiles, foldedFiles, skippedFiles };
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
  const { text, truncated } = shrinkDiff(diff);
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
    ...(truncated
      ? [
          "",
          "注意：diff 过长已被截断。已按文件分配额度——每个文件都保留了文件头与「（N 个 hunk，+A −D）」统计，正文只保留了开头；锁文件与生成物只给统计、正文折叠。请据此判断改动范围，只描述你确实看到的内容，勿臆造。",
        ]
      : []),
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