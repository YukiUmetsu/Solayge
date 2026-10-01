//! Provider command templates, auto code review, editor launch, and git remote
//! URL normalization.

use std::path::Path;
use std::process::Stdio;

use tokio::process::Command;

use crate::models::{
    CommandTemplates, PermissionProfile, Project, Provider, ResolvedConfig, ReviewMode,
    ReviewStatus, Settings, SystemPrompt, SystemPromptPosition,
};
use crate::permissions;

/// Selectable editors for the "open project" button. An empty binary means the
/// OS default handler (`open` / `xdg-open`).
pub const EDITORS: &[(&str, &str)] = &[
    ("vscode", "code"),
    ("cursor", "cursor"),
    ("zed", "zed"),
    ("windsurf", "windsurf"),
    ("sublime", "subl"),
    ("system", ""),
];

fn editor_binary(id: &str) -> Option<&'static str> {
    EDITORS
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, b)| *b)
}

/// Split a template into argv, dropping `[ … ]` groups whose placeholders are
/// empty and dropping tokens that resolve to nothing.
pub fn expand(template: &str, prompt: &str, model: Option<&str>, auto: bool) -> Vec<String> {
    // Strip optional groups first.
    let mut kept = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('[') {
        kept.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find(']') {
            Some(close) => {
                let group = &after[..close];
                if group_has_value(group, model, auto) {
                    kept.push(' ');
                    kept.push_str(group);
                    kept.push(' ');
                }
                rest = &after[close + 1..];
            }
            None => {
                kept.push_str(&rest[open..]);
                rest = "";
                break;
            }
        }
    }
    kept.push_str(rest);

    let model = model.unwrap_or("");
    let auto_arg = if auto { "--auto" } else { "" };

    kept.split_whitespace()
        .map(|tok| {
            tok.replace("{prompt}", prompt)
                .replace("{model}", model)
                .replace("{auto}", auto_arg)
        })
        .filter(|t| !t.is_empty())
        .collect()
}

fn group_has_value(group: &str, model: Option<&str>, auto: bool) -> bool {
    if group.contains("{model}") && model.map(|m| m.trim().is_empty()).unwrap_or(true) {
        return false;
    }
    if group.contains("{auto}") && !auto {
        return false;
    }
    true
}

/// Build the command that runs an agent for `prompt`.
pub fn build_command(
    provider: Provider,
    model: Option<&str>,
    templates: &CommandTemplates,
    prompt: &str,
    profile: PermissionProfile,
    cwd: &Path,
) -> Result<Command, String> {
    let template = templates.for_provider(provider);
    let argv = expand(template, prompt, model, permissions::uses_auto(profile));
    let (bin, args) = argv
        .split_first()
        .ok_or_else(|| format!("the {} command template is empty", provider.command_key()))?;
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if provider == Provider::Opencode {
        cmd.env("OPENCODE_CONFIG_CONTENT", permissions::config_json(profile));
    }
    Ok(cmd)
}

/// Add a project's environment variables to a command. Injected into every
/// agent process (task, review, and planner) so secrets never touch the repo.
pub fn apply_env(cmd: &mut Command, env: &[(String, String)]) {
    for (key, value) in env {
        cmd.env(key, value);
    }
}

/// Values a project system prompt can reference.
pub struct PromptContext<'a> {
    pub project_name: &'a str,
    pub project_path: &'a str,
    pub current_branch: Option<&'a str>,
    /// The project's environment variables (decrypted).
    pub env: &'a [(String, String)],
}

/// Prepend or append a project's system prompt to a task prompt, expanding
/// `{{ … }}` variables. Returns the prompt unchanged when disabled or empty.
pub fn apply_system_prompt(
    prompt: &str,
    system_prompt: Option<&SystemPrompt>,
    ctx: &PromptContext,
) -> String {
    let Some(sp) = system_prompt else {
        return prompt.to_string();
    };
    if !sp.enabled || sp.text.trim().is_empty() {
        return prompt.to_string();
    }
    let rendered = render_template(&sp.text, ctx);
    match sp.position {
        SystemPromptPosition::Prefix => format!("{rendered}\n\n{prompt}"),
        SystemPromptPosition::Suffix => format!("{prompt}\n\n{rendered}"),
    }
}

/// Built-in instruction prepended to every agent task prompt. Asking for access
/// once, before work starts, beats discovering a permission mid-task: the agent
/// can plan around it, and the user answers one prompt instead of several.
pub const ACCESS_PLANNING_INSTRUCTION: &str = "\
PLAN AHEAD - REQUEST ACCESS UP FRONT\n\
Before you change anything, do this in your first message:\n\
1. State a short plan of the steps you will take.\n\
2. List everything you will need that is NOT already inside this project folder, \
naming the exact absolute path, command, or host for each: directories/files outside \
the project you will read or write, shell commands or tools that may need approval, \
and any network access.\n\
If the environment gives you a way to ask the user (a question or approval tool), \
request ALL of it in one batch now, before starting, instead of getting blocked partway \
through. If you cannot ask up front, print the list so the user can approve it. Prefer \
staying inside the project folder; only ask for outside access you actually need.";

/// Wrap a task prompt with [`ACCESS_PLANNING_INSTRUCTION`] so the agent plans its
/// access before acting.
pub fn with_access_planning(prompt: &str) -> String {
    format!("{ACCESS_PLANNING_INSTRUCTION}\n\n{prompt}")
}

/// Expand `{{project_name}}`, `{{project_path}}`, `{{current_branch}}` and
/// `{{env.NAME}}` (also `{{env:NAME}}` and `{{$NAME}}`). Unknown tokens are
/// left as-is so typos are visible.
pub fn render_template(text: &str, ctx: &PromptContext) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                out.push_str(&resolve_token(after[..end].trim(), ctx));
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn resolve_token(token: &str, ctx: &PromptContext) -> String {
    let env_name = token
        .strip_prefix("env.")
        .or_else(|| token.strip_prefix("env:"))
        .or_else(|| token.strip_prefix('$'));
    if let Some(name) = env_name {
        if let Some((_, value)) = ctx.env.iter().find(|(k, _)| k == name) {
            return value.clone();
        }
        return std::env::var(name).unwrap_or_default();
    }
    match token {
        "project_name" => ctx.project_name.to_string(),
        "project_path" => ctx.project_path.to_string(),
        "current_branch" | "branch" => ctx.current_branch.unwrap_or("").to_string(),
        _ => format!("{{{{{token}}}}}"),
    }
}

/// Ask an agent for a short branch name for a task.
pub fn branch_name_prompt(title: &str) -> String {
    format!(
        "Suggest a short git branch name for the coding task below. Reply with ONLY the branch \
         name and nothing else: lowercase kebab-case, at most 4 words, letters, digits and \
         hyphens only, no prefix or slashes.\n\nTASK: {title}"
    )
}

/// Turn arbitrary agent output into a safe `[a-z0-9-]` branch name.
pub fn sanitize_branch_name(raw: &str) -> String {
    let line = raw
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in line.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let out: String = out.chars().take(48).collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "task".to_string()
    } else {
        out
    }
}

/// The default reviewer instructions. Deliberately adversarial: the reviewer is
/// told to distrust the change and the author, and to look for what is missing
/// rather than to confirm what is present.
const DEFAULT_REVIEW_INSTRUCTIONS: &str = "\
Be skeptical and adversarial. Assume the change is flawed until the evidence proves otherwise, and \
that the author stopped as soon as it looked done. Actively try to falsify the claim that the task \
is complete:\n\
- Re-read the original instruction and check every requirement is actually met, not merely plausible.\n\
- Inspect the real diff and the files on disk. Do not trust summaries, comments, or commit messages.\n\
- Hunt for bugs, unhandled edge cases, silent failures, security holes, data loss, and broken or missing tests.\n\
- Question happy-path assumptions, error handling, concurrency, permissions, and off-by-one mistakes.\n\
- Check whether the tests would actually fail if the change were wrong, and whether any test was weakened to pass.\n\
- Call out anything you could not verify, and any requirement you cannot confirm was implemented.";

/// The prompt handed to the review agent.
///
/// `custom` (project, then account) replaces only the instructions paragraph.
/// The task context and the required verdict line are always appended, so an
/// edited prompt cannot break the `REVIEW:` contract the scheduler parses.
pub fn review_prompt(
    title: &str,
    task_prompt: &str,
    mode: ReviewMode,
    project_path: Option<&str>,
    custom: Option<&str>,
) -> String {
    let fix = match mode {
        ReviewMode::Autofix => {
            "If you find correctness, security, or data-loss problems, FIX them directly in \
             the code. Do not touch unrelated code and do not refactor for style.\n"
        }
        _ => "Do NOT modify any files. Only report.\n",
    };
    let instructions = custom
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_REVIEW_INSTRUCTIONS);
    let repo_line = project_path
        .map(|p| format!("REPOSITORY: {p}\n"))
        .unwrap_or_default();
    format!(
        "You are a meticulous, skeptical senior engineer reviewing the changes just made for one \
         task. You did not write this code and must not assume it is correct.\n\n\
         TASK: {title}\n\
         {repo_line}\n\
         THE INSTRUCTION THE TASK WAS GIVEN:\n{task_prompt}\n\n\
         {instructions}\n\n\
         {fix}\n\
         Finish with EXACTLY one line, on its own, in this format:\n\
         REVIEW: PASS\n\
         or\n\
         REVIEW: ISSUES: <one short line describing the main problem>"
    )
}

/// Parse the reviewer's final verdict line.
pub fn parse_verdict(output: &str) -> (ReviewStatus, Option<String>) {
    let clean = crate::opencode::strip_ansi(output);
    let mut verdict: Option<(ReviewStatus, Option<String>)> = None;
    for line in clean.lines() {
        let t = line.trim();
        if t.len() < 7 || !t.is_char_boundary(7) || !t[..7].eq_ignore_ascii_case("REVIEW:") {
            continue;
        }
        let rest = t[7..].trim();
        let upper = rest.to_ascii_uppercase();
        if upper.starts_with("PASS") {
            verdict = Some((ReviewStatus::Passed, None));
        } else if upper.starts_with("ISSUES:") && rest.is_char_boundary(7) {
            verdict = Some((
                ReviewStatus::Issues,
                Some(rest[7..].trim().to_string()),
            ));
        } else if upper.starts_with("ISSUES") {
            verdict = Some((ReviewStatus::Issues, Some("issues found".to_string())));
        }
    }
    verdict.unwrap_or((
        // No verdict line is a failed review, not a pass: a reviewer that exits
        // 0 without reporting must not be silently recorded as "passed".
        ReviewStatus::Failed,
        Some("the reviewer did not report a verdict".to_string()),
    ))
}

/// Last ~40 lines of the review output, used as the recorded summary when there
/// is no explicit issue line.
pub fn tail(output: &str) -> String {
    let clean = crate::opencode::strip_ansi(output);
    let lines: Vec<&str> = clean.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(40);
    lines[start..].join("\n")
}

fn open_binary() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    }
}

/// Open a path (or URL) with the OS default handler.
pub fn open_with_system(target: &Path) -> std::io::Result<()> {
    // Refuse targets that the handler could mistake for command-line options
    // (`open -a Foo`), which would otherwise let a malicious remote URL launch
    // an application.
    if target.to_string_lossy().starts_with('-') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to open a target that starts with '-'",
        ));
    }
    std::process::Command::new(open_binary())
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

/// A command that opens a URL with the OS default handler, for use as a task
/// process (cross-platform: `open` / `explorer` / `xdg-open`).
pub fn open_url_command(url: &str) -> Command {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        // `rundll32` opens the URL with the default browser and needs no shell.
        "rundll32"
    } else {
        "xdg-open"
    };
    let mut cmd = Command::new(program);
    if cfg!(target_os = "windows") {
        cmd.arg("url.dll,FileProtocolHandler");
    }
    cmd.arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd
}

/// Whether a normalized remote is a safe http(s) URL to hand to the browser.
pub fn is_web_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Resolve an executable name on `PATH` (honoring `PATHEXT` on Windows), or
/// accept an explicit path. Returns the resolved file, if any.
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    use std::path::{Path, PathBuf};

    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if name.contains('/') || name.contains('\\') {
        let p = Path::new(name);
        return p.is_file().then(|| p.to_path_buf());
    }

    let path_var = std::env::var_os("PATH")?;
    let exts: Vec<String> = {
        #[cfg(windows)]
        {
            let pathext =
                std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
            let mut v = vec![String::new()];
            v.extend(pathext.split(';').filter(|e| !e.is_empty()).map(str::to_string));
            v
        }
        #[cfg(not(windows))]
        {
            vec![String::new()]
        }
    };

    for dir in std::env::split_paths(&path_var) {
        for ext in &exts {
            let candidate: PathBuf = if ext.is_empty() {
                dir.join(name)
            } else {
                dir.join(format!("{name}{ext}"))
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Open a project folder in the configured editor, falling back to the OS
/// handler when the editor CLI is not installed. Returns the editor used.
pub fn open_in_editor(path: &Path, editor: Option<&str>) -> Result<String, String> {
    let id = editor.unwrap_or("system");
    let binary = editor_binary(id).unwrap_or("");
    if !binary.is_empty() {
        match std::process::Command::new(binary)
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(_) => return Ok(id.to_string()),
            Err(_) => { /* fall through to the OS handler */ }
        }
    }
    open_with_system(path).map_err(|e| e.to_string())?;
    Ok("system".to_string())
}

/// Turn a git remote URL into a browsable web URL when possible.
pub fn normalize_remote(url: &str) -> String {
    let mut u = url.trim().to_string();
    if let Some(rest) = u.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            u = format!("https://{host}/{path}");
        }
    } else if let Some(rest) = u.strip_prefix("ssh://") {
        let rest = rest.strip_prefix("git@").unwrap_or(rest);
        u = format!("https://{rest}");
    }
    if let Some(stripped) = u.strip_suffix(".git") {
        u = stripped.to_string();
    }
    u
}

/// Resolve the editor id, applying defaults.
pub fn resolve_editor(project: Option<&str>, settings: Option<&str>) -> Option<String> {
    project.or(settings).map(|s| s.to_string())
}

/// Commands that list a provider's models, best first. Empty when the CLI has
/// no such command.
fn model_list_commands(provider: Provider) -> Vec<Vec<&'static str>> {
    match provider {
        Provider::Opencode => vec![vec!["opencode", "models"]],
        Provider::Cursor => vec![vec!["cursor-agent", "--list-models"]],
        // Claude Code exposes `--model` aliases but no listing command.
        Provider::Claude => Vec::new(),
        Provider::Codex => vec![vec!["codex", "models"], vec!["codex", "--list-models"]],
    }
}

/// Pull model ids out of a CLI listing.
fn parse_models(raw: &str) -> Vec<String> {
    let clean = crate::opencode::strip_ansi(raw);
    let mut out: Vec<String> = Vec::new();
    for line in clean.lines() {
        let t = line
            .trim()
            .trim_start_matches(['-', '*', '•', '>'])
            .trim();
        // Model ids never contain spaces; this filters out headings, progress
        // lines ("Loading models…"), and prose.
        if t.is_empty() || t.len() > 120 || t.contains(char::is_whitespace) {
            continue;
        }
        if !t.chars().any(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        if out.iter().any(|m| m == t) {
            continue;
        }
        out.push(t.to_string());
        if out.len() >= 1000 {
            break;
        }
    }
    out
}

/// Ask a provider CLI for its model list. Returns an empty list when the CLI
/// isn't installed or doesn't support listing.
pub async fn fetch_models(provider: Provider) -> Vec<String> {
    for cmd in model_list_commands(provider) {
        let Some((bin, args)) = cmd.split_first() else {
            continue;
        };
        let run = tokio::process::Command::new(bin)
            .args(args)
            .output();
        let Ok(Ok(out)) = tokio::time::timeout(std::time::Duration::from_secs(90), run).await
        else {
            continue;
        };
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        text.push('\n');
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        let models = parse_models(&text);
        if !models.is_empty() {
            return models;
        }
    }
    Vec::new()
}

/// The provider and model an auto-review should use.
///
/// An explicitly configured reviewer (project, then account) wins. Otherwise
/// the review runs with the same provider and model as the task it reviews, so
/// it is covered by the same account and subscription instead of the provider
/// picking a default model — which can be on a different, unfunded account.
pub fn resolve_reviewer(
    project: Option<&Project>,
    settings: &Settings,
    task_provider: Option<Provider>,
    task_model: Option<&str>,
) -> (Provider, Option<String>) {
    let explicit_provider = project
        .and_then(|p| p.review_provider)
        .or(settings.review_provider);
    let explicit_model = project
        .and_then(|p| p.review_model.clone())
        .or_else(|| settings.review_model.clone());
    let provider = explicit_provider
        .or(task_provider)
        .or(settings.provider)
        .unwrap_or(Provider::Opencode);
    let model = choose_review_model(explicit_model, task_model);
    (provider, model)
}

/// Pick the reviewer model.
///
/// An explicitly configured reviewer wins — but not when it is the *same model
/// as the task on a different account* (e.g. `opencode/x` while the task ran
/// `opencode-go/x`). The mixed provider list makes that mistake easy to make,
/// and it sends the review to the wrong — often unfunded — account. In that case
/// follow the task, so the review runs where the task ran.
fn choose_review_model(explicit: Option<String>, task_model: Option<&str>) -> Option<String> {
    let Some(review) = explicit else {
        return task_model.map(str::to_string);
    };
    if let Some(task) = task_model {
        let same_model = model_id(&review) == model_id(task);
        let different_account = model_provider(&review) != model_provider(task);
        if same_model && different_account {
            return Some(task.to_string());
        }
    }
    Some(review)
}

/// The provider segment of a `provider/model` id (`opencode-go` in
/// `opencode-go/deepseek-v4.1-flash`).
fn model_provider(model: &str) -> &str {
    model.split_once('/').map(|(provider, _)| provider).unwrap_or("")
}

/// The model segment of a `provider/model` id (`deepseek-v4.1-flash` in
/// `opencode-go/deepseek-v4.1-flash`).
fn model_id(model: &str) -> &str {
    model.split_once('/').map(|(_, id)| id).unwrap_or(model)
}

/// Merge a project's agent config over the account defaults.
pub fn resolve(project: Option<&Project>, settings: &Settings) -> ResolvedConfig {
    let provider = project
        .and_then(|p| p.provider)
        .or(settings.provider)
        .unwrap_or(Provider::Opencode);
    let model = project
        .and_then(|p| p.model.clone())
        .or_else(|| settings.model.clone());
    let fallback_provider = project
        .and_then(|p| p.fallback_provider)
        .or(settings.fallback_provider);
    let fallback_model = project
        .and_then(|p| p.fallback_model.clone())
        .or_else(|| settings.fallback_model.clone());
    let (review_provider, review_model) =
        resolve_reviewer(project, settings, Some(provider), model.as_deref());
    let review_mode = project
        .and_then(|p| p.review_mode)
        .unwrap_or(settings.review_mode);
    let review_prompt = project
        .and_then(|p| p.review_prompt.clone())
        .or_else(|| settings.review_prompt.clone());
    let editor = project
        .and_then(|p| p.editor.clone())
        .or_else(|| settings.editor.clone());
    ResolvedConfig {
        provider,
        model,
        fallback_provider,
        fallback_model,
        review_provider,
        review_model,
        review_mode,
        review_prompt,
        editor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_model_flag_only_when_set() {
        let with = expand("claude -p {prompt} [--model {model}]", "do it", Some("opus"), false);
        assert_eq!(with, vec!["claude", "-p", "do it", "--model", "opus"]);
        let without = expand("claude -p {prompt} [--model {model}]", "do it", None, false);
        assert_eq!(without, vec!["claude", "-p", "do it"]);
    }

    #[test]
    fn prompt_stays_one_argument() {
        let argv = expand("codex exec {prompt}", "fix the bug in a b c", None, false);
        assert_eq!(argv, vec!["codex", "exec", "fix the bug in a b c"]);
    }

    #[test]
    fn auto_flag_is_optional() {
        let on = expand("opencode run --standalone {auto} [--model {model}] {prompt}", "p", None, true);
        assert_eq!(on, vec!["opencode", "run", "--standalone", "--auto", "p"]);
        let off = expand("opencode run --standalone {auto} [--model {model}] {prompt}", "p", None, false);
        assert_eq!(off, vec!["opencode", "run", "--standalone", "p"]);
    }

    #[test]
    fn normalizes_ssh_remotes() {
        assert_eq!(
            normalize_remote("git@github.com:acme/solayge.git"),
            "https://github.com/acme/solayge"
        );
        assert_eq!(
            normalize_remote("https://github.com/acme/solayge.git"),
            "https://github.com/acme/solayge"
        );
    }

    #[test]
    fn parses_review_verdicts() {
        let (s, m) = parse_verdict("blah\nREVIEW: PASS\n");
        assert_eq!(s, ReviewStatus::Passed);
        assert!(m.is_none());
        let (s, m) = parse_verdict("blah\nREVIEW: ISSUES: null deref on cancel\n");
        assert_eq!(s, ReviewStatus::Issues);
        assert_eq!(m.as_deref(), Some("null deref on cancel"));
    }

    #[test]
    fn a_review_without_a_verdict_is_not_a_pass() {
        let (status, summary) = parse_verdict("I looked at the code and it seems fine");
        assert_eq!(status, ReviewStatus::Failed);
        assert!(summary.is_some(), "the reason is recorded");
    }

    #[test]
    fn renders_system_prompt_variables() {
        let env = vec![
            ("API_URL".to_string(), "https://x.test".to_string()),
            ("TOKEN".to_string(), "abc".to_string()),
        ];
        let ctx = PromptContext {
            project_name: "solayge",
            project_path: "/code/solayge",
            current_branch: Some("main"),
            env: &env,
        };
        assert_eq!(
            render_template("{{project_name}} on {{current_branch}}", &ctx),
            "solayge on main"
        );
        assert_eq!(
            render_template("{{env.API_URL}}/{{env.TOKEN}}", &ctx),
            "https://x.test/abc"
        );
        assert_eq!(render_template("{{$TOKEN}}", &ctx), "abc");
        // Unknown tokens are preserved.
        assert_eq!(render_template("{{nope}}", &ctx), "{{nope}}");
    }

    #[test]
    fn applies_system_prompt_prefix_and_suffix() {
        let env: Vec<(String, String)> = Vec::new();
        let ctx = PromptContext {
            project_name: "p",
            project_path: "/p",
            current_branch: None,
            env: &env,
        };
        let prefix = SystemPrompt {
            position: SystemPromptPosition::Prefix,
            text: "always be terse".into(),
            enabled: true,
        };
        assert_eq!(
            apply_system_prompt("do it", Some(&prefix), &ctx),
            "always be terse\n\ndo it"
        );
        let suffix = SystemPrompt {
            position: SystemPromptPosition::Suffix,
            text: "end".into(),
            enabled: true,
        };
        assert_eq!(apply_system_prompt("do it", Some(&suffix), &ctx), "do it\n\nend");
        let off = SystemPrompt {
            position: SystemPromptPosition::Prefix,
            text: "x".into(),
            enabled: false,
        };
        assert_eq!(apply_system_prompt("do it", Some(&off), &ctx), "do it");
    }

    #[test]
    fn access_planning_instruction_precedes_the_task() {
        let wrapped = with_access_planning("do the thing");
        assert!(wrapped.starts_with(ACCESS_PLANNING_INSTRUCTION));
        assert!(wrapped.ends_with("do the thing"));

        // A project prefix system prompt still frames the whole thing.
        let env: Vec<(String, String)> = Vec::new();
        let ctx = PromptContext {
            project_name: "p",
            project_path: "/p",
            current_branch: None,
            env: &env,
        };
        let prefix = SystemPrompt {
            position: SystemPromptPosition::Prefix,
            text: "rules".into(),
            enabled: true,
        };
        let composed = apply_system_prompt(&with_access_planning("do it"), Some(&prefix), &ctx);
        assert!(composed.starts_with("rules\n\n"));
        assert!(composed.contains(ACCESS_PLANNING_INSTRUCTION));
    }

    #[test]
    fn sanitizes_branch_names() {
        assert_eq!(sanitize_branch_name("Add Login Page!"), "add-login-page");
        assert_eq!(sanitize_branch_name("  fix: null deref  "), "fix-null-deref");
        assert_eq!(sanitize_branch_name("`feature/x`"), "feature-x");
        assert_eq!(sanitize_branch_name(""), "task");
        assert_eq!(sanitize_branch_name("***"), "task");
        assert_eq!(sanitize_branch_name("a".repeat(80).as_str()).len(), 48);
    }

    #[test]
    fn only_web_urls_are_openable() {
        assert!(is_web_url("https://github.com/a/b"));
        assert!(is_web_url("http://example.test"));
        assert!(!is_web_url("git@github.com:a/b"));
        assert!(!is_web_url("-a Calculator"));
        assert!(!is_web_url("file:///etc/passwd"));
    }

    #[test]
    fn a_reviewer_inherits_the_task_model_when_unset() {
        let settings = Settings {
            model: Some("opencode-go/deepseek-v4.1-flash".into()),
            ..Default::default()
        };
        let r = resolve(None, &settings);
        assert_eq!(
            r.review_model.as_deref(),
            Some("opencode-go/deepseek-v4.1-flash"),
            "an unset reviewer must not leave the provider to pick a default model"
        );
        assert_eq!(r.review_provider, Provider::Opencode);
    }

    #[test]
    fn an_explicit_reviewer_model_still_wins() {
        let settings = Settings {
            model: Some("opencode-go/deepseek-v4.1-flash".into()),
            review_model: Some("opencode-go/gpt-6-luna".into()),
            ..Default::default()
        };
        let r = resolve(None, &settings);
        assert_eq!(r.review_model.as_deref(), Some("opencode-go/gpt-6-luna"));
    }

    #[test]
    fn a_reviewer_falls_back_to_the_tasks_own_model() {
        // Live resolution at review time: the task's snapshot is the fallback,
        // so an existing task reviews with the same account it ran on.
        let settings = Settings::default();
        let (provider, model) = resolve_reviewer(
            None,
            &settings,
            Some(Provider::Opencode),
            Some("opencode-go/deepseek-v4.1-flash"),
        );
        assert_eq!(provider, Provider::Opencode);
        assert_eq!(model.as_deref(), Some("opencode-go/deepseek-v4.1-flash"));
    }

    #[test]
    fn a_reviewer_on_a_different_account_of_the_same_model_follows_the_task() {
        let settings = Settings {
            model: Some("opencode-go/deepseek-v4.1-flash".into()),
            review_model: Some("opencode/deepseek-v4.1-flash".into()),
            ..Default::default()
        };
        let r = resolve(None, &settings);
        assert_eq!(
            r.review_model.as_deref(),
            Some("opencode-go/deepseek-v4.1-flash"),
            "the same model on a different account must not review the task"
        );
    }

    #[test]
    fn a_reviewer_with_a_different_model_is_kept() {
        let settings = Settings {
            model: Some("opencode-go/deepseek-v4.1-flash".into()),
            review_model: Some("anthropic/claude-sonnet-4-5".into()),
            ..Default::default()
        };
        let r = resolve(None, &settings);
        assert_eq!(
            r.review_model.as_deref(),
            Some("anthropic/claude-sonnet-4-5")
        );
    }

    #[test]
    fn review_time_resolution_repairs_a_wrong_account() {
        let settings = Settings {
            review_model: Some("opencode/deepseek-v4.1-flash".into()),
            ..Default::default()
        };
        let (provider, model) = resolve_reviewer(
            None,
            &settings,
            Some(Provider::Opencode),
            Some("opencode-go/deepseek-v4.1-flash"),
        );
        assert_eq!(provider, Provider::Opencode);
        assert_eq!(model.as_deref(), Some("opencode-go/deepseek-v4.1-flash"));
    }
}
