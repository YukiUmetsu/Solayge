use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, Result};
use tokio::process::Command;

use crate::models::{BranchInfo, ChangedFile, DiffResult, FileDiff, GitStatus, Worktree};

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

/// Run `gh` in `dir` and capture its stdout. Mirrors [`git`]: an unsuccessful
/// exit becomes an error carrying the trimmed stderr.
pub async fn gh(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("gh")
        .current_dir(dir)
        .args(args)
        .output()
        .await
        .map_err(|e| anyhow!("failed to run gh: {e}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "gh {}: {}",
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
///
/// The two-character porcelain code is kept verbatim (`" M"` = modified but
/// unstaged, `"M "` = staged, `"??"` = untracked) so callers can tell staged
/// from unstaged changes by inspecting the first column.
fn parse_status(output: &str) -> Vec<ChangedFile> {
    let mut files = Vec::new();
    for line in output.lines() {
        if line.len() < 4 {
            continue;
        }
        let status = line[..2].to_string();
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

/// Committed work on the current branch since it diverged from `base`: the
/// changes `base...HEAD` introduces, with no staged, unstaged, or untracked
/// files.
pub async fn diff_committed(dir: &Path, base: &str) -> Result<DiffResult> {
    if !is_repo(dir).await {
        return Err(anyhow!("not a git repository"));
    }
    let range = format!("{base}...HEAD");
    let stat = git(dir, &["diff", &range, "--stat", "--no-color"])
        .await
        .unwrap_or_default();
    let name_status = git(dir, &["diff", &range, "--name-status", "--no-color"])
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
        let diff = git(dir, &["diff", &range, "--no-color", "--", path])
            .await
            .unwrap_or_default();
        files.push(FileDiff {
            path: path.to_string(),
            status: status.trim().to_string(),
            diff,
        });
    }

    Ok(DiffResult { stat, files })
}

/// Everything the working tree differs from `base` by — committed work on the
/// current branch plus staged and unstaged local changes and untracked files.
/// With `include_local` false it narrows to `diff_committed`.
pub async fn diff_against(dir: &Path, base: &str, include_local: bool) -> Result<DiffResult> {
    if !include_local {
        return diff_committed(dir, base).await;
    }
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

/// Whether `ancestor` is reachable from `descendant` (i.e. `descendant` already
/// contains `ancestor`). Used to tell "landed on the default branch" from
/// "sitting on a branch".
pub async fn is_ancestor(dir: &Path, ancestor: &str, descendant: &str) -> bool {
    git_ok(dir, &["merge-base", "--is-ancestor", ancestor, descendant]).await
}

/// `(behind, ahead)` of `branch` relative to `base`: how many commits each side
/// has that the other does not.
pub async fn ahead_behind(dir: &Path, base: &str, branch: &str) -> (i64, i64) {
    let range = format!("{base}...{branch}");
    match git(dir, &["rev-list", "--left-right", "--count", &range]).await {
        Ok(out) => {
            let parts: Vec<&str> = out.split_whitespace().collect();
            if parts.len() == 2 {
                (
                    parts[0].parse().unwrap_or(0),
                    parts[1].parse().unwrap_or(0),
                )
            } else {
                (0, 0)
            }
        }
        Err(_) => (0, 0),
    }
}

/// Local branches (plus remote-only branches) with their landing state against
/// the repository's default branch. This is what lets the UI say "not merged
/// into main" and offer to land or delete a branch, so stranded `devtools/*`
/// work is visible instead of silently succeeding in isolation.
pub async fn branches(repo: &Path) -> Result<Vec<BranchInfo>> {
    if !is_repo(repo).await {
        return Ok(Vec::new());
    }
    let default = default_branch(repo).await;
    let current = current_branch(repo).await;
    let worktree_for: HashMap<String, String> = worktrees(repo)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|w| w.branch.map(|b| (b, w.path)))
        .collect();

    let local_out = git(repo, &["for-each-ref", "refs/heads", "--format=%(refname:short)"])
        .await
        .unwrap_or_default();
    let remote_out = git(
        repo,
        &["for-each-ref", "refs/remotes/origin", "--format=%(refname:short)"],
    )
    .await
    .unwrap_or_default();

    let local: Vec<String> = local_out
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let local_set: HashSet<&str> = local.iter().map(String::as_str).collect();

    let mut out = Vec::new();
    for name in &local {
        let is_default = name == &default;
        let merged = is_default || is_ancestor(repo, name, &default).await;
        let (behind, ahead) = if is_default {
            (0, 0)
        } else {
            ahead_behind(repo, &default, name).await
        };
        out.push(BranchInfo {
            name: name.clone(),
            is_remote: false,
            is_default,
            is_current: current.as_deref() == Some(name.as_str()),
            merged,
            ahead,
            behind,
            worktree: worktree_for.get(name).cloned(),
            has_remote: remote_exists(repo, name).await,
        });
    }

    // Remote-tracking branches with no local counterpart: they can still be
    // landed or deleted from here.
    for r in remote_out
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let Some(name) = r.strip_prefix("origin/") else {
            continue;
        };
        if name == "HEAD" || local_set.contains(name) {
            continue;
        }
        let remote_ref = format!("origin/{name}");
        let is_default = name == default;
        let merged = is_default || is_ancestor(repo, &remote_ref, &default).await;
        let (behind, ahead) = if is_default {
            (0, 0)
        } else {
            ahead_behind(repo, &default, &remote_ref).await
        };
        out.push(BranchInfo {
            name: name.to_string(),
            is_remote: true,
            is_default,
            is_current: false,
            merged,
            ahead,
            behind,
            worktree: None,
            has_remote: true,
        });
    }
    Ok(out)
}

/// Whether `origin/<branch>` exists locally (i.e. the branch was pushed).
pub async fn remote_exists(repo: &Path, branch: &str) -> bool {
    git_ok(
        repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{branch}"),
        ],
    )
    .await
}

/// Staged or unstaged changes only (untracked files don't block a checkout).
async fn has_tracked_changes(dir: &Path) -> bool {
    !git_ok(dir, &["diff", "--quiet"]).await || !git_ok(dir, &["diff", "--cached", "--quiet"]).await
}

/// Merge `branch` into `target` (the default branch when not given) and leave
/// `target` checked out. Aborts and reports on conflict so the project folder is
/// never left mid-merge without a resolution UI.
pub async fn merge_branch_into(repo: &Path, branch: &str, target: Option<&str>) -> Result<String> {
    if !is_repo(repo).await {
        return Err(anyhow!("not a git repository"));
    }
    if !valid_ref(branch) {
        return Err(anyhow!("invalid branch name: {branch}"));
    }
    let target = match target.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => default_branch(repo).await,
    };
    if !valid_ref(&target) {
        return Err(anyhow!("invalid target branch: {target}"));
    }
    if branch == target {
        return Err(anyhow!("cannot merge {branch} into itself"));
    }
    if has_tracked_changes(repo).await {
        return Err(anyhow!(
            "the working tree has uncommitted changes; commit or stash them first"
        ));
    }
    git(repo, &["checkout", &target])
        .await
        .map_err(|e| anyhow!("could not check out {target}: {e}"))?;
    match git(repo, &["merge", "--no-edit", branch]).await {
        Ok(_) => Ok(format!("Merged {branch} into {target}.")),
        Err(e) => {
            merge_abort(repo).await;
            Err(anyhow!("could not merge {branch} into {target}: {e}"))
        }
    }
}

/// Force-delete a local branch. Refuses the default branch, the checked-out
/// branch, and any branch a worktree still has checked out (git refuses those
/// anyway; this turns the failure into a clear message). Enforced here, not just
/// in the UI, so the IPC surface can never delete the default branch.
pub async fn delete_local_branch(repo: &Path, branch: &str) -> Result<String> {
    if !valid_ref(branch) {
        return Err(anyhow!("invalid branch name: {branch}"));
    }
    if branch == default_branch(repo).await {
        return Err(anyhow!("refusing to delete the default branch {branch}"));
    }
    if current_branch(repo).await.as_deref() == Some(branch) {
        return Err(anyhow!("cannot delete the checked-out branch {branch}"));
    }
    if let Some(wt) = worktrees(repo)
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some(branch))
    {
        return Err(anyhow!("branch {branch} is checked out in {}", wt.path));
    }
    git(repo, &["branch", "-D", branch]).await?;
    Ok(format!("Deleted local branch {branch}."))
}

/// Delete `origin/<branch>` on the remote. Refuses the default branch.
pub async fn delete_remote_branch(repo: &Path, branch: &str) -> Result<String> {
    if !valid_ref(branch) {
        return Err(anyhow!("invalid branch name: {branch}"));
    }
    if branch == default_branch(repo).await {
        return Err(anyhow!("refusing to delete the default branch origin/{branch}"));
    }
    git(repo, &["push", "origin", "--delete", branch]).await?;
    Ok(format!("Deleted origin/{branch}."))
}

/// Remove a linked worktree by path. Refuses the main worktree and any worktree
/// with uncommitted changes (so "remove" can never silently discard work), then
/// prunes stale metadata.
pub async fn remove_worktree(repo: &Path, worktree: &str) -> Result<String> {
    let path = Path::new(worktree);
    if worktrees(repo)
        .await
        .unwrap_or_default()
        .iter()
        .any(|w| w.is_main && Path::new(&w.path) == path)
    {
        return Err(anyhow!("cannot remove the main worktree"));
    }
    if !path.is_dir() {
        return Err(anyhow!("worktree not found: {worktree}"));
    }
    if has_changes(path).await {
        return Err(anyhow!(
            "the worktree at {worktree} has uncommitted changes; commit or stash them first"
        ));
    }
    worktree_remove(repo, path).await?;
    let _ = git(repo, &["worktree", "prune"]).await;
    Ok(format!("Removed worktree {worktree}."))
}

#[cfg(test)]
mod tests {
    use super::{
        branch_exists, branches, default_branch, delete_local_branch, delete_remote_branch,
        merge_branch_into, parse_status, remove_worktree, valid_ref,
    };
    use std::path::Path;

    #[test]
    fn parses_porcelain_status() {
        let out = " M src/main.rs\n?? new.txt\nR  a.rs -> b.rs\n";
        let files = parse_status(out);
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].status, " M");
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

    async fn init_repo(dir: &Path) -> String {
        std::fs::create_dir_all(dir).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "test"],
            // Don't inherit a global commit-signing config from this machine.
            vec!["config", "commit.gpgsign", "false"],
            vec!["commit", "--allow-empty", "-qm", "init"],
        ] {
            let out = crate::git::git_cmd(dir, &args).output().await.unwrap();
            assert!(
                out.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        default_branch(dir).await
    }

    async fn run(dir: &Path, args: &[&str]) {
        let out = crate::git::git_cmd(dir, args).output().await.unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn branches_flags_unmerged_then_merged_after_landing() {
        let dir = std::env::temp_dir().join(format!("solayge-branches-{}", uuid::Uuid::new_v4()));
        let default = init_repo(&dir).await;
        assert_eq!(default, "main");

        std::fs::write(dir.join("a.txt"), "x").unwrap();
        run(&dir, &["checkout", "-q", "-b", "feat"]).await;
        run(&dir, &["add", "-A"]).await;
        run(&dir, &["commit", "-qm", "feat work"]).await;

        let list = branches(&dir).await.unwrap();
        let feat = list.iter().find(|b| b.name == "feat").expect("feat listed");
        assert!(!feat.merged, "unlanded branch must be marked not merged");
        assert_eq!(feat.ahead, 1);
        assert!(feat.is_current);
        let main = list.iter().find(|b| b.name == "main").unwrap();
        assert!(main.merged && main.is_default);

        // Land it locally, then the branch reports as merged with nothing ahead.
        merge_branch_into(&dir, "feat", None).await.unwrap();
        let list = branches(&dir).await.unwrap();
        let feat = list.iter().find(|b| b.name == "feat").unwrap();
        assert!(feat.merged, "a landed branch must be marked merged");
        assert_eq!(feat.ahead, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delete_helpers_refuse_checked_out_and_worktree_branches() {
        let dir = std::env::temp_dir().join(format!("solayge-delbranch-{}", uuid::Uuid::new_v4()));
        init_repo(&dir).await;

        // The checked-out default branch is never deletable.
        assert!(delete_local_branch(&dir, "main").await.is_err());

        // A plain branch is.
        run(&dir, &["branch", "throwaway"]).await;
        delete_local_branch(&dir, "throwaway").await.unwrap();
        assert!(!branch_exists(&dir, "throwaway").await);

        // A branch a worktree holds is not deletable, and the main worktree
        // cannot be removed; the linked worktree can.
        let wt = dir.join("wt");
        let wt_arg = wt.to_string_lossy().to_string();
        run(&dir, &["worktree", "add", "-q", "-b", "held", &wt_arg]).await;
        assert!(delete_local_branch(&dir, "held").await.is_err());
        let main_arg = dir.to_string_lossy().to_string();
        assert!(remove_worktree(&dir, &main_arg).await.is_err());
        // A worktree with uncommitted work is never removed (no silent loss).
        std::fs::write(wt.join("wip.txt"), "x").unwrap();
        assert!(remove_worktree(&dir, &wt_arg).await.is_err());
        std::fs::remove_file(wt.join("wip.txt")).unwrap();
        remove_worktree(&dir, &wt_arg).await.unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delete_helpers_never_delete_the_default_branch() {
        let dir = std::env::temp_dir().join(format!("solayge-default-{}", uuid::Uuid::new_v4()));
        init_repo(&dir).await;
        // Switch away so `main` exists but is not checked out.
        run(&dir, &["checkout", "-q", "-b", "work"]).await;

        // Neither the local nor the remote default branch may be deleted, even
        // though it is no longer the checked-out branch.
        assert!(delete_local_branch(&dir, "main").await.is_err());
        assert!(delete_remote_branch(&dir, "main").await.is_err());

        // A non-default branch is still deletable.
        run(&dir, &["branch", "tmp"]).await;
        delete_local_branch(&dir, "tmp").await.unwrap();
        assert!(!branch_exists(&dir, "tmp").await);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
