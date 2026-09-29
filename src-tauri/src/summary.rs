//! Recovery of an agent's final summary from a task log.
//!
//! Tasks that finished before results were captured directly still have the
//! summary at the end of their log, so this reconstructs it (best effort) to
//! give those tasks a Result tab too. It is deliberately conservative: it keeps
//! the last contiguous block of agent prose and returns `None` when there is
//! nothing clear to show.

use std::path::Path;

/// Bytes read from the end of a log; summaries live at the end.
pub const TAIL_BYTES: u64 = 256 * 1024;
/// Cap a recovered result so a huge final message cannot bloat the state file.
pub const MAX_RESULT_CHARS: usize = 64 * 1024;

/// App notes the app writes into a log. A `[Solayge]`-tagged line that is *not*
/// one of these is old agent text — the tag used to be applied to agent output.
const APP_NOTES: &[&str] = &[
    "opencode session ",
    "Waiting for you:",
    "Interrupted:",
    "On new branch ",
    "interactive mode unavailable",
    "git step failed:",
    "Nothing to commit.",
    "gh not found; opening",
    "Conflicts merging ",
    "Resolution already committed.",
    "Integrating ",
    "Running tests:",
    "Tests failed - asking",
    "Integration failed:",
    "failed to start ",
    "restored from a deleted task",
    "reviewing with ",
    "task result",
    "---- new attempt",
    "Permission requested",
];

/// Whether a line is tool/system decoration rather than agent prose.
fn is_noise(body: &str) -> bool {
    body.starts_with("[tool] ")
        || body.starts_with("$ ")
        || body.starts_with("> build")
        || matches!(body.chars().next(), Some('→' | '⧗' | '•' | '✗' | '⬢'))
}

/// Remove a leading log timestamp (`HH:MM:SS ` or `MM-DD HH:MM:SS `) if present,
/// so stamped lines are classified by their content.
fn strip_stamp(line: &str) -> &str {
    let b = line.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    let time = |o: usize| {
        b.len() >= o + 8
            && digit(o)
            && digit(o + 1)
            && b[o + 2] == b':'
            && digit(o + 3)
            && digit(o + 4)
            && b[o + 5] == b':'
            && digit(o + 6)
            && digit(o + 7)
    };
    // "MM-DD HH:MM:SS "
    if time(6)
        && b.len() >= 15
        && digit(0)
        && digit(1)
        && b[2] == b'-'
        && digit(3)
        && digit(4)
        && b[5] == b' '
        && b[14] == b' '
    {
        return &line[15..];
    }
    // "HH:MM:SS "
    if time(0) && b.len() >= 9 && b[8] == b' ' {
        return &line[9..];
    }
    line
}

/// The agent prose a line carries, or `None` when it is tool/app/system noise.
fn prose(line: &str) -> Option<String> {
    let trimmed = strip_stamp(line.trim_start()).trim();
    let tagged = trimmed.starts_with("[Solayge] ");
    let body = trimmed.strip_prefix("[Solayge] ").unwrap_or(trimmed);
    if tagged && APP_NOTES.iter().any(|p| body.starts_with(p)) {
        return None;
    }
    if is_noise(body) {
        return None;
    }
    Some(body.to_string())
}

/// Recover the last contiguous block of agent prose from a task log.
pub fn from_log(text: &str) -> Option<String> {
    let clean = crate::opencode::strip_ansi(text);
    let attempt = last_attempt(&clean);

    // A wrapped <summary> is unambiguous in any format, so prefer it.
    if let Some(inner) = unwrap_tag(attempt, "summary") {
        let inner = inner.trim();
        if inner.chars().count() >= 4 {
            return Some(truncate_chars(inner, MAX_RESULT_CHARS));
        }
    }

    // The non-interactive CLI prints tool results inline with prose and does not
    // mark where one ends, so without a <summary> tag it cannot be separated
    // reliably. The structured server format logs tools distinctly, below.
    if attempt.contains("> build ·") {
        return None;
    }

    let mut last: Vec<String> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in attempt.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                current.push(String::new());
            }
            continue;
        }
        match prose(line) {
            Some(text) => current.push(text),
            // A tool/app/system line ends the current block.
            None => {
                if !current.is_empty() {
                    last = std::mem::take(&mut current);
                }
            }
        }
    }
    if !current.is_empty() {
        last = current;
    }

    let block = trim_blank(last);
    let block = block.trim();
    if block.chars().count() < 4 {
        return None;
    }
    Some(truncate_chars(block, MAX_RESULT_CHARS))
}

/// Read the tail of a log file and recover its result.
pub fn from_log_file(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
        .ok()?;
    let mut buf = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut buf).ok()?;
    from_log(&String::from_utf8_lossy(&buf))
}

/// The part of a log after the most recent attempt separator, if any.
fn last_attempt(clean: &str) -> &str {
    match clean.rfind("---- new attempt") {
        Some(i) => match clean[i..].find('\n') {
            Some(j) => &clean[i + j + 1..],
            None => "",
        },
        None => clean,
    }
}

fn trim_blank(mut lines: Vec<String>) -> String {
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    lines.join("\n")
}

/// The body of the last `<tag>…</tag>` in `text`, if present.
fn unwrap_tag<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.rfind(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(&text[start..end])
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::from_log;

    #[test]
    fn recovers_the_final_prose_block_and_drops_noise() {
        let log = "\
[Solayge] opencode session ses_1
[Solayge] I'll start by reading the code.
[tool] read: src/main.rs
[Solayge] Working on it.
[tool] shell: pnpm test
[Solayge] Here is the summary.
## Done
- a
- b
";
        assert_eq!(
            from_log(log).unwrap(),
            "Here is the summary.\n## Done\n- a\n- b"
        );
    }

    #[test]
    fn strips_the_old_solayge_tag_from_agent_text() {
        let log = "[Solayge] opencode session ses_1\n[Solayge] The final answer.\nsecond line\n";
        assert_eq!(from_log(log).unwrap(), "The final answer.\nsecond line");
    }

    #[test]
    fn unwraps_a_summary_tag() {
        let log = "preamble\n<summary>\n# Result\nok\n</summary>\n";
        assert_eq!(from_log(log).unwrap(), "# Result\nok");
    }

    #[test]
    fn uses_a_summary_tag_from_cli_output() {
        let log = "> build · model\n→ Read README.md\nREADME contents\n$ ls\n./a\n<summary>\n## Summary\ndone\n</summary>\n";
        assert_eq!(from_log(log).unwrap(), "## Summary\ndone");
    }

    #[test]
    fn does_not_guess_from_cli_output_without_a_summary() {
        // Tool results and prose are interleaved with no marker, so a wrong
        // guess (showing command output as the result) is worse than none.
        let log = "> build · model\n$ ls -la\n./a\n./b\n## Maybe\n";
        assert!(from_log(log).is_none());
    }

    #[test]
    fn ignores_timestamp_prefixes() {
        let log = "\
09-29 17:20:00 [Solayge] opencode session ses_1
17:20:05 [tool] read: src/main.rs
17:20:10 [Solayge] Here it is.
## Done
";
        assert_eq!(from_log(log).unwrap(), "Here it is.\n## Done");
    }

    #[test]
    fn uses_only_the_latest_attempt() {
        let log =
            "[Solayge] old attempt\n---- new attempt: 01-01 00:00:00 ----\n[Solayge] new attempt\n";
        assert_eq!(from_log(log).unwrap(), "new attempt");
    }

    #[test]
    fn no_result_without_prose() {
        assert!(from_log("").is_none());
        assert!(from_log("[tool] read: x\n$ ls\n[Solayge] Waiting for you: hi\n").is_none());
        assert!(from_log("x\n").is_none(), "too short to be a summary");
    }
}
