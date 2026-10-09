use std::path::{Path, PathBuf};

use serde::Serialize;

use super::process;
use crate::config::spec::{Spec, SpecSource};
use crate::error::GitError;

/// 脚本头部里的标记行。判"这个 commit-msg 是不是我们写的"只认这一行——
/// 用文件路径判不准（用户可以把它拷到别处），用内容判才准。
const MARKER: &str = "git-tidy-hook/v1";
/// 规则快照指纹那一行的前缀。规范改了而 hook 没重装，界面据此提示"规则已过期"。
const FINGERPRINT_PREFIX: &str = "# spec-fingerprint: ";
/// 无信息量词里那些纯标点/纯数字的项在 shell 侧不可达（切词时已被当成分隔符），
/// 生成脚本时剔掉，免得脚本里的词表看着比内核那份多。
fn is_reachable_word(word: &str) -> bool {
    !word.chars().all(|c| {
        c.is_whitespace()
            || matches!(
                c,
                ',' | '.' | '!' | '?' | ';' | ':' | '，' | '。' | '！' | '？' | '、'
            )
    })
}

/// commit-msg 这个位置上现在是什么。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookState {
    /// 没有 commit-msg（`.sample` 不算）
    Missing,
    /// 是本工具生成的那一份
    Installed,
    /// 有别的 commit-msg：husky 的、团队自己写的、模板生成的
    Foreign,
}

/// 需求 7.20：安装前的探测结果。界面据此决定显示"一键安装"还是"共存方案"。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    /// git 实际会去执行的那个 hook 文件（`core.hooksPath` 生效后的结果）
    pub hook_path: String,
    pub state: HookState,
    /// `core.hooksPath` 的原值（没配就是 None）。配了就是"这个位置被别人占着"
    pub hooks_path: Option<String>,
    pub spec_source: SpecSource,
    /// 已装脚本里的规则快照是否还等于当前规范。false 表示规范改过、该重装
    pub rule_snapshot_current: bool,
}

/// 安装 / 卸载的结果。两种结局都用这一个结构带回去，界面不用分支两套字段。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookInstall {
    /// false = 没有写入任何文件，文件里 `message` 是拒绝理由，`suggestion` 是共存方案
    pub installed: bool,
    pub hook_path: String,
    pub message: String,
    pub suggestion: Option<String>,
    /// 被替换掉的既有 hook 备份到了哪里（force 覆盖别人的文件时才有）
    pub backup: Option<String>,
    pub status: HookStatus,
}

/// 探测。不写任何东西，所以界面每次进页面都能放心调。
pub fn status(repo: &Path, spec: &Spec) -> Result<HookStatus, GitError> {
    let hook_path = hooks_dir(repo)?;
    let configured = configured_hooks_path(repo)?;
    let existing = std::fs::read_to_string(&hook_path).ok();
    let state = match existing.as_deref() {
        None => HookState::Missing,
        Some(text) if is_ours(text) => HookState::Installed,
        Some(_) => HookState::Foreign,
    };
    Ok(HookStatus {
        rule_snapshot_current: existing
            .as_deref()
            .is_some_and(|text| fingerprint_of(text) == Some(fingerprint(spec))),
        hook_path: display(&hook_path),
        state,
        hooks_path: configured,
        spec_source: spec.source,
    })
}

/// 生成脚本内容。界面用它做"查看规则快照"，安装时也是同一份文本——
/// 界面上看到的就是会被写进磁盘的，不存在"预览一份、装另一份"。
pub fn render(spec: &Spec) -> String {
    let types: Vec<&str> = spec
        .types
        .iter()
        .map(String::as_str)
        // 含空白的 type 在 shell 词表里没法安全表示，宁可漏掉也不能让整张表解析坏
        .filter(|t| !t.contains(char::is_whitespace))
        .collect();
    let words: Vec<&str> = crate::config::check::NON_INFORMATIVE
        .iter()
        .copied()
        .filter(|w| is_reachable_word(w))
        .collect();
    let fingerprint_line = format!("{FINGERPRINT_PREFIX}{}", fingerprint(spec));

    format!(
        r#"#!/bin/sh
# {marker} — 由 Git Tidy 生成（需求 7.20）
#
# 这份脚本是自包含的：规则以字面量写在下面，不依赖 Git Tidy 在 PATH 里。
# 团队里没用这个工具的人，在终端 git commit 也会被同一条规则校验；
# 反过来，卸载 Git Tidy 之后这个仓库的提交规范照旧生效。
#
# 规则来源：{source}
# 重新生成：Git Tidy 的「提交」页点「更新 hook」
# 卸载：删掉本文件，或在「提交」页点「卸载 hook」
{fingerprint}
TYPES={types}
SCOPE_REQUIRED={scope_required}
SUBJECT_MAX_LENGTH={max_length}
TASK_ID_PATTERN={task_id}
NON_INFORMATIVE={words}

MSG_FILE="$1"
[ -n "$MSG_FILE" ] || exit 0
[ -f "$MSG_FILE" ] || exit 0

# git rev-parse 在 hook 里也可能失败（比如仓库刚被删），失败就放行：
# hook 的第一职责是不把用户的提交卡死，拦截是第二职责。
GIT_DIR=$(git rev-parse --git-dir 2>/dev/null) || exit 0

# 合并 / 变基 / 摘取 / 回滚：这些提交信息是 git 自己生成的，不是作者手写的，
# 按同一把尺子量它们只会把历史改写整段拦死，直接跳过。
[ -f "$GIT_DIR/MERGE_HEAD" ] && exit 0
[ -f "$GIT_DIR/CHERRY_PICK_HEAD" ] && exit 0
[ -f "$GIT_DIR/REVERT_HEAD" ] && exit 0
[ -d "$GIT_DIR/rebase-merge" ] && exit 0
[ -d "$GIT_DIR/rebase-apply" ] && exit 0

SUBJECT=$(sed -n '1p' "$MSG_FILE" | tr -d '\r')
BODY=$(sed -n '1d' "$MSG_FILE" | tr -d '\r')
LOWER=$(printf '%s' "$SUBJECT" | tr 'A-Z' 'a-z')

# squash!/fixup!/amend! 这类临时信息会被 git 后续步骤合并掉，拦它等于拦一个不存在的提交
case "$LOWER" in
  squash!*|fixup!*|amend!*|merge!*) exit 0 ;;
esac

ERRORS=''
WARNINGS=''

add_error() {{ ERRORS="$ERRORS\n  - $1"; }}
add_warning() {{ WARNINGS="$WARNINGS\n  - $1"; }}

TRIMMED=$(printf '%s' "$SUBJECT" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
if [ -z "$TRIMMED" ]; then
  add_error '提交标题为空：写一句说清楚这次改了什么。'
  printf '提交信息不符合本仓库规范：%b\n' "$ERRORS" >&2
  exit 1
fi

# 1. 中文冒号：先于一切解析判掉，不然 `feat：x` 会被当成"没有 type"的裸标题，报错会指错地方
HAS_FULLWIDTH=0
case "$SUBJECT" in
  *'：'*) HAS_FULLWIDTH=1 ;;
esac
if [ "$HAS_FULLWIDTH" = 1 ]; then
  add_error '使用了中文冒号：把全角冒号换成半角 `:`，规范要求 `type: 描述`。'
fi

HAS_COLON=0
case "$SUBJECT" in
  *:*) HAS_COLON=1 ;;
esac
if [ "$HAS_COLON" = 1 ]; then
  HEAD_PART=${{SUBJECT%%:*}}
  DESC=${{SUBJECT#*:}}
else
  HEAD_PART=$SUBJECT
  DESC=$SUBJECT
fi
DESC=$(printf '%s' "$DESC" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
HEAD_PART=$(printf '%s' "$HEAD_PART" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')

TYPE=''
SCOPE=''
TYPE_SHAPE_OK=0
if [ "$HAS_COLON" = 1 ] && [ "$HAS_FULLWIDTH" = 0 ]; then
  # `type(scope)!:`：先摘 `!`（breaking 标记），再摘 `(scope)`
  case "$HEAD_PART" in
    *'!') HEAD_PART=${{HEAD_PART%!}} ;;
  esac
  HEAD_PART=$(printf '%s' "$HEAD_PART" | sed -e 's/[[:space:]]*$//')
  case "$HEAD_PART" in
    *'('*')')
      TYPE=${{HEAD_PART%%(*}}
      SCOPE=${{HEAD_PART#*(}}
      SCOPE=${{SCOPE%)}}
      SCOPE=$(printf '%s' "$SCOPE" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
      ;;
    *)
      TYPE=$HEAD_PART
      ;;
  esac
  TYPE=$(printf '%s' "$TYPE" | tr 'A-Z' 'a-z')
  case "$TYPE" in
    ''|*[!A-Za-z0-9_-]*) TYPE_SHAPE_OK=0 ;;
    *) TYPE_SHAPE_OK=1 ;;
  esac
fi

# 2. type 缺失
if [ "$HAS_COLON" = 0 ] && [ "$HAS_FULLWIDTH" = 0 ]; then
  add_error '缺少 type 前缀：按 `type(scope): 描述` 重写，type 从下拉里选。'
fi

# 3. type 不在白名单里
TYPE_ALLOWED=0
if [ "$TYPE_SHAPE_OK" = 1 ] && [ -n "$TYPES" ]; then
  case " $TYPES " in
    *" $TYPE "*) TYPE_ALLOWED=1 ;;
  esac
fi
if [ "$TYPE_SHAPE_OK" = 1 ] && [ "$TYPE_ALLOWED" = 0 ]; then
  add_error "type 不在白名单里：\`$TYPE\` 不允许，本仓库允许的 type：$TYPES。"
fi

# 4. 冒号左边不是合法 type
if [ "$HAS_COLON" = 1 ] && [ "$HAS_FULLWIDTH" = 0 ] && [ -n "$DESC" ] && [ "$TYPE_SHAPE_OK" = 0 ]; then
  add_error '提交头部格式无法解析：冒号左边要写成字母/数字/-/_ 组成的 type，例如 `fix` 或 `feat`。'
fi

# 5. scope 必填
if [ "$SCOPE_REQUIRED" = 1 ] && [ "$TYPE_SHAPE_OK" = 1 ] && [ -z "$SCOPE" ]; then
  add_error '缺少 scope：本仓库要求写 scope，形如 `feat(界面): 描述`。'
fi

# 6. 冒号右边空着（表单只选了 type 时就是这个形状）
if [ "$HAS_COLON" = 1 ] && [ -z "$DESC" ]; then
  add_error '提交标题为空：冒号右边还空着，写清楚这次改了什么。'
fi

# 7. 无信息量词：描述拆词后每个词都命中才算无信息（`feat: 修复崩溃` 通过，`feat: wip` 拦下）
if [ -n "$DESC" ]; then
  ANY=0
  ALL_NONINFO=1
  for W in $(printf '%s' "$DESC" | tr 'A-Z' 'a-z' | sed -e 's/[,.!?;: ，。！？、]/ /g'); do
    ANY=1
    case " $NON_INFORMATIVE " in
      *" $W "*) ;;
      *) ALL_NONINFO=0; break ;;
    esac
  done
  if [ "$ANY" = 1 ] && [ "$ALL_NONINFO" = 1 ]; then
    add_error '提交信息无实际内容：描述要有具体对象和动作，例如 `修复导出 CSV 时列错位`。'
  fi

  # 8. 句尾句号
  case "$DESC" in
    *.|*。) add_error '提交标题以句号结尾：去掉标题末尾的句号。' ;;
  esac
fi

# 9. 标题超长只告警（需求 7.5：超长是阈值，不是失败）
# wc -m 在 C locale 下退化成字节数，中文标题会被高估——所以先试 UTF-8 locale，
# 实在没有就退回字节数，只影响这条告警，不影响任何拦截。
LENGTH=$(printf '%s' "$SUBJECT" | LC_ALL=C.UTF-8 wc -m 2>/dev/null | tr -d ' ')
case "$LENGTH" in
  ''|*[!0-9]*) LENGTH=$(printf '%s' "$SUBJECT" | wc -c | tr -d ' ') ;;
esac
if [ -n "$LENGTH" ] && [ "$LENGTH" -gt "$SUBJECT_MAX_LENGTH" ]; then
  add_warning "提交标题过长：当前 $LENGTH 个字符，阈值 $SUBJECT_MAX_LENGTH，建议把细节挪进正文。"
fi

# 10. 任务 ID。模式编译不过时 grep 会报错，这时放行——与内核一致：
# 一条写坏的团队配置不该让所有人的提交都过不去。
if [ -n "$TASK_ID_PATTERN" ]; then
  if ! printf '%s\n%s' "$SUBJECT" "$BODY" | grep -qE -e "$TASK_ID_PATTERN" 2>/dev/null; then
    add_error "缺少任务 ID：标题或正文里要有一处匹配 \`$TASK_ID_PATTERN\` 的任务 ID。"
  fi
fi

if [ -n "$WARNINGS" ]; then
  printf '提交信息有告警（不拦截）：%b\n' "$WARNINGS" >&2
fi

if [ -n "$ERRORS" ]; then
  printf '提交信息不符合本仓库规范：%b\n' "$ERRORS" >&2
  printf '\n改完再提交；确实要绕过可以用 git commit --no-verify，绕过的提交会在符合率报告里记为「疑似绕过」。\n' >&2
  exit 1
fi

exit 0
"#,
        marker = MARKER,
        source = source_label(spec.source),
        fingerprint = fingerprint_line,
        types = quote(&types.join(" ")),
        scope_required = u8::from(spec.scope_required),
        max_length = spec.subject_max_length,
        task_id = quote(&to_ere(spec.task_id_pattern.as_deref().unwrap_or(""))),
        words = quote(&words.join(" ")),
    )
}

/// 一键安装。规则与表单同源（同一个 `Spec` 渲染出来的），写的是字面量，
/// 装完不依赖本工具。`force` 只对"默认 .git/hooks 下别人写的 commit-msg"有意义：
/// 先备份再覆盖，且在结果里把备份路径带回去。
pub fn install(repo: &Path, spec: &Spec, force: bool) -> Result<HookInstall, GitError> {
    let hook_path = hooks_dir(repo)?;
    let existing = std::fs::read_to_string(&hook_path).ok();
    let ours = existing.as_deref().is_some_and(is_ours);

    // `core.hooksPath` 被别人设着（husky 是典型）就不碰：这个目录里的 hook 是那条链的一环，
    // 我们覆盖它等于把别人的校验整段弄坏。给出共存方案，让人自己决定。
    if let Some(configured) = configured_hooks_path(repo)? {
        return Ok(refused(
            &hook_path,
            Some(configured),
            status(repo, spec)?,
            "这个仓库把 `core.hooksPath` 指到了别的目录，安装动作会落到那个目录里去。",
        ));
    }

    if let Some(existing) = existing.as_deref() {
        if ours {
            // 已经是我们的：直接重写，把规则刷新到当前规范
            write_script(&hook_path, spec)?;
            return Ok(HookInstall {
                installed: true,
                hook_path: display(&hook_path),
                message: "已按当前规范更新 hook。".into(),
                suggestion: None,
                backup: None,
                status: status(repo, spec)?,
            });
        }
        if !force {
            return Ok(refused(
                &hook_path,
                None,
                status(repo, spec)?,
                &format!(
                    "这个位置已经有一份不是本工具生成的 commit-msg（第一行：{}），覆盖它会破坏别人的校验链",
                    existing.lines().next().unwrap_or("空文件").trim()
                ),
            ));
        }
    }

    let backup = match existing.as_deref().filter(|_| force) {
        None => None,
        Some(_) => {
            let target = backup_path(&hook_path);
            std::fs::copy(&hook_path, &target)
                .map_err(|err| GitError::Internal(err.to_string()))?;
            Some(display(&target))
        }
    };

    write_script(&hook_path, spec)?;
    Ok(HookInstall {
        installed: true,
        hook_path: display(&hook_path),
        message: match &backup {
            Some(_) => "已安装 hook，原有的 commit-msg 已备份（路径见 backup）。".into(),
            None => "已安装 commit-msg hook，规则与提交表单同源。".into(),
        },
        suggestion: None,
        backup,
        status: status(repo, spec)?,
    })
}

/// 本工具生成的 hook 是从哪一刻开始生效的（脚本文件的修改时间，Unix 秒）。
///
/// 符合率报告用它推断"疑似绕过 `--no-verify`"（需求 7.21）：早于这一刻的提交
/// 本来就没有 hook 可绕，不能算绕过；晚于这一刻还不合规的才标疑似。
/// 脚本只在安装/更新时被写，所以 mtime 就是"从这一刻起有 hook"。
pub fn installed_since(repo: &Path) -> Option<i64> {
    let path = hooks_dir(repo).ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    let modified = meta.modified().ok()?;
    let secs = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(secs as i64)
}

/// 卸载。只删自己写的那一份：别人的 hook 一个字节都不动。
pub fn uninstall(repo: &Path, spec: &Spec) -> Result<HookInstall, GitError> {
    let hook_path = hooks_dir(repo)?;
    let text = std::fs::read_to_string(&hook_path).unwrap_or_default();
    if !is_ours(&text) {
        return Ok(HookInstall {
            installed: false,
            hook_path: display(&hook_path),
            message: if text.is_empty() {
                "这个仓库没有本工具生成的 commit-msg hook，无需卸载。".into()
            } else {
                "这份 commit-msg 不是本工具生成的，卸载不会碰它。".into()
            },
            suggestion: None,
            backup: None,
            status: status(repo, spec)?,
        });
    }
    std::fs::remove_file(&hook_path).map_err(|err| GitError::Internal(err.to_string()))?;
    Ok(HookInstall {
        installed: false,
        hook_path: display(&hook_path),
        message: "已卸载 commit-msg hook。".into(),
        suggestion: None,
        backup: None,
        status: status(repo, spec)?,
    })
}

/// 拒绝时的文案：既说清为什么不写，也说清接下来怎么办。
/// 需求 7.20 要求给的是共存方案，而不是"请手动处理"。
fn refused(
    hook_path: &Path,
    hooks_path: Option<String>,
    status: HookStatus,
    reason: &str,
) -> HookInstall {
    let snippet = "git rev-parse --git-path hooks   # 得到生效的 hooks 目录\n\
         # 把 Git Tidy「查看规则快照」里的脚本内容存成 <hooks目录>/git-tidy-commit-msg.sh，然后在既有\n\
         # commit-msg 末尾追加一行（husky 的 _ /commit-msg 里同理）：\n\
         #   sh <hooks目录>/git-tidy-commit-msg.sh \"$1\"\n\
         # 这样 husky 与本规范各跑各的，谁都不覆盖谁。"
        .to_string();
    HookInstall {
        installed: false,
        hook_path: display(hook_path),
        message: format!(
            "{reason}（{}）",
            hooks_path
                .as_deref()
                .map(|p| format!("core.hooksPath = {p}"))
                .unwrap_or_else(|| "该文件已存在".to_string())
        ),
        suggestion: Some(snippet),
        backup: None,
        status,
    }
}

fn write_script(path: &Path, spec: &Spec) -> Result<(), GitError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|err| GitError::Internal(err.to_string()))?;
    }
    std::fs::write(path, render(spec)).map_err(|err| GitError::Internal(err.to_string()))?;
    // 没有可执行位，git 会静默跳过这个 hook——装完了却"不生效"是最难查的一类问题
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|err| GitError::Internal(err.to_string()))?;
    }
    Ok(())
}

fn backup_path(hook_path: &Path) -> PathBuf {
    hook_path.with_file_name("commit-msg.git-tidy-backup")
}

fn is_ours(text: &str) -> bool {
    text.lines().take(20).any(|line| line.contains(MARKER))
}

fn fingerprint_of(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(FINGERPRINT_PREFIX))
        .map(str::to_string)
}

/// 规则快照指纹（FNV-1a）。规范任何一项变了，指纹就变，界面据此提示 hook 过期。
fn fingerprint(spec: &Spec) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |part: &str| {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0x1f;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    feed(&spec.types.join(","));
    feed(if spec.scope_required { "1" } else { "0" });
    feed(&spec.subject_max_length.to_string());
    feed(spec.task_id_pattern.as_deref().unwrap_or(""));
    format!("{hash:016x}")
}

fn source_label(source: SpecSource) -> &'static str {
    match source {
        SpecSource::RepoConfig => "git-tidy.config.json",
        SpecSource::Commitlint => "commitlint 配置推导",
        SpecSource::Versionrc => ".versionrc 推导",
        SpecSource::Cliff => "cliff.toml 推导",
        SpecSource::Fallback => "内置默认（仓库没配置）",
    }
}

/// git 实际会去找 hook 的目录。`core.hooksPath` 生效，所以 husky 那套也走这里。
fn hooks_dir(repo: &Path) -> Result<PathBuf, GitError> {
    // 新版 git 直接给绝对路径；老版本给相对路径，那就按仓库根拼
    let absolute = process::run(
        Some(repo),
        &["rev-parse", "--path-format=absolute", "--git-path", "hooks"],
    )
    .ok()
    .filter(|out| out.success)
    .map(|out| out.stdout.trim().to_string());

    let raw = match absolute {
        Some(path) if !path.is_empty() => path,
        _ => {
            let out = process::run(Some(repo), &["rev-parse", "--git-path", "hooks"])?;
            if !out.success {
                return Err(GitError::NotARepo);
            }
            out.stdout.trim().to_string()
        }
    };
    let path = PathBuf::from(&raw);
    Ok(if path.is_absolute() {
        path
    } else {
        repo.join(path)
    }
    // `core.hooksPath` 指的是目录，hook 本体是目录里的 `commit-msg`
    .join("commit-msg"))
}

fn configured_hooks_path(repo: &Path) -> Result<Option<String>, GitError> {
    let out = process::run(Some(repo), &["config", "--get", "core.hooksPath"])?;
    Ok(out
        .success
        .then(|| out.stdout.trim().to_string())
        .filter(|value| !value.is_empty()))
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// 内核用的是 Rust 正则，脚本侧只有 POSIX ERE（`grep -E`）。两者的差别集中在
/// 这几个字符类上：`\d` 在 ERE 里是"字母 d"而不是数字，直接透传会让任务 ID 规则
/// 静默失效——脚本还在拦，但理由是错的。这里在生成时把它们翻成 POSIX 写法。
fn to_ere(pattern: &str) -> String {
    const CLASSES: [(&str, &str); 6] = [
        ("d", "[0-9]"),
        ("D", "[^0-9]"),
        ("w", "[[:alnum:]_]"),
        ("W", "[^[:alnum:]_]"),
        ("s", "[[:space:]]"),
        ("S", "[^[:space:]]"),
    ];
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            None => out.push('\\'),
            Some(next) => match CLASSES.iter().find(|(key, _)| next.to_string() == **key) {
                Some((_, replacement)) => out.push_str(replacement),
                // `\\`、`\.`、`\-` 这些两个字符在两种方言里含义相同，原样带过去
                None => {
                    out.push('\\');
                    out.push(next);
                }
            },
        }
    }
    out
}

/// 单引号包裹 + 转义内嵌单引号。生成的脚本里所有来自配置的文本都过这一道，
/// 免得一条 `taskIdPattern` 把脚本语法弄坏（那等于给全仓库的提交装了个哑弹）。
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::commit::Draft;
    use std::path::PathBuf;

    fn git_in(dir: &Path, args: &[&str]) {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        dir
    }

    /// 在真仓库里跑一次 `git commit`，让 git 自己调起 hook。返回是否成功。
    /// 只看退出码不够——要看 stderr 里有没有我们那行拒绝文案。
    fn try_commit(dir: &Path, subject: &str, extra: &[&str]) -> (bool, String) {
        std::fs::write(dir.join("f.txt"), "x\n").expect("write");
        git_in(dir, &["add", "f.txt"]);
        let mut args = vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "-m",
            subject,
        ];
        args.extend_from_slice(extra);
        let out = process::run(Some(dir), &args).expect("spawn git");
        (out.success, out.stderr)
    }

    #[test]
    fn the_generated_script_rejects_a_bare_wip_subject() {
        let dir = repo();
        install(dir.path(), &Spec::default(), false).expect("install");
        let (ok, stderr) = try_commit(dir.path(), "wip", &[]);
        assert!(!ok, "无 type 的提交必须被终端里的 git 拦下");
        assert!(
            stderr.contains("缺少 type 前缀"),
            "拒绝文案要指出具体原因：{stderr}"
        );
    }

    #[test]
    fn a_conventional_subject_passes_and_lands() {
        let dir = repo();
        install(dir.path(), &Spec::default(), false).expect("install");
        let (ok, stderr) = try_commit(dir.path(), "feat: 支持只读浏览", &[]);
        assert!(ok, "合规提交不该被拦：{stderr}");
    }

    #[test]
    fn an_overlong_subject_warns_but_still_commits() {
        let dir = repo();
        install(dir.path(), &Spec::default(), false).expect("install");
        let subject = format!("feat: {}", "修".repeat(80));
        let (ok, stderr) = try_commit(dir.path(), &subject, &[]);
        assert!(ok, "超长只告警（需求 7.5）：{stderr}");
        assert!(stderr.contains("提交标题过长"), "要给出告警：{stderr}");
    }

    #[test]
    fn the_task_id_rule_comes_from_the_spec_not_from_the_binary() {
        let dir = repo();
        let spec = Spec {
            task_id_pattern: Some(r"[A-Z]+-\d+".to_string()),
            ..Spec::default()
        };
        install(dir.path(), &spec, false).expect("install");
        let (ok, _) = try_commit(dir.path(), "feat: 改了个地方", &[]);
        assert!(!ok, "要求任务 ID 时没写单号必须被拦");
        let (ok, stderr) = try_commit(dir.path(), "feat(TIDY-12): 带上单号", &[]);
        assert!(ok, "带单号的应当放行：{stderr}");
    }

    #[test]
    fn a_foreign_commit_msg_is_never_overwritten_without_force() {
        let dir = repo();
        let hooks = dir.path().join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).expect("hooks dir");
        let foreign = hooks.join("commit-msg");
        std::fs::write(&foreign, "#!/bin/sh\necho husky >/dev/null\n").expect("write");

        let first = install(dir.path(), &Spec::default(), false).expect("status");
        assert!(!first.installed);
        assert!(first.suggestion.is_some(), "被拒必须给共存方案");
        assert_eq!(
            std::fs::read_to_string(&foreign).expect("read"),
            "#!/bin/sh\necho husky >/dev/null\n",
            "别人的 hook 一个字节都不能动"
        );

        let forced = install(dir.path(), &Spec::default(), true).expect("force install");
        assert!(forced.installed);
        let backup = forced.backup.expect("覆盖前必须备份");
        assert!(PathBuf::from(&backup).exists(), "备份路径要真的存在");
    }

    #[test]
    fn a_configured_hooks_path_is_refused_even_with_force() {
        let dir = repo();
        let other = dir.path().join(".husky").join("_");
        std::fs::create_dir_all(&other).expect("dir");
        std::fs::write(other.join("commit-msg"), "#!/bin/sh\nexit 0\n").expect("write");
        git_in(
            dir.path(),
            &["config", "core.hooksPath", other.to_str().expect("utf8")],
        );

        let result = install(dir.path(), &Spec::default(), true).expect("install");
        assert!(!result.installed, "hooksPath 被别人占着就不能写");
        assert!(
            result.message.contains("core.hooksPath"),
            "{}",
            result.message
        );
        assert!(result.suggestion.is_some());
        assert!(
            std::fs::read_to_string(other.join("commit-msg"))
                .expect("read")
                .contains("exit 0"),
            "husky 那份必须原封不动"
        );
    }

    #[test]
    fn uninstall_leaves_a_foreign_hook_alone() {
        let dir = repo();
        let hooks = dir.path().join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).expect("hooks dir");
        let foreign = hooks.join("commit-msg");
        std::fs::write(&foreign, "#!/bin/sh\nexit 0\n").expect("write");

        let result = uninstall(dir.path(), &Spec::default()).expect("uninstall");
        assert!(!result.installed);
        assert!(foreign.exists(), "别人的 hook 不能被卸载掉");
    }

    #[test]
    fn status_flips_from_missing_to_installed_and_back() {
        let dir = repo();
        let spec = Spec::default();
        assert_eq!(
            status(dir.path(), &spec).expect("status").state,
            HookState::Missing
        );

        install(dir.path(), &spec, false).expect("install");
        let installed = status(dir.path(), &spec).expect("status");
        assert_eq!(installed.state, HookState::Installed);
        assert!(installed.rule_snapshot_current);

        uninstall(dir.path(), &spec).expect("uninstall");
        assert_eq!(
            status(dir.path(), &spec).expect("status").state,
            HookState::Missing
        );
    }

    #[test]
    fn a_changed_spec_marks_the_installed_script_as_stale() {
        let dir = repo();
        install(dir.path(), &Spec::default(), false).expect("install");
        let stricter = Spec {
            scope_required: true,
            ..Spec::default()
        };
        let current = status(dir.path(), &stricter).expect("status");
        assert!(
            !current.rule_snapshot_current,
            "规范改了、hook 还没重装，要能看出来"
        );

        install(dir.path(), &stricter, false).expect("reinstall");
        assert!(
            status(dir.path(), &stricter)
                .expect("status")
                .rule_snapshot_current
        );
        let (ok, stderr) = try_commit(dir.path(), "feat: 没写 scope", &[]);
        assert!(!ok, "重装后新规则要立刻生效：{stderr}");
        assert!(stderr.contains("缺少 scope"), "{stderr}");
    }

    /// 脚本里的规则快照与界面预算是同一个内核出的：拿内核判一遍，
    /// 再确认这段文字确实出现在脚本里，避免"改了 check.rs 却忘了同步脚本"。
    #[test]
    fn the_script_carries_the_same_verdicts_as_the_kernel() {
        let spec = Spec::default();
        let script = render(&spec);
        for subject in [
            "wip",
            "更新了一批文件",
            "feat：中文冒号",
            "feat: wip",
            "feaat: 拼错的类型",
            "feat: 加个开关。",
            "some stuff: 值",
            "feat: ",
        ] {
            let outcome = crate::config::check::evaluate(&spec, subject, "");
            assert!(
                !outcome.conformant,
                "用例 {subject:?} 本该不合规，否则这条断言没有意义"
            );
            let title = &outcome
                .violations
                .iter()
                .find(|v| v.blocking)
                .expect("至少一条阻断级原因")
                .title;
            assert!(
                script.contains(title.as_str()),
                "脚本里必须能产出内核同名的原因文案：{title}"
            );
        }
        // 反向：合规标题里不该被脚本判出问题
        let clean = crate::config::check::evaluate(&spec, "feat: 支持按仓库只读浏览", "");
        assert!(clean.conformant);
    }

    /// hook 是拦截的第一道，但绕过的兜底在提交表单（需求 7.5）。
    /// 这条测试锁住"两边同源"：同一个 Draft，内核的判定与界面的判定必须是同一个函数。
    #[test]
    fn the_hook_and_the_form_share_one_kernel() {
        let spec = Spec {
            scope_required: true,
            ..Spec::default()
        };
        let draft = Draft {
            subject: "feat: 忘了 scope".into(),
            body: String::new(),
            footer: String::new(),
        };
        let form = crate::git::commit::preview(&spec, &draft);
        assert!(!form.conformant);
        assert!(form
            .violations
            .iter()
            .any(|v| v.reason == crate::config::check::Reason::MissingScope));
        assert!(render(&spec).contains("缺少 scope"));
    }

    #[test]
    fn quoting_survives_a_single_quote_in_the_config() {
        // 一条含单引号的 taskIdPattern 不该把脚本语法弄坏：能匹配上就说明脚本还活着，
        // 语法坏掉时 grep 那一段会静默失败，表现为"谁都提交不了"
        let spec = Spec {
            task_id_pattern: Some("it's-\\d+".into()),
            ..Spec::default()
        };
        let dir = repo();
        install(dir.path(), &spec, false).expect("install");
        let (ok, stderr) = try_commit(dir.path(), "fix: 修好 it's-42 这条", &[]);
        assert!(ok, "模式里的单引号不能让脚本失效：{stderr}");
    }

    /// Rust 正则的 `\d` 在 POSIX ERE 里是"字母 d"。不翻译的话脚本还在拦，
    /// 但报出来的理由跟任务 ID 毫无关系——这种"能跑但说谎"的规则最耗人。
    #[test]
    fn rust_regex_classes_are_translated_to_posix_ones() {
        assert_eq!(to_ere(r"[A-Z]+-\d+"), "[A-Z]+-[0-9]+");
        assert_eq!(to_ere(r"\w+"), "[[:alnum:]_]+");
        assert_eq!(to_ere(r"a\.b"), r"a\.b");
        assert_eq!(to_ere(r"TIDY-\d{2}"), "TIDY-[0-9]{2}");
        assert_eq!(to_ere(r"dangling\"), r"dangling\");
    }

    #[test]
    fn a_digit_class_from_the_config_actually_matches_a_digit() {
        let dir = repo();
        let spec = Spec {
            task_id_pattern: Some(r"TIDY-\d+".into()),
            ..Spec::default()
        };
        install(dir.path(), &spec, false).expect("install");
        let (ok, stderr) = try_commit(dir.path(), "feat(TIDY-7): 带单号", &[]);
        assert!(ok, "`\\d` 必须被当成数字：{stderr}");
    }
}
