use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use tauri::State;

use super::commit::MAX_PAGE_SIZE;
use crate::config::spec::{self, Spec};
use crate::error::GitError;
use crate::git::log::{self, Filter};
use crate::git::graph;
use crate::store::db::{query, Db};
use crate::store::repos::{self, RepoKind};

/// 一个仓库的一次图计算结果。
///
/// 键是 "HEAD 的 sha + 筛选条件签名"：提交、amend、reset 之后 sha 自己就变了，
/// 而换一组筛选条件可见集合也变了，两样都必须进键——只拿 sha 当键的话，
/// 筛完之后会拿到未筛选那份泳道，那是最难发现的一类错图（需求 7.7）。
/// 值按 `Arc` 存，翻页时只多拷一个指针，不拷 5 万行。
#[derive(Clone)]
struct Entry {
    key: String,
    rows: Arc<Vec<graph::Row>>,
}

/// 每个注册仓库留最近几份分配结果。5 万提交也就几 MB，所以不做磁盘缓存，只覆盖。
///
/// 留 3 份是因为常见用法是"筛一下→看一眼→清掉筛选"，反复切换时能命中；
/// 再多就是拿内存换没人用得上的命中率。
const KEEP_PER_REPO: usize = 3;

#[derive(Default)]
pub struct Cache {
    by_repo: Mutex<HashMap<i64, Vec<Entry>>>,
}

impl Cache {
    fn reuse(&self, id: i64, key: &str) -> Option<Arc<Vec<graph::Row>>> {
        lock(&self.by_repo)
            .get(&id)
            .and_then(|entries| entries.iter().find(|entry| entry.key == key))
            .map(|entry| entry.rows.clone())
    }

    fn remember(&self, id: i64, key: String, rows: Arc<Vec<graph::Row>>) {
        let mut by_repo = lock(&self.by_repo);
        let entries = by_repo.entry(id).or_default();
        entries.retain(|entry| entry.key != key);
        entries.insert(0, Entry { key, rows });
        entries.truncate(KEEP_PER_REPO);
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
    filter: Option<Filter>,
) -> Result<graph::Page, GitError> {
    let (path, kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let cache = cache.inner().clone();
    // 前端算错了个数也不至多拷一屏之外的行进来：这个夹取两边共用同一个常数
    let count = count.clamp(1, MAX_PAGE_SIZE);
    let filter = filter.unwrap_or_default();

    tauri::async_runtime::spawn_blocking(move || {
        let spec = match kind {
            RepoKind::Worktree => spec::load(&path),
            RepoKind::Browse => spec::load_at_head(&path),
        };
        page_of(&path, &cache, id, skip, count, &filter, &spec)
    })
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
    filter: &Filter,
    spec: &Spec,
) -> Result<graph::Page, GitError> {
    let Some(head) = graph::head_sha(path)? else {
        // 空仓库：列表也是空的，两边一致
        cache.invalidate(id);
        return Ok(graph::Page::default());
    };
    // 键带筛选签名：未筛选时就是 "HEAD sha|"，形状与以前一致
    let key = format!("{head}|{}", filter.key());

    let rows = match cache.reuse(id, &key) {
        Some(rows) => rows,
        None => {
            // 两个请求同时落空会各算一遍，结果一致，所以不为这个加跨 await 的异步锁：
            // 多花一次读，换掉一整层同步原语。
            let rows = Arc::new(graph::plan(&visible_walk(path, filter, spec)?));
            cache.remember(id, key, rows.clone());
            rows
        }
    };

    Ok(graph::page(&rows, skip, count))
}

/// 图的输入永远是"可见集合"（§7.7）。
///
/// 未筛选时只读父子关系，不碰 subject/body，5 万提交也就几 MB；
/// 筛选时要判 type 与合规，只能把列表那一层的解析重来一遍——这一遍更贵，
/// 所以结果进缓存，再翻页不再重算。
fn visible_walk(
    path: &Path,
    filter: &Filter,
    spec: &Spec,
) -> Result<Vec<graph::Node>, GitError> {
    if filter.is_empty() {
        return graph::history(path);
    }
    log::visible_nodes(path, filter, spec)
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
        cache.remember(1, "old|".to_string(), rows(&["old", "a"]));

        // 注意这里得先把 Arc 落成绑定再取 &str：`reuse(..).map(|rows| rows.iter()..)`
        // 会返回借自一个闭包局部 Arc 的引用，编译不过
        let reused = cache.reuse(1, "old|").expect("键没变就该复用");
        assert_eq!(
            reused.iter().map(|row| row.sha.as_str()).collect::<Vec<_>>(),
            vec!["old", "a"]
        );
        assert!(cache.reuse(1, "new|").is_none(), "HEAD 变了必须重算");
    }

    /// 换筛选条件必须换一份可见集合。只拿 sha 当键的话，筛完会拿到未筛选那份泳道
    #[test]
    fn a_different_filter_is_a_different_entry() {
        let cache = Cache::default();
        let plain = format!("head|{}", Filter::default().key());
        let only_feat = format!(
            "head|{}",
            Filter {
                types: vec!["feat".into()],
                ..Filter::default()
            }
            .key()
        );
        assert_ne!(plain, only_feat, "两组筛选不能同键");

        cache.remember(1, plain.clone(), rows(&["a", "b"]));
        cache.remember(1, only_feat.clone(), rows(&["a"]));

        assert_eq!(cache.reuse(1, &only_feat).map(|rows| rows.len()), Some(1));
        assert_eq!(cache.reuse(1, &plain).map(|rows| rows.len()), Some(2));
    }

    #[test]
    fn caches_are_kept_per_repo() {
        let cache = Cache::default();
        cache.remember(1, "x|x".to_string(), rows(&["x"]));
        cache.remember(2, "y|x".to_string(), rows(&["y", "x"]));

        assert_eq!(cache.reuse(1, "x|x").map(|rows| rows.len()), Some(1));
        assert_eq!(cache.reuse(2, "y|x").map(|rows| rows.len()), Some(2));
        assert_eq!(cache.reuse(1, "y|x").map(|rows| rows.len()), None, "另一个仓库的键不算");

        cache.invalidate(1);
        assert_eq!(cache.reuse(1, "x|x").map(|rows| rows.len()), None);
        assert_eq!(cache.reuse(2, "y|x").map(|rows| rows.len()), Some(2), "别的一直在");
    }

    /// 反复切筛选时不重算；超过保留份数才挤掉最老的那份
    #[test]
    fn only_a_few_entries_are_kept_per_repo() {
        let cache = Cache::default();
        for index in 0..(KEEP_PER_REPO + 2) {
            cache.remember(1, format!("k{index}"), rows(&["x"]));
        }
        assert_eq!(
            lock(&cache.by_repo).get(&1).map(Vec::len),
            Some(KEEP_PER_REPO),
            "留太多份就是拿内存换没人用得上的命中率"
        );
        assert!(cache.reuse(1, "k0").is_none(), "最老那几份已经被挤掉了");
        assert!(cache.reuse(1, &format!("k{}", KEEP_PER_REPO + 1)).is_some());
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
