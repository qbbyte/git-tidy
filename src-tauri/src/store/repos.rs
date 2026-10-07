use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::error::GitError;

/// 仓库的可写能力等级，对应注册表 kind 列的两个取值。
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RepoKind {
    /// 有工作区：可提交、可改写、可读取待提交文件
    Worktree,
    /// treeless 浏览：只有 commit 对象，只读，读改动文件时依赖远程按需取对象
    Browse,
}

impl RepoKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Browse => "browse",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub kind: RepoKind,
    pub remote_url: Option<String>,
}

/// 登记一个仓库。路径已存在时返回既有记录——重复添加同一个目录不该报错，也不该出双行。
pub fn add(
    conn: &Connection,
    path: &str,
    kind: RepoKind,
    remote_url: Option<&str>,
) -> Result<Repo, GitError> {
    let added_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default();

    conn.execute(
        "INSERT INTO repos (name, path, kind, remote_url, added_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(path) DO NOTHING",
        rusqlite::params![derive_name(path), path, kind.as_str(), remote_url, added_at],
    )
    .map_err(store_failure)?;

    select_by_path(conn, path)?.ok_or_else(|| GitError::Internal("注册后读不回记录".into()))
}

pub fn list(conn: &Connection) -> Result<Vec<Repo>, GitError> {
    let mut stmt = conn
        .prepare("SELECT id, name, path, kind, remote_url FROM repos ORDER BY name COLLATE NOCASE")
        .map_err(store_failure)?;
    let rows = stmt
        .query_map([], read_row)
        .map_err(store_failure)?
        .map(|row| build(row.map_err(store_failure)?));
    rows.collect::<Result<Vec<_>, _>>()
}

pub fn rename(conn: &Connection, id: i64, name: &str) -> Result<Repo, GitError> {
    let changed = conn
        .execute(
            "UPDATE repos SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, id],
        )
        .map_err(store_failure)?;
    if changed == 0 {
        return Err(GitError::RepoNotFound);
    }
    select_by_id(conn, id)?.ok_or_else(|| GitError::Internal("改名后读不回记录".into()))
}

/// 只删注册记录，绝不碰磁盘上的仓库文件（§6.1）。
pub fn remove(conn: &Connection, id: i64) -> Result<(), GitError> {
    let changed = conn
        .execute("DELETE FROM repos WHERE id = ?1", rusqlite::params![id])
        .map_err(store_failure)?;
    if changed == 0 {
        return Err(GitError::RepoNotFound);
    }
    Ok(())
}

/// 按 id 取整行记录。克隆补齐既要路径和类型，也要远程地址，所以单独给一个入口。
pub fn get(conn: &Connection, id: i64) -> Result<Repo, GitError> {
    select_by_id(conn, id)?.ok_or(GitError::RepoNotFound)
}

/// §7.1 的落点：git 命令层只拿 id，路径在这里由 Rust 侧解析，前端构造不出任意路径。
pub fn locate(conn: &Connection, id: i64) -> Result<(PathBuf, RepoKind), GitError> {
    let repo = select_by_id(conn, id)?.ok_or(GitError::RepoNotFound)?;
    Ok((PathBuf::from(repo.path), repo.kind))
}

/// 可写命令的入口闸门：拿路径，同时把只读浏览仓库挡在外面。
/// 前端弹窗可以绕过，Rust 侧这一层不行（§7.5）。
pub fn ensure_worktree(conn: &Connection, id: i64) -> Result<PathBuf, GitError> {
    let (path, kind) = locate(conn, id)?;
    match kind {
        RepoKind::Worktree => Ok(path),
        RepoKind::Browse => Err(GitError::ReadOnlyRepo),
    }
}

/// 克隆补齐后把只读浏览升成完整仓库。路径和 URL 都不动——目录本来就是那个目录，
/// 只是对象补齐了、工作区建出来了。
pub fn set_kind(conn: &Connection, id: i64, kind: RepoKind) -> Result<Repo, GitError> {
    let changed = conn
        .execute(
            "UPDATE repos SET kind = ?1 WHERE id = ?2",
            rusqlite::params![kind.as_str(), id],
        )
        .map_err(store_failure)?;
    if changed == 0 {
        return Err(GitError::RepoNotFound);
    }
    select_by_id(conn, id)?.ok_or_else(|| GitError::Internal("升级后读不回记录".into()))
}

/// 展示名取目录名；URL 抓取下来的目录常带 .git 后缀，那不算名字的一部分。
pub(crate) fn derive_name(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    Path::new(trimmed)
        .file_name()
        .map(|name| name.to_string_lossy().trim_end_matches(".git").to_string())
        .filter(|name| !name.is_empty())
        // 根目录这类没有上一级名字的写法，退回用户原本输入，而不是裁空后的结果
        .unwrap_or_else(|| path.to_string())
}

fn select_by_path(conn: &Connection, path: &str) -> Result<Option<Repo>, GitError> {
    let raw = conn
        .query_row(
            "SELECT id, name, path, kind, remote_url FROM repos WHERE path = ?1",
            rusqlite::params![path],
            read_row,
        )
        .optional()
        .map_err(store_failure)?;
    raw.map(build).transpose()
}

fn select_by_id(conn: &Connection, id: i64) -> Result<Option<Repo>, GitError> {
    let raw = conn
        .query_row(
            "SELECT id, name, path, kind, remote_url FROM repos WHERE id = ?1",
            rusqlite::params![id],
            read_row,
        )
        .optional()
        .map_err(store_failure)?;
    raw.map(build).transpose()
}

/// 库里的一行，kind 还没解释成枚举。分开两步是为了让未知类型能带着原值报成 GitError，
/// 而不是被塞进 rusqlite 的错误里丢失文案。
struct RawRow {
    id: i64,
    name: String,
    path: String,
    kind: String,
    remote_url: Option<String>,
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        kind: row.get(3)?,
        remote_url: row.get(4)?,
    })
}

fn build(raw: RawRow) -> Result<Repo, GitError> {
    let kind = match raw.kind.as_str() {
        "worktree" => RepoKind::Worktree,
        "browse" => RepoKind::Browse,
        // 出现未知类型说明库被人为改过或来自更新的版本，兜一个默认值会把只读仓库当可写仓库使
        other => {
            return Err(GitError::Internal(format!(
                "注册表里出现未知的仓库类型 {other}"
            )))
        }
    };
    Ok(Repo {
        id: raw.id,
        name: raw.name,
        path: raw.path,
        kind,
        remote_url: raw.remote_url,
    })
}

fn store_failure(err: rusqlite::Error) -> GitError {
    GitError::Internal(format!("注册表读写失败：{err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::{lock, Db};

    fn open_db(dir: &Path) -> Db {
        Db::open(dir).expect("open db")
    }

    #[test]
    fn add_then_list_returns_the_repo() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = open_db(tmp.path());
        let conn = lock(&db.conn);

        let added = add(&conn, "D:/proj/demo", RepoKind::Worktree, None).expect("add");
        assert_eq!(added.name, "demo", "展示名应取目录名");
        assert_eq!(added.kind, RepoKind::Worktree);
        assert!(added.remote_url.is_none());

        let listed = list(&conn).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, added.id);
    }

    #[test]
    fn adding_the_same_path_twice_is_not_an_error_and_not_a_second_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = open_db(tmp.path());
        let conn = lock(&db.conn);

        let first = add(&conn, "/repos/a", RepoKind::Worktree, None).expect("first");
        let second = add(&conn, "/repos/a", RepoKind::Worktree, None).expect("second");
        assert_eq!(first.id, second.id, "重复添加同一目录应返回既有记录");
        assert_eq!(list(&conn).expect("list").len(), 1);
    }

    #[test]
    fn browse_kind_and_remote_url_survive_a_round_trip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = open_db(tmp.path());
        let conn = lock(&db.conn);

        add(
            &conn,
            "C:/appdata/remote/git-tidy",
            RepoKind::Browse,
            Some("https://github.com/qbbyte/git-tidy.git"),
        )
        .expect("add browse");

        let repo = &list(&conn).expect("list")[0];
        assert_eq!(repo.kind, RepoKind::Browse);
        assert_eq!(
            repo.remote_url.as_deref(),
            Some("https://github.com/qbbyte/git-tidy.git")
        );
    }

    #[test]
    fn locate_gives_the_path_back_for_the_id() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = open_db(tmp.path());
        let conn = lock(&db.conn);

        let added = add(&conn, "D:/proj/demo", RepoKind::Worktree, None).expect("add");
        let (path, kind) = locate(&conn, added.id).expect("locate");
        assert_eq!(path, PathBuf::from("D:/proj/demo"));
        assert_eq!(kind, RepoKind::Worktree);

        assert!(matches!(locate(&conn, 4242), Err(GitError::RepoNotFound)));
    }

    #[test]
    fn rename_and_remove_report_unknown_ids() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = open_db(tmp.path());
        let conn = lock(&db.conn);

        let added = add(&conn, "/repos/a", RepoKind::Worktree, None).expect("add");
        let renamed = rename(&conn, added.id, "我的仓库").expect("rename");
        assert_eq!(renamed.name, "我的仓库");
        assert_eq!(renamed.path, "/repos/a", "改名只动展示名，不动磁盘路径");

        remove(&conn, added.id).expect("remove");
        assert!(list(&conn).expect("list").is_empty());

        assert!(matches!(rename(&conn, 1, "x"), Err(GitError::RepoNotFound)));
        assert!(matches!(remove(&conn, 1), Err(GitError::RepoNotFound)));
    }

    #[test]
    fn unknown_kind_in_the_table_surfaces_as_an_error_not_a_default() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = open_db(tmp.path());
        let conn = lock(&db.conn);
        // 绕过 CHECK 约束，专门验证解析分支不会静默兜底
        conn.execute_batch(
            "PRAGMA ignore_check_constraints=ON;
             INSERT INTO repos (name, path, kind, added_at) VALUES ('x', '/repos/x', 'weird', 1);",
        )
        .expect("raw insert");

        let err = match list(&conn) {
            Ok(_) => panic!("未知类型必须报错，不能兜默认值"),
            Err(err) => err,
        };
        assert!(
            format!("{err:?}").contains("weird"),
            "报错要带上那个脏值，实际：{err:?}"
        );
    }

    #[test]
    fn list_is_ordered_by_name_case_insensitively() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = open_db(tmp.path());
        let conn = lock(&db.conn);

        add(&conn, "/repos/zebra", RepoKind::Worktree, None).expect("z");
        add(&conn, "/repos/Alpha", RepoKind::Worktree, None).expect("a");
        add(&conn, "/repos/middle", RepoKind::Worktree, None).expect("m");

        let names: Vec<String> = list(&conn)
            .expect("list")
            .into_iter()
            .map(|repo| repo.name)
            .collect();
        assert_eq!(names, vec!["Alpha", "middle", "zebra"]);
    }

    #[test]
    fn a_browse_repo_is_refused_before_the_git_command_runs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = open_db(tmp.path());
        let conn = lock(&db.conn);

        let browse = add(&conn, "/remote/git-tidy", RepoKind::Browse, None).expect("add browse");
        assert!(matches!(
            ensure_worktree(&conn, browse.id),
            Err(GitError::ReadOnlyRepo)
        ));

        let local = add(&conn, "/repos/a", RepoKind::Worktree, None).expect("add worktree");
        assert_eq!(
            ensure_worktree(&conn, local.id).expect("可写"),
            PathBuf::from("/repos/a")
        );
    }

    #[test]
    fn derived_name_drops_trailing_slash_and_git_suffix() {
        assert_eq!(derive_name("D:/proj/demo/"), "demo");
        assert_eq!(derive_name("D:\\proj\\demo.git"), "demo");
        assert_eq!(derive_name("/"), "/", "根目录没有上一级名字时退回原路径");
    }
}
