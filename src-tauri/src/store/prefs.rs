use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::GitError;

/// 个人偏好（需求 6.7 的第二层）。
///
/// 与团队规范（仓库里的 `git-tidy.config.json`）、本地索引（app data 里的 SQLite）
/// 分开的三层之一：**这一层不提交进仓库**，跟着机器走，所以它存 app config dir 的
/// `preferences.json`，而不是仓库目录。
///
/// 序列化用 camelCase 且每项都有默认值：老版本写的文件缺字段时按默认补齐，
/// 而不是整份读不出来——丢一份设置比多一个默认值糟糕得多。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    /// 拉取策略。快进优先是这里的默认：合不进去就停住让人决定（需求 6.14）
    pub pull_strategy: PullStrategy,
    /// Git Flow 的命名前缀（需求 7.16）：可改，不用就改空串
    pub flow_prefixes: FlowPrefixes,
    /// 启动时检查一次更新（需求 7.24）。关掉就完全不联网问版本
    pub auto_update: bool,
    /// 提交列表显示哪些列。默认全开——省的是眼睛，不是数据
    pub columns: Columns,
    pub window: WindowState,
    /// AI 生成提交信息的接入配置（§AI）。空 = 未启用。
    /// api_key 明文落在本地 preferences.json 里，这是本地桌面应用的取舍：
    /// 走前端直接调 LLM（参见前端 api/ai.ts），不引入 Rust 网络依赖。
    pub ai: AiConfig,
}

/// AI 接入配置。`endpoint` 是 OpenAI 兼容的 `/chat/completions` 基址
/// （可接 OpenAI / DeepSeek / 通义 / 本地 Ollama 等），`model` 是该服务上的模型名。
///
/// 三个字段默认全空 = 未启用：没填完的半套配置不该被当成可用状态。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiConfig {
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullStrategy {
    /// 快进优先：合不进去就停，不自动制造合并提交（需求 6.14）
    #[default]
    FfOnly,
    Rebase,
    Merge,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FlowPrefixes {
    pub feature: String,
    pub hotfix: String,
    pub release: String,
}

impl Default for FlowPrefixes {
    fn default() -> Self {
        Self {
            feature: "feature/".into(),
            hotfix: "hotfix/".into(),
            release: "release/".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Columns {
    pub refs: bool,
    pub author: bool,
    pub time: bool,
    pub sha: bool,
}

impl Default for Columns {
    fn default() -> Self {
        Self {
            refs: true,
            author: true,
            time: true,
            sha: true,
        }
    }
}

/// 窗口尺寸。下限写死在这儿而不是散在两处：小于这个尺寸的窗口会让左栏与主区
/// 互相挤成一条缝，而那是用户拖出来的、不是默认值。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowState {
    pub width: u32,
    pub height: u32,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
        }
    }
}

pub const MIN_WINDOW_WIDTH: u32 = 960;
pub const MIN_WINDOW_HEIGHT: u32 = 600;

impl Default for Preferences {
    fn default() -> Self {
        Self {
            pull_strategy: PullStrategy::default(),
            flow_prefixes: FlowPrefixes::default(),
            auto_update: true,
            columns: Columns::default(),
            window: WindowState::default(),
            ai: AiConfig::default(),
        }
    }
}

impl Preferences {
    /// 写盘之前收一收边：窗口小到某个程度就没法用了，夹住比让它存进去再打不开强。
    /// 放在这一层而不是命令层，是因为改设置的入口不止一个（设置页、命令行、测试）。
    fn clamped(&self) -> Preferences {
        let mut fixed = self.clone();
        fixed.window.width = fixed.window.width.max(MIN_WINDOW_WIDTH);
        fixed.window.height = fixed.window.height.max(MIN_WINDOW_HEIGHT);
        fixed
    }
}

/// 偏好存储。内存里一份 + 磁盘一份 JSON。
///
/// 用 `RwLock` 而不是每次读盘：设置页的控件会连续改好几次，每次起一次进程读文件
/// 太浪费；而写盘是同步的，代价也只有一个几百字节的文件。
pub struct Prefs {
    path: PathBuf,
    current: RwLock<Preferences>,
}

impl Prefs {
    /// 读不到就用默认值，**读坏了也用默认值**：设置文件坏了不该让应用起不来，
    /// 而坏掉的那份内容留在原处，用户还能自己去修。
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("preferences.json");
        let current = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Preferences>(&raw).ok())
            .unwrap_or_default();
        Self {
            path,
            current: RwLock::new(current),
        }
    }

    pub fn get(&self) -> Preferences {
        self.current
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// 整份替换（前端每次都交完整对象，省掉一层"哪些字段变了"的推断）。
    pub fn replace(&self, next: Preferences) -> Result<Preferences, GitError> {
        let next = next.clamped();
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|err| GitError::Internal(err.to_string()))?;
        }
        let body = serde_json::to_string_pretty(&next)
            .map_err(|err| GitError::Internal(err.to_string()))?;
        // 同目录临时文件 + rename：写到一半断电不会留下半个 JSON，
        // 而这个文件是应用启动就要读的
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, body).map_err(|err| GitError::Internal(err.to_string()))?;
        std::fs::rename(&temp, &self.path).map_err(|err| GitError::Internal(err.to_string()))?;
        if let Ok(mut guard) = self.current.write() {
            *guard = next.clone();
        }
        Ok(next)
    }

    /// 设置文件在哪。设置页要显示它——用户得知道自己在改哪个文件
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Prefs) {
        let dir = tempfile::tempdir().expect("tempdir");
        let prefs = Prefs::load(dir.path());
        (dir, prefs)
    }

    #[test]
    fn defaults_are_the_ones_the_ui_shows() {
        let (_dir, prefs) = store();
        let got = prefs.get();
        assert_eq!(got.pull_strategy, PullStrategy::FfOnly);
        assert_eq!(got.flow_prefixes.feature, "feature/");
        assert!(got.auto_update, "默认要检查更新，要关只能显式关");
        assert!(got.columns.refs && got.columns.sha);
    }

    #[test]
    fn a_change_survives_a_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = Prefs::load(dir.path());
        let mut next = first.get();
        next.pull_strategy = PullStrategy::Rebase;
        next.columns.author = false;
        first.replace(next).expect("write");

        // 重新 load 模拟下次启动
        let second = Prefs::load(dir.path());
        assert_eq!(second.get().pull_strategy, PullStrategy::Rebase);
        assert!(!second.get().columns.author);
    }

    /// 旧版本写的文件没有新字段：补默认值，而不是整份丢掉
    #[test]
    fn an_older_file_gains_new_fields_as_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("preferences.json"),
            r#"{"pullStrategy":"merge"}"#,
        )
        .expect("write");
        let prefs = Prefs::load(dir.path());
        let got = prefs.get();
        assert_eq!(got.pull_strategy, PullStrategy::Merge);
        assert_eq!(got.flow_prefixes.hotfix, "hotfix/", "缺的字段走默认");
    }

    /// 设置文件坏了不挡启动：应用起得来比设置准确重要
    #[test]
    fn a_broken_file_falls_back_to_defaults_without_failing_startup() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("preferences.json"), "{ 这不是 JSON").expect("write");
        let prefs = Prefs::load(dir.path());
        assert_eq!(prefs.get(), Preferences::default());
        assert!(
            dir.path().join("preferences.json").exists(),
            "坏掉的那份留在原处，用户还能自己去修"
        );
    }

    #[test]
    fn a_window_smaller_than_usable_is_clamped() {
        let (_dir, prefs) = store();
        let mut next = prefs.get();
        next.window.width = 100;
        next.window.height = 10;
        let saved = prefs.replace(next).expect("write");
        assert_eq!(saved.window.width, MIN_WINDOW_WIDTH);
        assert_eq!(saved.window.height, MIN_WINDOW_HEIGHT);
    }

    /// 存进去的必须是合法 JSON，且读回来等于写进去的
    #[test]
    fn what_is_written_can_be_read_back() {
        let (dir, prefs) = store();
        let mut next = prefs.get();
        next.flow_prefixes.feature = "feat/".into();
        prefs.replace(next.clone()).expect("write");
        let raw = std::fs::read_to_string(dir.path().join("preferences.json")).expect("read");
        let parsed: Preferences = serde_json::from_str(&raw).expect("合法 JSON");
        assert_eq!(parsed, next);
        assert_eq!(prefs.get(), next, "内存里也要立刻是新值");
    }

    #[test]
    fn the_settings_file_lives_next_to_the_app_config() {
        let (dir, prefs) = store();
        assert_eq!(prefs.path(), dir.path().join("preferences.json"));
    }
}
