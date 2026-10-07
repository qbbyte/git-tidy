use std::path::Path;
use std::sync::Mutex;
use std::sync::MutexGuard;

use rusqlite::Connection;

use crate::error::GitError;

/// 每加一张表就把这个数加一，并在 migrate 里补一条对应的建表语句。
const SCHEMA_VERSION: i64 = 1;

const SCHEMA_V1: &str = "
CREATE TABLE repos (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL,
    path       TEXT NOT NULL UNIQUE,
    kind       TEXT NOT NULL CHECK (kind IN ('worktree', 'browse')),
    remote_url TEXT,
    added_at   INTEGER NOT NULL
);
";

/// 本地索引库，全应用一个连接。仓库注册表、缓存、治理操作日志都在这一个文件里。
pub struct Db {
    pub conn: Mutex<Connection>,
}

impl Db {
    pub fn open(data_dir: &Path) -> Result<Db, GitError> {
        std::fs::create_dir_all(data_dir).map_err(|err| {
            GitError::Internal(format!("无法创建数据目录 {}：{err}", data_dir.display()))
        })?;

        let conn = Connection::open(data_dir.join("git-tidy.db"))
            .map_err(|err| GitError::Internal(format!("无法打开索引库：{err}")))?;

        // §6.7 的并发验收：WAL 让读不阻塞写，busy_timeout 让写锁排队而不是当场报 SQLITE_BUSY。
        conn.pragma_update(None, "journal_mode", "WAL")
            .and_then(|_| conn.pragma_update(None, "busy_timeout", 5000))
            .map_err(|err| GitError::Internal(format!("索引库配置失败：{err}")))?;

        migrate(&conn)?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }
}

/// 取连接。锁中毒只说明某个线程持锁时 panic 过，连接本身仍然可用，
/// 恢复它而不是让一次 panic 把之后所有命令都变成同一个错误。
pub fn lock(mutex: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 数据库读写也放 spawn_blocking：SQLite 落盘是阻塞 IO，不能占 IPC 的异步运行时（§5.7）。
pub async fn query<R>(
    db: std::sync::Arc<Db>,
    f: impl FnOnce(&Connection) -> Result<R, GitError> + Send + 'static,
) -> Result<R, GitError>
where
    R: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || f(&lock(&db.conn)))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

fn migrate(conn: &Connection) -> Result<(), GitError> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sqlite_failure)?;

    if version > SCHEMA_VERSION {
        return Err(GitError::Internal(format!(
            "索引库版本 {version} 比当前程序（{SCHEMA_VERSION}）更新，请升级 git-tidy"
        )));
    }
    if version < 1 {
        conn.execute_batch(SCHEMA_V1).map_err(sqlite_failure)?;
    }
    if version != SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sqlite_failure)?;
    }
    Ok(())
}

fn sqlite_failure(err: rusqlite::Error) -> GitError {
    GitError::Internal(format!("索引库读写失败：{err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wal_and_busy_timeout_are_applied() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = Db::open(tmp.path()).expect("open");
        let conn = lock(&db.conn);

        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal_mode");
        assert_eq!(journal.to_lowercase(), "wal", "§6.7 要求 WAL");

        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("busy_timeout");
        assert_eq!(timeout, 5000);
    }

    #[test]
    fn reopening_an_existing_db_keeps_the_registry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        {
            let db = Db::open(tmp.path()).expect("open");
            lock(&db.conn)
                .execute(
                    "INSERT INTO repos (name, path, kind, added_at) VALUES ('demo', '/tmp/demo', 'worktree', 1)",
                    [],
                )
                .expect("insert");
        }

        // 关掉再重开，模拟重启应用：§6.1 的验收就是注册记录还在
        let db = Db::open(tmp.path()).expect("reopen");
        let count: i64 = lock(&db.conn)
            .query_row("SELECT COUNT(*) FROM repos", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 1, "重开数据库后注册记录必须还在");
    }

    #[test]
    fn migration_is_idempotent_and_versioned() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = Db::open(tmp.path()).expect("open");
        {
            let conn = lock(&db.conn);
            let version: i64 = conn
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .expect("version");
            assert_eq!(version, SCHEMA_VERSION);

            let tables: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='repos'",
                    [],
                    |row| row.get(0),
                )
                .expect("tables");
            assert_eq!(tables, 1, "建表只能有一份");
        }
        drop(db);
        // 同一目录再开一次不能因为表已存在而失败
        Db::open(tmp.path()).expect("second open");
    }

    #[test]
    fn a_newer_db_than_this_program_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        {
            let db = Db::open(tmp.path()).expect("open");
            lock(&db.conn)
                .pragma_update(None, "user_version", SCHEMA_VERSION + 3)
                .expect("bump");
        }
        let err = match Db::open(tmp.path()) {
            Ok(_) => panic!("比程序新的库应被拒绝"),
            Err(err) => err,
        };
        assert!(
            format!("{err:?}").contains("更新"),
            "要让用户看出是版本问题，实际输出：{err:?}"
        );
    }
}
