use std::path::Path;

use serde::Serialize;

/// 团队规范：一个仓库"什么算合规提交"的全部可变项。
///
/// 语法拆解在 `git::message`（只看 Conventional Commits 的形状），本模块只提供判定要用的
/// 取值，两层分开是为了让提交表单、commit-msg hook、符合率报告、CHANGELOG 共用同一份规则，
/// 而不是四处各写一遍再各自漂移（需求 6.7）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    /// 合法 type 白名单。为空表示"任何 `word: ` 头部都算有 type"。
    pub types: Vec<String>,
    pub scope_required: bool,
    /// 超过只告警不阻断（需求 6.2：subject ≤ 72 是告警阈值，不是失败）
    pub subject_max_length: usize,
    /// 任务 ID 的正则，None = 不要求。只有本工具自己的配置文件能给这一项：
    /// commitlint 侧各家插件写法不统一，从文本里猜正则等于凭空造规则。
    pub task_id_pattern: Option<String>,
    /// CHANGELOG 分组，顺序即章节顺序（需求 6.5：feat → fix → perf → refactor → 其他）
    pub groups: Vec<TypeGroup>,
    /// 规范从哪一层读来的，界面必须显示——用户得知道自己在改哪个文件
    pub source: SpecSource,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpecSource {
    /// 仓库根的 `git-tidy.config.json`
    RepoConfig,
    /// commitlint 配置推导
    Commitlint,
    /// standard-version 的 `.versionrc*` 推导
    Versionrc,
    /// git-cliff 的 `cliff.toml` 推导
    Cliff,
    /// 什么都没读到，用内置默认
    Fallback,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeGroup {
    pub ty: String,
    pub section: String,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            types: [
                "feat", "fix", "docs", "refactor", "style", "test", "perf", "build", "ci", "chore",
                "revert",
            ]
            .iter()
            .map(|t| t.to_string())
            .collect(),
            scope_required: false,
            subject_max_length: DEFAULT_SUBJECT_MAX,
            task_id_pattern: None,
            groups: default_groups(),
            source: SpecSource::Fallback,
        }
    }
}

const DEFAULT_SUBJECT_MAX: usize = 72;

fn default_groups() -> Vec<TypeGroup> {
    [
        ("feat", "Features"),
        ("fix", "Bug Fixes"),
        ("perf", "Performance"),
        ("refactor", "Refactoring"),
        ("revert", "Reverts"),
        ("docs", "Documentation"),
        ("test", "Tests"),
    ]
    .iter()
    .map(|(ty, section)| TypeGroup {
        ty: ty.to_string(),
        section: section.to_string(),
    })
    .collect()
}

/// 从仓库根读规范。查找顺序就是优先级顺序（需求 6.7）：
/// 本工具的配置文件 > commitlint > versionrc > cliff > 内置默认。
/// 兼容既有配置是硬性要求：一个只写了 commitlint 的仓库不该被要求再配一遍。
pub fn load(root: &Path) -> Spec {
    load_from(|name| read_text(&root.join(name)))
}

/// 只读浏览（treeless）仓库没有工作区，配置文件只能从 HEAD 的对象里读。
/// 走的是同一套解析逻辑，区别只在文件内容从哪来。
pub fn load_at_head(root: &Path) -> Spec {
    load_from(|name| show_at_head(root, name))
}

fn show_at_head(root: &Path, name: &str) -> Option<String> {
    let spec_name = name.to_string();
    let out = crate::git::process::run(Some(root), &["show", &format!("HEAD:{spec_name}")]).ok()?;
    out.success.then_some(out.stdout)
}

/// `read` 返回 None 表示这一层没有配置文件，继续往下找；返回 Some(内容) 但解析不出
/// 可用规范时同样往下找——一层坏配置不该让整个规范变成空的。
fn load_from(read: impl Fn(&str) -> Option<String>) -> Spec {
    if let Some(spec) = read_json_config(&read, "git-tidy.config.json") {
        return spec;
    }
    for name in [
        "commitlint.config.js",
        "commitlint.config.cjs",
        "commitlint.config.mjs",
        "commitlint.config.ts",
        ".commitlintrc.json",
        ".commitlintrc.js",
        ".commitlintrc.cjs",
        ".commitlintrc.yaml",
        ".commitlintrc.yml",
        ".commitlintrc",
    ] {
        if let Some(spec) = read_commitlint(&read, name) {
            return spec;
        }
    }
    for name in [
        ".versionrc.json",
        ".versionrc",
        ".versionrc.js",
        ".versionrc.cjs",
    ] {
        if let Some(spec) = read_versionrc(&read, name) {
            return spec;
        }
    }
    if let Some(spec) = read_cliff(&read, "cliff.toml") {
        return spec;
    }
    Spec::default()
}

fn read_text(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// `git-tidy.config.json` 是本工具的原生格式，字段直接映射，不做推导。
fn read_json_config(read: &impl Fn(&str) -> Option<String>, name: &str) -> Option<Spec> {
    let raw = read(name)?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let mut spec = Spec {
        source: SpecSource::RepoConfig,
        ..Spec::default()
    };
    if let Some(types) = owned_string_array(value.get("types")) {
        spec.types = types;
    }
    if let Some(required) = value.get("scopeRequired").and_then(|v| v.as_bool()) {
        spec.scope_required = required;
    }
    if let Some(max) = value
        .get("subjectMaxLength")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
    {
        spec.subject_max_length = max;
    }
    if let Some(pattern) = value
        .get("taskIdPattern")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    {
        // 正则编译不过就当作没配：一条写坏的团队配置不该把所有提交都拦成"缺任务 ID"
        spec.task_id_pattern = regex::Regex::new(&pattern).ok().map(|_| pattern);
    }
    if let Some(entries) = value.get("groups").and_then(|v| v.as_array()) {
        let parsed = parse_groups(entries.iter().map(|entry| {
            (
                entry.get("type").and_then(|v| v.as_str()),
                entry.get("section").and_then(|v| v.as_str()),
            )
        }));
        if !parsed.is_empty() {
            spec.groups = parsed;
        }
    }
    Some(spec)
}

/// commitlint 的 `rules` 里能可靠映射到本工具规范的只有三项：
/// type-enum → 白名单、scope-empty → scope 必填、subject-max-length → 长度。
/// JSON 走结构化读取，其余（`.js`/`.cjs`/yaml）走文本提取——求值一段 JS 等于执行仓库里的代码。
fn read_commitlint(read: &impl Fn(&str) -> Option<String>, name: &str) -> Option<Spec> {
    let raw = read(name)?;
    let mut spec = Spec {
        source: SpecSource::Commitlint,
        ..Spec::default()
    };

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
        let rules = value.get("rules").unwrap_or(&value);
        let types = rules
            .get("type-enum")
            .and_then(rule_list)
            .map(|list| {
                list.into_iter()
                    .filter(|word| !is_condition_word(word))
                    .collect::<Vec<_>>()
            })
            .filter(|list| !list.is_empty() && list.iter().any(|word| is_typeish(word)));
        // 只有一串严重级别（`[2,'error']`）等于没给白名单，这时候要继续往下一层找
        let types = types?;
        spec.types = types;
        if let Some(words) = rules.get("scope-empty").and_then(rule_list) {
            spec.scope_required = words.iter().any(|w| w == "always");
        }
        // header-max-length 故意不读：它在 config-conventional 里默认 100，
        // 量的是 `type(scope)!: subject` 整行，和"subject ≤ 72"不是同一件事
        if let Some(max) = rules.get("subject-max-length").and_then(rule_number) {
            spec.subject_max_length = max;
        }
        return Some(spec);
    }

    let types = extract_list(&raw, "type-enum")?;
    let types: Vec<String> = types
        .into_iter()
        .filter(|word| !is_condition_word(word))
        .collect();
    if types.is_empty() || !types.iter().any(|word| is_typeish(word)) {
        return None;
    }
    spec.types = types;
    if let Some(words) = extract_list(&raw, "scope-empty") {
        spec.scope_required = words.iter().any(|w| w == "always");
    }
    if let Some(max) = extract_number(&raw, "subject-max-length") {
        spec.subject_max_length = max;
    }
    Some(spec)
}

/// `.versionrc*` 的 `types[]` 同时给出白名单和 CHANGELOG 分组，是它最有用的两部分。
fn read_versionrc(read: &impl Fn(&str) -> Option<String>, name: &str) -> Option<Spec> {
    let raw = read(name)?;
    let value = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(value) => value,
        Err(_) => serde_json::from_str(&json_slice(&raw)?).ok()?,
    };
    let entries = value.get("types")?.as_array()?;
    let parsed = parse_groups(entries.iter().map(|entry| {
        (
            entry.get("type").and_then(|v| v.as_str()),
            entry.get("section").and_then(|v| v.as_str()),
        )
    }));
    if parsed.is_empty() {
        return None;
    }
    Some(Spec {
        source: SpecSource::Versionrc,
        types: parsed.iter().map(|g| g.ty.clone()).collect(),
        groups: parsed,
        ..Spec::default()
    })
}

/// git-cliff 没有"type 白名单"这个概念，能从它的 `commit_parsers` 里读出的只有
/// `message = "^feat"` 这一串前缀和各自的 group。
/// `conventional_commits = false` 时它压根不按规范解析，此时不该冒充它的规范。
fn read_cliff(read: &impl Fn(&str) -> Option<String>, name: &str) -> Option<Spec> {
    let raw = read(name)?;
    if raw.contains("conventional_commits = false") || raw.contains("conventional_commits=false") {
        return None;
    }
    let at = raw
        .find("[git.commit_parsers]")
        .or_else(|| raw.find("commit_parsers"))?;
    // 只取这一段：下一个 `\n[` 开始就是别的配置项了
    let block = raw[at..].split("\n[").next().unwrap_or("");
    let types = regex::Regex::new(r#"(?m)message\s*=\s*["']?\^([A-Za-z][A-Za-z0-9_-]*)"#)
        .ok()?
        .captures_iter(block)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_lowercase()))
        .collect::<Vec<_>>();
    if types.is_empty() {
        return None;
    }
    let sections = regex::Regex::new(r#"(?m)group\s*=\s*"([^"]+)""#)
        .ok()?
        .captures_iter(block)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect::<Vec<_>>();
    let groups = types
        .iter()
        .enumerate()
        .map(|(i, ty)| TypeGroup {
            ty: ty.clone(),
            section: sections
                .get(i)
                .cloned()
                .unwrap_or_else(|| "Other Changes".into()),
        })
        .collect();
    Some(Spec {
        source: SpecSource::Cliff,
        types,
        groups,
        ..Spec::default()
    })
}

/// commitlint 的一条规则：`[2, 'always', ['feat','fix']]` 或 `[2, 100]`。
/// 取值在**最后一个数组元素**里；没有内层数组时取那几个开关词。
fn rule_list(value: &serde_json::Value) -> Option<Vec<String>> {
    let entries = value.as_array()?;
    if let Some(inner) = entries.iter().rev().find_map(|v| v.as_array()) {
        let list: Vec<String> = inner
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        if !list.is_empty() {
            return Some(list);
        }
    }
    let words: Vec<String> = entries
        .iter()
        .filter_map(|v| v.as_str())
        .map(str::to_string)
        .filter(|w| !matches!(w.as_str(), "error" | "warning" | "disabled" | "off"))
        .collect();
    (!words.is_empty()).then_some(words)
}

fn rule_number(value: &serde_json::Value) -> Option<usize> {
    let entries = value.as_array()?;
    // `[2, 100]` 前面那个是严重级别，取最后一个数字才是阈值
    entries
        .iter()
        .filter_map(|v| v.as_u64())
        .filter(|n| *n > 2)
        .max()
        .map(|n| n as usize)
}

fn owned_string_array(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    let value = value?;
    if let Some(entries) = value.as_array() {
        let out: Vec<String> = entries
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        return (!out.is_empty()).then_some(out);
    }
    None
}

fn parse_groups<'a>(
    entries: impl Iterator<Item = (Option<&'a str>, Option<&'a str>)>,
) -> Vec<TypeGroup> {
    entries
        .filter_map(|(ty, section)| {
            let ty = ty?.to_string();
            Some(TypeGroup {
                ty,
                section: section.unwrap_or("Other Changes").to_string(),
            })
        })
        .collect()
}

fn is_typeish(word: &str) -> bool {
    word.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        && word.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
}

/// commitlint 的规则里除了取值还会有级别和开关词，提取时要能分辨。
fn is_condition_word(word: &str) -> bool {
    matches!(
        word,
        "always" | "never" | "error" | "warning" | "disabled" | "off"
    )
}

/// 在任意文本里找 `key` 后面第一个配平的 `[...]`，取出其中的引号字符串。
/// commitlint 的 JS 写法是 `'type-enum': [2, 'always', ['feat','fix']]`，所以**里层**优先；
/// 里层没有时，若外层引用了 `const types = [...]` 这样的变量（JS 配置里非常常见），按名字回去找它。
fn extract_list(text: &str, key: &str) -> Option<Vec<String>> {
    let at = text.find(key)?;
    let rest = &text[at..];
    let open = rest.find('[')?;
    let close = matching_bracket(rest, open)?;
    let outer = &rest[open..=close];
    if let Some(inner_open) = outer[1..].find('[') {
        let inner_open = inner_open + 1;
        if let Some(inner_close) = matching_bracket(outer, inner_open) {
            let words = quoted(&outer[inner_open..=inner_close]);
            if !words.is_empty() {
                return Some(words);
            }
        }
    }
    let words = quoted(outer);
    // 去掉引号内容后剩下的裸标识符就是要解析的变量名
    let bare = strip_quoted(outer);
    for ident in identifiers(&bare) {
        if let Some(resolved) = resolve_array(text, &ident) {
            return Some(resolved);
        }
    }
    (!words.is_empty()).then_some(words)
}

fn extract_number(text: &str, key: &str) -> Option<usize> {
    let at = text.find(key)?;
    let rest = &text[at..];
    let open = rest.find('[')?;
    let close = matching_bracket(rest, open)?;
    rest[open + 1..close]
        .chars()
        .rev()
        .collect::<String>()
        .split(|c: char| !c.is_ascii_digit())
        .find(|run| !run.is_empty())
        .and_then(|run| run.chars().rev().collect::<String>().parse().ok())
        .filter(|n: &usize| *n > 2)
}

fn matching_bracket(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, byte) in text.as_bytes().iter().enumerate().skip(open) {
        match byte {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn strip_quoted(text: &str) -> String {
    let mut out = String::new();
    let mut quote = None::<char>;
    for c in text.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if matches!(c, '\'' | '"' | '`') {
                    quote = Some(c);
                    out.push(' ');
                } else {
                    out.push(c);
                }
            }
        }
    }
    out
}

fn identifiers(text: &str) -> Vec<String> {
    regex::Regex::new(r"[A-Za-z_][A-Za-z0-9_]*")
        .map(|re| {
            re.find_iter(text)
                .map(|m| m.as_str().to_string())
                .filter(|word| !matches!(word.as_str(), "always" | "never" | "true" | "false"))
                .collect()
        })
        .unwrap_or_default()
}

fn resolve_array(text: &str, ident: &str) -> Option<Vec<String>> {
    let re = regex::Regex::new(&format!(r"(?s)\b{}[\s:=]+?\[", regex::escape(ident))).ok()?;
    let at = re.find(text)?;
    let open = text[at.end() - 1..].find('[')? + at.end() - 1;
    // matching_bracket 给的是相对这个切片起点的下标，要换回绝对位置再切
    let close = open + matching_bracket(&text[open..], 0)?;
    let words = quoted(&text[open..=close]);
    (!words.is_empty()).then_some(words)
}

fn quoted(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if matches!(c, '\'' | '"' | '`') {
            let mut word = String::new();
            for next in chars.by_ref() {
                if next == c {
                    break;
                }
                word.push(next);
            }
            if !word.is_empty() {
                out.push(word);
            }
        }
    }
    out
}

/// `module.exports = { types: [...] }` 这类 JS：取括号配平的对象片段当 JSON 解析。
/// 只在整份文本里找第一个 `{` 到它配平的 `}`，键名要求是标准双引号写法。
fn json_slice(raw: &str) -> Option<String> {
    let start = raw.find('{')?;
    let mut depth = 0usize;
    for (i, byte) in raw.as_bytes().iter().enumerate().skip(start) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(raw[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个 fixture 各占一个目录：配置文件是"按文件名找"的，同目录放多份会互相遮蔽。
    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).expect("write fixture");
    }

    #[test]
    fn no_config_file_falls_back_to_the_builtin_spec() {
        let dir = tempfile::tempdir().expect("tempdir");
        let spec = load(dir.path());
        assert_eq!(spec.source, SpecSource::Fallback);
        assert!(spec.types.contains(&"feat".to_string()));
        assert_eq!(spec.subject_max_length, 72);
        assert!(!spec.scope_required);
        assert_eq!(spec.task_id_pattern, None);
    }

    #[test]
    fn own_config_file_wins_over_everything_else() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            ".commitlintrc.json",
            r#"{"rules":{"type-enum":[2,["fix"]]}}"#,
        );
        write(
            dir.path(),
            "git-tidy.config.json",
            r#"{"types":["feat","fix"],"scopeRequired":true,"subjectMaxLength":50,"taskIdPattern":"[A-Z]+-[0-9]+","groups":[{"type":"feat","section":"新功能"}]}"#,
        );
        let spec = load(dir.path());
        assert_eq!(spec.source, SpecSource::RepoConfig);
        assert_eq!(spec.types, ["feat", "fix"]);
        assert!(
            spec.scope_required,
            "本工具配置里 scopeRequired=true 必须生效"
        );
        assert_eq!(spec.subject_max_length, 50);
        assert_eq!(spec.task_id_pattern.as_deref(), Some("[A-Z]+-[0-9]+"));
        assert_eq!(spec.groups[0].section, "新功能");
    }

    /// 需求 6.7 的验收本体：仓库只有 commitlint 配置时，白名单必须从它推导出来。
    #[test]
    fn commitlint_json_gives_the_type_whitelist() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            ".commitlintrc.json",
            r#"{"extends":["@commitlint/config-conventional"],"rules":{"type-enum":[2,"always",["feat","fix","chore"]],"scope-empty":[2,"always"],"header-max-length":[2,100]}}"#,
        );
        let spec = load(dir.path());
        assert_eq!(spec.source, SpecSource::Commitlint);
        assert_eq!(spec.types, ["feat", "fix", "chore"]);
        assert!(spec.scope_required, "scope-empty=always 就是必填");
        // header-max-length 是 commitlint 默认的整行 100，不拿来覆盖本工具的 72
        assert_eq!(spec.subject_max_length, 72);
    }

    #[test]
    fn commitlint_subject_max_length_is_the_number_not_the_severity() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            ".commitlintrc.json",
            r#"{"rules":{"type-enum":[2,["feat"]],"subject-max-length":[2,40]}}"#,
        );
        assert_eq!(load(dir.path()).subject_max_length, 40);
    }

    /// `commitlint.config.js` 是 JS，不能求值（那是执行仓库里的代码），只做文本提取。
    #[test]
    fn commitlint_js_is_read_as_text_not_executed() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "commitlint.config.js",
            r#"const types = ['feat', 'fix', 'perf', 'docs'];
module.exports = {
  extends: ['@commitlint/config-conventional'],
  rules: {
    'type-enum': [2, 'always', types],
    'scope-empty': [2, 'never'],
    'subject-max-length': [1, 60],
  },
};"#,
        );
        let spec = load(dir.path());
        assert_eq!(spec.source, SpecSource::Commitlint);
        assert_eq!(spec.types, ["feat", "fix", "perf", "docs"]);
        assert!(!spec.scope_required, "scope-empty=never 表示不必填");
        assert_eq!(spec.subject_max_length, 60);
    }

    #[test]
    fn commitlint_js_with_inline_array_is_read_too() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "commitlint.config.js",
            "module.exports = { rules: { 'type-enum': [2, 'always', ['feat','fix']] } };",
        );
        assert_eq!(load(dir.path()).types, ["feat", "fix"]);
    }

    #[test]
    fn versionrc_types_become_whitelist_and_changelog_groups() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            ".versionrc.json",
            r#"{"types":[{"type":"feat","section":"✨ 新功能"},{"type":"fix","section":"🐛 修复"},{"type":"chore"}]}"#,
        );
        let spec = load(dir.path());
        assert_eq!(spec.source, SpecSource::Versionrc);
        assert_eq!(spec.types, ["feat", "fix", "chore"]);
        assert_eq!(spec.groups[0].section, "✨ 新功能");
        assert_eq!(
            spec.groups[2].section, "Other Changes",
            "没有 section 的类型要有兜底分组"
        );
    }

    #[test]
    fn cliff_parsers_give_types_and_groups() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "cliff.toml",
            "[git]\nconventional_commits = true\n[git.commit_parsers]\n- { message = \"^feat\", group = \"新功能\" }\n- { message = \"^fix\\\\(ui\\\\)\", group = \"界面修复\" }\n",
        );
        let spec = load(dir.path());
        assert_eq!(spec.source, SpecSource::Cliff);
        assert_eq!(spec.types, ["feat", "fix"]);
        assert_eq!(spec.groups[1].section, "界面修复");
    }

    #[test]
    fn cliff_that_ignores_conventions_is_not_treated_as_a_spec() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "cliff.toml",
            "[git]\nconventional_commits = false\n[git.commit_parsers]\n- { message = \"^feat\", group = \"F\" }\n",
        );
        assert_eq!(load(dir.path()).source, SpecSource::Fallback);
    }

    #[test]
    fn a_layer_that_yields_nothing_falls_through_to_the_next_layer() {
        let dir = tempfile::tempdir().expect("tempdir");
        // 这份 commitlint 配置里没有 type-enum：不能因此就认为"白名单是空的"，
        // 要接着往下找 versionrc
        write(dir.path(), "commitlint.config.js", "module.exports = {};");
        write(
            dir.path(),
            ".versionrc.json",
            r#"{"types":[{"type":"feat","section":"Features"}]}"#,
        );
        assert_eq!(load(dir.path()).source, SpecSource::Versionrc);
    }

    #[test]
    fn broken_json_is_not_reported_as_a_valid_layer() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), "git-tidy.config.json", "{ this is not json");
        assert_eq!(
            load(dir.path()).source,
            SpecSource::Fallback,
            "坏掉的本工具配置不能静默变成空规范"
        );
    }

    #[test]
    fn extract_list_reaches_the_inner_array_past_the_severity_level() {
        let text = r#"'type-enum': [2, 'always', ['feat','fix','docs']],"#;
        assert_eq!(
            extract_list(text, "type-enum").unwrap(),
            ["feat", "fix", "docs"]
        );
        let flat = r#"'scope-empty': ['always']"#;
        assert_eq!(extract_list(flat, "scope-empty").unwrap(), ["always"]);
    }

    #[test]
    fn extract_number_skips_the_severity_level() {
        assert_eq!(
            extract_number("'subject-max-length': [1, 60]", "subject-max-length"),
            Some(60)
        );
        assert_eq!(
            extract_number("'subject-max-length': [2]", "subject-max-length"),
            None
        );
    }

    /// 只读浏览仓库没有工作区，规范只能从 HEAD 的对象里读，优先级顺序必须和磁盘版一致。
    #[test]
    fn head_objects_follow_the_same_priority_as_the_worktree() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            ".commitlintrc.json",
            r#"{"rules":{"type-enum":[2,"always",["fix","docs"]]}}"#,
        );
        write(dir.path(), "git-tidy.config.json", r#"{"types":["feat"]}"#);
        for args in [
            &["init", "-q", "-b", "main", "."][..],
            &["add", "-A"][..],
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                "chore: fixtures",
            ][..],
        ] {
            let out = crate::git::process::run(Some(dir.path()), args).expect("spawn git");
            assert!(out.success, "git {args:?} 失败：{}", out.stderr);
        }

        let spec = load_at_head(dir.path());
        assert_eq!(spec.source, SpecSource::RepoConfig);
        assert_eq!(spec.types, ["feat"]);

        // 没有本工具配置时，HEAD 里也要能读到 commitlint
        std::fs::remove_file(dir.path().join("git-tidy.config.json")).expect("remove");
        let committed = load_at_head(dir.path());
        assert_eq!(
            committed.source,
            SpecSource::RepoConfig,
            "配置文件的删除还没提交，HEAD 里应当还看得见旧的那份"
        );
        let from_disk = load(dir.path());
        assert_eq!(from_disk.source, SpecSource::Commitlint);
        assert_eq!(from_disk.types, ["fix", "docs"]);
    }
}
