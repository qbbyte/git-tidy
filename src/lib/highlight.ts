/**
 * diff 里的轻量语法高亮（需求 7.23）。
 *
 * **它只影响观感，不进正确性依赖**（需求原文就是这么写的），所以这里的实现刻意保守：
 * - 手写正则扫描，不引语法分析库：diff 面板要同时吃下几百上千行，
 *   每行都跑一遍真正的 parser 会让展开大文件时卡一下，而收益只是颜色；
 * - 判不出来就当普通文本，颜色少几处不会有人在意，**但一个字都不能少**。
 *   唯一硬约束是 `tokens.map(t => t.text).join("") === line`——
 *   高亮器一旦吃掉字符，diff 就在骗人了。`scripts/check-highlight.mjs` 守着这一条。
 *
 * 每种语言只认四类东西：注释、字符串、数字、关键字/字面量，外加"看起来像调用"和
 * "首字母大写"这两个启发（函数名与类型名）。启发判错就是颜色错，不会造成别的。
 */

export type Lang =
  | "ts"
  | "js"
  | "vue"
  | "rust"
  | "shell"
  | "css"
  | "html"
  | "json"
  | "yaml"
  | "toml"
  | "md"
  | "text";

export type TokenKind =
  | "comment"
  | "string"
  | "number"
  | "keyword"
  | "function"
  | "type"
  | "plain";

export interface Token {
  kind: TokenKind;
  text: string;
}

const EXTENSION_LANG: Record<string, Lang> = {
  ts: "ts",
  tsx: "ts",
  mts: "ts",
  js: "js",
  mjs: "js",
  cjs: "js",
  jsx: "ts",
  vue: "vue",
  rs: "rust",
  sh: "shell",
  bash: "shell",
  zsh: "shell",
  ps1: "shell",
  css: "css",
  scss: "css",
  less: "css",
  html: "html",
  htm: "html",
  json: "json",
  jsonc: "json",
  yml: "yaml",
  yaml: "yaml",
  toml: "toml",
  md: "md",
  markdown: "md",
};

/** 按扩展名认语言。认不出来就是 `text`：不猜，比猜错再整体上色好 */
export function detectLang(path: string): Lang {
  const name = path.split(/[\\/]/).pop() ?? path;
  const dot = name.lastIndexOf(".");
  if (dot < 0) return "text";
  return EXTENSION_LANG[name.slice(dot + 1).toLowerCase()] ?? "text";
}

export const LANG_LABEL: Record<Lang, string> = {
  ts: "TypeScript",
  js: "JavaScript",
  vue: "Vue",
  rust: "Rust",
  shell: "Shell",
  css: "CSS",
  html: "HTML",
  json: "JSON",
  yaml: "YAML",
  toml: "TOML",
  md: "Markdown",
  text: "纯文本",
};

/** 值里出现什么就上什么色；`plain` 不需要单独定义，直接落回默认色 */
const KEYWORDS: Record<string, ReadonlySet<string>> = {
  ts: set(
    "abstract as async await break case catch class const continue debugger declare default delete do else enum export extends finally for from function get if implements import in infer instanceof interface is keyof let namespace new of private protected public readonly return satisfies set static switch throw try type typeof var void while with yield as const true false null undefined this super",
  ),
  js: set(
    "async await break case catch class const continue debugger default delete do else export extends finally for function if import in instanceof let new of return static switch throw try typeof var void while with yield true false null undefined this super",
  ),
  vue: set("defineProps defineEmits defineExpose ref reactive computed watch watchEffect onMounted onUnmounted true false null undefined"),
  rust: set(
    "as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait type unsafe use where while async_await true false",
  ),
  shell: set(
    "if then else elif fi for while do done case esac function return export local readonly source in break continue",
  ),
  css: set("important media supports keyframes import charset font-face layer container"),
  html: set("DOCTYPE"),
  json: set("true false null"),
  yaml: set("true false null yes no"),
  toml: set("true false"),
};

function set(words: string): ReadonlySet<string> {
  return new Set(words.split(/\s+/));
}

/**
 * C 家族（TS / JS / Rust / Vue 脚本）的扫描器。
 *
 * 顺序即优先级：注释 → 字符串 → 数字 → 标识符。落空的部分（括号、运算符、空白）
 * 逐字归入 `plain`，所以拼回去一定是原文。
 */
const C_LIKE = new RegExp(
  [
    "(?<comment>//[^\\n]*|/\\*[\\s\\S]*?\\*/)",
    '(?<string>"(?:[^"\\\\\\n]|\\\\.)*"|\'(?:[^\'\\\\\\n]|\\\\.)*\'|`(?:[^`\\\\]|\\\\.)*`)',
    "(?<number>0[xXbBoO][0-9a-fA-F_]+|\\d[\\d_]*(?:\\.\\d+)?(?:[eE][+-]?\\d+)?)",
    "(?<ident>[A-Za-z_$][\\w$]*)",
  ].join("|"),
  "g",
);

const SHELL = new RegExp(
  [
    "(?<comment>#[^\\n]*)",
    "(?<string>\"(?:[^\"\\\\]|\\\\.)*\"|'[^']*')",
    "(?<param>\\$\\{[^}]*\\}|\\$[A-Za-z_][\\w]*|\\$\\d|\\$[@*#?$!])",
    "(?<number>\\b\\d+\\b)",
    "(?<ident>[A-Za-z_][\\w-]*)",
  ].join("|"),
  "g",
);

const CSS_LIKE = new RegExp(
  [
    "(?<comment>/\\*[\\s\\S]*?\\*/)",
    '(?<string>"(?:[^"\\\\\\n]|\\\\.)*"|\'(?:[^\'\\\\\\n]|\\\\.)*\')',
    "(?<number>#[0-9a-fA-F]{3,8}\\b|-?\\d*\\.?\\d+(?:%|[a-z]{1,4})?)",
    "(?<ident>--[\\w-]+|[a-z-]+(?=\\s*:))",
    "(?<at>@[a-z-]+)",
  ].join("|"),
  "g",
);

const MARKUP = new RegExp(
  [
    "(?<comment><!--[\\s\\S]*?-->)",
    '(?<string>"(?:[^"\\\\\\n]|\\\\.)*"|\'(?:[^\'\\\\\\n]|\\\\.)*\')',
    "(?<tag></?[A-Za-z][\\w:-]*)",
    "(?<number>&#[0-9]+;|\\b\\d+\\b)",
  ].join("|"),
  "g",
);

const KEY_VALUE = new RegExp(
  [
    "(?<comment>[#;][^\\n]*)",
    // **带前瞻的引号串是键**（"a": 1），不带的是值——JSON / YAML / TOML 里两者颜色不同，
    // 而键的数量远多于值，所以这个前瞻是这三个语言唯一值得加的复杂度
    '(?<qkey>"(?:[^"\\\\\\n]|\\\\.)*"(?=\\s*:)|\'(?:[^\'\\\\\\n]|\\\\.)*\'(?=\\s*:))',
    '(?<string>"(?:[^"\\\\\\n]|\\\\.)*"|\'(?:[^\'\\\\\\n]|\\\\.)*\')',
    "(?<number>\\b\\d+(?:\\.\\d+)?\\b)",
    "(?<key>[A-Za-z_][\\w.-]*(?=\\s*[:=]))",
    "(?<ident>[A-Za-z_][\\w.-]*)",
  ].join("|"),
  "g",
);

function scannerFor(lang: Lang): RegExp | null {
  if (lang === "ts" || lang === "js" || lang === "rust" || lang === "vue") return C_LIKE;
  if (lang === "shell") return SHELL;
  if (lang === "css") return CSS_LIKE;
  if (lang === "html") return MARKUP;
  if (lang === "json" || lang === "yaml" || lang === "toml") return KEY_VALUE;
  // Markdown 不做高亮：标题、列表、表格一堆符号，给它们上色只会把正文衬得花
  return null;
}

/**
 * 把一行拆成若干 token。**`tokens.map(t => t.text).join("")` 必须等于入参**——
 * 这是这个模块唯一的硬约束，改动时先用 `scripts/check-highlight.mjs` 验一遍。
 */
export function highlight(line: string, lang: Lang): Token[] {
  const scanner = scannerFor(lang);
  if (scanner === null || line === "") return line === "" ? [] : [{ kind: "plain", text: line }];

  const keywords = KEYWORDS[lang];
  const out: Token[] = [];
  scanner.lastIndex = 0;
  let plainFrom = 0;
  let match: RegExpExecArray | null;

  const pushPlain = (upTo: number) => {
    if (upTo > plainFrom) out.push({ kind: "plain", text: line.slice(plainFrom, upTo) });
  };

  while ((match = scanner.exec(line)) !== null) {
    const groups = match.groups ?? {};
    if (match[0] === "") {
      // 零宽匹配会让 exec 卡死在这里，直接跳过这一位
      scanner.lastIndex += 1;
      continue;
    }
    pushPlain(match.index);

    let kind: TokenKind = "plain";
    if (groups.comment !== undefined) kind = "comment";
    else if (groups.string !== undefined) kind = "string";
    else if (groups.number !== undefined) kind = "number";
    else if (groups.param !== undefined) kind = "keyword";
    else if (groups.at !== undefined) kind = "keyword";
    else if (groups.qkey !== undefined) kind = "type";
    else if (groups.key !== undefined) kind = "type";
    else if (groups.tag !== undefined) kind = "keyword";
    else if (groups.ident !== undefined) kind = classifyIdent(
      groups.ident,
      lang,
      keywords,
      line.slice(match.index + match[0].length),
    );

    out.push({ kind, text: match[0] });
    plainFrom = match.index + match[0].length;
  }
  pushPlain(line.length);

  return out;
}

/**
 * 标识符归到哪一类：`keyword` 查表 → 后面跟 `(` 视为调用 → 首字母大写视为类型 → 其余普通。
 *
 * 后两条是启发，判错只是颜色不对；这比"解析出一棵语法树"便宜一个数量级。
 */
function classifyIdent(
  ident: string,
  lang: Lang,
  keywords: ReadonlySet<string> | undefined,
  rest: string,
): TokenKind {
  if (keywords?.has(ident)) return "keyword";
  if (lang === "json" || lang === "yaml" || lang === "toml") {
    // 这三种语言里"标识符"只可能是键或裸值，都按关键字处理，免得正文被染成一团
    return "keyword";
  }
  if (/^\s*\(/.test(rest)) return "function";
  if (/^[A-Z]/.test(ident) && /[A-Za-z]/.test(ident)) return "type";
  return "plain";
}

/**
 * 同一个片段在高亮后常常重复出现（缩进、括号、逗号）。这里按"文本 + 语言"缓存，
 * 让大 diff 展开时不必对每一格重复跑一遍正则。
 *
 * 上限是必要的：diff 面板可能一直开着，不设上限的缓存就是个只增不减的内存池。
 */
const CACHE_LIMIT = 4000;
const cache = new Map<string, Token[]>();

export function highlightCached(line: string, lang: Lang): Token[] {
  const key = `${lang}\u0000${line}`;
  const hit = cache.get(key);
  if (hit !== undefined) return hit;
  const tokens = highlight(line, lang);
  if (cache.size >= CACHE_LIMIT) cache.clear();
  cache.set(key, tokens);
  return tokens;
}