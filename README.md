# git-tidy

Tidy your commit history

把不符合规范的提交整理成符合 Conventional Commits 的 Git 历史。核心能力是批量治理一段连续区间内的提交（squash / reword）、生成 CHANGELOG，并给出仓库的规范符合率报告。

历史改写只支持从 HEAD 往回的连续区间，执行前写入还原点、执行后校验 tree 一致，随时可回退。

## 当前状态

工程骨架已通，端到端只打通了 `repo_probe` 一条切片（探测工作区、git 目录、当前分支、HEAD 提交与脏状态）。批量治理、CHANGELOG 生成、符合率报告都还没开始实现。

## 技术栈

- 桌面壳：Tauri v2
- 前端：Vue 3 + TypeScript + Vite 8，Naive UI，Pinia，vue-router（hash history，适配 Tauri 的 asset 协议）
- Git 操作：Rust 直接子进程调用本机 `git`，不引入 libgit2 绑定

## 开发环境

- Rust MSVC 工具链（`stable-x86_64-pc-windows-msvc`）
- Windows 上还需要 VS 2022 Build Tools 的「使用 C++ 的生成工具」workload，Tauri 链接阶段依赖 `link.exe`
- WebView2 运行时
- Node.js 与 git

开发时使用的版本：rustc 1.96.0 / Node 24.18.0 / git 2.54.0。

## 运行

```bash
npm install
npm run tauri dev      # 启动桌面窗口
npm run build          # vue-tsc 类型检查 + 前端构建
cd src-tauri && cargo test && cargo clippy
```

VS Code 用户会自动收到 Volar、tauri-vscode、rust-analyzer 三个扩展推荐。

## 代码分层

Rust 侧 `git/process.rs` 是唯一的 git 子进程出口，环境固定为英文 stderr、UTF-8 输出、`core.quotepath=false`、`--no-pager`，Windows 下附加 `CREATE_NO_WINDOW`。所有对前端的错误都走 `error.rs` 的结构化 `{ code, message, detail }`，前端按 `code` 分支，不解析错误文本。

前端按 `api` / `stores` / `router` / `views` 分层，IPC 调用统一收敛在 `api/client.ts`。
