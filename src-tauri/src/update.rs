use serde::Serialize;

use crate::error::GitError;

/// 「检查更新」（需求 7.24，但**只查不装**）。
///
/// 边界先说清：这个模块只回答"有没有新版本、去哪下"。它**不下载、不替换、不静默升级**——
/// 替换正在运行的 exe 是整件事里最脏的一段（文件被占用、下载中断、版本回退），
/// 而 Git 客户端不是天天更新的软件，让人自己点一次下载代价小得多。
///
/// **请求在前端发，判定在这里做**。不是图省事，而是两个理由：
/// 1. Rust 侧目前没有 HTTP 客户端依赖，为一次 GET 引入一个（含 TLS）依赖不划算——
///    等真要做完整 updater 时再一起加，那时它才有存在的理由；
/// 2. 请求就是一个公开的 `GET api.github.com/.../releases/latest`，前端 `fetch` 走
///    WebView 的网络栈，与本应用其它"打开一个网址"的动作同一性质。
///    出网面仍然只有那一个 URL，且可在设置里完全关掉。
///
/// 解析与比较放在这里（而不是前端），是为了让它们能被 Rust 测试守住——
/// 版本号比较是最容易写错又最难看出错的地方。
pub const DEFAULT_REPO: &str = "qbbyte/git-tidy";

/// 当前版本。与 `Cargo.toml` 的 `version` 同源，不在这里再写一个数
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 仓库坐标。**构建期注入**：`GIT_TIDY_REPO` 环境变量（CI 里设），没设就用上面这个默认值。
/// 写死在代码里不如让构建决定——fork 出去的版本自动去查上游的最新版是最糟的一种行为。
pub fn repo_slug() -> &'static str {
    match option_env!("GIT_TIDY_REPO") {
        Some(slug) if !slug.trim().is_empty() => slug,
        _ => DEFAULT_REPO,
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    /// 去掉 `v` 前缀的版本号
    pub version: String,
    /// GitHub 上这个 Release 的页面：下载由那边负责
    pub url: String,
    pub name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub current: String,
    pub repo: String,
    pub latest: Option<ReleaseInfo>,
    /// true 表示有更新，且 `latest` 一定不是 None
    pub available: bool,
}

/// 拿 GitHub Releases 的最新一条原始 JSON 来判定。
///
/// 入参是**原文**而不是解析好的结构：前端的职责止于"把那个 URL 拿到手"，
/// 字段怎么读、tag 怎么比较都在这里——这样规则改动只需要改一处、只需要测 Rust 侧。
pub fn compare(payload: &str) -> Result<CheckResult, GitError> {
    let release = parse_release(payload)?;
    Ok(CheckResult {
        current: current_version().to_string(),
        repo: repo_slug().to_string(),
        available: is_newer(&release.version, current_version()),
        latest: Some(release),
    })
}

/// 只取三个字段，别的（正文、附件列表）一律不看：
/// 这里的解析要是跟着 GitHub 的响应长出依赖来，一个字段改名就会让检查更新坏掉。
fn parse_release(body: &str) -> Result<ReleaseInfo, GitError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| GitError::Internal("版本信息不是合法 JSON".into()))?;
    let tag = value
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitError::Internal("版本信息里没有 tag_name".into()))?;
    Ok(ReleaseInfo {
        version: tag.trim().trim_start_matches(['v', 'V']).to_string(),
        url: value
            .get("html_url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(tag)
            .to_string(),
    })
}

/// 版本比较：只按点分数字逐段比，不引 semver 依赖。
///
/// 规则只有两条，后缀按“预发布版小于同号正式版”处理：
/// 1. 数字段逐段比，缺的那几段当 0（所以 `0.1` 与 `0.1.0` 是同一个版本）；
/// 2. 数字全同的时候，正式版比预发布版新（否则自己构建的 `0.1.0-beta.1`
///    会把自己当成"有新版"，每次启动都弹一次）。
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let left = clean(candidate);
    let right = clean(current);
    let width = left.numbers.len().max(right.numbers.len());
    for index in 0..width {
        let mine = left.numbers.get(index).copied().unwrap_or(0);
        let theirs = right.numbers.get(index).copied().unwrap_or(0);
        if mine != theirs {
            return mine > theirs;
        }
    }
    // 数字完全相同：只有“我是正式版、对方是预发布版”才算更新
    left.pre.is_none() && right.pre.is_some()
}

struct Version {
    numbers: Vec<u64>,
    pre: Option<String>,
}

fn clean(raw: &str) -> Version {
    let text = raw.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, Some(pre.to_string())),
        None => (text, None),
    };
    Version {
        numbers: core
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect(),
        pre,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_comes_from_the_manifest_not_from_a_second_literal() {
        // Cargo.toml 里改一次就够；这里写死第二个数才是版本对不上的常见原因
        assert_eq!(current_version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_repo_slug_is_injectable_at_build_time() {
        let slug = repo_slug();
        assert!(
            !slug.trim().is_empty(),
            "不能是空串：不然请求会打到一个空路径上"
        );
        assert_eq!(slug.split('/').count(), 2, "应当是 owner/repo：{slug}");
    }

    #[test]
    fn only_three_fields_are_read_out_of_the_release_payload() {
        let body = r#"{
          "tag_name": "v9.9.9",
          "name": "支持符合率报告",
          "html_url": "https://github.com/qbbyte/git-tidy/releases/tag/v9.9.9",
          "body": "一大堆正文，长度不重要",
          "assets": [{ "name": "git-tidy.msi" }],
          "draft": false
        }"#;
        let result = compare(body).expect("compare");
        let latest = result.latest.expect("有最新版本");
        assert_eq!(latest.version, "9.9.9");
        assert_eq!(latest.name, "支持符合率报告");
        assert!(latest.url.ends_with("/v9.9.9"));
        assert!(result.available, "9.9.9 比当前 0.1.0 新");
        assert_eq!(result.current, env!("CARGO_PKG_VERSION"));
    }

    /// 字段改名或响应变成 HTML 错误页，只该让这次检查失败，不能 panic
    #[test]
    fn a_payload_without_a_tag_is_an_error_not_a_panic() {
        assert!(compare(r#"{"name":"x"}"#).is_err());
        assert!(compare("不是 JSON").is_err());
        assert!(compare("").is_err());
    }

    #[test]
    fn versions_compare_by_the_numbers_that_matter() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.0"), "不能按字符串比：0.10 > 0.9");
    }

    /// 自己构建的开发版带 `-beta`，不该把自己当成"有新版"反复提示
    #[test]
    fn a_prerelease_of_the_current_version_is_not_an_update() {
        assert!(!is_newer("0.1.0-beta.1", "0.1.0"));
        assert!(!is_newer("0.1.0-beta.1", "0.1.0-beta.2"));
        assert!(is_newer("0.1.0", "0.1.0-beta.1"));
    }

    /// `0.1` 与 `0.1.0` 是同一个版本的不同写法，报"有新版"纯属骚扰
    #[test]
    fn trailing_zero_differences_are_not_updates() {
        assert!(!is_newer("0.1", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
    }

    /// 当前版本对应的 tag 不该报"有更新"——这是最常见的误报来源
    #[test]
    fn the_current_version_is_not_reported_as_an_update() {
        let payload = format!(
            r#"{{"tag_name":"v{}","html_url":"https://example.invalid","name":"同版本"}}"#,
            current_version()
        );
        let result = compare(&payload).expect("compare");
        assert!(!result.available, "tag 与当前版本相同时不该提示更新");
    }
}
