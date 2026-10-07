use serde::Serialize;

use super::spec::Spec;
use crate::git::message;

/// 一条不合规原因。名字与需求 6.6 的"原因分类穷举"一一对应，
/// 报告按它分桶计数，表单按它 `blocking` 决定能不能提交。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// subject 为空
    EmptySubject,
    /// 完全没有 `type:` 头部
    MissingType,
    /// 有 type 但不在白名单里
    InvalidType,
    /// 规范要求 scope 却没写
    MissingScope,
    /// subject 超过阈值（只告警）
    SubjectTooLong,
    /// 规范要求任务 ID 却没找到
    MissingTaskId,
    /// 整条 subject 都是无信息量词
    NonInformative,
    /// 有冒号但冒号左边不是合法的 type
    MalformedHeader,
    /// 用了中文冒号
    FullwidthColon,
    /// subject 以句号结尾
    TrailingPeriod,
}

impl Reason {
    /// 需求 6.2：subject 超长只告警不阻断，其余都拦。
    pub fn blocking(self) -> bool {
        !matches!(self, Self::SubjectTooLong)
    }

    /// 报告与表单共用的一句话说明。
    pub fn title(self) -> &'static str {
        match self {
            Self::EmptySubject => "提交标题为空",
            Self::MissingType => "缺少 type 前缀",
            Self::InvalidType => "type 不在白名单里",
            Self::MissingScope => "缺少 scope",
            Self::SubjectTooLong => "提交标题过长",
            Self::MissingTaskId => "缺少任务 ID",
            Self::NonInformative => "提交信息无实际内容",
            Self::MalformedHeader => "提交头部格式无法解析",
            Self::FullwidthColon => "使用了中文冒号",
            Self::TrailingPeriod => "提交标题以句号结尾",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Violation {
    pub reason: Reason,
    pub blocking: bool,
    pub title: String,
    /// 具体到这条消息该怎么改，界面直接显示
    pub hint: String,
}

/// 一次校验的完整结果。`conformant` 是"没有阻断级问题"，
/// 也就是符合率报告统计时用的那把尺子——告警级不影响合规。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub violations: Vec<Violation>,
    pub conformant: bool,
    pub commit_type: Option<String>,
    pub scope: Option<String>,
    pub breaking: bool,
}

/// 无信息量词表。判定方式是"整条描述拆词后每个词都命中"，
/// 所以 `feat: 修复崩溃` 正常通过，而 `更新` / `wip fix` 会被拦下。
const NON_INFORMATIVE: &[&str] = &[
    "wip", "wips", "tmp", "temp", "todo", "misc", "test", "tests", "update", "updates", "updated",
    "change", "changes", "commit", "commits", "merge", "fix", "fixes", "fixing", "bug", "bugfix",
    "stuff", "asdf", "aaa", "1", "111", "0", ".", "临时", "更新", "修改", "提交", "测试", "修复",
    "优化", "整理", "补交", "无", "空", "略",
];

pub fn evaluate(spec: &Spec, subject: &str, body: &str) -> Outcome {
    let summary = message::summarize(subject, body);
    let mut violations = Vec::new();
    // 有冒号时"描述"是冒号右边，没有冒号时整条 subject 就是描述——
    // 无信息量词和句尾句号两种情况在裸标题（`wip`、`更新。`）上同样要拦
    let desc = subject
        .split_once(':')
        .map_or(subject, |(_, rest)| rest)
        .trim();
    let fullwidth = has_fullwidth_colon(subject);

    if subject.trim().is_empty() {
        push(
            &mut violations,
            Reason::EmptySubject,
            "写一句说清楚这次改了什么。",
        );
    } else {
        if fullwidth {
            push(
                &mut violations,
                Reason::FullwidthColon,
                "把全角冒号换成半角 `:`，规范要求 `type: 描述`。",
            );
        }
        if summary.commit_type.is_none() && !subject.contains(':') && !fullwidth {
            push(
                &mut violations,
                Reason::MissingType,
                "按 `type(scope): 描述` 重写，type 从下拉里选。",
            );
        }
        if let Some(invalid) = invalid_type(spec, &summary.commit_type) {
            push(
                &mut violations,
                Reason::InvalidType,
                &format!(
                    "`{invalid}` 不在白名单里，本仓库允许的 type：{}。",
                    spec.types.join(", ")
                ),
            );
        }
        // 解析不出 type 有两种原因，各报各的：冒号左边写坏了才算"头部无法解析"，
        // 右边空着是"标题为空"（message.rs 里空描述本来就不算规范头部，见 empty_description_is_not_conventional）
        let unparsable_header = subject.contains(':') && !fullwidth && !desc.is_empty();
        if summary.commit_type.is_none() && unparsable_header {
            push(
                &mut violations,
                Reason::MalformedHeader,
                "冒号左边要写成字母/数字/-/_ 组成的 type，例如 `fix` 或 `feat`。",
            );
        }
        if spec.scope_required && summary.commit_type.is_some() && summary.scope.is_none() {
            push(
                &mut violations,
                Reason::MissingScope,
                "本仓库要求写 scope，形如 `feat(界面): 描述`。",
            );
        }
        // 表单在用户还没打描述时就会拼出 `feat:`，这种"有头部没描述"的消息同样是空标题，
        // 而 is_non_informative 对空串返回 false——不补这一条，前端就会显示一片绿灯的可提交。
        if subject.contains(':') && desc.is_empty() {
            push(
                &mut violations,
                Reason::EmptySubject,
                "冒号右边还空着，写清楚这次改了什么。",
            );
        }
        if is_non_informative(desc) {
            push(
                &mut violations,
                Reason::NonInformative,
                "描述要有具体对象和动作，例如 `修复导出 CSV 时列错位`。",
            );
        }
        if ends_with_period(desc) {
            push(
                &mut violations,
                Reason::TrailingPeriod,
                "去掉标题末尾的句号。",
            );
        }
        let length = subject.chars().count();
        if length > spec.subject_max_length {
            push(
                &mut violations,
                Reason::SubjectTooLong,
                &format!(
                    "当前 {length} 个字符，阈值 {}，建议把细节挪进正文。",
                    spec.subject_max_length
                ),
            );
        }
        if let Some(pattern) = &spec.task_id_pattern {
            let present = regex::Regex::new(pattern)
                .map(|re| re.is_match(subject) || re.is_match(body))
                // 配置里的正则编译不过：这是配置写错了，不该拦住用户提交
                .unwrap_or(true);
            if !present {
                push(
                    &mut violations,
                    Reason::MissingTaskId,
                    &format!("标题或正文里要有一处匹配 `{pattern}` 的任务 ID。"),
                );
            }
        }
    }

    Outcome {
        conformant: violations.iter().all(|v| !v.blocking),
        commit_type: summary.commit_type,
        scope: summary.scope,
        breaking: summary.breaking,
        violations,
    }
}

fn push(out: &mut Vec<Violation>, reason: Reason, hint: &str) {
    out.push(Violation {
        reason,
        blocking: reason.blocking(),
        title: reason.title().to_string(),
        hint: hint.to_string(),
    });
}

fn has_fullwidth_colon(subject: &str) -> bool {
    match (subject.find('：'), subject.find(':')) {
        (Some(_), None) => true,
        (Some(full), Some(ascii)) => full < ascii,
        (None, _) => false,
    }
}

/// commit_type 为 None 时不报"非法 type"（那是"无 type"或"格式解析失败"的范畴），
/// 避免同一条提交被三个原因重复计数。
fn invalid_type<'a>(spec: &Spec, commit_type: &'a Option<String>) -> Option<&'a str> {
    if spec.types.is_empty() {
        return None;
    }
    commit_type
        .as_ref()
        .filter(|ty| !spec.types.iter().any(|allowed| allowed == *ty))
        .map(String::as_str)
}

fn is_non_informative(description: &str) -> bool {
    let words: Vec<String> = description
        .split(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    ',' | '.' | '!' | '?' | ';' | ':' | '，' | '。' | '！' | '？' | '、'
                )
        })
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    !words.is_empty() && words.iter().all(|w| NON_INFORMATIVE.contains(&w.as_str()))
}

fn ends_with_period(description: &str) -> bool {
    let trimmed = description.trim_end();
    trimmed.ends_with('.') || trimmed.ends_with('。')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(subject: &str) -> Outcome {
        evaluate(&Spec::default(), subject, "")
    }

    fn reasons(subject: &str) -> Vec<Reason> {
        outcome(subject)
            .violations
            .iter()
            .map(|v| v.reason)
            .collect()
    }

    #[test]
    fn a_clean_conventional_subject_has_no_violations() {
        let check = outcome("feat: 支持按仓库地址只读浏览");
        assert!(
            check.violations.is_empty(),
            "不该有问题：{:?}",
            check.violations
        );
        assert!(check.conformant);
        assert_eq!(check.commit_type.as_deref(), Some("feat"));
    }

    #[test]
    fn missing_type_is_blocking() {
        let check = outcome("更新了一批文件");
        assert!(reasons("更新了一批文件").contains(&Reason::MissingType));
        assert!(!check.conformant, "无 type 必须拦下来");
    }

    #[test]
    fn empty_subject_is_flagged_once_not_three_times() {
        let check = outcome("   ");
        assert_eq!(
            check.violations.len(),
            1,
            "空标题只报一条：{:?}",
            check.violations
        );
        assert_eq!(check.violations[0].reason, Reason::EmptySubject);
        assert!(check.violations[0].blocking);
    }

    #[test]
    fn type_outside_the_whitelist_is_named_in_the_hint() {
        let check = outcome("feaat: 拼错的类型");
        let violation = check
            .violations
            .iter()
            .find(|v| v.reason == Reason::InvalidType)
            .expect("要报非法 type");
        assert!(violation.hint.contains("feaat"));
        assert!(violation.hint.contains("feat"), "提示里要能看见允许的取值");
        assert!(!check.conformant);
    }

    #[test]
    fn malformed_header_is_separate_from_missing_type() {
        // 有冒号但左边不是合法 type：报"格式解析失败"，不重复报"缺少 type"
        assert_eq!(reasons("some stuff: 值"), vec![Reason::MalformedHeader]);
        assert!(!reasons("更新了一批文件").contains(&Reason::MalformedHeader));
    }

    #[test]
    fn fullwidth_colon_is_reported_and_not_double_counted_as_missing_type() {
        assert_eq!(reasons("feat：用了中文冒号"), vec![Reason::FullwidthColon]);
    }

    #[test]
    fn overlong_subject_warns_without_blocking() {
        let long = format!("feat: {}", "修".repeat(80));
        let check = outcome(&long);
        assert!(reasons(&long).contains(&Reason::SubjectTooLong));
        assert!(
            check.violations.iter().all(|v| !v.blocking),
            "超长只能是告警"
        );
        assert!(check.conformant, "告警不影响合规判定");
        let hint = &check.violations[0].hint;
        assert!(
            hint.contains('8') || hint.contains("86"),
            "提示里要带实际长度：{hint}"
        );
    }

    #[test]
    fn a_header_without_a_description_is_still_an_empty_subject() {
        // 表单只选了 type、描述还没打时拼出来的就是这个形状，必须拦住
        assert_eq!(reasons("feat: "), vec![Reason::EmptySubject]);
        assert_eq!(reasons("fix(cli)!:"), vec![Reason::EmptySubject]);
        assert!(!outcome("feat: ").conformant);
    }

    #[test]
    fn non_informative_descriptions_are_caught_but_specific_ones_pass() {
        assert!(reasons("feat: wip").contains(&Reason::NonInformative));
        assert!(reasons("fix: 更新 修改").contains(&Reason::NonInformative));
        assert!(!reasons("fix: 更新依赖版本").contains(&Reason::NonInformative));
        assert!(!reasons("feat: 修复崩溃").contains(&Reason::NonInformative));
    }

    #[test]
    fn trailing_period_is_a_violation() {
        assert!(reasons("feat: 加个开关。").contains(&Reason::TrailingPeriod));
        assert!(!reasons("feat: 加个开关").contains(&Reason::TrailingPeriod));
    }

    #[test]
    fn scope_requirement_comes_from_the_spec() {
        let spec = Spec {
            scope_required: true,
            ..Spec::default()
        };
        assert!(reasons_with(&spec, "feat: 没有 scope").contains(&Reason::MissingScope));
        assert!(!reasons_with(&spec, "feat(cli): 有 scope").contains(&Reason::MissingScope));
        // 没 type 的时候不叠加"缺 scope"，一条提交不该同时背两个前缀类原因
        assert!(!reasons_with(&spec, "随手写的标题").contains(&Reason::MissingScope));
    }

    #[test]
    fn task_id_pattern_is_searched_in_subject_and_body() {
        let spec = Spec {
            task_id_pattern: Some(r"[A-Z]+-\d+".to_string()),
            ..Spec::default()
        };
        assert!(reasons_with(&spec, "feat: 缺少单号").contains(&Reason::MissingTaskId));
        assert!(
            !reasons_with(&spec, "feat(TIDY-12): 标题里带单号").contains(&Reason::MissingTaskId)
        );
        let in_body = evaluate(&spec, "feat: 单号在正文", "关联 TIDY-13 处理");
        assert!(!in_body
            .violations
            .iter()
            .any(|v| v.reason == Reason::MissingTaskId));
    }

    #[test]
    fn a_broken_task_id_pattern_does_not_block_the_user() {
        let spec = Spec {
            task_id_pattern: Some("([未闭合".to_string()),
            ..Spec::default()
        };
        assert!(!reasons_with(&spec, "feat: 正常标题").contains(&Reason::MissingTaskId));
    }

    #[test]
    fn an_empty_whitelist_means_any_type_shape_is_accepted() {
        let spec = Spec {
            types: Vec::new(),
            ..Spec::default()
        };
        let check = evaluate(&spec, "spike: 探索性提交", "");
        assert!(check.violations.is_empty(), "{:?}", check.violations);
    }

    #[test]
    fn breaking_mark_and_revert_are_reported_back_to_the_form() {
        let check = outcome("fix(parser)!: 换掉解析器入口");
        assert!(check.breaking);
        assert_eq!(check.scope.as_deref(), Some("parser"));
    }

    fn reasons_with(spec: &Spec, subject: &str) -> Vec<Reason> {
        evaluate(spec, subject, "")
            .violations
            .iter()
            .map(|v| v.reason)
            .collect()
    }
}
