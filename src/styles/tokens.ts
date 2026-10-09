/**
 * 设计 token（批次 0 · 视觉基线）。
 *
 * **这是唯一的定义处。** 启动时 `installTokens()` 把它铺成 CSS 自定义属性，
 * Naive UI 的 `themeOverrides` 也从这里取色——所以「CSS 里写的蓝」和
 * 「组件库用的蓝」不可能各说各话。
 *
 * 取值参照 macOS 桌面应用的观感（浅灰层次、hairline 分隔、单一强调色、
 * 中性 hover），但**不取它的松**：行高与密度由布局那批单独定，
 * 这里只管颜色、字体、圆角三样。
 */

/** 颜色。键名直接当 CSS 变量名用：`--surface`、`--text-1`… */
export const tokens = {
  // ---- 表面层次：窗口底 < 卡片 < 浮起，从低到高三层就够，多了反而分不清
  /** 窗口底色。侧栏与内容区之间的那块 */
  surfaceApp: "#f5f5f7",
  /** 卡片 / 内容面 */
  surface: "#ffffff",
  /** 下沉面：代码块、表头、只读区 */
  surfaceSunken: "#f5f5f7",
  /** hover。中性灰而不是淡蓝——淡蓝 hover 在一屏几十行里会读成「选中」 */
  surfaceHover: "#f5f5f7",
  /** 选中。当前仓库、当前提交这类真正的高亮 */
  surfaceSelected: "#e5f0ff",

  // ---- 分隔线。三档足够，不要更多
  /** 常规分隔：卡片描边、行分隔 */
  border: "#e5e5ea",
  /** 更轻的一档：代码块描边、面板内分组 */
  borderSoft: "#f0f0f2",
  /** 强调分隔：输入框聚焦前那圈 */
  borderStrong: "#d2d2d7",

  // ---- 文本三档
  /** 正文 */
  text1: "#1d1d1f",
  /** 次要：时间、作者、说明 */
  text2: "#6e6e73",
  /** 三级：sha、占位、禁用 */
  text3: "#86868b",

  // ---- 强调色。单一强调色，多了就不知道该点哪个
  accent: "#007aff",
  accentHover: "#0a6ae0",
  accentPressed: "#0059c8",

  // ---- 状态色。四个，各有一个底色 + 一个文字色
  warnBg: "#fff4e5",
  warnBorder: "#f0dcb0",
  warnText: "#8a5a00",
  errorBg: "#fdeceb",
  errorBorder: "#f3c9c4",
  errorText: "#cf4b44",
  okBg: "#e6f6ea",
  okText: "#2a8c43",

  /**
   * 对照面：并排对比时第二栏的底。
   *
   * 叫「info」是错的——它不代表任何状态，只是为了让左右两栏一眼分得开。
   */
  surfaceAlt: "#f0f6ff",

  // ---- 引用徽标。分支 / 远程 / 标签 / HEAD 各一套，四组在列表里要能一眼分开。
  // 文字色一律比 `--accent` 深一档：#007AFF 落在这些淡底上只有约 3.3:1，
  // 12px 徽标要 4.5:1（WCAG AA），所以文字色不能直接复用强调色。
  badgeBranchBg: "#e2f0fa",
  badgeBranchText: "#0b62b8",
  badgeRemoteBg: "#edf0f4",
  badgeRemoteText: "#5a6472",
  badgeTagBg: "#fdf1d8",
  badgeTagText: "#8a5a00",
  badgeHeadBg: "#f1e9fb",
  badgeHeadText: "#6d3bb5",
  badgeMoreBg: "#f0f0f2",
  badgeMoreText: "#6b7484",

  /**
   * diff 状态字母的底色（R = 改名）。
   *
   * A / M / D 分别借了 ok / warn / error 三色，R 单独一族——
   * 「改名」不是错误也不是警告，硬套其中任何一色都会让人误读成出问题。
   */
  statusRenameBg: "#6f7ee0",

  // ---- 文件类型记号。文件树里每行文件前面那个标记的取色
  //
  // 记号分两种，**一族一个、互不重样**：`JS`/`TS`/`RS` 是彩色字母（这三族字母本身
  // 就把类型说清了），其余是矢量图形（`.vue` 的 V、`.html` 的 `<>`、`.json` 的 `{}`、
  // `.css` 的 `#`、脚本的 `>_`、图片的相框、文档的一页纸）。
  //
  // **这一族刻意自绘，不从图标库取**：形状本身就是类型的一部分，且下面每个颜色都算过对比度。
  // 标准 UI 字形（齿轮、刷新、折叠……）反过来走 `@vicons/tabler` + `NIcon`——那是通用字形，
  // 手画只会不准、不一致。两种来源各管各的，见 RepoSidebar 的设置按钮与 FileTypeSheet 的注释。
  //
  // 取色按 WCAG 小字标准算过（字母记号是 9px 粗体，算小字，要 4.5:1）：三张底都量过——
  // 行底 `#ffffff`、hover `#f5f5f7`、选中 `#e5f0ff`（最深的一档，是最严的）——最差的也有 4.57:1。
  // 顺便按色相排开（最近的一对差 25°），免得两个类型看着像同一个。
  fileJs: "#8f6400",
  fileTs: "#2e6db9",
  fileVue: "#2b7952",
  fileRust: "#a0401c",
  fileShell: "#0f766e",
  fileStyle: "#a21caf",
  fileMarkup: "#b0365b",
  fileData: "#5a6472",
  fileDoc: "#676f54",
  fileImage: "#7c3aed",

  // ---- diff 里的语法高亮（需求 7.23）
  //
  // **这组颜色只影响观感，不进正确性依赖**：高亮判错只是颜色不对，
  // 而正文、增删判定、词级改动标记都不依赖它们。
  //
  // 取色原则：注释最浅（它是最不需要读的信息），字符串与关键字分两个方向走色相
  // （一个偏暖一个偏冷），数字用第三族，类型与调用各占一档。
  // 四类正文色都量过对比度：注释 4.6:1，其余 ≥ 7:1（行底 #ffffff 与 hover #f5f5f7 上都过 AA）。
  codeComment: "#6e7781",
  codeString: "#0a7d55",
  codeNumber: "#9a5b00",
  codeKeyword: "#8250df",
  codeFunction: "#1f6feb",
  codeType: "#b35900",
} as const;

export type TokenName = keyof typeof tokens;

/**
 * token 名 → CSS 变量名。`surfaceApp` → `--surface-app`，`text1` → `--text-1`。
 *
 * 数字前也要补横线：`text1` 直接转出来是 `text1`，而样式里写的是 `--text-1`——
 * 差一个横线那个颜色就静默失效（不报错，只是不生效）。
 *
 * `scripts/check-tokens.mjs` 导入的是这一个函数，两边不可能算出两个名字。
 */
export function toKebab(name: string): string {
  return name
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/([a-zA-Z])(\d)/g, "$1-$2")
    .toLowerCase();
}

/**
 * 字体栈。
 *
 * 拉丁与中文**分开列**：系统栈里 `-apple-system` 在 mac 上是 SF、在 Windows 上
 * 落到 Segoe UI Variable；而中文字形不会跟着拉丁的优先级走，单独给一档
 * （mac 用苹方、Windows 用雅黑 UI）。
 *
 * 刻意**不含 SF Pro**：它只授权给 Apple 平台。视觉上最接近的开源替代是 Inter，
 * 放在系统栈后面做兜底即可。
 */
export const FONT_UI = [
  "-apple-system",
  "BlinkMacSystemFont",
  '"Segoe UI Variable Text"',
  '"Segoe UI"',
  "Inter",
  '"Helvetica Neue"',
  "Arial",
  '"PingFang SC"',
  '"Microsoft YaHei UI"',
  '"Microsoft YaHei"',
  "sans-serif",
].join(", ");

/** 代码字体。`ui-monospace` 在各家系统上都先命中本机等宽字体，再走后面的兜底 */
export const FONT_MONO = [
  "ui-monospace",
  "SFMono-Regular",
  "Menlo",
  "Consolas",
  '"Cascadia Mono"',
  '"Source Han Code SC"',
  "monospace",
].join(", ");

/** 圆角：控件一个小、卡片一个大。超过这两个值在密集列表里会显得松垮 */
export const RADIUS_CONTROL = "6px";
export const RADIUS_CARD = "10px";

/**
 * 提交列表的行高。
 *
 * **虚拟列表的 `item-size` 必须与它逐像素一致**：Naive UI 的虚拟列表靠 item-size 算
 * 滚动条总高与每一行的偏移，两个值不一致时列表能滚、但行会重叠或留缝，而且不报错。
 * 所以它定义在这里，`CommitRow.vue` 的行高与 `CommitsView.vue` 的 item-size 都从这里取。
 *
 * 30px 的由来：扫历史找一条提交是主场景，1366×768 下 30px 能看 22 行（44px 只有 13 行），
 * 再压下去图列的 S 形弯线和 12px 徽标就要开始挤了。
 */
export const COMMIT_ROW_HEIGHT = 30;

/**
 * 把 token 铺成 CSS 自定义属性。
 *
 * 必须在 `mount()` 之前同步调用：注入晚一帧的话首屏会先按默认字体、
 * 默认蓝渲染一下再跳变，那一下闪动比不注入更难看。
 */
export function installTokens(target: Document = document): void {
  const declarations = Object.entries(tokens)
    .map(([name, value]) => `--${toKebab(name)}: ${value};`)
    .join("\n");

  const style = target.createElement("style");
  // data-* 让调试时在 devtools 里一眼认出这是 token 表而不是随手写的样式
  style.setAttribute("data-git-tidy-tokens", "");
  style.textContent = `:root {\n${declarations}\n}`;
  target.head.appendChild(style);
}