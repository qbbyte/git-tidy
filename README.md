# git-tidy

[![CI](https://github.com/qbbyte/git-tidy/actions/workflows/ci.yml/badge.svg)](https://github.com/qbbyte/git-tidy/actions/workflows/ci.yml)

把不符合规范的提交整理成符合 Conventional Commits 的 Git 历史。

> **这不是什么**：GitHub 上叫 git-tidy 的项目不止本仓库一个（Git 相关的库、CLI 辅助工具、
> hook 套件都有同名或近名项目）。本项目是一个**桌面应用（Tauri + Vue）**，核心是把一段
> 连续区间里的提交治理干净，不是一个可以 `pip install` 的库，也不是一个 git 别名。
> 如果你要找的是"在终端里用的清理脚本"，这里不是你要找的地方。

## 它做什么

- **批量治理一段连续区间内的提交**：交互式改写（reword / squash / fixup / drop / 重排），
  自驱临时分支执行，不使用 `git rebase -i`
- **给出仓库的规范符合率报告**：逐条列出不合规的提交与原因，跳到 diff 就地整改
- **生成 CHANGELOG**

通用客户端那部分（提交历史、提交图泳道、commit 详情、文件级 diff、blame、文件历史、
跨修订文件树、筛选与搜索）是这些能力的放大器：任何一次改写之前，你都能看清它将要改什么。

历史改写只支持从 HEAD 往回的**连续区间**。执行前写入还原点，改写类操作执行后校验 tree
必须一致，随时可回退。压缩与丢弃保留原作者与作者日期。

## 当前状态

**在开发中，尚无可用版本。** 按需求文档的 M1–M5 分期：

- **M1 读透历史（完）**：注册表与只读浏览、提交历史 + 提交图泳道、ref 可见性与中断态、commit 详情、文件级 diff（含图片三种画法）、blame / 文件历史 / 跨修订文件树、筛选与搜索（筛选态重跑图计算）。
- **M2 写索引与工作区（完）**：所有写命令经 `write_guard` 的六步——逐行/hunk 暂存、stash、分支与标签、cherry-pick / revert / reset、fetch / pull / push，外加还原点、写操作日志与「撤销上一步」。
- **M3 危险面（做了一半）**：冲突解决器（六类冲突的逐块取舍与降级视图）已落地；交互式改写的自驱引擎与 todo 面板已落地（**冲突续跑还缺**）；reflog 恢复、GPG / LFS / Git Flow 还没有。
- **M4 治理闭环（完）**：规范配置三层与提交表单、commit-msg hook 一键安装（`core.hooksPath` 被占时只给共存方案）、符合率报告（绕过只标「疑似」）、CHANGELOG 生成与追加写回（不合规单独计数，不静默丢）。
- **M5 交付**：设置页（偏好落 `preferences.json`：拉取策略 / Flow 前缀 / 列显示 / 自动更新）、快捷键与自文档化的快捷键面板、外部终端与资源管理器入口、diff 语法高亮已就位。**主题切换、自动更新本体、打包还没有**；CI 已就位。

上面刻意保留了未完成项：仓库描述的是**目标**，当前状态以本节为准。

## 许可

MIT，见 [LICENSE](LICENSE)。

## 技术栈

- 桌面壳：Tauri v2
- 前端：Vue 3 + TypeScript + Vite 8，Naive UI，Pinia，vue-router（hash history，适配 Tauri 的 asset 协议）
- Git 操作：Rust 直接子进程调用本机 `git`，不引入 libgit2 绑定

## 开发环境

- Rust MSVC 工具链（`stable-x86_64-pc-windows-msvc`）
- Windows 上还需要 VS 2022 Build Tools 的「使用 C++ 的生成工具」workload，Tauri 链接阶段依赖 `link.exe`
- Linux 上还需要 Tauri 的系统依赖：`libwebkit2gtk-4.1-dev`、`libappindicator3-dev`、`librsvg2-dev`、`patchelf`、`build-essential`、`libxdo-dev`、`libssl-dev`（CI 里已装）
- WebView2 运行时
- Node.js 与 git

开发时使用的版本：rustc 1.96.0 / Node 24.18.0 / git 2.54.0。

## 从源码构建

目前没有发布安装包，需要自行构建：

```bash
git clone https://github.com/qbbyte/git-tidy.git
cd git-tidy
npm install
npm run tauri dev      # 启动桌面窗口
npm run tauri build    # 产出安装包（Windows 走 NSIS）
```

## 运行

```bash
npm install
npm run tauri dev      # 启动桌面窗口
npm run build          # vue-tsc 类型检查 + 前端构建
npm run check:conflict         # 冲突标记解析器的边界用例
npm run check:conflict-real    # 拿真 git 造出的冲突草稿跑一遍解析器
cd src-tauri && cargo test && cargo clippy
```

VS Code 用户会自动收到 Volar、tauri-vscode、rust-analyzer 三个扩展推荐。

## 代码分层

Rust 侧 `git/process.rs` 是唯一的 git 子进程出口，环境固定为英文 stderr、UTF-8 输出、`core.quotepath=false`、`--no-pager`，Windows 下附加 `CREATE_NO_WINDOW`。所有对前端的错误都走 `error.rs` 的结构化 `{ code, message, detail }`，前端按 `code` 分支，不解析错误文本。

前端按 `api` / `stores` / `router` / `views` 分层，IPC 调用统一收敛在 `api/client.ts`。
