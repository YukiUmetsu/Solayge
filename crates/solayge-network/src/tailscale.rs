//! Tailscale CLI integration for the desktop host's optional backup transport.
//!
//! Solayge never embeds a VPN and never modifies the user's Tailscale account or
//! `serve` configuration beyond the endpoints it created itself. This module is
//! a thin, explicit wrapper over the installed `tailscale` CLI:
//!
//! * **Detection** searches the platform-appropriate locations and the login
//!   `PATH` for the executable; the CLI ships inside the app bundle on macOS.
//! * **Status** reads `tailscale status --json` for backend state, authentication
//!   and the MagicDNS name used to build the `wss://` endpoint.
//! * **Serve** reads `tailscale serve status --json`, starts Solayge's own
//!   loopback proxy, and stops it again — but only when the configured handlers
//!   on that HTTPS port all point at Solayge's loopback endpoint. Unrelated
//!   `serve` entries are never modified.
//!
//! Every command is executed with an explicit argument vector (`Command::new`)
//! and no shell, so a hostname or path can never be interpreted as shell syntax.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// The loopback host a Solayge Serve proxy must never leave.
pub const LOOPBACK_PROXY_HOST: &str = "127.0.0.1";

/// Run one process invocation and return its output. Implemented for the real
/// system and stubbed in tests.
pub trait CommandRunner: Send + Sync + 'static {
    /// Run `exe` with `args`. Returns `Err` only when the process could not be
    /// spawned at all (for example the executable does not exist).
    fn run(&self, exe: &Path, args: &[String]) -> Result<CommandOutput, String>;
}

/// The captured result of one process invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// The process exit status code, when the process exited normally.
    pub code: Option<i32>,
    /// Whether the process exited with status 0.
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    /// A successful invocation with `stdout`.
    pub fn ok(stdout: impl Into<String>) -> Self {
        CommandOutput {
            code: Some(0),
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    /// A failed invocation with `stderr`.
    pub fn err(message: impl Into<String>) -> Self {
        CommandOutput {
            code: Some(1),
            success: false,
            stdout: String::new(),
            stderr: message.into(),
        }
    }
}

/// The real process runner. No shell is involved: `exe` and `args` are passed
/// directly to `Command::new`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCommandRunner;

/// The `TERM` value to inject for a spawned CLI, or `None` when the host already
/// provides one.
///
/// The macOS `Tailscale.app` bundle binary decides whether it was launched as
/// the GUI app or as the command-line tool by testing for terminal environment
/// variables (`TERM`, `SHLVL`, `TERM_PROGRAM`). A GUI-launched host such as
/// Solayge has none of them, so the binary wrongly takes the GUI path, fails to
/// start a second instance, and prints
/// `The Tailscale GUI failed to start: …` to **stdout with exit code 0**. That
/// sentence is not JSON, so `status()` cannot parse it. Injecting a non-empty
/// `TERM` when the host has none forces the CLI path and is a no-op elsewhere.
fn term_override(current: Option<&str>) -> Option<&'static str> {
    match current {
        Some(value) if !value.is_empty() => None,
        _ => Some("dumb"),
    }
}

impl CommandRunner for SystemCommandRunner {
    fn run(&self, exe: &Path, args: &[String]) -> Result<CommandOutput, String> {
        let mut command = std::process::Command::new(exe);
        command.args(args);
        if let Some(term) = term_override(std::env::var("TERM").ok().as_deref()) {
            command.env("TERM", term);
        }
        let output = command
            .output()
            .map_err(|e| format!("failed to run {}: {e}", exe.display()))?;
        Ok(CommandOutput {
            code: output.status.code(),
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Why a Tailscale operation could not complete. Every variant carries enough
/// context to render an actionable Settings message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailscaleError {
    /// The executable was not found on this machine.
    NotInstalled,
    /// Tailscale is installed but the daemon is not running.
    NotRunning { backend_state: String },
    /// Tailscale is running but the user is not signed in / the node is offline.
    NotAuthenticated,
    /// The requested HTTPS port already serves something that is not Solayge.
    ServeConflict {
        https_port: u16,
        existing: Vec<String>,
    },
    /// A `tailscale` invocation failed.
    CommandFailed { command: String, message: String },
    /// The Serve configuration did not point at Solayge after it was applied.
    Unverified { https_port: u16 },
}

impl std::fmt::Display for TailscaleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TailscaleError::NotInstalled => write!(
                f,
                "Tailscale is not installed. Install it from tailscale.com/download"
            ),
            TailscaleError::NotRunning { backend_state } => write!(
                f,
                "Tailscale is not running (state: {backend_state}). Open Tailscale and try again"
            ),
            TailscaleError::NotAuthenticated => write!(
                f,
                "Tailscale is not signed in. Sign in and make sure this device is online"
            ),
            TailscaleError::ServeConflict {
                https_port,
                existing,
            } => write!(
                f,
                "Tailscale Serve on port {https_port} is already used by another service ({}). \
                 Solayge will not overwrite it",
                existing.join(", ")
            ),
            TailscaleError::CommandFailed { command, message } => {
                write!(f, "`tailscale {command}` failed: {message}")
            }
            TailscaleError::Unverified { https_port } => write!(
                f,
                "Tailscale Serve on port {https_port} does not point at Solayge after setup"
            ),
        }
    }
}

impl std::error::Error for TailscaleError {}

/// The parsed `tailscale status --json` view that Solayge cares about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailscaleStatus {
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exe: Option<PathBuf>,
    /// `Running`, `Stopped`, `NeedsLogin`, `NoState`, …
    pub backend_state: String,
    pub authenticated: bool,
    pub online: bool,
    /// MagicDNS name without the trailing dot, e.g.
    /// `macbook-pro.berylline-goblin.ts.net`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tailnet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// A user-facing explanation when the node is not usable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl TailscaleStatus {
    /// The status of a machine with no Tailscale at all.
    pub fn absent() -> Self {
        TailscaleStatus {
            installed: false,
            exe: None,
            backend_state: "NotInstalled".to_string(),
            authenticated: false,
            online: false,
            dns_name: None,
            tailnet: None,
            version: None,
            message: Some("Tailscale is not installed".to_string()),
        }
    }

    /// True when the daemon reports `Running`.
    pub fn running(&self) -> bool {
        self.backend_state.eq_ignore_ascii_case("Running")
    }

    /// True when the node is running and has a MagicDNS name to serve on.
    pub fn ready(&self) -> bool {
        self.running() && self.dns_name.is_some()
    }
}

/// One `serve` handler that proxies a path to a local target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeProxy {
    /// The proxy target (a local URL).
    pub target: String,
    /// The HTTPS port this handler belongs to, when it could be determined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// The parsed `tailscale serve status --json` view.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServeStatus {
    /// Whether any HTTPS/TCP proxy entry exists.
    pub configured: bool,
    /// The HTTPS ports that currently have a TLS listener.
    pub https_ports: Vec<u16>,
    /// Every proxy target found, regardless of port.
    pub proxies: Vec<ServeProxy>,
    /// Funnel-enabled ports. Solayge never configures Funnel and refuses to use
    /// a port that is publicly funneled.
    pub funnel_ports: Vec<u16>,
}

impl ServeStatus {
    /// A port with no configuration at all.
    pub fn unconfigured() -> Self {
        ServeStatus::default()
    }

    /// The distinct proxy targets on the wire, for conflict reporting.
    pub fn targets(&self) -> Vec<String> {
        let mut targets: Vec<String> = self.proxies.iter().map(|p| p.target.clone()).collect();
        targets.sort();
        targets.dedup();
        targets
    }

    /// The distinct proxy targets configured on one HTTPS port.
    pub fn targets_for_port(&self, https_port: u16) -> Vec<String> {
        let mut targets: Vec<String> = self
            .proxies
            .iter()
            .filter(|p| p.port == Some(https_port))
            .map(|p| p.target.clone())
            .collect();
        targets.sort();
        targets.dedup();
        targets
    }

    /// Whether `https_port` has a TLS listener.
    pub fn has_https_port(&self, https_port: u16) -> bool {
        self.https_ports.contains(&https_port)
    }

    /// Whether a specific port is publicly funneled.
    pub fn is_funneled(&self, https_port: u16) -> bool {
        self.funnel_ports.contains(&https_port)
    }
}

/// What [`TailscaleCli::serve_stop`] actually did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeStopOutcome {
    /// Solayge's handler was removed.
    Removed,
    /// The port was already absent or pointed elsewhere; nothing was changed.
    LeftUntouched,
    /// The CLI reported that no Serve configuration exists.
    NotConfigured,
}

/// A located `tailscale` executable bound to a runner.
pub struct TailscaleCli {
    exe: PathBuf,
    runner: Arc<dyn CommandRunner>,
}

impl std::fmt::Debug for TailscaleCli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TailscaleCli")
            .field("exe", &self.exe)
            .finish_non_exhaustive()
    }
}

impl TailscaleCli {
    /// Bind a runner to a known executable path.
    pub fn with_runner(exe: impl Into<PathBuf>, runner: Arc<dyn CommandRunner>) -> Self {
        TailscaleCli {
            exe: exe.into(),
            runner,
        }
    }

    /// The absolute path to the executable.
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// Prove a candidate path is a runnable `tailscale` by invoking `version`.
    fn probe(runner: &dyn CommandRunner, candidate: &Path) -> bool {
        matches!(runner.run(candidate, &["version".to_string()]), Ok(out) if out.success)
    }

    /// Detect the CLI on this machine using the platform search path.
    pub fn detect() -> Option<Self> {
        Self::detect_with(Arc::new(SystemCommandRunner), candidate_paths())
    }

    /// Detect the CLI using an explicit runner and candidate list (for tests and
    /// non-standard installs).
    pub fn detect_with(runner: Arc<dyn CommandRunner>, candidates: Vec<PathBuf>) -> Option<Self> {
        for candidate in candidates {
            if Self::probe(runner.as_ref(), &candidate) {
                return Some(TailscaleCli {
                    exe: candidate,
                    runner,
                });
            }
        }
        None
    }

    /// Run one command and return its raw output.
    fn run(&self, args: &[&str]) -> Result<CommandOutput, TailscaleError> {
        let owned: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        self.runner.run(&self.exe, &owned).map_err(|message| {
            TailscaleError::CommandFailed {
                command: args.join(" "),
                message,
            }
        })
    }

    /// Read and parse `tailscale status --json`. Never errors: an unreachable
    /// daemon is reported as a non-ready status with an actionable message.
    pub fn status(&self) -> TailscaleStatus {
        let output = match self.run(&["status", "--json"]) {
            Ok(output) => output,
            Err(error) => {
                return TailscaleStatus {
                    installed: true,
                    exe: Some(self.exe.clone()),
                    backend_state: "Unavailable".to_string(),
                    authenticated: false,
                    online: false,
                    dns_name: None,
                    tailnet: None,
                    version: None,
                    message: Some(error.to_string()),
                }
            }
        };
        let mut status = parse_status(&output.stdout);
        status.installed = true;
        status.exe = Some(self.exe.clone());
        // When the CLI emitted no JSON at all, quote what it actually said so the
        // Settings panel is actionable instead of a bare "could not read". This
        // is also the fallback that would surface the macOS GUI-vs-CLI confusion
        // should a future Tailscale build regress.
        if serde_json::from_str::<serde_json::Value>(&output.stdout).is_err() {
            let detail = first_line(&output.stderr).or_else(|| first_line(&output.stdout));
            if let Some(detail) = detail {
                status.message = Some(format!("Could not read Tailscale status: {detail}"));
            }
        }
        if status.message.is_none() && !status.ready() {
            status.message = Some(if status.running() {
                "Tailscale is online but has no MagicDNS name yet".to_string()
            } else {
                format!(
                    "Tailscale is not running (state: {})",
                    status.backend_state
                )
            });
        }
        status
    }

    /// Read and parse `tailscale serve status --json`.
    pub fn serve_status(&self) -> Result<ServeStatus, TailscaleError> {
        let output = self.run(&["serve", "status", "--json"])?;
        // Older CLIs print "No serve config" on stdout for an empty config.
        if !output.success && output.stdout.trim().is_empty() && output.stderr.trim().is_empty() {
            return Ok(ServeStatus::unconfigured());
        }
        if output.stdout.trim().is_empty() {
            return Ok(ServeStatus::unconfigured());
        }
        Ok(parse_serve_status(&output.stdout))
    }

    /// The loopback proxy target for a local port.
    pub fn loopback_target(local_port: u16) -> String {
        format!("http://{LOOPBACK_PROXY_HOST}:{local_port}")
    }

    /// True when every proxy on `https_port` points at `local_port`, and the port
    /// is not funneled. An unconfigured port is also "owned" (safe to create).
    pub fn owns_https_port(
        &self,
        serve: &ServeStatus,
        https_port: u16,
        local_port: u16,
    ) -> bool {
        if serve.is_funneled(https_port) {
            return false;
        }
        if !serve.has_https_port(https_port) {
            return true;
        }
        let target = Self::loopback_target(local_port);
        let port_targets = serve.targets_for_port(https_port);
        // A port with a TLS listener but no handler we could parse is not ours;
        // never assume ownership of an unknown configuration.
        port_targets.len() == 1 && port_targets[0] == target
    }

    /// Start (or verify) a background HTTPS proxy on `https_port` pointing at
    /// `http://127.0.0.1:<local_port>`.
    ///
    /// Refuses to touch a port that already serves something else, and never
    /// passes `--funnel`.
    pub fn serve_start(
        &self,
        local_port: u16,
        https_port: u16,
    ) -> Result<ServeStatus, TailscaleError> {
        let before = self.serve_status()?;
        if !self.owns_https_port(&before, https_port, local_port) {
            let mut existing = before.targets_for_port(https_port);
            if existing.is_empty() {
                existing = before.targets();
            }
            return Err(TailscaleError::ServeConflict {
                https_port,
                existing,
            });
        }

        let target = Self::loopback_target(local_port);
        let port_flag = format!("--https={https_port}");
        let output = self.run(&["serve", "--bg", &port_flag, &target])?;
        if !output.success {
            return Err(TailscaleError::CommandFailed {
                command: format!("serve --bg {port_flag} {target}"),
                message: first_line(&output.stderr).unwrap_or_else(|| "unknown error".to_string()),
            });
        }

        // Verify the change actually took effect and still points at Solayge.
        let after = self.serve_status()?;
        if !after.has_https_port(https_port)
            || !after
                .proxies
                .iter()
                .any(|p| p.port == Some(https_port) && p.target == target)
            || !self.owns_https_port(&after, https_port, local_port)
        {
            return Err(TailscaleError::Unverified { https_port });
        }
        Ok(after)
    }

    /// Stop Solayge's Serve handler on `https_port`, but only when it is the
    /// only thing configured there. Unrelated configuration is left intact.
    pub fn serve_stop(
        &self,
        local_port: u16,
        https_port: u16,
    ) -> Result<ServeStopOutcome, TailscaleError> {
        let before = self.serve_status()?;
        if !before.has_https_port(https_port) {
            return Ok(ServeStopOutcome::NotConfigured);
        }
        if !self.owns_https_port(&before, https_port, local_port) {
            // Someone else added configuration on this port; never remove it.
            return Ok(ServeStopOutcome::LeftUntouched);
        }
        let port_flag = format!("--https={https_port}");
        let output = self.run(&["serve", &port_flag, "off"])?;
        if !output.success {
            return Err(TailscaleError::CommandFailed {
                command: format!("serve {port_flag} off"),
                message: first_line(&output.stderr).unwrap_or_else(|| "unknown error".to_string()),
            });
        }
        Ok(ServeStopOutcome::Removed)
    }
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// Parse `tailscale status --json` tolerantly; unknown or absent fields become
/// conservative defaults rather than a hard failure.
pub fn parse_status(json: &str) -> TailscaleStatus {
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(value) => value,
        Err(_) => {
            return TailscaleStatus {
                installed: true,
                exe: None,
                backend_state: "Unknown".to_string(),
                authenticated: false,
                online: false,
                dns_name: None,
                tailnet: None,
                version: None,
                message: Some("Could not read Tailscale status".to_string()),
            }
        }
    };
    let backend_state = value
        .get("BackendState")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let running = backend_state.eq_ignore_ascii_case("Running");
    let self_obj = value.get("Self");
    // The node is online when the backend is running and (if present) Self.Online
    // is true; a missing Self means the node is not signed in.
    let self_online = self_obj
        .and_then(|s| s.get("Online"))
        .and_then(|v| v.as_bool());
    let online = running && self_online.unwrap_or(self_obj.is_some());
    let dns_name = self_obj
        .and_then(|s| s.get("DNSName"))
        .and_then(|v| v.as_str())
        .map(|name| name.trim_end_matches('.').to_string())
        .filter(|name| !name.is_empty());
    let tailnet = value
        .get("CurrentTailnet")
        .and_then(|t| t.get("Name"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("MagicDNSSuffix")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        });
    let version = value
        .get("Version")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    // `Running` means the daemon holds a valid node key and is signed in. A
    // MagicDNS name is a *separate* requirement for the Serve backup
    // (`ready()`), so it must not be conflated with being signed in: a
    // signed-in user whose node lacks a MagicDNS name would otherwise be told
    // to sign in.
    let authenticated = running;

    TailscaleStatus {
        installed: true,
        exe: None,
        backend_state,
        authenticated,
        online,
        dns_name,
        tailnet,
        version,
        message: None,
    }
}

/// Parse `tailscale serve status --json` tolerantly across CLI versions.
///
/// The schema has changed shape over Tailscale releases, so instead of binding
/// to one form we walk the document: TCP keys are ports, `Web` keys carry the
/// HTTPS port and their `Proxy` strings are handler targets, and `AllowFunnel`
/// marks publicly-exposed ports.
pub fn parse_serve_status(json: &str) -> ServeStatus {
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(value) => value,
        Err(_) => return ServeStatus::unconfigured(),
    };

    let mut status = ServeStatus::default();

    if let Some(tcp) = value.get("TCP").and_then(|v| v.as_object()) {
        for key in tcp.keys() {
            if let Ok(port) = key.parse::<u16>() {
                status.https_ports.push(port);
            }
        }
    }
    status.https_ports.sort_unstable();
    status.https_ports.dedup();

    // Scope each proxy to the HTTPS port named by its `Web` key so unrelated
    // ports can never be mistaken for, or block, ours.
    let mut web_found = false;
    if let Some(web) = value.get("Web").and_then(|v| v.as_object()) {
        for (key, subtree) in web {
            web_found = true;
            let port = port_from_host_key(key);
            let mut targets = Vec::new();
            collect_targets(subtree, &mut targets);
            for target in targets {
                status.proxies.push(ServeProxy { target, port });
            }
        }
    }
    if !web_found {
        // A schema we do not recognize: collect targets without a port so we
        // never claim ownership of them.
        let mut targets = Vec::new();
        collect_targets(&value, &mut targets);
        for target in targets {
            status.proxies.push(ServeProxy { target, port: None });
        }
    }
    status
        .proxies
        .sort_by(|a, b| (a.port, &a.target).cmp(&(b.port, &b.target)));
    status.proxies.dedup_by(|a, b| a.port == b.port && a.target == b.target);

    if let Some(funnel) = value.get("AllowFunnel").and_then(|v| v.as_object()) {
        // `AllowFunnel` is keyed by the node's `host:port` (Tailscale's
        // `HostPort` type), not by a bare port, so parse the right-hand port.
        for (key, enabled) in funnel {
            if enabled.as_bool().unwrap_or(false) {
                if let Some(port) = port_from_host_key(key) {
                    status.funnel_ports.push(port);
                }
            }
        }
    }
    status.funnel_ports.sort_unstable();
    status.funnel_ports.dedup();

    status.configured = !status.proxies.is_empty() || !status.https_ports.is_empty();
    status
}

/// Extract the HTTPS port from a serve-status key. Handles the modern
/// (`host:443`), legacy (`https://host:443`), and bare (`443`) shapes that
/// different CLI versions use for `Web` and `AllowFunnel`. `None` when no port
/// is present.
fn port_from_host_key(key: &str) -> Option<u16> {
    let key = key.trim();
    if let Ok(port) = key.parse::<u16>() {
        return Some(port);
    }
    let key = key
        .strip_prefix("https://")
        .or_else(|| key.strip_prefix("http://"))
        .unwrap_or(key);
    let key = key.split('/').next().unwrap_or(key);
    let (_, port) = key.rsplit_once(':')?;
    port.parse::<u16>().ok()
}

/// Recursively collect every `"Proxy": "..."` string in a Serve status subtree.
fn collect_targets(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if key == "Proxy" {
                    if let Some(target) = child.as_str() {
                        if !target.is_empty() {
                            out.push(target.to_string());
                        }
                    }
                }
                collect_targets(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_targets(item, out);
            }
        }
        _ => {}
    }
}

/// Platform-appropriate candidate locations for the `tailscale` executable.
///
/// On macOS the CLI lives inside the app bundle rather than on `PATH` by
/// default. Windows installs under `Program Files`; Linux packages install into
/// `/usr/bin` or `/usr/local/bin`. The login `PATH` is searched last so Homebrew
/// and custom installs are found too.
pub fn candidate_paths() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale"));
        candidates.push(PathBuf::from("/usr/local/bin/tailscale"));
        candidates.push(PathBuf::from("/opt/homebrew/bin/tailscale"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            candidates.push(PathBuf::from(program_files).join("Tailscale\\tailscale.exe"));
        }
        if let Ok(program_files) = std::env::var("ProgramFiles(x86)") {
            candidates.push(PathBuf::from(program_files).join("Tailscale\\tailscale.exe"));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            candidates.push(PathBuf::from(local).join("Tailscale\\tailscale.exe"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/usr/bin/tailscale"));
        candidates.push(PathBuf::from("/usr/local/bin/tailscale"));
        candidates.push(PathBuf::from("/snap/bin/tailscale"));
    }

    // The login PATH is appended for every platform so custom installs work.
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let exe = if cfg!(windows) {
                dir.join("tailscale.exe")
            } else {
                dir.join("tailscale")
            };
            candidates.push(exe);
        }
    }

    candidates.dedup();
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    /// A runner that returns canned responses keyed by the joined argument
    /// string and records every invocation. Multiple responses for the same
    /// command are consumed in order; a single response repeats.
    #[derive(Default)]
    struct FakeRunner {
        responses: Mutex<HashMap<String, VecDeque<CommandOutput>>>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeRunner {
        fn new() -> Self {
            FakeRunner::default()
        }

        fn on(self, args: &str, output: CommandOutput) -> Self {
            self.responses
                .lock()
                .unwrap()
                .entry(args.to_string())
                .or_default()
                .push_back(output);
            self
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, _exe: &Path, args: &[String]) -> Result<CommandOutput, String> {
            let joined = args.join(" ");
            self.calls.lock().unwrap().push(joined.clone());
            let mut responses = self.responses.lock().unwrap();
            match responses.get_mut(&joined) {
                Some(queue) if queue.len() > 1 => {
                    Ok(queue.pop_front().expect("length checked above"))
                }
                Some(queue) => Ok(queue.front().cloned().expect("non-empty queue")),
                None => Ok(CommandOutput::err(format!("unexpected call: {joined}"))),
            }
        }
    }

    fn cli(runner: Arc<FakeRunner>) -> TailscaleCli {
        TailscaleCli::with_runner("/fake/tailscale", runner)
    }

    const STATUS_RUNNING: &str = r#"{
        "Version": "1.58.2",
        "BackendState": "Running",
        "Self": {
            "DNSName": "macbook-pro.berylline-goblin.ts.net.",
            "Online": true,
            "HostName": "macbook-pro"
        },
        "CurrentTailnet": { "Name": "berylline-goblin.ts.net" }
    }"#;

    #[test]
    fn detect_probes_candidates_in_order_and_returns_the_first_runnable() {
        /// Succeeds only for one path, so the search order is observable.
        struct ProbeRunner(PathBuf);
        impl CommandRunner for ProbeRunner {
            fn run(&self, exe: &Path, args: &[String]) -> Result<CommandOutput, String> {
                if exe == self.0 && args == ["version"] {
                    Ok(CommandOutput::ok("1.58.2"))
                } else {
                    Ok(CommandOutput::err("not runnable"))
                }
            }
        }
        let runner = Arc::new(ProbeRunner(PathBuf::from("/found/tailscale")));
        let found = TailscaleCli::detect_with(
            runner,
            vec![PathBuf::from("/missing/tailscale"), PathBuf::from("/found/tailscale")],
        );
        let found = found.expect("a runnable candidate is detected");
        assert_eq!(found.exe(), Path::new("/found/tailscale"));
    }

    #[test]
    fn detect_returns_none_when_no_candidate_runs() {
        let runner = Arc::new(FakeRunner::new());
        assert!(TailscaleCli::detect_with(runner, vec![PathBuf::from("/missing")]).is_none());
    }

    #[test]
    fn running_status_yields_a_ready_tailnet_name_without_a_trailing_dot() {
        let runner = Arc::new(
            FakeRunner::new().on("status --json", CommandOutput::ok(STATUS_RUNNING)),
        );
        let status = cli(runner).status();
        assert!(status.installed);
        assert!(status.running());
        assert!(status.authenticated);
        assert!(status.online);
        assert_eq!(
            status.dns_name.as_deref(),
            Some("macbook-pro.berylline-goblin.ts.net")
        );
        assert_eq!(status.tailnet.as_deref(), Some("berylline-goblin.ts.net"));
        assert!(status.ready());
    }

    #[test]
    fn needs_login_status_is_not_ready_and_carries_actionable_context() {
        let json = r#"{"BackendState":"NeedsLogin","Self":null}"#;
        let runner = Arc::new(FakeRunner::new().on("status --json", CommandOutput::ok(json)));
        let status = cli(runner).status();
        assert!(!status.running());
        assert!(!status.authenticated);
        assert!(!status.ready());
        assert_eq!(status.backend_state, "NeedsLogin");
    }

    #[test]
    fn a_running_node_without_a_magicdns_name_is_signed_in_but_not_ready() {
        // A signed-in node whose MagicDNS name is unavailable must not be
        // reported as signed out: the Settings "sign in" hint keys off
        // `authenticated`, while `ready()` gates the Serve backup.
        let json = r#"{"BackendState":"Running","Self":{"Online":true}}"#;
        let status = parse_status(json);
        assert!(status.running());
        assert!(status.authenticated);
        assert!(!status.ready());
        assert!(status.dns_name.is_none());
    }

    #[test]
    fn status_parse_survives_malformed_json() {
        let status = parse_status("not json");
        assert!(!status.ready());
        assert!(status.message.is_some());
    }

    #[test]
    fn a_missing_or_empty_term_is_filled_so_the_macos_cli_takes_the_cli_path() {
        assert_eq!(term_override(None), Some("dumb"));
        assert_eq!(term_override(Some("")), Some("dumb"));
        assert_eq!(term_override(Some("xterm-256color")), None);
    }

    #[test]
    fn a_non_json_status_quotes_the_cli_message_instead_of_a_generic_error() {
        // The macOS Tailscale binary prints this (exit 0) when it wrongly takes
        // the GUI path instead of the CLI path.
        let runner = Arc::new(FakeRunner::new().on(
            "status --json",
            CommandOutput::ok("The Tailscale GUI failed to start: nope\n"),
        ));
        let status = cli(runner).status();
        assert!(status.installed);
        assert!(!status.ready());
        let message = status.message.expect("a message is reported");
        assert!(
            message.contains("The Tailscale GUI failed to start"),
            "message should quote the CLI: {message}"
        );
    }

    #[test]
    fn serve_status_parses_the_modern_nested_shape() {
        let json = r#"{
            "TCP": { "443": { "HTTPS": true } },
            "Web": {
                "macbook-pro.berylline-goblin.ts.net:443": {
                    "Handlers": { "/": { "Proxy": "http://127.0.0.1:51234" } }
                }
            },
            "AllowFunnel": {}
        }"#;
        let status = parse_serve_status(json);
        assert!(status.configured);
        assert_eq!(status.https_ports, vec![443]);
        assert_eq!(status.targets(), vec!["http://127.0.0.1:51234".to_string()]);
        assert!(status.funnel_ports.is_empty());
    }

    #[test]
    fn serve_status_parses_the_legacy_flat_shape() {
        let json = r#"{
            "TCP": { "443": { "HTTPS": true } },
            "Web": {
                "https://host.tailnet.ts.net:443": {
                    "/": { "Proxy": "http://127.0.0.1:9000" }
                }
            }
        }"#;
        let status = parse_serve_status(json);
        assert_eq!(status.targets(), vec!["http://127.0.0.1:9000".to_string()]);
    }

    #[test]
    fn empty_serve_status_is_unconfigured() {
        assert!(!parse_serve_status("{}").configured);
        assert!(!parse_serve_status("").configured);
    }

    #[test]
    fn ownership_accepts_free_and_self_owned_ports_and_rejects_conflicts() {
        let runner = Arc::new(FakeRunner::new());
        let cli = cli(runner);
        assert!(cli.owns_https_port(&ServeStatus::unconfigured(), 443, 51234));

        let ours = parse_serve_status(
            r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"h:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:51234"}}}}}"#,
        );
        assert!(cli.owns_https_port(&ours, 443, 51234));

        let theirs = parse_serve_status(
            r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"h:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8080"}}}}}"#,
        );
        assert!(!cli.owns_https_port(&theirs, 443, 51234));
    }

    #[test]
    fn a_funneled_port_is_never_considered_owned() {
        let runner = Arc::new(FakeRunner::new());
        let cli = cli(runner);
        // `AllowFunnel` is keyed by `host:port`, matching real CLI output.
        let funneled = parse_serve_status(
            r#"{"TCP":{"443":{"HTTPS":true}},"AllowFunnel":{"h.tailnet.ts.net:443":true},"Web":{"h:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:51234"}}}}}"#,
        );
        assert!(funneled.is_funneled(443));
        assert!(!cli.owns_https_port(&funneled, 443, 51234));

        // Older/alternative shapes may key by the bare port; still detected.
        let bare = parse_serve_status(
            r#"{"TCP":{"443":{"HTTPS":true}},"AllowFunnel":{"443":true}}"#,
        );
        assert!(bare.is_funneled(443));
    }

    #[test]
    fn unrelated_serve_config_on_another_port_never_blocks_a_free_port() {
        let runner = Arc::new(FakeRunner::new());
        let cli = cli(runner);
        let serve = parse_serve_status(
            r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"h:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:9000"}}}}}"#,
        );
        // 443 is free, so Solayge may create it; 8443 belongs to someone else.
        assert!(cli.owns_https_port(&serve, 443, 51234));
        assert!(!cli.owns_https_port(&serve, 8443, 51234));
        assert_eq!(serve.targets_for_port(8443), vec!["http://127.0.0.1:9000".to_string()]);
    }

    #[test]
    fn a_second_unrelated_handler_on_our_port_blocks_ownership() {
        let runner = Arc::new(FakeRunner::new());
        let cli = cli(runner);
        let serve = parse_serve_status(
            r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"h:443":{"Handlers":{
                "/":{"Proxy":"http://127.0.0.1:51234"},
                "/admin":{"Proxy":"http://127.0.0.1:8080"}
            }}}}"#,
        );
        assert!(!cli.owns_https_port(&serve, 443, 51234));
    }

    #[test]
    fn serve_start_refuses_a_conflicting_port_without_touching_it() {
        let conflicting = r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"h:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8080"}}}}}"#;
        let runner = Arc::new(
            FakeRunner::new().on("serve status --json", CommandOutput::ok(conflicting)),
        );
        let error = cli(runner.clone()).serve_start(51234, 443).unwrap_err();
        assert!(matches!(error, TailscaleError::ServeConflict { https_port: 443, .. }));
        assert!(
            !runner.calls().iter().any(|c| c.starts_with("serve --bg")),
            "a conflicting port must not be modified"
        );
    }

    #[test]
    fn serve_start_applies_the_loopback_proxy_and_verifies_it() {
        let after = r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"h:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:51234"}}}}}"#;
        let runner = Arc::new(
            FakeRunner::new()
                .on("serve status --json", CommandOutput::ok("{}"))
                .on("serve status --json", CommandOutput::ok(after))
                .on(
                    "serve --bg --https=443 http://127.0.0.1:51234",
                    CommandOutput::ok(""),
                ),
        );
        let status = cli(runner.clone()).serve_start(51234, 443).unwrap();
        assert_eq!(status.targets(), vec!["http://127.0.0.1:51234".to_string()]);
        let calls = runner.calls();
        assert!(calls
            .iter()
            .any(|c| c == "serve --bg --https=443 http://127.0.0.1:51234"));
        assert!(!calls.iter().any(|c| c.contains("funnel")));
    }

    #[test]
    fn serve_start_reports_unverified_when_the_proxy_did_not_take_effect() {
        let runner = Arc::new(
            FakeRunner::new()
                .on("serve status --json", CommandOutput::ok("{}"))
                .on(
                    "serve --bg --https=443 http://127.0.0.1:51234",
                    CommandOutput::ok(""),
                ),
        );
        // Every subsequent status read still returns "{}".
        let error = cli(runner).serve_start(51234, 443).unwrap_err();
        assert!(matches!(error, TailscaleError::Unverified { https_port: 443 }));
    }

    #[test]
    fn serve_stop_removes_only_our_own_handler() {
        let ours = r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"h:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:51234"}}}}}"#;
        let runner = Arc::new(
            FakeRunner::new()
                .on("serve status --json", CommandOutput::ok(ours))
                .on("serve --https=443 off", CommandOutput::ok("")),
        );
        let outcome = cli(runner.clone()).serve_stop(51234, 443).unwrap();
        assert_eq!(outcome, ServeStopOutcome::Removed);
        assert!(runner.calls().iter().any(|c| c == "serve --https=443 off"));
    }

    #[test]
    fn serve_stop_leaves_a_port_that_now_belongs_to_someone_else() {
        let theirs = r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"h:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8080"}}}}}"#;
        let runner = Arc::new(
            FakeRunner::new().on("serve status --json", CommandOutput::ok(theirs)),
        );
        let outcome = cli(runner.clone()).serve_stop(51234, 443).unwrap();
        assert_eq!(outcome, ServeStopOutcome::LeftUntouched);
        assert!(!runner.calls().iter().any(|c| c.contains(" off")));
    }

    #[test]
    fn loopback_target_is_always_127_0_0_1() {
        assert_eq!(
            TailscaleCli::loopback_target(51234),
            "http://127.0.0.1:51234"
        );
    }
}
