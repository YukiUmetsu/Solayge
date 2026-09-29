use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::agent;
use crate::models::{
    CommandTemplates, PermissionProfile, PlanDraft, PlanResult, PlanTask, ProjectSkill, Provider,
};

fn skills_section(skills: &[ProjectSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\nPROJECT SKILLS — reusable commands defined for this project. When a task is \
         essentially one of these, reference it by name and use the command as given:\n",
    );
    for s in skills {
        out.push_str(&format!("- {}: {}", s.name, s.command));
        if !s.description.trim().is_empty() {
            out.push_str(&format!(" ({})", s.description.trim()));
        }
        out.push('\n');
    }
    out
}

pub fn planner_prompt(goal: &str, project_path: &str, max_tasks: usize, skills: &[ProjectSkill]) -> String {
    format!(
        r#"You are a planning assistant. You design work for coding agents that will run
inside the git repository at: {project_path}

GOAL FROM THE USER:
{goal}
{skills}
Produce a plan of {max_tasks} or fewer self-contained tasks that accomplish the goal.
Do NOT perform any of the work. Do NOT modify any files. Do NOT run tools.
Respond with ONLY a single JSON object, no prose, no markdown fences.

JSON schema:
{{
  "summary": "one short paragraph describing the plan",
  "tasks": [
    {{
      "id": "t1",
      "title": "short human title",
      "prompt": "a complete, self-contained instruction for a fresh coding agent, including context, files/areas to change, and acceptance criteria",
      "isolation": "worktree" | "shared",
      "after": ["t0"],
      "delay_seconds": 0
    }}
  ]
}}

Rules:
- "after" lists the ids of tasks that must succeed before this task starts. Use [] for tasks that can start immediately.
- Tasks with no dependency between them may run in parallel.
- Use "worktree" for tasks that change code (they run in an isolated git worktree, so they can run in parallel safely).
- Use "shared" only for read-only analysis tasks. At most one "shared" task runs at a time.
- Keep the dependency graph acyclic.
- Make each "prompt" detailed enough that an agent could do it without seeing this plan.
- Output only the JSON object."#,
        skills = skills_section(skills)
    )
}

pub fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            // Skip CSI/OSC escape sequences.
            i += 1;
            if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else if i < bytes.len() && bytes[i] == b']' {
                i += 1;
                while i < bytes.len() && bytes[i] != 0x07 {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else if i < bytes.len() {
                i += 1;
            }
        } else {
            let ch = input[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

pub fn extract_plan(raw: &str) -> Option<PlanDraft> {
    let clean = strip_ansi(raw);
    let start = clean.find('{')?;
    let end = clean.rfind('}')?;
    if end <= start {
        return None;
    }
    let slice = &clean[start..=end];
    serde_json::from_str::<PlanDraft>(slice).ok()
}

/// Run an agent once and collect its whole output. Used by the planner.
pub async fn run_agent_once(
    provider: Provider,
    model: Option<&str>,
    templates: &CommandTemplates,
    dir: &Path,
    prompt: &str,
    timeout_secs: u64,
    env: &[(String, String)],
) -> Result<String> {
    let mut cmd = agent::build_command(
        provider,
        model,
        templates,
        prompt,
        PermissionProfile::Readonly,
        dir,
    )
    .map_err(|e| anyhow!(e))?;
    agent::apply_env(&mut cmd, env);
    let fut = cmd.output();
    match tokio::time::timeout(Duration::from_secs(timeout_secs), fut).await {
        Ok(Ok(out)) => {
            if !out.status.success() {
                return Err(anyhow!(
                    "{} exited with {}: {}",
                    provider.command_key(),
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        }
        Ok(Err(e)) => Err(anyhow!("failed to run {}: {e}", provider.command_key())),
        Err(_) => Err(anyhow!("planner timed out after {timeout_secs}s")),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn plan(
    project_path: &str,
    goal: &str,
    max_tasks: usize,
    timeout_secs: u64,
    provider: Provider,
    model: Option<&str>,
    templates: &CommandTemplates,
    skills: &[ProjectSkill],
    env: &[(String, String)],
) -> Result<PlanResult> {
    let prompt = planner_prompt(goal, project_path, max_tasks, skills);
    let raw = run_agent_once(
        provider,
        model,
        templates,
        Path::new(project_path),
        &prompt,
        timeout_secs,
        env,
    )
    .await?;
    let draft = extract_plan(&raw)
        .ok_or_else(|| anyhow!("could not parse a plan from the model output (see raw output)"))?;
    let tasks: Vec<PlanTask> = draft.tasks;
    Ok(PlanResult {
        summary: draft.summary,
        tasks,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::{extract_plan, strip_ansi};

    #[test]
    fn strips_ansi_sequences() {
        assert_eq!(strip_ansi("\u{1b}[0mhello\u{1b}[1;32m!\u{1b}[0m"), "hello!");
    }

    #[test]
    fn extracts_plan_from_noisy_output() {
        let raw = "\u{1b}[0m\n> build · model\ntext before\n```json\n\
{\"summary\":\"do it\",\"tasks\":[{\"title\":\"a\",\"prompt\":\"p\",\"after\":[]}]}\n\
```\ntrailing";
        let draft = extract_plan(raw).expect("should parse");
        assert_eq!(draft.summary, "do it");
        assert_eq!(draft.tasks.len(), 1);
        assert_eq!(draft.tasks[0].title, "a");
    }

    #[test]
    fn rejects_output_without_json() {
        assert!(extract_plan("no object here").is_none());
    }
}
