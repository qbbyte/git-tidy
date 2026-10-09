# git-tidy

> 一个用 **Tauri v2 + Vue 3 + Rust** 编写的 Git 桌面客户端，把「通用历史浏览与仓库操作」和「提交规范治理闭环」放进同一个进程。

[![CI](https://github.com/qbbyte/git-tidy/actions/workflows/ci.yml/badge.svg)](https://github.com/qbbyte/git-tidy/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-0.1.0-blue)](https://github.com/qbbyte/git-tidy/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)](https://v2.tauri.app)
[![Tauri](https://img.shields.io/badge/Tauri-2.x-24C8DB)](https://v2.tauri.app)
[![Vue](https://img.shields.io/badge/Vue-3.x-42b883)](https://vuejs.org)

---

> **项目名说明**
>
> GitHub 上名为 `git-tidy` 的项目不止本仓库一个，涉及 Git 库的库、CLI 辅助工具、hook 套件都有同名或近名项目。本仓库是一个**桌面应用**，不是可安装的库，也不是 git 别名或 shell 脚本。如果你找的是终端里用的清理脚本，这里不是。

## 目录

- [它解决什么问题](#它解决什么问题)
- [核心能力](#核心能力)
  - [历史改写引擎](#历史改写引擎)
  - [规范治理闭环](#规范治理闭环)
  - [通用客户端能力](#通用客户端能力)
  - [合并冲突解决](#合并冲突解决)
  - [AI 生成提交信息（可选）](#ai-生成提交信息可选)
- [界面预览](#界面预览)
- [从源码构建](#从源码构建)
- [开发与校验](#开发与校验)
- [架构与代码分层](#架构与代码分层)
- [安全模型](#安全模型)
- [贡献](#贡献)
- [许可证](#许可证)

## 它解决什么问题

git-tidy 把两件事放进同一个进程：

1. **通用的历史浏览与仓库操作能力**——提交历史、提交图、commit 详情、文件级 diff、blame、文件历史、跨修订文件树、暂存与提交、分支与标签、stash、远端同步、合并冲突解决；
2. **提交规范治理闭环**——规范配置、可视化提交表单、commit-msg hook 安装、符合率报告、就地整改、CHANGELOG 生成。

闭环部分是这个项目的重点。单独看每一环都有替代品：`git-cliff` 能生成 CHANGELOG，Commitizen 能规范化输入，GitButler 的 virtual branches 能做提交级压缩与拖拽；但把「判定不合规 → 就地改写 → 度量结果 → 生成日志」串在同一个工具里、并且在任何改写之前都能看清将要改什么，目前没有现成方案。

## 核心能力

### 历史改写引擎

对 HEAD 往回的**任意连续区间**做 reword / squash / fixup / drop / 重排，区间可以位于历史中部。

- 实现上**不使用 `git rebase -i`**。改为自驱引擎：`rev-list --reverse` 生成 todo，在临时分支上按序执行，每一步做 tree 校验，成功后用 `update-ref` 回到原分支。这样做是为了避开 `GIT_SEQUENCE_EDITOR` 需要 git 反向调用客户端、Windows 上要维护单实例互斥进程、崩溃后 todo 状态难以诊断这三类问题。
- 区间内任一提交被 remote-tracking ref 包含时拒绝执行，除非用户显式确认改写已推送历史，并接受 `--force-with-lease` 的后果。
- 区间含 merge commit 时拒绝。
- squash 与 drop 保留原作者与作者日期。
- 执行前写入还原点，执行后校验最终 tree 与改写前一致，任何一步失败自动回退，临时分支保留供诊断。

### 规范治理闭环

- **三层配置**：团队规范（仓库根 `git-tidy.config.json`）、个人偏好（app config dir 的 `preferences.json`）、本地索引（app data dir 的 SQLite，WAL + `busy_timeout`）。
- **兼容既有生态**：没有自研配置时按序从 `commitlint.config.js` / `.commitlintrc*`、`.versionrc.json`、`cliff.toml` 推导等价规范。自研格式是覆盖手段，不是唯一入口，因此对既有项目零迁移成本。
- **提交表单**：type / scope / subject / body / footer / 任务 ID，实时校验与预览，拦截空 subject、无信息量词、中文冒号、句尾句号等；提交走 `git commit -F`。
- **commit-msg hook 一键安装**：规则以字面量写进自包含的 POSIX 脚本，安装时快照，不依赖本工具在 PATH 里。检测到 `core.hooksPath` 被 husky 等占用时不覆盖文件，只给共存方案。
- **符合率报告**：整体符合率、type 分布、原因分桶、按作者与按月趋势，每条不合规明细可跳详情与 diff。被 `--no-verify` 绕过的提交在 git 里不留痕迹，因此只标「疑似绕过」而非断言。
- **CHANGELOG 生成**：按 Conventional Commits 1.0.0 解析，Breaking 判定涵盖 type 后 `!` 与 `BREAKING CHANGE:` footer，按 feat / fix / perf / refactor 分组、scope 作二级、revert 单独一组；不合规提交不静默丢弃而是单独计数并链回报告。

### 通用客户端能力

提交历史与虚拟滚动、提交图 DAG 泳道、commit 详情、文件级 diff（含图片的并排 / 滑动对位 / 差异叠加三种画法）、逐行与逐 hunk 部分暂存、stash 面板、分支与标签全量操作、cherry-pick / revert / reset、fetch / pull / push 与 force-with-lease、blame（含 `.blame-ignore-revs`）、单文件全部改动记录（`--follow`）、任意历史快照的文件树、跨分支 / 作者 / 时间 / type / 关键词筛选与搜索。

### 合并冲突解决

六类冲突逐块取舍：内容冲突、改删、删改、双 rename、二进制、子模块。共同祖先缺失时降级为两方视图并说明冲突类型，二进制只提供「选一边」。全部标记后按来源续跑（merge 走 `-c core.editor=true git commit`，rebase / cherry-pick 走 `--continue`）。写回内容即用户所见，不做额外加工。

### AI 生成提交信息（可选）

用户自带 OpenAI 兼容端点（可接 OpenAI / DeepSeek / 通义 / 本地 Ollama 等任意 `/chat/completions` 服务），在设置里填 Base URL / API Key / 模型名后启用。

**API Key 一律由用户自己提供**：本项目不内置、不附带、不分发任何 key，空即未启用，未启用时不发任何请求；发出的请求也只会到你填的那个地址。

- 只把 `git diff --cached` 的原文送给模型，与提交按钮要落盘的范围严格一致——模型不该根据还没 `git add` 的草稿编造提交说明。
- 结果先进弹窗，确认后才填入表单；不提供自动提交。
- type 只在落在仓库规范白名单内时才写入，否则留空由人选择。
- diff 过长时截断，并在提示词与界面上都说明；请求带超时；解析不出格式时保留模型原文而不丢弃内容。
- 报错文本里可能被网关回显的 key 会被抹掉——错误文案会被截图、被粘进 issue。

未配置时不发任何请求，其余功能不受影响。

### 外壳

常驻左栏 + 主区页签（历史 / 文件 / 提交 / 报告 / CHANGELOG / 设置），数字键直达。快捷键面板与按键绑定由 `src/shortcuts.ts` 的同一份数组驱动，文档不会与实现脱节。在终端打开、在资源管理器显示、复制路径均不经 shell 拼接。检查更新只查不装。

## 界面预览

应用仍在开发中，**尚无可用发布版本**，界面截图将随首个发布版本补充。预览窗口默认尺寸为 1280 × 820（最小 960 × 600），采用 Naive UI 浅色主题。

## 从源码构建

### 前置条件

- Rust MSVC 工具链（`stable-x86_64-pc-windows-msvc`）
- Windows 上需要 Visual Studio 2022 Build Tools 的「使用 C++ 的生成工具」workload，Tauri 在链接阶段依赖 `link.exe`
- Linux 上需要 Tauri 的系统依赖：`libwebkit2gtk-4.1-dev`、`libappindicator3-dev`、`librsvg2-dev`、`patchelf`、`build-essential`、`libxdo-dev`、`libssl-dev`
- WebView2 运行时（Windows）
- Node.js 与 git

开发时使用的版本：rustc 1.96.0 / Node 24.18.0 / git 2.54.0。

### 构建步骤

```bash
git clone https://github.com/qbbyte/git-tidy.git
cd git-tidy
npm install
npm run tauri dev      # 启动桌面窗口
npm run tauri build    # 产出安装包（Windows 走 NSIS）
```

## 开发与校验

```bash
npm run build                # vue-tsc 类型检查 + 前端构建
npm run check:conflict       # 冲突标记解析器的边界用例
npm run check:conflict-real  # 用真实 git 造冲突，跑一遍解析器（PowerShell）
npm run check:highlight      # diff 语法高亮的「拼回去必须等于原文」不变量
npm run check:ai             # AI 提交信息的 endpoint 归一 / 截断上报 / 解析退化 / type 收敛
npm run check:tokens         # 设计 token 表
npm run check:pane           # 面板分隔条
```

Rust 侧的格式、lint 与测试：

```bash
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

其中 `check:highlight` 守的是一条硬约束：语法高亮只影响观感，不进正确性依赖，但一旦吃掉字符，diff 就在骗人，所以逐行校验「分词后拼回去必须严格等于原文」。

VS Code 用户会自动收到 Volar、tauri-vscode、rust-analyzer 三个扩展推荐。

## 架构与代码分层

```
src/                     前端
  api/                   IPC 封装，调用统一收敛在 api/client.ts
  stores/                Pinia 状态
  components/            展示与交互组件
  views/                 页面
  lib/                   纯函数（冲突标记解析、语法高亮）
src-tauri/src/
  git/                   按能力域拆分的 git 操作，每个模块的注释写明取舍理由
  write/                 write_guard 六步、还原点、写操作日志
  commands/              Tauri command 层，参数校验与结构化错误出口
  store/                 SQLite 索引与 preferences.json
  error.rs               结构化错误 { code, message, detail }
```

工程约束（write_guard 六步、git 调用硬约束、安全模型、错误模型、性能预算）的详细文档见 [`docs/DESIGN.md`](docs/DESIGN.md)（编写中）。要点如下：

- **一切走系统 git CLI**，不重实现 git，不引入 libgit2 绑定。`src-tauri/src/git/process.rs` 是唯一的 git 子进程出口，固定英文 stderr、UTF-8 输出、`core.quotepath=false`、`--no-pager`，Windows 下附加 `CREATE_NO_WINDOW`。
- **纯本地，不自建服务端**。出网面被显式枚举：clone / fetch / pull / push / 删除远程分支 / LFS 对象拉取（规划中）/ 更新检查（`api.github.com` 单个 URL，仓库坐标由构建期变量 `GIT_TIDY_REPO` 注入）/ AI 端点（可选，默认关闭，由用户在设置中自行填写）。除此之外全部离线可用。不做遥测、不做账号体系、不代理 AI 请求、不缓存送出的 diff。
- **错误模型**：前端错误一律走结构化的 `{ code, message, detail }`，前端按 `code` 分支，不解析错误文本。

## 安全模型

威胁前提是 WebView 可以调用任意 command 并传入任意路径。因此：

- 所有 git 命令只接受已注册仓库的 id，路径由 Rust 侧解析；
- 写命令必须在 `write_guard` 白名单内登记，未登记的 action 直接拒绝，前端传不了 argv 数组与文件路径；
- `treeless` 只读仓库在解析路径那一步就拒绝全部写命令，返回 `ReadOnlyRepo`，不靠前端弹窗；
- 高危操作的前置校验、还原点、tree 校验与回滚都在 Rust 侧兜底，确认文案必须包含受影响的 sha 区间与还原 ref。

## 贡献

项目处于早期开发阶段，欢迎以 Issue 反馈问题、以 PR 提交改动。提交前请先跑通「[开发与校验](#开发与校验)」中的前端与 Rust 校验。涉及架构或工程约束的改动，建议先开 Issue 讨论。

## 许可证

[MIT](LICENSE)。
