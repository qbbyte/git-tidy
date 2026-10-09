// AI 提交信息生成：提示词与解析是这个功能里最容易悄悄坏掉的两段——
// 模型换个说法、裹一层 markdown 代码块、或者 type 词表与仓库白名单错位，
// 结果都是「界面上看不出哪里错了，只看到提交说明变得莫名其妙」。
//
// 所以这里逐条钉住四件事：endpoint 归一、截断必须如实上报、解析的退化策略、
// type 收敛到白名单。全是纯函数，不碰网络也不碰 git。
import {
  MAX_DIFF_CHARS,
  buildPrompt,
  normalizeEndpoint,
  normalizeType,
  parseCommitMessage,
  redactKey,
  shrinkDiff,
  validateConfig,
} from "../src/lib/ai.ts";

let failed = 0;

function check(name, actual, expected) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a !== e) {
    failed += 1;
    console.log(`FAIL ${name}\n  实际 ${a}\n  期望 ${e}`);
  } else {
    console.log(`ok   ${name}`);
  }
}

const config = { endpoint: "https://api.example.com/v1", apiKey: "k", model: "m" };

// ── 配置校验：三件套缺一不可，协议只放行 http/https ─────────────────────────
check("配置齐全", validateConfig(config), null);
check("缺 Base URL", validateConfig({ ...config, endpoint: "  " }) !== null, true);
check("缺 Key", validateConfig({ ...config, apiKey: "" }) !== null, true);
check("缺模型", validateConfig({ ...config, model: "" }) !== null, true);
check(
  "拒绝非 http 协议",
  validateConfig({ ...config, endpoint: "file:///etc/passwd" }),
  "Base URL 必须以 http:// 或 https:// 开头",
);
check("放行本地 http", validateConfig({ ...config, endpoint: "http://localhost:11434/v1" }), null);

// ── endpoint 归一：填到 /v1 要补后缀，整条 URL 粘进来要原样用 ───────────────
check("补 chat/completions", normalizeEndpoint("https://api.openai.com/v1"), "https://api.openai.com/v1/chat/completions");
check("末尾斜杠", normalizeEndpoint("https://api.openai.com/v1/"), "https://api.openai.com/v1/chat/completions");
check(
  "整条 URL 原样用",
  normalizeEndpoint("https://api.openai.com/v1/chat/completions"),
  "https://api.openai.com/v1/chat/completions",
);
check("本地 Ollama", normalizeEndpoint("http://localhost:11434/v1"), "http://localhost:11434/v1/chat/completions");
check("首尾空白", normalizeEndpoint("  https://h/v1  "), "https://h/v1/chat/completions");

// ── 压缩：没超预算原样送出；超了按文件分配额度，每个文件都必须露头 ─────────
const small = "x".repeat(100);
check("短 diff 原样送出", shrinkDiff(small), {
  text: small,
  truncated: false,
  clippedFiles: 0,
  foldedFiles: 0,
  skippedFiles: 0,
});

/** 造一段像样的 diff：文件头 + 一个 hunk，正文按 marker 生成，方便断言谁被留下 */
function diffBlock(path, lines, marker = "行") {
  const body = Array.from({ length: lines }, (_, i) => `+${marker} 内容填充填充填充 ${i}`).join("\n");
  return [
    `diff --git a/${path} b/${path}`,
    "index 1111111..2222222 100644",
    `--- a/${path}`,
    `+++ b/${path}`,
    `@@ -1,0 +1,${lines} @@`,
    body,
    "",
  ].join("\n");
}

const files = ["a.ts", "b.ts", "c.ts", "d.ts", "e.ts", "f.ts"];
const big = files.map((path) => diffBlock(path, 200)).join("");
const long = shrinkDiff(big);
check("超预算时标记压缩", long.truncated, true);
check("压缩后不超预算", long.text.length <= MAX_DIFF_CHARS, true);
// 这条是这次改动的核心：旧写法 `slice(0, MAX)` 会把第 3 个文件之后的全丢掉
check(
  "每个文件都露头（旧写法只留得下前两个）",
  files.every((path) => long.text.includes(`b/${path}`)),
  true,
);
check("被切短的正文如实计数", long.clippedFiles > 0, true);
check("没有文件被整个丢掉", long.skippedFiles, 0);

// 锁文件 / 生成物：正文折叠成统计，但仍要出现在清单里（否则模型不知道依赖也改了）
const withLock = diffBlock("package-lock.json", 300, "LOCKLINE") + diffBlock("src/app.ts", 300, "APPLINE");
const shrunk = shrinkDiff(withLock);
check("锁文件正文被折叠", shrunk.foldedFiles, 1);
check("锁文件只留统计不给正文", shrunk.text.includes("LOCKLINE"), false);
check("锁文件仍在清单里", shrunk.text.includes("b/package-lock.json"), true);
check("普通文件正文保留", shrunk.text.includes("APPLINE"), true);
check("折叠也算压缩过", shrunk.truncated, true);

// ── 提示词：白名单必须进提示词；截断过必须在提示词里说明 ───────────────────
const prompt = buildPrompt("diff --git a/x b/x", ["feat", "fix", "chore"]);
check("白名单进提示词", prompt.includes("feat、fix、chore"), true);
check("未截断时不提截断", prompt.includes("已被截断"), false);
check("截断时提示词里说明", buildPrompt(big, ["feat"]).includes("已被截断"), true);
check("空白名单也给指引", buildPrompt("d", []).includes("未限制"), true);
check("diff 原样带进提示词", buildPrompt("diff --git a/x b/x", ["feat"]).includes("diff --git a/x b/x"), true);
// 正文分条：日志要简洁，正文按「一行一个要点」的列表给，且不再索要长篇的动机/影响
check("提示词要求正文分条", prompt.includes("有序列表分条"), true);
check("提示词不再索要「动机与影响」", prompt.includes("动机与影响"), false);
check("提示词不再要描述解释「为什么」", prompt.includes("说明做了什么、为什么"), false);

// ── 解析：header + scope + 正文；以及各种不规范返回的退化处理 ─────────────
check(
  "标准形式",
  parseCommitMessage("feat(store): 增加 stash 面板\n\n面板支持 list/push/pop"),
  {
    type: "feat",
    scope: "store",
    description: "增加 stash 面板",
    body: "面板支持 list/push/pop",
    raw: "feat(store): 增加 stash 面板\n\n面板支持 list/push/pop",
  },
);
check("无 scope", parseCommitMessage("fix: 修复导出列错位").scope, "");
check("type 小写化", parseCommitMessage("FEAT: 大写 type").type, "feat");
check(
  "剥掉 markdown 代码块",
  parseCommitMessage("```\nfeat(api): 加接口\n```").description,
  "加接口",
);
check(
  "剥块后 scope 仍在",
  parseCommitMessage("```text\nfeat(api): 加接口\n```").scope,
  "api",
);
check("空返回", parseCommitMessage("   \n ").description, "");
// 退化策略：认不出 header 就整段当描述，绝不把内容丢掉
check("无 header 时整段进描述", parseCommitMessage("随便写的一句话").description, "随便写的一句话");
check("无 header 时 type 留空", parseCommitMessage("随便写的一句话").type, null);
check("描述里有冒号也能取到", parseCommitMessage("feat: 支持 A: B").description, "支持 A: B");

// ── type 收敛：命中、前缀兜底、不猜 ───────────────────────────────────────
const allowed = ["feat", "fix", "docs", "chore"];
check("命中", normalizeType("fix", allowed), "fix");
check("大小写不敏感", normalizeType("FIX", allowed), "fix");
check("feature 收敛到 feat", normalizeType("feature", allowed), "feat");
// 后缀不猜：bugfix 与 fix 意思相近，但「相近」不是白名单里的任何一个
check("只做前缀不猜后缀", normalizeType("bugfix", allowed), null);
check("白名单外不猜", normalizeType("refactor", allowed), null);
check("null 进 null 出", normalizeType(null, allowed), null);
check("白名单为空则原样返回", normalizeType("whatever", []), "whatever");

// ── key 处理：我们不提供、不内置、不分发任何 key，只用用户自己填的那个 ──────
check("抹掉回显的 key", redactKey("401: bad key sk-secret-123", { ...config, apiKey: "sk-secret-123" }), "401: bad key ***");
check("多处出现全抹", redactKey("sk-1 / sk-1 / sk-1", { ...config, apiKey: "sk-1" }), "*** / *** / ***");
check("空 key 不动文本", redactKey("nothing here", { ...config, apiKey: "  " }), "nothing here");
// key 是别的串的前缀时多抹一点：宁可少报也不把 key 尾巴留在界面上
check("部分重叠宁多抹", redactKey("sk-1x 与 sk-1", { ...config, apiKey: "sk-1" }), "***x 与 ***");

if (failed > 0) {
  console.log(`\n${failed} 项未通过`);
  process.exit(1);
}
console.log("\n全部通过");