use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, Result};
use tokio::process::Command;

use crate::models::{ChangedFile, DiffResult, FileDiff, GitStatus, Worktree};

pub async fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .await
        .map_err(|e| anyhow!("failed to run git: {e}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

async fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub async fn is_repo(dir: &Path) -> bool {
    git_ok(dir, &["rev-parse", "--is-inside-work-tree"]).await
}

/// Parse `git status --porcelain=v1` output into changed files.
fn parse_status(output: &str) -> Vec<ChangedFile> {
    let mut files = Vec::new();
    for line in output.lines() {
        if line.len() < 4 {
            continue;
        }
        let status = line[..2].trim().to_string();
        let mut path = line[3..].to_string();
        if let Some(idx) = path.find(" -> ") {
            path = path[idx + 4..].to_string();
        }
        files.push(ChangedFile { path, status });
    }
    files
}

pub async fn status(dir: &Path) -> Result<GitStatus> {
    if !is_repo(dir).await {
        return Ok(GitStatus {
            is_repo: false,
            branch: None,
            dirty: false,
            changed_files: Vec::new(),
            ahead: 0,
            behind: 0,
        });
    }
    let branch = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let porcelain = git(dir, &["status", "--porcelain=v1"])
        .await
        .unwrap_or_default();
    let changed_files = parse_status(&porcelain);

    let (mut ahead, mut behind) = (0i64, 0i64);
    if let Ok(counts) = git(
        dir,
        &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
    )
    .await
    {
        let parts: Vec<&str> = counts.split_whitespace().collect();
        if parts.len() == 2 {
            behind = parts[0].parse().unwrap_or(0);
            ahead = parts[1].parse().unwrap_or(0);
        }
    }

    Ok(GitStatus {
        is_repo: true,
        branch,
        dirty: !changed_files.is_empty(),
        changed_files,
        ahead,
        behind,
    })
}

pub async fn worktrees(repo: &Path) -> Result<Vec<Worktree>> {
    let out = git(repo, &["worktree", "list", "--porcelain"]).await?;
    let mut list = Vec::new();
    let mut path: Option<String> = None;
    let mut head: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut is_main = true;
    let flush = |path: &mut Option<String>,
                 head: &mut Option<String>,
                 branch: &mut Option<String>,
                 is_main: &mut bool,
                 list: &mut Vec<Worktree>| {
        if let Some(p) = path.take() {
            list.push(Worktree {
                path: p,
                branch: branch.take(),
                head: head.take(),
                is_main: *is_main,
            });
        }
        *is_main = false;
    };
    for line in out.lines() {
        if line.is_empty() {
            flush(&mut path, &mut head, &mut branch, &mut is_main, &mut list);
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            path = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            head = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = Some(rest.trim_start_matches("refs/heads/").to_string());
        }
    }
    flush(&mut path, &mut head, &mut branch, &mut is_main, &mut list);
    Ok(list)
}

fn ensure_excluded(repo: &Path, entry: &str) -> std::io::Result<()> {
    let info = repo.join(".git").join("info");
    std::fs::create_dir_all(&info)?;
    let exclude = info.join("exclude");
    let current = std::fs::read_to_string(&exclude).unwrap_or_default();
    if !current.lines().any(|l| l.trim() == entry) {
        let mut next = current;
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(entry);
        next.push('\n');
        std::fs::write(exclude, next)?;
    }
    Ok(())
}

pub async fn worktree_add(repo: &Path, path: &Path, branch: &str, base: &str) -> Result<()> {
    ensure_excluded(repo, ".dev-tools/")?;
    let ps = path.to_string_lossy().to_string();
    git(repo, &["worktree", "add", "-b", branch, &ps, base]).await?;
    Ok(())
}

pub async fn worktree_remove(repo: &Path, path: &Path) -> Result<()> {
    let ps = path.to_string_lossy().to_string();
    git(repo, &["worktree", "remove", "--force", &ps]).await?;
    Ok(())
}

pub async fn branch_delete(repo: &Path, branch: &str) -> Result<()> {
    git(repo, &["branch", "-D", branch]).await?;
    Ok(())
}

/// Diff of the working tree (tracked vs HEAD, plus untracked file contents).
pub async fn diff(dir: &Path) -> Result<DiffResult> {
    if !is_repo(dir).await {
        return Err(anyhow!("not a git repository"));
    }
    let porcelain = git(dir, &["status", "--porcelain=v1"])
        .await
        .unwrap_or_default();
    let changed = parse_status(&porcelain);
    let stat = git(dir, &["diff", "HEAD", "--stat", "--no-color"])
        .await
        .unwrap_or_default();

    let mut files = Vec::new();
    for cf in changed {
        let is_untracked = cf.status == "??";
        let diff = if is_untracked {
            read_untracked(dir, &cf.path)
        } else {
            let mut d = git(dir, &["diff", "HEAD", "--no-color", "--", &cf.path])
                .await
                .unwrap_or_default();
            if d.trim().is_empty() {
                d = git(dir, &["diff", "--cached", "--no-color", "--", &cf.path])
                    .await
                    .unwrap_or_default();
            }
            d
        };
        files.push(FileDiff {
            path: cf.path,
            status: cf.status,
            diff,
        });
    }

    Ok(DiffResult { stat, files })
}

/// Render an untracked file's contents as a pseudo-diff (first 400 lines, at
/// most 256 KiB). Refuses symlinks and non-regular files so a crafted repo can't
/// make the diff view read arbitrary paths on disk.
fn read_untracked(dir: &Path, path: &str) -> String {
    use std::io::Read as _;

    let full = dir.join(path);
    let Ok(meta) = std::fs::symlink_metadata(&full) else {
        return "(unreadable)".to_string();
    };
    if meta.file_type().is_symlink() {
        return "(symlink — not followed)".to_string();
    }
    if !meta.is_file() {
        return "(not a regular file)".to_string();
    }
    let Ok(file) = std::fs::File::open(&full) else {
        return "(unreadable)".to_string();
    };
    let mut bytes = Vec::new();
    if file.take(256 * 1024).read_to_end(&mut bytes).is_err() {
        return "(unreadable)".to_string();
    }
    let content = String::from_utf8_lossy(&bytes);

    let mut body = String::new();
    let mut total = 0usize;
    for (i, line) in content.lines().enumerate() {
        total += 1;
        if i >= 400 {
            break;
        }
        body.push_str(&format!("+{}:{}\n", i + 1, line));
    }
    if total > 400 {
        body.push_str("... (truncated)\n");
    }
    body
}

/// Everything the working tree differs from `base` by — committed work on the
/// current branch plus staged and unstaged local changes and untracked files.
pub async fn diff_against(dir: &Path, base: &str) -> Result<DiffResult> {
    if !is_repo(dir).await {
        return Err(anyhow!("not a git repository"));
    }
    let stat = git(dir, &["diff", base, "--stat", "--no-color"])
        .await
        .unwrap_or_default();
    let name_status = git(dir, &["diff", base, "--name-status", "--no-color"])
        .await
        .unwrap_or_default();

    let mut files = Vec::new();
    for line in name_status.lines() {
        let mut parts = line.split('\t');
        let Some(status) = parts.next() else { continue };
        let rest: Vec<&str> = parts.collect();
        let Some(path) = rest.last() else { continue };
        if path.is_empty() {
            continue;
        }
        let diff = git(dir, &["diff", base, "--no-color", "--", path])
            .await
            .unwrap_or_default();
        files.push(FileDiff {
            path: path.to_string(),
            status: status.trim().to_string(),
            diff,
        });
    }

    // `git diff` never shows untracked files, so add them explicitly.
    if let Ok(untracked) = git(dir, &["ls-files", "--others", "--exclude-standard"]).await {
        for path in untracked.lines().map(str::trim).filter(|p| !p.is_empty()) {
            files.push(FileDiff {
                path: path.to_string(),
                status: "??".to_string(),
                diff: read_untracked(dir, path),
            });
        }
    }

    Ok(DiffResult { stat, files })
}

pub fn worktree_root(repo: &Path) -> PathBuf {
    repo.join(".dev-tools").join("worktrees")
}

/// Whether a local or `origin` branch with this name already exists.
pub async fn branch_exists(dir: &Path, branch: &str) -> bool {
    if git_ok(
        dir,
        &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")],
    )
    .await
    {
        return true;
    }
    git_ok(
        dir,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{branch}"),
        ],
    )
    .await
}

/// The `origin` remote URL, if the repo has one.
pub async fn remote_url(repo: &Path) -> Option<String> {
    git(repo, &["remote", "get-url", "origin"])
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The checked-out branch name, or `None` in a detached/empty repo.
pub async fn current_branch(dir: &Path) -> Option<String> {
    git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "HEAD")
}

/// A `git` command in `dir`, ready to be spawned and streamed.
pub fn git_cmd(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// A shell running `script` in `dir` (login shell on unix so the user's PATH is
/// available; `cmd /C` on Windows).
pub fn shell_cmd(dir: &Path, script: &str) -> Command {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", script]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("bash");
        c.args(["-lc", script]);
        c
    };
    cmd.current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// A command that runs `gh`, with stdio piped for streaming. Used for PR steps.
pub fn gh_cmd(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("gh");
    cmd.current_dir(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// Whether the GitHub CLI is installed and runnable.
pub async fn gh_available() -> bool {
    Command::new("gh")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Whether the index has staged changes (`git diff --cached --quiet` fails when
/// there are any).
pub async fn has_staged_changes(dir: &Path) -> bool {
    !git_ok(dir, &["diff", "--cached", "--quiet"]).await
}

/// Whether a branch/ref name is safe to pass as a single `git` argument: not
/// empty, not option-like (leading `-`), and free of whitespace/control chars.
pub fn valid_ref(name: &str) -> bool {
    let t = name.trim();
    !t.is_empty()
        && !t.starts_with('-')
        && !t.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The repository's default branch: `origin/HEAD`, else `main`/`master`, else
/// the current branch.
pub async fn default_branch(repo: &Path) -> String {
    if let Ok(s) = git(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).await {
        if let Some(name) = s.trim().strip_prefix("origin/") {
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    for cand in ["main", "master"] {
        if git_ok(
            repo,
            &["show-ref", "--verify", "--quiet", &format!("refs/heads/{cand}")],
        )
        .await
        {
            return cand.to_string();
        }
    }
    current_branch(repo)
        .await
        .unwrap_or_else(|| "main".to_string())
}

/// Whether the working tree has staged or unstaged changes.
pub async fn has_changes(dir: &Path) -> bool {
    git(dir, &["status", "--porcelain"])
        .await
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

/// Staged files that still have merge conflicts.
pub async fn conflicted_files(dir: &Path) -> Vec<String> {
    git(dir, &["diff", "--name-only", "--diff-filter=U"])
        .await
        .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

/// Abort an in-progress merge/rebase, best effort.
pub async fn merge_abort(dir: &Path) {
    let _ = git(dir, &["merge", "--abort"]).await;
    let _ = git(dir, &["rebase", "--abort"]).await;
}

#[cfg(test)]
mod tests {
    use super::{parse_status, valid_ref};

    #[test]
    fn parses_porcelain_status() {
        let out = " M src/main.rs\n?? new.txt\nR  a.rs -> b.rs\n";
        let files = parse_status(out);
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].status, "M");
        assert_eq!(files[1].status, "??");
        assert_eq!(files[2].path, "b.rs");
    }

    #[test]
    fn ignores_short_lines() {
        let files = parse_status("\nfoo\n");
        assert!(files.is_empty());
    }

    #[test]
    fn validates_refs() {
        assert!(valid_ref("main"));
        assert!(valid_ref("feature/x-2"));
        assert!(valid_ref("HEAD~2"));
        assert!(!valid_ref(""));
        assert!(!valid_ref("-x"));
        assert!(!valid_ref("a b"));
        assert!(!valid_ref("a\nb"));
    }
}
