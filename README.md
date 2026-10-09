# git-tidy

Tidy your commit history

把不符合规范的提交整理成符合 Conventional Commits 的 Git 历史。核心能力是批量治理一段连续区间内的提交（squash / reword）、生成 CHANGELOG，并给出仓库的规范符合率报告；通用客户端那部分（历史、图、详情、diff、blame、文件树、筛选）是它的放大器。

历史改写只支持从 HEAD 往回的连续区间，执行前写入还原点、执行后校验 tree 一致，随时可回退。

## 当前状态

按需求文档的 M1–M5 分期：

- **M1 读透历史（完）**：注册表与只读浏览、提交历史 + 提交图泳道、ref 可见性与中断态、commit 详情、文件级 diff（含图片三种画法）、blame / 文件历史 / 跨修订文件树、筛选与搜索（筛选态重跑图计算）。
- **M2 写索引与工作区（完）**：所有写命令经 `write_guard` 的六步——逐行/hunk 暂存、stash、分支与标签、cherry-pick / revert / reset、fetch / pull / push，外加还原点、写操作日志与「撤销上一步」。
- **M3 危险面（做了一半）**：冲突解决器（六类冲突的逐块取舍与降级视图）已落地；交互式改写的自驱引擎与 todo 面板已落地（冲突续跑还缺）；reflog 恢复、GPG / LFS / Git Flow 还没有。
- **M4 治理闭环 / M5 外壳与交付**：未开始。规范配置三层与提交表单已就位，批量治理、符合率报告、CHANGELOG、打包与 CI 都还没有。

下一步是交互式改写的冲突续跑（把剩余 todo 落到本地，冲突解决后从停住的那一条接着走），然后是 M3 的 reflog 恢复。

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
npm run check:conflict         # 冲突标记解析器的边界用例
npm run check:conflict-real    # 拿真 git 造出的冲突草稿跑一遍解析器
cd src-tauri && cargo test && cargo clippy
```

VS Code 用户会自动收到 Volar、tauri-vscode、rust-analyzer 三个扩展推荐。

## 代码分层

Rust 侧 `git/process.rs` 是唯一的 git 子进程出口，环境固定为英文 stderr、UTF-8 输出、`core.quotepath=false`、`--no-pager`，Windows 下附加 `CREATE_NO_WINDOW`。所有对前端的错误都走 `error.rs` 的结构化 `{ code, message, detail }`，前端按 `code` 分支，不解析错误文本。

前端按 `api` / `stores` / `router` / `views` 分层，IPC 调用统一收敛在 `api/client.ts`。
