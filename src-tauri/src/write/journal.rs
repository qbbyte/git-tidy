use rusqlite::Connection;

use crate::error::GitError;
use crate::store::db::sqlite_failure;

/// 一条写操作日志（§7.17）。
///
/// `detail` 存失败原文或说明，不存凭据：远程命令的 stderr 里可能带 URL 里的 token，
/// 写进日志就等于落盘（M2 的远程命令自己先脱敏再交给这里）。
#[derive(Debug, Clone)]
pub struct WriteOp {
    pub id: i64,
    pub repo_id: i64,
    /// Unix 秒
    pub ts: i64,
    pub action: String,
    pub affected_from: Option<String>,
    pub affected_to: Option<String>,
    /// 还原 ref（`refs/git-tidy/backup-<时间戳>`）。撤销与"已回到什么状态"都靠它
    pub backup_ref: String,
    pub head_before: Option<String>,
    pub head_after: Option<String>,
    pub status: Status,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    RolledBack,
    Interrupted,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::RolledBack => "rolled_back",
            Self::Interrupted => "interrupted",
        }
    }

    fn parse(raw: &str) -> Status {
        match raw {
            "rolled_back" => Self::RolledBack,
            "interrupted" => Self::Interrupted,
            _ => Self::Ok,
        }
    }
}

const COLUMNS: &str = "id, repo_id, ts, action, affected_from, affected_to, backup_ref, \
                       head_before, head_after, status, detail";

/// 落一条"开始执行"。第 2 步就要写，不等结果：进程崩了也要留得住"当时动了什么"。
pub fn start(
    conn: &Connection,
    repo_id: i64,
    action: &str,
    affected_from: Option<&str>,
    affected_to: Option<&str>,
    backup_ref: &str,
    head_before: Option<&str>,
) -> Result<i64, GitError> {
    conn.execute(
        "INSERT INTO write_op (repo_id, ts, action, affected_from, affected_to, backup_ref, \
                              head_before, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'ok')",
        rusqlite::params![
            repo_id,
            now(),
            action,
            affected_from,
            affected_to,
            backup_ref,
            head_before,
        ],
    )
    .map_err(sqlite_failure)?;
    Ok(conn.last_insert_rowid())
}

/// 收尾：写最终状态与执行后的 HEAD。
pub fn finish(
    conn: &Connection,
    id: i64,
    status: Status,
    head_after: Option<&str>,
    detail: Option<&str>,
) -> Result<(), GitError> {
    conn.execute(
        "UPDATE write_op SET status = ?2, head_after = ?3, detail = ?4 WHERE id = ?1",
        rusqlite::params![id, status.as_str(), head_after, detail],
    )
    .map_err(sqlite_failure)?;
    Ok(())
}

/// 某个仓库最近的若干条，新的在前。界面上的"操作记录"就按这个顺序显示。
pub fn recent(conn: &Connection, repo_id: i64, limit: usize) -> Result<Vec<WriteOp>, GitError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM write_op WHERE repo_id = ?1 ORDER BY ts DESC, id DESC LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql).map_err(sqlite_failure)?;
    let rows = stmt
        .query_map(rusqlite::params![repo_id, limit as i64], |row| {
            Ok(WriteOp {
                id: row.get(0)?,
                repo_id: row.get(1)?,
                ts: row.get(2)?,
                action: row.get(3)?,
                affected_from: row.get(4)?,
                affected_to: row.get(5)?,
                backup_ref: row.get(6)?,
                head_before: row.get(7)?,
                head_after: row.get(8)?,
                status: Status::parse(&row.get::<_, String>(9)?),
                detail: row.get(10)?,
            })
        })
        .map_err(sqlite_failure)?;

    rows.collect::<Result<Vec<WriteOp>, _>>()
        .map_err(sqlite_failure)
}

/// 最近一条。找不到就是"没有可撤销的写操作"，不是错误。
pub fn last(conn: &Connection, repo_id: i64) -> Result<Option<WriteOp>, GitError> {
    Ok(recent(conn, repo_id, 1)?.into_iter().next())
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::Db;

    fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(dir.path()).expect("db");
        (dir, db)
    }

    #[test]
    fn a_write_is_logged_before_it_runs_and_closed_after() {
        let (_dir, db) = db();
        let conn = crate::store::db::lock(&db.conn);

        let id = start(
            &conn,
            1,
            "branch_delete",
            Some("side"),
            None,
            "refs/git-tidy/backup-1",
            Some("aaa"),
        )
        .expect("start");
        // 第 2 步之后就该查得到：进程崩在执行中间也要留得住"当时动了什么"
        let pending = last(&conn, 1).expect("last").expect("有一条");
        assert_eq!(pending.id, id);
        assert_eq!(pending.action, "branch_delete");
        assert_eq!(pending.head_after, None);

        finish(&conn, id, Status::Ok, Some("bbb"), None).expect("finish");
        let done = last(&conn, 1).expect("last").expect("有一条");
        assert_eq!(done.status, Status::Ok);
        assert_eq!(done.head_after.as_deref(), Some("bbb"));
    }

    #[test]
    fn every_repo_keeps_its_own_log() {
        let (_dir, db) = db();
        let conn = crate::store::db::lock(&db.conn);
        start(
            &conn,
            1,
            "stash_push",
            None,
            None,
            "refs/git-tidy/backup-1",
            None,
        )
        .expect("1");
        start(
            &conn,
            2,
            "fetch",
            None,
            None,
            "refs/git-tidy/backup-2",
            None,
        )
        .expect("2");

        assert_eq!(recent(&conn, 1, 10).expect("recent").len(), 1);
        assert_eq!(recent(&conn, 2, 10).expect("recent")[0].action, "fetch");
        assert!(
            last(&conn, 3).expect("last").is_none(),
            "没写过的仓库没有可撤销项"
        );
    }

    #[test]
    fn the_newest_entry_comes_first() {
        let (_dir, db) = db();
        let conn = crate::store::db::lock(&db.conn);
        start(&conn, 1, "a", None, None, "refs/git-tidy/backup-1", None).expect("a");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        start(&conn, 1, "b", None, None, "refs/git-tidy/backup-2", None).expect("b");

        let recent = recent(&conn, 1, 10).expect("recent");
        assert_eq!(recent[0].action, "b", "最近的写操作要排在最前");
    }

    /// 撤销的判据是状态字段：只有成功的那条才谈得上"撤销上一步"
    #[test]
    fn a_rolled_back_entry_is_marked_as_such() {
        let (_dir, db) = db();
        let conn = crate::store::db::lock(&db.conn);
        let id = start(
            &conn,
            1,
            "reset",
            None,
            None,
            "refs/git-tidy/backup-1",
            Some("aaa"),
        )
        .expect("start");
        finish(
            &conn,
            id,
            Status::RolledBack,
            Some("aaa"),
            Some("已恢复到 aaa"),
        )
        .expect("finish");

        let entry = last(&conn, 1).expect("last").expect("有一条");
        assert_eq!(entry.status, Status::RolledBack);
        assert_eq!(entry.detail.as_deref(), Some("已恢复到 aaa"));
    }

    /// 数据库来自旧版本时迁移要补出这张表
    #[test]
    fn an_older_database_gains_the_write_log_table() {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            // 造一个只有 v1 的库
            let conn = Connection::open(dir.path().join("git-tidy.db")).expect("open");
            conn.execute_batch(
                "CREATE TABLE repos (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    path TEXT NOT NULL UNIQUE,
                    kind TEXT NOT NULL CHECK (kind IN ('worktree', 'browse')),
                    remote_url TEXT,
                    added_at INTEGER NOT NULL
                );
                PRAGMA user_version = 1;",
            )
            .expect("seed v1");
        }

        let db = Db::open(dir.path()).expect("迁移后应能开");
        let conn = crate::store::db::lock(&db.conn);
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='write_op'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(tables, 1, "旧库升级后要补出写操作日志表");
    }
}
