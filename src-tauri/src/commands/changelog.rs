use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;

use crate::config::spec;
use crate::error::GitError;
use crate::git::changelog::{self, Changelog, Range};
use crate::git::process;
use crate::store::db::{lock, query, Db};
use crate::store::repos::{self, RepoKind};
use crate::write::{backup, journal};

/// CHANGELOG 生成（需求 6.5）。
///
/// 区间默认 `<上一个 tag>..HEAD`（`git describe --tags --abbrev=0`）；
/// 仓库一个 tag 都没有时 `previous_tag` 是 `null`，界面让人手选起点。
/// 判定与分组都走 `config::check` 与 `Spec.groups`——和表单、hook、报告同一份规则。
#[tauri::command]
pub async fn changelog_build(
    state: State<'_, Arc<Db>>,
    id: i64,
    from: Option<String>,
    to: Option<String>,
) -> Result<Changelog, GitError> {
    let (path, kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let range = Range { from, to };
    tauri::async_runtime::spawn_blocking(move || {
        let loaded = match kind {
            RepoKind::Worktree => spec::load(&path),
            RepoKind::Browse => spec::load_at_head(&path),
        };
        changelog::build(&path, &loaded, &range)
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 区间候选：上一个 tag（可为空）+ 最近若干 tag，供界面下拉。
#[tauri::command]
pub async fn changelog_previous_tag(
    state: State<'_, Arc<Db>>,
    id: i64,
    to: Option<String>,
) -> Result<Option<String>, GitError> {
    let (path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let to = to.unwrap_or_else(|| "HEAD".to_string());
    tauri::async_runtime::spawn_blocking(move || changelog::previous_tag(&path, &to))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))
}

/// 写操作的结果：除了"写到哪了"，还要给得出"怎么回去"。
#[derive(serde::Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppendResult {
    pub path: String,
    pub bytes: usize,
    pub journal_id: i64,
    pub backup_ref: String,
    /// 撤销办法：写的是工作区文件，还原点是仓库状态，所以这句话必须写出来，
    /// 界面不能只显示"成功"
    pub undo_hint: String,
}

/// 写入前的预览：目标文件当前长什么样。
///
/// 需求 6.5 要求"写前展示 diff"。工具侧没有文件读命令，所以这里按用户选定的路径
/// 读一次，把末尾一段带回去给界面拼"上面已有 / 下面新增"两段预览。
/// 只给末尾：CHANGELOG 可能很长，整份塞进 IPC 没有意义，用户要看的也是接缝那一段。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetPreview {
    pub path: String,
    pub exists: bool,
    pub bytes: usize,
    pub digest: String,
    /// 文件末尾最多 TAIL_LIMIT 个字符
    pub tail: String,
}

/// 尾部预览上限。够看清接缝，又不至于把 IPC 撑爆。
const TAIL_LIMIT: usize = 4000;

#[tauri::command]
pub async fn changelog_read_target(path: String) -> Result<TargetPreview, GitError> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let tail = if text.chars().count() > TAIL_LIMIT {
            let start = text
                .char_indices()
                .nth(text.chars().count() - TAIL_LIMIT)
                .map_or(0, |(index, _)| index);
            text[start..].to_string()
        } else {
            text
        };
        let exists = std::path::Path::new(&path).exists();
        Ok(TargetPreview {
            exists,
            path,
            bytes: bytes.len(),
            digest: changelog::digest(&bytes),
            tail,
        })
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 追加/写入 CHANGELOG 文件（需求 6.5：写前展示 diff，路径走 dialog 授权）。
///
/// 这一步改的是**工作区文件**，所以：
/// - `expected_digest` 是预览时目标文件内容的指纹。对不上就拒写——预览之后
///   文件被别人改过时，硬写就是把别人的改动冲掉；
/// - 目标在工作区里且有未提交改动时一并拒绝：这个文件正被别人手改着；
/// - 落一条写操作日志（§7.17），让这次改动在「操作记录」里看得见。
#[tauri::command]
pub async fn changelog_write(
    state: State<'_, Arc<Db>>,
    id: i64,
    path: String,
    markdown: String,
    expected_digest: String,
) -> Result<AppendResult, GitError> {
    let target = PathBuf::from(&path);
    if target.as_os_str().is_empty() {
        return Err(GitError::Internal("没有选择写入路径".into()));
    }
    if target.is_dir() {
        return Err(GitError::Internal("选中的是一个目录，不是文件".into()));
    }
    let repo = query(state.inner().clone(), move |conn| {
        repos::ensure_worktree(conn, id)
    })
    .await?;

    let db = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        write_changelog(&db, id, &repo, &target, &markdown, &expected_digest)
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

fn write_changelog(
    db: &Arc<Db>,
    id: i64,
    repo: &std::path::Path,
    target: &std::path::Path,
    markdown: &str,
    expected_digest: &str,
) -> Result<AppendResult, GitError> {
    let existing = std::fs::read(target).unwrap_or_default();
    let actual = changelog::digest(&existing);
    if actual != expected_digest {
        return Err(GitError::ConcurrentEdit {
            detail: format!(
                "{} 的内容和预览时不一致（现在 {actual}，预览时 {expected_digest}）。重新生成一次再写",
                file_name(target)
            ),
        });
    }
    // 目标在工作区里且正被手改时拒写：这不是并发问题，是"两个人改同一个文件"
    if target.starts_with(repo) && is_dirty(repo, target)? {
        return Err(GitError::NotClean {
            detail: format!(
                "{} 有未提交改动，先提交或暂存它，再写 CHANGELOG",
                file_name(target)
            ),
        });
    }

    let head_before = process::run(Some(repo), &["rev-parse", "HEAD"])
        .ok()
        .filter(|out| out.success)
        .map(|out| out.stdout.trim().to_string());
    let backup_ref = backup::create(repo, head_before.as_deref().unwrap_or(zero_oid()))?;

    let journal_id = {
        let conn = lock(&db.conn);
        journal::start(
            &conn,
            id,
            "changelog_write",
            None,
            Some(&file_name(target)),
            &backup_ref,
            head_before.as_deref(),
        )?
    };

    // 追加而不是覆盖：已有内容保持在上面，新的一段接在后面。
    // 末尾先补一个空行，否则新章节会粘在上一行文字上
    let mut buffer = String::from_utf8_lossy(&existing).into_owned();
    if !buffer.is_empty() && !buffer.ends_with("\n\n") {
        if !buffer.ends_with('\n') {
            buffer.push('\n');
        }
        buffer.push('\n');
    }
    buffer.push_str(markdown);

    let outcome = atomic_write(target, buffer.as_bytes()).and_then(|_| {
        let conn = lock(&db.conn);
        journal::finish(
            &conn,
            journal_id,
            journal::Status::Ok,
            head_before.as_deref(),
            None,
        )
    });

    if let Err(err) = outcome {
        let conn = lock(&db.conn);
        journal::finish(
            &conn,
            journal_id,
            journal::Status::RolledBack,
            head_before.as_deref(),
            Some(&format!("{err:?}")),
        )?;
        return Err(err);
    }

    Ok(AppendResult {
        path: target.to_string_lossy().into_owned(),
        bytes: buffer.len(),
        journal_id,
        backup_ref,
        undo_hint: format!(
            "这次只改了工作区文件，仓库状态没动。要撤销：git checkout -- {}",
            relative_or_absolute(repo, target)
        ),
    })
}

/// 目标文件在工作区里是否有未提交改动（含未跟踪的新文件）。
fn is_dirty(repo: &std::path::Path, target: &std::path::Path) -> Result<bool, GitError> {
    let relative = target.strip_prefix(repo).unwrap_or(target);
    let out = process::run(
        Some(repo),
        &["status", "--porcelain", "--", &relative.to_string_lossy()],
    )?;
    Ok(out.success && !out.stdout.trim().is_empty())
}

/// 同目录临时文件 + rename：写到一半断电也不会把原来的 CHANGELOG 截断成半个文件。
fn atomic_write(target: &std::path::Path, bytes: &[u8]) -> Result<(), GitError> {
    let temp = target.with_extension("md.git-tidy-tmp");
    std::fs::write(&temp, bytes).map_err(|err| GitError::Internal(err.to_string()))?;
    if let Err(err) = replace_file(&temp, target) {
        let _ = std::fs::remove_file(&temp);
        return Err(GitError::Internal(err.to_string()));
    }
    Ok(())
}

#[cfg(unix)]
fn replace_file(from: &std::path::Path, to: &std::path::Path) -> Result<(), std::io::Error> {
    std::fs::rename(from, to)
}

/// Windows 上 `rename` 不覆盖已存在的目标，得先删——所以走"删旧 + 改名"两步。
/// 删与改之间有一个极短的窗口，这是 Windows 文件 API 的限制；
/// 真要原子覆盖得用 `ReplaceFileW`，那要引 WinAPI 依赖，不值当。
#[cfg(windows)]
fn replace_file(from: &std::path::Path, to: &std::path::Path) -> Result<(), std::io::Error> {
    if to.exists() {
        std::fs::remove_file(to)?;
    }
    std::fs::rename(from, to)
}

fn zero_oid() -> &'static str {
    "0000000000000000000000000000000000000000"
}

/// "文件不存在"与"文件是空的"是同一件事：写入结果一样，指纹也该一样。
/// 不然预览时给的是空串、写入时算出的是空内容的哈希，第一次追加就被自己拒掉。
#[cfg(test)]
pub fn empty_digest() -> String {
    changelog::digest(&[])
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn relative_or_absolute(repo: &std::path::Path, target: &std::path::Path) -> String {
    match target.strip_prefix(repo) {
        Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
        Err(_) => target.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::process;

    fn git_in(dir: &std::path::Path, args: &[&str]) {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        std::fs::write(dir.path().join("a.txt"), "x\n").expect("write");
        git_in(dir.path(), &["add", "a.txt"]);
        git_in(
            dir.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                "feat: 一",
            ],
        );
        dir
    }

    /// 追加而不是覆盖：用户已有内容不能被一段新生成的文件吃掉
    #[test]
    fn existing_content_is_kept_and_the_new_block_is_separated() {
        let dir = repo();
        let target = dir.path().join("CHANGELOG.md");
        std::fs::write(&target, "# Changelog\n\n## v0.0.1\n\n- 最早的改动\n").expect("write");
        // 已有 CHANGELOG 必须是干净的工作区状态：正被手改的文件一律不覆盖
        git_in(dir.path(), &["add", "CHANGELOG.md"]);
        git_in(
            dir.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                "docs: 已有变更日志",
            ],
        );
        let digest = changelog::digest(&std::fs::read(&target).expect("read"));

        let db_dir = tempfile::tempdir().expect("db dir");
        let state = Arc::new(Db::open(db_dir.path()).expect("db"));

        let result = write_changelog(
            &state,
            1,
            dir.path(),
            &target,
            "## v0.1.0\n\n- 新的改动\n",
            &digest,
        )
        .expect("write");

        let content = std::fs::read_to_string(&target).expect("read");
        assert!(content.contains("## v0.0.1"), "旧内容必须还在");
        assert!(content.contains("## v0.1.0"), "新内容要追加在后面");
        assert!(
            content.contains("- 最早的改动\n\n## v0.1.0"),
            "两段之间要空一行：{content}"
        );
        assert!(
            result.undo_hint.contains("git checkout"),
            "{}",
            result.undo_hint
        );
    }

    /// 预览之后文件被别人改过 → 拒写。不能拿乐观并发那套"再让用户点一次"糊过去：
    /// 这里的冲突是"整份内容会变"，覆盖等于丢东西
    #[test]
    fn a_file_changed_after_the_preview_is_not_overwritten() {
        let dir = repo();
        let target = dir.path().join("CHANGELOG.md");
        std::fs::write(&target, "# Changelog\n").expect("write");
        let stale = changelog::digest(b"# Changelog\n");

        let db_dir = tempfile::tempdir().expect("db dir");
        let state = Arc::new(Db::open(db_dir.path()).expect("db"));
        // 预览之后有人改了文件
        std::fs::write(&target, "# Changelog\n\n别人写的\n").expect("write");

        let err = write_changelog(&state, 1, dir.path(), &target, "## 新\n", &stale)
            .expect_err("digest 对不上必须拒写");
        assert!(matches!(err, GitError::ConcurrentEdit { .. }), "{err:?}");
        assert!(std::fs::read_to_string(&target)
            .expect("read")
            .contains("别人写的"));
    }

    /// 工作区里正被手改的目标文件一律不覆盖
    #[test]
    fn a_dirty_target_file_is_refused() {
        let dir = repo();
        let target = dir.path().join("CHANGELOG.md");
        std::fs::write(&target, "# Changelog\n").expect("write");
        git_in(dir.path(), &["add", "CHANGELOG.md"]);
        // 工作区里已经有人手改过这个文件；预览拿到的就是这份内容
        std::fs::write(&target, "# Changelog\n\n未提交的改动\n").expect("write");
        let digest = changelog::digest(&std::fs::read(&target).expect("read"));

        let db_dir = tempfile::tempdir().expect("db dir");
        let state = Arc::new(Db::open(db_dir.path()).expect("db"));

        let err = write_changelog(&state, 1, dir.path(), &target, "## 新\n", &digest)
            .expect_err("正被手改的文件不能覆盖");
        assert!(matches!(err, GitError::NotClean { .. }), "{err:?}");
    }

    /// 写成功了要能在操作日志里查到——不然这次改动就凭空消失了
    #[test]
    fn a_successful_write_leaves_a_journal_entry() {
        let dir = repo();
        let target = dir.path().join("CHANGELOG.md");
        let db_dir = tempfile::tempdir().expect("db dir");
        let state = Arc::new(Db::open(db_dir.path()).expect("db"));

        let result = write_changelog(
            &state,
            1,
            dir.path(),
            &target,
            "# Changelog\n",
            &empty_digest(),
        )
        .expect("write");
        let conn = lock(&state.conn);
        let entry = journal::last(&conn, 1).expect("last").expect("有一条");
        assert_eq!(entry.action, "changelog_write");
        assert_eq!(entry.backup_ref, result.backup_ref);
        assert!(matches!(entry.status, journal::Status::Ok));
    }
}
