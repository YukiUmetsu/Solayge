//! Client for the opencode v2 HTTP server (`opencode serve`).
//!
//! `opencode run` is non-interactive: it auto-dismisses the agent's `question`
//! tool, so tasks that need input die. The server exposes the same session over
//! HTTP, with explicit endpoints for questions (forms) and permission requests,
//! so Solayge can surface them and answer.

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::models::{AskField, AskFieldKind, AskKind, AskOption, TaskAsk};

/// A running `opencode serve` process and how to reach it.
pub struct ServerProc {
    pub base: String,
    pub password: String,
    child: tokio::process::Child,
}

/// Connection info handed to callers (cheap to clone; the process stays in the
/// module-level slot).
#[derive(Clone)]
pub struct Connection {
    pub base: String,
    pub password: String,
}

// A single shared server for the whole app.
static SERVER: std::sync::Mutex<Option<ServerProc>> = std::sync::Mutex::new(None);

fn client() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("http client")
    })
}

/// Start (or reuse) the shared opencode server.
pub async fn ensure_server() -> Result<Connection, String> {
    {
        let mut guard = crate::state::lock(&SERVER);
        if let Some(p) = guard.as_mut() {
            if matches!(p.child.try_wait(), Ok(None)) {
                return Ok(Connection {
                    base: p.base.clone(),
                    password: p.password.clone(),
                });
            }
        }
    }
    // Spawn outside the lock (reading the banner awaits).
    let proc = spawn_server().await?;
    let conn = Connection {
        base: proc.base.clone(),
        password: proc.password.clone(),
    };
    *crate::state::lock(&SERVER) = Some(proc);
    Ok(conn)
}

/// Kill the shared server. Called on app exit so it is not left orphaned.
pub fn shutdown() {
    if let Some(mut p) = crate::state::lock(&SERVER).take() {
        let _ = p.child.start_kill();
    }
}

fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    Ok(port)
}

async fn spawn_server() -> Result<ServerProc, String> {
    let port = free_port()?;
    let mut child = tokio::process::Command::new("opencode")
        .args([
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to start `opencode serve`: {e}"))?;

    let stdout = child.stdout.take().ok_or("no server stdout")?;
    let stderr = child.stderr.take().ok_or("no server stderr")?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    for stream in [Box::new(stdout) as Box<dyn tokio::io::AsyncRead + Unpin + Send>, Box::new(stderr)] {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(line);
            }
        });
    }
    drop(tx);

    let mut base: Option<String> = None;
    let mut password: Option<String> = None;
    let deadline = Instant::now() + Duration::from_secs(20);
    while (base.is_none() || password.is_none()) && Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(line)) => {
                if let Some(u) = line.split("server listening on ").nth(1) {
                    base = Some(u.trim().trim_end_matches('/').to_string());
                }
                if let Some(p) = line.split("server password ").nth(1) {
                    password = Some(p.trim().to_string());
                }
            }
            _ => break,
        }
    }
    // Keep draining so the server never blocks on a full pipe.
    tokio::spawn(async move { while rx.recv().await.is_some() {} });

    let base = base.ok_or("opencode server did not report a listening URL")?;
    let password =
        password.ok_or("opencode server did not report a password")?;
    Ok(ServerProc {
        base,
        password,
        child,
    })
}

async fn request(
    conn: &Connection,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let url = format!("{}{}", conn.base, path);
    let mut rb = client()
        .request(method, &url)
        .basic_auth("opencode", Some(&conn.password));
    if let Some(b) = body {
        rb = rb.json(&b);
    }
    let resp = rb.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("{status}: {}", text.trim()));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("bad JSON from server: {e}"))
}

fn data(v: &Value) -> Value {
    v.get("data").cloned().unwrap_or_else(|| v.clone())
}

/// Parse a `provider/model` id into the server's `{providerID, id}` shape.
pub fn model_ref(model: Option<&str>) -> Option<Value> {
    let m = model?.trim();
    if m.is_empty() {
        return None;
    }
    match m.split_once('/') {
        Some((provider, id)) if !provider.is_empty() && !id.is_empty() => {
            Some(json!({ "providerID": provider, "id": id }))
        }
        _ => None,
    }
}

pub async fn create_session(
    conn: &Connection,
    directory: &str,
    title: &str,
    model: Option<&str>,
) -> Result<String, String> {
    let mut body = json!({
        "title": title,
        "location": { "directory": directory },
    });
    if let Some(m) = model_ref(model) {
        body["model"] = m;
    }
    let v = request(conn, reqwest::Method::POST, "/api/session", Some(body)).await?;
    data(&v)
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "server did not return a session id".to_string())
}

pub async fn prompt(conn: &Connection, session: &str, text: &str) -> Result<(), String> {
    let path = format!("/api/session/{session}/prompt");
    request(conn, reqwest::Method::POST, &path, Some(json!({ "text": text })))
        .await
        .map(|_| ())
}

pub async fn interrupt(conn: &Connection, session: &str) -> Result<(), String> {
    let path = format!("/api/session/{session}/interrupt");
    request(conn, reqwest::Method::POST, &path, Some(json!({}))).await.map(|_| ())
}

pub async fn messages(conn: &Connection, session: &str) -> Result<Vec<Value>, String> {
    let path = format!("/api/session/{session}/message");
    let v = request(conn, reqwest::Method::GET, &path, None).await?;
    Ok(list(&v))
}

pub async fn forms(conn: &Connection, session: &str) -> Result<Vec<Value>, String> {
    let path = format!("/api/session/{session}/form");
    let v = request(conn, reqwest::Method::GET, &path, None).await?;
    Ok(list(&v))
}

pub async fn permissions(conn: &Connection, session: &str) -> Result<Vec<Value>, String> {
    let path = format!("/api/session/{session}/permission");
    let v = request(conn, reqwest::Method::GET, &path, None).await?;
    Ok(list(&v))
}

pub async fn reply_form(
    conn: &Connection,
    session: &str,
    form: &str,
    answer: Value,
) -> Result<(), String> {
    let path = format!("/api/session/{session}/form/{form}/reply");
    request(conn, reqwest::Method::POST, &path, Some(json!({ "answer": answer })))
        .await
        .map(|_| ())
}

pub async fn reply_permission(
    conn: &Connection,
    session: &str,
    request_id: &str,
    decision: &str,
    message: Option<&str>,
) -> Result<(), String> {
    let path = format!("/api/session/{session}/permission/{request_id}/reply");
    let mut body = json!({ "decision": decision });
    if let Some(m) = message.filter(|m| !m.trim().is_empty()) {
        body["message"] = json!(m);
    }
    request(conn, reqwest::Method::POST, &path, Some(body))
        .await
        .map(|_| ())
}

fn list(v: &Value) -> Vec<Value> {
    data(v).as_array().cloned().unwrap_or_default()
}

/// The terminal outcome of a session, read from its `idle` message.
pub fn session_outcome(messages: &[Value]) -> Option<String> {
    messages.iter().rev().find_map(|m| {
        if m.get("type").and_then(Value::as_str) == Some("idle") {
            m.get("outcome").and_then(Value::as_str).map(str::to_string)
        } else {
            None
        }
    })
}

/// Collapse whitespace so a tool line stays a single line.
fn one_line(s: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for ch in s.chars() {
        let c = if ch.is_whitespace() { ' ' } else { ch };
        if c == ' ' {
            if space {
                continue;
            }
            space = true;
        } else {
            space = false;
        }
        out.push(c);
    }
    out.trim().to_string()
}

/// Cap a string at `max` characters, marking the cut.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// What a tool part acted on (the command, file, or query), so the log says
/// more than just the tool's name.
fn tool_target(name: &str, input: &Value) -> Option<String> {
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    match name {
        "shell" | "bash" => field("command").map(str::to_string),
        "read" | "write" | "edit" | "patch" => field("path").map(str::to_string),
        "grep" => {
            let pattern = field("pattern").unwrap_or_default();
            let scope = field("path").or_else(|| field("include"));
            Some(match scope {
                Some(s) => format!("/{pattern}/ in {s}"),
                None => format!("/{pattern}/"),
            })
        }
        "glob" | "list" => field("pattern").map(str::to_string),
        "webfetch" | "fetch" => field("url").map(str::to_string),
        "task" | "agent" => field("description").map(str::to_string),
        _ => None,
    }
}

/// One short line per useful part of an assistant message: its text, or a tool
/// call with what it acted on. Structural parts are skipped.
pub fn part_lines(message: &Value) -> Vec<String> {
    if message.get("type").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    let Some(content) = message.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for part in content {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = part.get("text").and_then(Value::as_str) {
                    if !t.trim().is_empty() {
                        out.push(t.to_string());
                    }
                }
            }
            Some("tool") => {
                let name = part.get("name").and_then(Value::as_str).unwrap_or("tool");
                let state = part.get("state");
                let status = state
                    .and_then(|s| s.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let input = state.and_then(|s| s.get("input")).cloned().unwrap_or(Value::Null);
                let mut line = format!("[tool] {name}");
                if let Some(target) = tool_target(name, &input).filter(|t| !t.trim().is_empty()) {
                    line.push_str(": ");
                    line.push_str(&clip(&one_line(&target), 160));
                }
                // "running" status is implied; only surface unusual states.
                if !status.is_empty() && status != "completed" && status != "running" {
                    line.push_str(&format!(" ({status})"));
                }
                out.push(line);
            }
            _ => {}
        }
    }
    out
}

/// The agent's final assistant message as markdown text, if any. Used as the
/// task's "result" so it can be rendered and re-read later.
///
/// The server returns messages newest-first while exports are oldest-first, so
/// pick the text with the latest completion time rather than trusting position.
pub fn final_text(messages: &[Value]) -> Option<String> {
    let mut best: Option<(i64, String)> = None;
    for m in messages {
        if m.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let text = assistant_text(m);
        if text.trim().is_empty() {
            continue;
        }
        let at = m
            .get("time")
            .and_then(|t| t.get("completed").or_else(|| t.get("created")))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if best.as_ref().is_none_or(|(previous, _)| at > *previous) {
            best = Some((at, text));
        }
    }
    best.map(|(_, text)| text)
}

/// The text parts of one message, joined.
fn assistant_text(message: &Value) -> String {
    message
        .get("content")
        .and_then(Value::as_array)
        .map(|content| {
            content
                .iter()
                .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default()
}

fn field_kind(s: &str) -> Option<AskFieldKind> {
    Some(match s {
        "string" => AskFieldKind::String,
        "number" => AskFieldKind::Number,
        "integer" => AskFieldKind::Integer,
        "boolean" => AskFieldKind::Boolean,
        "multiselect" => AskFieldKind::Multiselect,
        _ => return None,
    })
}

fn field_from_json(f: &Value) -> Option<AskField> {
    let key = f.get("key").and_then(Value::as_str)?.to_string();
    let kind = field_kind(f.get("type").and_then(Value::as_str)?)?;
    let label = f
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(&key)
        .to_string();
    let options = f
        .get("options")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|o| {
                    Some(AskOption {
                        value: o.get("value").and_then(Value::as_str)?.to_string(),
                        label: o
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        description: o
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(AskField {
        key,
        label,
        description: f.get("description").and_then(Value::as_str).map(str::to_string),
        kind,
        required: f.get("required").and_then(Value::as_bool).unwrap_or(false),
        options,
        default: f.get("default").cloned(),
        placeholder: f.get("placeholder").and_then(Value::as_str).map(str::to_string),
    })
}

/// Map an opencode form (the `question` tool) to a [`TaskAsk`].
pub fn form_to_ask(session: &str, form: &Value) -> Option<TaskAsk> {
    let id = form.get("id").and_then(Value::as_str)?.to_string();
    let title = form
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("The agent has a question")
        .to_string();
    let fields: Vec<AskField> = form
        .get("fields")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(field_from_json).collect())
        .unwrap_or_default();
    Some(TaskAsk {
        id,
        kind: AskKind::Question,
        title,
        message: None,
        action: None,
        resource: None,
        purpose: None,
        fields,
        options: Vec::new(),
        created_at: None,
        session_id: Some(session.to_string()),
    })
}

/// A short, human purpose for a provider permission action, so the in-app prompt
/// and notification explain *why* the access is wanted rather than just naming a
/// tool.
fn permission_purpose(action: &str) -> &'static str {
    match action {
        "external_directory" => "Access a folder outside this project",
        "read" => "Read a file",
        "write" | "edit" | "patch" => "Modify a file",
        "shell" | "bash" | "execute" => "Run a shell command",
        "webfetch" | "fetch" | "websearch" => "Use the network",
        "glob" | "list" => "List files",
        "grep" => "Search file contents",
        _ => "Use a tool",
    }
}

/// Map an opencode permission request to a [`TaskAsk`].
pub fn permission_to_ask(session: &str, req: &Value) -> Option<TaskAsk> {
    let id = req.get("id").and_then(Value::as_str)?.to_string();
    let action = req
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("use a tool");
    let resources = req
        .get("resources")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let message = req
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string);
    let purpose = permission_purpose(action);
    Some(TaskAsk {
        id,
        kind: AskKind::Permission,
        title: format!("Permission: {action}"),
        message,
        action: Some(action.to_string()),
        resource: (!resources.is_empty()).then_some(resources),
        purpose: Some(purpose.to_string()),
        fields: Vec::new(),
        options: vec!["once".into(), "always".into(), "reject".into()],
        created_at: None,
        session_id: Some(session.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_model_refs() {
        assert_eq!(
            model_ref(Some("anthropic/claude-sonnet-4-5")),
            Some(json!({"providerID":"anthropic","id":"claude-sonnet-4-5"}))
        );
        assert_eq!(model_ref(Some("bare-model")), None);
        assert_eq!(model_ref(None), None);
    }

    #[test]
    fn reads_session_outcome_from_idle() {
        let msgs = vec![
            json!({"type":"assistant","content":[]}),
            json!({"type":"idle","outcome":"succeeded"}),
        ];
        assert_eq!(session_outcome(&msgs).as_deref(), Some("succeeded"));
        assert_eq!(session_outcome(&[]), None);
    }

    #[test]
    fn maps_a_question_form() {
        let form = json!({
            "id": "frm_1",
            "sessionID": "ses_1",
            "title": "Favorite color",
            "fields": [{
                "key": "color", "type": "string", "title": "What is your favorite color?",
                "required": true,
                "options": [
                    {"value":"red","label":"Red","description":"warm"},
                    {"value":"blue","label":"Blue"}
                ]
            }]
        });
        let ask = form_to_ask("ses_1", &form).expect("ask");
        assert_eq!(ask.kind, AskKind::Question);
        assert_eq!(ask.id, "frm_1");
        assert_eq!(ask.fields.len(), 1);
        assert_eq!(ask.fields[0].kind, AskFieldKind::String);
        assert_eq!(ask.fields[0].options.len(), 2);
        assert_eq!(ask.fields[0].options[1].value, "blue");
    }

    #[test]
    fn maps_a_permission_request() {
        let req = json!({
            "id":"per_1","sessionID":"ses_1","action":"bash",
            "resources":["rm -rf build"],"message":"allow?"
        });
        let ask = permission_to_ask("ses_1", &req).expect("ask");
        assert_eq!(ask.kind, AskKind::Permission);
        assert_eq!(ask.options, vec!["once", "always", "reject"]);
        // The concrete resource is surfaced on its own line, separate from the
        // provider's message, so the UI can show exactly what is requested.
        assert_eq!(ask.resource.as_deref(), Some("rm -rf build"));
        assert_eq!(ask.message.as_deref(), Some("allow?"));
        assert_eq!(ask.action.as_deref(), Some("bash"));
        assert!(ask.purpose.is_some());
    }

    #[test]
    fn renders_assistant_text_and_tools() {
        let msg = json!({"type":"assistant","content":[
            {"type":"text","text":"Hello"},
            {"type":"tool","name":"bash","state":{"status":"completed"}}
        ]});
        let lines = part_lines(&msg);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Hello");
        assert_eq!(lines[1], "[tool] bash");
    }

    #[test]
    fn tool_lines_name_what_the_tool_acted_on() {
        let msg = json!({"type":"assistant","content":[
            {"type":"tool","name":"read","state":{"status":"completed",
             "input":{"path":"src/main.rs"}}},
            {"type":"tool","name":"shell","state":{"status":"completed",
             "input":{"command":"pnpm test\n--watch"}}},
            {"type":"tool","name":"grep","state":{"status":"completed",
             "input":{"pattern":"TODO","include":"*.rs"}}},
            {"type":"tool","name":"edit","state":{"status":"error",
             "input":{"path":"src/lib.rs"}}}
        ]});
        let lines = part_lines(&msg);
        assert_eq!(lines[0], "[tool] read: src/main.rs");
        // The command is collapsed onto one line.
        assert_eq!(lines[1], "[tool] shell: pnpm test --watch");
        assert_eq!(lines[2], "[tool] grep: /TODO/ in *.rs");
        // An unusual status is surfaced.
        assert_eq!(lines[3], "[tool] edit: src/lib.rs (error)");
    }

    #[test]
    fn ignores_non_assistant_messages() {
        assert!(part_lines(&json!({"type":"idle","outcome":"succeeded"})).is_empty());
    }

    #[test]
    fn takes_the_most_recent_assistant_text_regardless_of_order() {
        let older = json!({"type":"assistant","time":{"completed":100},
            "content":[{"type":"text","text":"I'll start by exploring."}]});
        let tool_only = json!({"type":"assistant","time":{"completed":150},
            "content":[{"type":"tool","name":"shell",
             "state":{"status":"completed","input":{"command":"ls"}}}]});
        let newer = json!({"type":"assistant","time":{"completed":200},
            "content":[{"type":"text","text":"# Done\n\nAll good."}]});

        // The server returns newest-first; exports are oldest-first. Either way
        // the latest text must win, not the first one encountered.
        let newest_first = vec![newer.clone(), tool_only.clone(), older.clone()];
        assert_eq!(final_text(&newest_first).as_deref(), Some("# Done\n\nAll good."));

        let oldest_first = vec![older, tool_only, newer];
        assert_eq!(final_text(&oldest_first).as_deref(), Some("# Done\n\nAll good."));
    }

    #[test]
    fn no_final_text_when_there_is_none() {
        assert_eq!(final_text(&[]), None);
        assert_eq!(final_text(&[json!({"type":"assistant","content":[]})]), None);
        assert_eq!(
            final_text(&[json!({"type":"user","content":[{"type":"text","text":"hi"}]})]),
            None
        );
    }
}
