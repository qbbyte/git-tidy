use serde::Serialize;

/// Conventional Commits 1.0.0 的**头部语法**拆解结果。
///
/// 这里只回答"有没有规范形状"，不回答"这个 type 合不合法"——type 白名单、scope 是否必填、
/// 任务 ID 规则都来自仓库配置（需求 6.7），由配置层在本结果之上做判定。
/// 拆成两层是为了让 commit 列表、CHANGELOG、符合率报告共用同一份语法解析，
/// 而不是三处各写一遍规则、日后各自漂移。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// 解析不出 `type: ` 头部时为 None，即"非规范提交"。统一转小写比对交给配置层。
    pub commit_type: Option<String>,
    pub scope: Option<String>,
    /// 头部 `!` 或 body 里的 BREAKING CHANGE，两种都算（需求 6.5）
    pub breaking: bool,
}

pub fn summarize(subject: &str, body: &str) -> Summary {
    let header = parse_header(subject);
    Summary {
        commit_type: header.as_ref().and_then(|h| h.commit_type.clone()),
        scope: header.as_ref().and_then(|h| h.scope.clone()),
        breaking: header.is_some_and(|h| h.breaking) || has_breaking_footer(body),
    }
}

/// git revert 默认生成 `Revert "原标题"`，Conventional 里也有 `revert:` 这一类。
pub fn is_revert(subject: &str, commit_type: Option<&str>) -> bool {
    commit_type == Some("revert") || subject.starts_with("Revert \"")
}

struct Header {
    commit_type: Option<String>,
    scope: Option<String>,
    breaking: bool,
}

fn parse_header(subject: &str) -> Option<Header> {
    // 描述为空的 `feat:` 不算规范提交（CC 1.0.0 里 description 是必填项）
    let (head, description) = subject.split_once(':')?;
    if description.trim().is_empty() {
        return None;
    }

    let head = head.trim_end();
    let (head, breaking) = match head.strip_suffix('!') {
        Some(stripped) => (stripped.trim_end(), true),
        None => (head, false),
    };

    let (type_part, scope_part) = match (head.find('('), head.strip_suffix(')')) {
        (Some(open), Some(inner)) => (&head[..open], inner.get(open + 1..)),
        _ => (head, None),
    };

    if !is_type_token(type_part) {
        return None;
    }
    let scope = scope_part
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Some(Header {
        commit_type: Some(type_part.to_lowercase()),
        scope,
        breaking,
    })
}

/// 允许字母数字与 `-`/`_`，禁止空白与括号冒号——"Revert \"feat: x\"" 这类
/// 带空白的头部会在这里被拒，交由 revert 标记识别。
fn is_type_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn has_breaking_footer(body: &str) -> bool {
    body.lines()
        .map(str::trim_start)
        .any(|line| line.starts_with("BREAKING CHANGE:") || line.starts_with("BREAKING-CHANGE:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(subject: &str) -> Summary {
        summarize(subject, "")
    }

    #[test]
    fn plain_type_with_chinese_description() {
        let s = summary("feat: 支持中文提交标题");
        assert_eq!(s.commit_type.as_deref(), Some("feat"));
        assert_eq!(s.scope, None);
        assert!(!s.breaking);
    }

    #[test]
    fn scope_and_bang_marks_breaking() {
        let s = summary("fix(parser)!: 修正解析边界");
        assert_eq!(s.commit_type.as_deref(), Some("fix"));
        assert_eq!(s.scope.as_deref(), Some("parser"));
        assert!(s.breaking);
    }

    #[test]
    fn breaking_footer_counts_even_without_bang() {
        let s = summarize(
            "feat: 换掉存储引擎",
            "正文若干行\n\nBREAKING CHANGE: 旧配置不再兼容",
        );
        assert!(s.breaking, "footer 里的 BREAKING CHANGE 必须被识别");
        assert_eq!(s.commit_type.as_deref(), Some("feat"));
    }

    #[test]
    fn type_is_case_insensitive_but_scope_survives() {
        let s = summary("Feat(API): 大小写混用");
        assert_eq!(s.commit_type.as_deref(), Some("feat"));
        assert_eq!(s.scope.as_deref(), Some("API"));
    }

    #[test]
    fn fullwidth_colon_is_not_a_conventional_header() {
        // 中文冒号是需求 6.2 要明确拦截的写法，语法层先保证不误判成规范
        let s = summary("feat：用了中文冒号");
        assert_eq!(s.commit_type, None);
    }

    #[test]
    fn empty_description_is_not_conventional() {
        assert_eq!(summary("feat:").commit_type, None);
        assert_eq!(summary("feat:   ").commit_type, None);
    }

    #[test]
    fn no_colon_or_spaces_in_head_are_not_conventional() {
        assert_eq!(summary("just some words").commit_type, None);
        assert_eq!(summary("update stuff").commit_type, None);
    }

    #[test]
    fn later_colon_belongs_to_description_not_the_header() {
        let s = summary("chore: 发布时间 12:30");
        assert_eq!(s.commit_type.as_deref(), Some("chore"));
    }

    #[test]
    fn malformed_scope_parentheses_are_rejected() {
        assert_eq!(summary("fix(unclosed: x").commit_type, None);
    }

    #[test]
    fn revert_detected_from_subject_and_type() {
        assert!(is_revert("Revert \"feat: 加个开关\"", None));
        assert!(is_revert("revert: 撤回上一条", Some("revert")));
        assert!(!is_revert("feat: 正常提交", Some("feat")));
    }
}
