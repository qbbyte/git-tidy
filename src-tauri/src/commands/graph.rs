use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use tauri::State;

use super::commit::MAX_PAGE_SIZE;
use crate::error::GitError;
use crate::git::graph;
use crate::store::db::{query, Db};
use crate::store::repos;

/// 一个仓库的整条泳道分配结果。
///
/// 键是 HEAD 的 sha：提交、amend、reset 之后 sha 自己就变了，没有"谁忘了让缓存失效"
/// 这种状态。值按 `Arc` 存，翻页时只多拷一个指针，不拷 5 万行。
struct Snapshot {
    head: String,
    rows: Arc<Vec<graph::Row>>,
}

/// 每个注册仓库留最近一份分配结果。5 万提交也就几 MB，所以不做 LRU，只覆盖。
#[derive(Default)]
pub struct Cache {
    by_repo: Mutex<HashMap<i64, Snapshot>>,
}

impl Cache {
    fn reuse(&self, id: i64, head: &str) -> Option<Arc<Vec<graph::Row>>> {
        lock(&self.by_repo)
            .get(&id)
            .filter(|snap| snap.head == head)
            .map(|snap| snap.rows.clone())
    }

    fn remember(&self, id: i64, head: String, rows: Arc<Vec<graph::Row>>) {
        lock(&self.by_repo).insert(id, Snapshot { head, rows });
    }

    /// 丢掉这个仓库的分配结果。仓库被移除、或 HEAD 消失（空仓库）时走这里；
    /// 键本身已经能挡住"提交后拿旧图"的情况，所以这个方法是兜底，不是失效机制的主力。
    pub fn invalidate(&self, id: i64) {
        lock(&self.by_repo).remove(&id);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 读一页提交图。
///
/// `count` 是列表那一页**实际拿到**的行数，不是页大小：尾页常常不足一页，
/// 两边用同一个数才切得出同一批行（Rust 侧 `--topo-order` 一致，行按 sha 对齐）。
/// 传成页大小会让前端出现"有行没有图"的空洞。
///
/// browse（treeless 只读浏览）仓库允许读图：它只要父子关系，不碰文件内容。
#[tauri::command]
pub async fn commit_graph(
    state: State<'_, Arc<Db>>,
    cache: State<'_, Arc<Cache>>,
    id: i64,
    skip: usize,
    count: usize,
) -> Result<graph::Page, GitError> {
    let (path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let cache = cache.inner().clone();
    // 前端算错了个数也不至多拷一屏之外的行进来：这个夹取两边共用同一个常数
    let count = count.clamp(1, MAX_PAGE_SIZE);

    tauri::async_runtime::spawn_blocking(move || page_of(&path, &cache, id, skip, count))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 同一次 IPC 内的流程：问 HEAD → 命中就切片，没命中才走全历史。
fn page_of(
    path: &Path,
    cache: &Cache,
    id: i64,
    skip: usize,
    count: usize,
) -> Result<graph::Page, GitError> {
    let Some(head) = graph::head_sha(path)? else {
        // 空仓库：列表也是空的，两边一致
        cache.invalidate(id);
        return Ok(graph::Page::default());
    };

    let rows = match cache.reuse(id, &head) {
        Some(rows) => rows,
        None => {
            // 两个请求同时落空会各算一遍，结果一致，所以不为这个加跨 await 的异步锁：
            // 多花一次读，换掉一整层同步原语。
            let rows = Arc::new(graph::plan(&graph::history(path)?));
            cache.remember(id, head.clone(), rows.clone());
            rows
        }
    };

    Ok(graph::page(&rows, skip, count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(shas: &[&str]) -> Arc<Vec<graph::Row>> {
        Arc::new(
            shas.iter()
                .map(|sha| graph::Row {
                    sha: (*sha).to_string(),
                    lane: 0,
                    color: 0,
                    incoming: false,
                    segments: Vec::new(),
                })
                .collect(),
        )
    }

    #[test]
    fn a_new_head_invalidates_the_stale_plan() {
        let cache = Cache::default();
        cache.remember(1, "old".to_string(), rows(&["old", "a"]));

        // 注意这里得先把 Arc 落成绑定再取 &str：`reuse(..).map(|rows| rows.iter()..)`
        // 会返回借自一个闭包局部 Arc 的引用，编译不过
        let reused = cache.reuse(1, "old").expect("HEAD 没变就该复用");
        assert_eq!(
            reused.iter().map(|row| row.sha.as_str()).collect::<Vec<_>>(),
            vec!["old", "a"]
        );
        assert!(cache.reuse(1, "new").is_none(), "HEAD 变了必须重算");

        cache.remember(1, "new".to_string(), rows(&["new", "old", "a"]));
        assert!(cache.reuse(1, "old").is_none(), "旧的键不该还能被查到");
        assert_eq!(cache.reuse(1, "new").map(|rows| rows.len()), Some(3));
    }

    #[test]
    fn caches_are_kept_per_repo() {
        let cache = Cache::default();
        cache.remember(1, "x".to_string(), rows(&["x"]));
        cache.remember(2, "y".to_string(), rows(&["y", "x"]));

        assert_eq!(cache.reuse(1, "x").map(|rows| rows.len()), Some(1));
        assert_eq!(cache.reuse(2, "y").map(|rows| rows.len()), Some(2));
        assert_eq!(cache.reuse(2, "x").map(|rows| rows.len()), None, "另一个仓库的 HEAD 不算");

        cache.invalidate(1);
        assert_eq!(cache.reuse(1, "x").map(|rows| rows.len()), None);
        assert_eq!(cache.reuse(2, "y").map(|rows| rows.len()), Some(2), "别的一直在");
    }

    /// 行对齐是这张图唯一不能出错的地方：图页和列表页是两次独立 IPC
    #[test]
    fn the_page_slices_at_the_same_offsets_as_the_list() {
        let all = rows(&["e", "d", "c", "b", "a"]);
        let page = graph::page(&all, 1, 2);
        assert_eq!(
            page.rows.iter().map(|row| row.sha.as_str()).collect::<Vec<_>>(),
            vec!["d", "c"]
        );
        assert_eq!(graph::page(&all, 0, 5).rows.len(), 5, "limit 大于剩余就给剩余");
        assert!(graph::page(&all, 5, 5).rows.is_empty(), "skip 落在末尾之后是空页");
    }
}
