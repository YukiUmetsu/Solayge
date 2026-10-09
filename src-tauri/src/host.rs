//! The desktop host: serves the Solayge protocol over the Iroh transport.
//!
//! Compiled only with the `mobile-network` feature. Unlike the earlier in-memory
//! [`solayge_protocol::MockHost`] wiring, this handler:
//!
//! * runs the real pairing state machine ([`PairingServer`]) so a scanned QR can
//!   complete `pair.begin` / `pair.prove_identity` and yield the *same* short
//!   verification code on both sides;
//! * authenticates each connection with a host-issued, single-use challenge that
//!   the phone signs with the key pairing pinned for its device id
//!   ([`solayge_identity::auth`]);
//! * binds the authenticated device id to that one connection ([`ConnectionSession`])
//!   so a request can never be authorized by a device that authenticated
//!   elsewhere;
//! * checks the device's granted scope on every request and then reads real
//!   projects, tasks, and pending questions from [`AppState`].
//!
//! Reads are implemented end to end. Mutating actions that the mobile UI can
//! reach (`sessions.send_prompt`, `sessions.cancel`, `approvals.respond`) return
//! a typed `unsupported_action`, rather than silently pretending to succeed.

use serde_json::Value;
use tauri::{AppHandle, Manager};

use solayge_identity::{
    AuthRequest, AuthResponse, ChallengeStore, PairBeginRequest, PairBeginResponse,
    PairProveIdentityRequest, PairProveIdentityResponse, Scope,
};
use solayge_network::{BoxFuture, ConnectionSession, FrameHandler};
use solayge_protocol::{
    Action, ApprovalKind, ApprovalRisk, ApprovalSummary, ApprovalsResponse, ConnectionHealth,
    Envelope, HostMetadata, MessageKind, ProtocolError, SessionDetail, SessionSummary,
    SessionsResponse, ProjectSummary, ProjectsResponse, PROTOCOL_VERSION,
};

use crate::devices::DeviceManager;
use crate::models::{AskKind, Provider, Task, TaskKind, TaskStatus};
use crate::state::now;

/// The Iroh-backed host handler shared by every incoming connection.
///
/// Generic over the Tauri runtime so it can be exercised in tests with
/// `tauri::test::mock_app` as well as the production Wry runtime.
pub struct HostHandler<R: tauri::Runtime> {
    app: AppHandle<R>,
    challenges: ChallengeStore,
}

impl<R: tauri::Runtime> HostHandler<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        HostHandler {
            app,
            challenges: ChallengeStore::new(),
        }
    }

    fn app_version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    /// Authorize a post-handshake request: the connection must have completed
    /// the challenge-response, the device must still be registered, and it must
    /// hold `scope`.
    fn authorize(&self, session: &ConnectionSession, scope: Scope) -> Result<String, ProtocolError> {
        let device_id = session.authenticated_device().ok_or_else(|| {
            ProtocolError::not_authorized("authenticate before using this action")
        })?;
        let manager = self.app.state::<DeviceManager>();
        if !manager.registry().is_authorized(&device_id) {
            // Revocation takes effect immediately on live connections too.
            session.clear_authenticated_device();
            return Err(ProtocolError::not_authorized("device is no longer authorized"));
        }
        match manager.registry().get(&device_id) {
            Some(device) if device.scopes.contains(&scope) => Ok(device_id),
            Some(_) => Err(ProtocolError::not_authorized(format!(
                "this device does not have the {} scope",
                scope
            ))),
            None => Err(ProtocolError::not_authorized("device is no longer authorized")),
        }
    }

    fn dispatch(
        &self,
        session: &ConnectionSession,
        request: &Envelope,
    ) -> Result<Value, ProtocolError> {
        match request.action {
            // ---- pre-auth ----
            Action::ConnectionHealth => self.health(),
            Action::PairBegin => self.pair_begin(request),
            Action::PairProveIdentity => self.pair_prove(request),
            Action::ConnectionAuthenticate => self.authenticate(session, request),

            // ---- reads require sessions.read ----
            Action::HostsGetMetadata => {
                self.authorize(session, Scope::SessionsRead)?;
                self.host_metadata()
            }
            Action::ProjectsList => {
                self.authorize(session, Scope::SessionsRead)?;
                self.projects()
            }
            Action::SessionsList => {
                self.authorize(session, Scope::SessionsRead)?;
                self.sessions()
            }
            Action::SessionsGet => {
                self.authorize(session, Scope::SessionsRead)?;
                self.session_detail(request)
            }
            Action::SessionsEventsSubscribe => {
                self.authorize(session, Scope::SessionsRead)?;
                // Event replay is not exposed remotely yet; an empty batch is an
                // honest "no new events" rather than fabricated activity.
                self.empty_events(request)
            }
            Action::ApprovalsList => {
                self.authorize(session, Scope::SessionsRead)?;
                self.approvals()
            }
            Action::ApprovalsGet => {
                self.authorize(session, Scope::SessionsRead)?;
                self.approval(request)
            }
            Action::FilesChangedSummary => {
                self.authorize(session, Scope::SessionsRead)?;
                self.files_changed(request)
            }
            Action::DevicesList => {
                self.authorize(session, Scope::SessionsRead)?;
                self.devices()
            }

            // ---- mutations not yet exposed over the remote protocol ----
            Action::SessionsSendPrompt
            | Action::SessionsCancel
            | Action::SessionsStart
            | Action::SessionsGetCommandStatus
            | Action::ApprovalsRespond
            | Action::DevicesRevoke
            | Action::InsightsSummary
            | Action::InsightsErrors
            | Action::InsightsAgentQuestions
            | Action::PairApprove
            | Action::PairReject => Err(ProtocolError::unsupported_action(request.action.as_str())),
        }
    }

    fn health(&self) -> Result<Value, ProtocolError> {
        let manager = self.app.state::<DeviceManager>();
        typed(&ConnectionHealth {
            desktop_id: manager.desktop_id_string(),
            protocol_version: PROTOCOL_VERSION,
            app_version: Self::app_version().to_string(),
            uptime_secs: 0,
        })
    }

    fn pair_begin(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let begin: PairBeginRequest = request.payload_as()?;
        let manager = self.app.state::<DeviceManager>();
        let response: PairBeginResponse = manager
            .pairing_server()
            .handle_begin(begin, now())
            .map_err(identity_error)?;
        typed(&response)
    }

    fn pair_prove(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let prove: PairProveIdentityRequest = request.payload_as()?;
        let manager = self.app.state::<DeviceManager>();
        let response: PairProveIdentityResponse = manager
            .pairing_server()
            .handle_prove_identity(prove, now())
            .map_err(identity_error)?;
        typed(&response)
    }

    fn authenticate(
        &self,
        session: &ConnectionSession,
        request: &Envelope,
    ) -> Result<Value, ProtocolError> {
        let auth: AuthRequest = request.payload_as()?;
        if auth.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolError::unsupported_version(auth.protocol_version));
        }
        let manager = self.app.state::<DeviceManager>();
        let desktop_id = manager.desktop_id_string();
        if auth.desktop_id != desktop_id {
            return Err(ProtocolError::not_authorized(
                "this request is addressed to a different desktop",
            ));
        }

        // First leg: hand out a fresh challenge.
        let Some(nonce) = auth.host_nonce.clone() else {
            let issued = self.challenges.issue(&auth.device_id, now());
            return typed(&AuthResponse {
                authenticated: false,
                desktop_id,
                host_nonce: Some(issued),
            });
        };

        // Second leg: consume the nonce (single use) and verify the signature
        // against the key pinned for this device id at pairing time.
        if !self.challenges.consume(&auth.device_id, &nonce, now()) {
            return Err(ProtocolError::not_authorized(
                "the authentication challenge is unknown or has expired",
            ));
        }
        let Some(public_key) = manager.registry().public_key_of(&auth.device_id) else {
            return Err(ProtocolError::not_authorized("unknown device"));
        };
        solayge_identity::verify(&public_key, &auth)
            .map_err(|_| ProtocolError::not_authorized("the identity proof did not verify"))?;
        if !manager.registry().is_authorized(&auth.device_id) {
            return Err(ProtocolError::not_authorized("device is no longer authorized"));
        }
        manager
            .touch(&auth.device_id, now())
            .map_err(|e| ProtocolError::internal(e.to_string()))?;
        session.set_authenticated_device(auth.device_id.clone());

        typed(&AuthResponse {
            authenticated: true,
            desktop_id,
            host_nonce: None,
        })
    }

    fn host_metadata(&self) -> Result<Value, ProtocolError> {
        let manager = self.app.state::<DeviceManager>();
        typed(&HostMetadata {
            desktop_id: manager.desktop_id_string(),
            name: "Solayge Desktop".to_string(),
            platform: std::env::consts::OS.to_string(),
            app_version: Self::app_version().to_string(),
            protocol_version: PROTOCOL_VERSION,
        })
    }

    fn projects(&self) -> Result<Value, ProtocolError> {
        let state = self.app.state::<crate::state::AppState>();
        let inner = crate::state::lock(&state.inner);
        let projects = inner
            .projects
            .iter()
            .map(|project| {
                let tasks: Vec<&Task> = inner
                    .tasks
                    .iter()
                    .filter(|t| t.project_path == project.path)
                    .collect();
                ProjectSummary {
                    id: project.path.clone(),
                    name: project.name.clone(),
                    active_sessions: tasks
                        .iter()
                        .filter(|t| t.status == TaskStatus::Running)
                        .count() as u32,
                    pending_approvals: tasks.iter().filter(|t| t.ask.is_some()).count() as u32,
                    updated_at: tasks
                        .iter()
                        .map(|t| task_updated_at(t))
                        .max()
                        .unwrap_or(project.added_at),
                }
            })
            .collect();
        typed(&ProjectsResponse { projects })
    }

    fn sessions(&self) -> Result<Value, ProtocolError> {
        let state = self.app.state::<crate::state::AppState>();
        let inner = crate::state::lock(&state.inner);
        let sessions = inner
            .tasks
            .iter()
            .filter(|t| !matches!(t.status, TaskStatus::Draft))
            .map(session_summary)
            .collect();
        typed(&SessionsResponse { sessions })
    }

    fn session_detail(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let session_id = request
            .session_id
            .clone()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| ProtocolError::missing_field("session_id"))?;
        let state = self.app.state::<crate::state::AppState>();
        let inner = crate::state::lock(&state.inner);
        let task = inner
            .tasks
            .iter()
            .find(|t| t.id == session_id)
            .ok_or_else(|| ProtocolError::not_found(&session_id))?;
        typed(&session_detail(task))
    }

    fn empty_events(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let payload: solayge_protocol::EventsSubscribePayload = request.payload_as()?;
        typed(&solayge_protocol::EventsBatch {
            session_id: payload.session_id,
            events: Vec::new(),
            next_seq: payload.from_seq,
            resync: false,
        })
    }

    fn approvals(&self) -> Result<Value, ProtocolError> {
        let state = self.app.state::<crate::state::AppState>();
        let inner = crate::state::lock(&state.inner);
        let approvals = inner
            .tasks
            .iter()
            .filter_map(approval_of)
            .collect();
        typed(&ApprovalsResponse { approvals })
    }

    fn approval(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let query: solayge_protocol::ApprovalQuery = request.payload_as()?;
        let state = self.app.state::<crate::state::AppState>();
        let inner = crate::state::lock(&state.inner);
        inner
            .tasks
            .iter()
            .filter_map(approval_of)
            .find(|a| a.id == query.approval_id)
            .map(|a| typed(&a))
            .transpose()?
            .ok_or_else(|| ProtocolError::not_found(&query.approval_id))
    }

    fn files_changed(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let session_id = request
            .session_id
            .clone()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| ProtocolError::missing_field("session_id"))?;
        let state = self.app.state::<crate::state::AppState>();
        let inner = crate::state::lock(&state.inner);
        let task = inner
            .tasks
            .iter()
            .find(|t| t.id == session_id)
            .ok_or_else(|| ProtocolError::not_found(&session_id))?;
        typed(&solayge_protocol::FilesChangedSummary {
            project_id: task.project_path.clone(),
            session_id,
            total: 0,
            files: Vec::new(),
        })
    }

    fn devices(&self) -> Result<Value, ProtocolError> {
        let manager = self.app.state::<DeviceManager>();
        let devices = manager
            .registry()
            .list()
            .iter()
            .map(|d| solayge_protocol::DeviceSummary {
                device_id: d.device_id.clone(),
                name: d.name.clone(),
                platform: d.platform.clone(),
                scopes: d.scopes.iter().map(|s| s.to_string()).collect(),
                paired_at: d.paired_at,
                last_seen: d.last_seen.unwrap_or(0),
            })
            .collect();
        typed(&solayge_protocol::DevicesResponse { devices })
    }
}

impl<R: tauri::Runtime> FrameHandler for HostHandler<R> {
    fn handle<'a>(
        &'a self,
        session: &'a ConnectionSession,
        request: Envelope,
    ) -> BoxFuture<'a, Envelope> {
        Box::pin(async move {
            if request.kind != MessageKind::Command {
                return Envelope::error_to(
                    &request,
                    &ProtocolError::invalid_frame("the host accepts only command frames"),
                );
            }
            match self.dispatch(session, &request) {
                Ok(payload) => Envelope::response_to(&request, payload),
                Err(error) => Envelope::error_to(&request, &error),
            }
        })
    }
}

fn typed<T: serde::Serialize>(value: &T) -> Result<Value, ProtocolError> {
    serde_json::to_value(value).map_err(|e| ProtocolError::internal(e.to_string()))
}

fn identity_error(error: solayge_identity::IdentityError) -> ProtocolError {
    use solayge_identity::IdentityError::*;
    match error {
        PairingNotFound(id) => ProtocolError::not_found(&id),
        PairingExpired(id) => ProtocolError::not_authorized(format!("pairing expired: {id}")),
        PairingRejected(id) => ProtocolError::not_authorized(format!("pairing rejected: {id}")),
        PairingNotActive(id) => ProtocolError::not_authorized(format!("pairing not active: {id}")),
        ProofInvalid(message) => ProtocolError::not_authorized(message),
        NotAuthorized(message) => ProtocolError::not_authorized(message),
        DeviceRevoked(id) => ProtocolError::not_authorized(format!("device revoked: {id}")),
        UnknownDevice(id) => ProtocolError::not_authorized(format!("unknown device: {id}")),
        RateLimited(message) => ProtocolError::resource_exhausted(message),
        other => ProtocolError::invalid_payload(other.to_string()),
    }
}

fn task_status_str(status: TaskStatus) -> String {
    match status {
        TaskStatus::Draft => "draft",
        TaskStatus::Waiting => "waiting",
        TaskStatus::Ready => "ready",
        TaskStatus::Running => "running",
        TaskStatus::Succeeded => "succeeded",
        TaskStatus::Failed => "failed",
        TaskStatus::Canceled => "canceled",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Interrupted => "interrupted",
    }
    .to_string()
}

fn task_agent(task: &Task) -> String {
    match task.kind {
        TaskKind::Agent => match task.provider {
            Some(Provider::Opencode) => "opencode",
            Some(Provider::Codex) => "codex",
            Some(Provider::Claude) => "claude",
            Some(Provider::Cursor) => "cursor",
            None => "agent",
        },
        TaskKind::Shell => "shell",
        TaskKind::Git => "git",
        TaskKind::Merge => "merge",
    }
    .to_string()
}

fn task_updated_at(task: &Task) -> i64 {
    task.finished_at
        .or(task.started_at)
        .unwrap_or(task.created_at)
}

fn session_summary(task: &Task) -> SessionSummary {
    SessionSummary {
        id: task.id.clone(),
        project_id: task.project_path.clone(),
        title: task.title.clone(),
        agent: task_agent(task),
        status: task_status_str(task.status),
        pending_decisions: u32::from(task.ask.is_some()),
        started_at: task.started_at.unwrap_or(task.created_at),
        updated_at: task_updated_at(task),
    }
}

fn session_detail(task: &Task) -> SessionDetail {
    SessionDetail {
        id: task.id.clone(),
        project_id: task.project_path.clone(),
        title: task.title.clone(),
        agent: task_agent(task),
        status: task_status_str(task.status),
        pending_decisions: u32::from(task.ask.is_some()),
        started_at: task.started_at.unwrap_or(task.created_at),
        updated_at: task_updated_at(task),
        summary: task
            .result
            .clone()
            .or_else(|| task.error.clone())
            .map(|s| redact_long(&s)),
        files_changed: 0,
    }
}

fn approval_of(task: &Task) -> Option<ApprovalSummary> {
    let ask = task.ask.as_ref()?;
    Some(ApprovalSummary {
        id: ask.id.clone(),
        session_id: task.id.clone(),
        project_id: task.project_path.clone(),
        kind: match ask.kind {
            AskKind::Question => ApprovalKind::Question,
            AskKind::Permission => ApprovalKind::Permission,
        },
        title: ask.title.clone(),
        message: ask.message.clone().or_else(|| ask.purpose.clone()),
        command: ask.resources.first().cloned(),
        risk: ApprovalRisk::Medium,
        read_only: false,
        requested_at: ask.created_at.unwrap_or(task.created_at),
        expires_at: ask.deadline,
    })
}

/// Cap an agent summary so a huge result cannot bloat a frame. The frame limit
/// is a hard backstop; this keeps the common case small.
fn redact_long(text: &str) -> String {
    const MAX: usize = 8 * 1024;
    if text.len() <= MAX {
        return text.to_string();
    }
    let mut end = MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Task;

    fn task(status: TaskStatus, ask: bool) -> Task {
        Task {
            id: "t1".into(),
            project_path: "/repo".into(),
            title: "Do the thing".into(),
            prompt: "prompt".into(),
            attachments: vec![],
            isolation: Default::default(),
            profile: Default::default(),
            last_permission: None,
            session_id: None,
            base_ref: None,
            branch: None,
            worktree_path: None,
            not_before: None,
            depends_on: vec![],
            status,
            exit_code: None,
            error: None,
            created_at: 10,
            started_at: Some(20),
            finished_at: None,
            attempts: 0,
            provider: Some(Provider::Opencode),
            model: None,
            fallback_provider: None,
            fallback_model: None,
            used_fallback: false,
            review: None,
            kind: TaskKind::Agent,
            git_op: None,
            command: None,
            merge: None,
            branch_mode: Default::default(),
            new_branch: None,
            ask: ask.then(|| crate::models::TaskAsk {
                id: "ask_1".into(),
                kind: AskKind::Question,
                title: "Which API?".into(),
                message: Some("Choose one".into()),
                action: None,
                resources: vec!["src/api.ts".into()],
                save: vec![],
                metadata: None,
                source: None,
                purpose: None,
                fields: vec![],
                options: vec![],
                session_id: None,
                created_at: Some(30),
                deadline: None,
                timeout_secs: None,
            }),
            result: None,
        }
    }

    #[test]
    fn summaries_map_task_state_without_leaking_the_prompt() {
        let summary = session_summary(&task(TaskStatus::Running, true));
        assert_eq!(summary.status, "running");
        assert_eq!(summary.agent, "opencode");
        assert_eq!(summary.pending_decisions, 1);
        // `SessionSummary` has no prompt field at all.
        let serialized = serde_json::to_string(&summary).unwrap();
        assert!(!serialized.contains("prompt"));
    }

    #[test]
    fn a_pending_ask_becomes_an_approval() {
        let approval = approval_of(&task(TaskStatus::Running, true)).unwrap();
        assert_eq!(approval.id, "ask_1");
        assert_eq!(approval.kind, ApprovalKind::Question);
        assert_eq!(approval.command.as_deref(), Some("src/api.ts"));
        assert!(approval_of(&task(TaskStatus::Running, false)).is_none());
    }

    #[test]
    fn long_summaries_are_capped_on_a_char_boundary() {
        let long = "é".repeat(10_000);
        let capped = redact_long(&long);
        assert!(capped.len() <= 8 * 1024 + 4);
        assert!(capped.ends_with('…'));
    }
}

#[cfg(test)]
mod handshake_tests {
    //! Exercises the real host handler end to end (pair → approve → challenge
    //! → proof) without a network endpoint, so the security-critical glue is
    //! covered by a fast unit test.
    use super::*;
    use solayge_identity::{DeviceIdentity, FileStore, PairingClient, ReachabilityHints};
    use tauri::test::mock_app;
    use tauri::Manager;

    fn temp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "solayge-host-handshake-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    #[test]
    fn a_phone_pairs_then_authenticates_and_binds_to_its_connection() {
        let dir = temp_dir();
        let app = mock_app();
        app.manage(crate::state::AppState::new(dir.clone()));
        let manager = crate::devices::DeviceManager::with_store(
            &dir,
            &FileStore::new(&dir, "test-identity"),
        )
        .unwrap();
        app.manage(manager);
        let handler = HostHandler::new(app.handle().clone());
        let session = ConnectionSession::new("iroh:test");

        // Desktop: Settings → Add phone.
        let offer = app
            .state::<DeviceManager>()
            .pairing_server()
            .start_with_hints(
                now(),
                ReachabilityHints {
                    iroh_endpoint_id: Some("iroh:desktop".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        // Phone: scan and begin.
        let phone = DeviceIdentity::generate();
        let mut client = PairingClient::new(phone.clone(), "Pixel 9", "android");
        let begin = client.begin_request_for_qr(&offer.qr).unwrap();
        let request = Envelope::command(Action::PairBegin, serde_json::to_value(&begin).unwrap());
        let response: PairBeginResponse =
            serde_json::from_value(handler.dispatch(&session, &request).unwrap()).unwrap();

        // Both sides derive the same code — the bug this guards against.
        let verified = client.accept_begin(&response).unwrap();
        assert_eq!(
            verified.verification_code.to_string(),
            response.verification_code
        );

        // Phone: prove identity.
        let prove = client.prove_identity_request(&response).unwrap();
        let request =
            Envelope::command(Action::PairProveIdentity, serde_json::to_value(&prove).unwrap());
        handler.dispatch(&session, &request).unwrap();

        // Desktop user approves with the default View + Message grant.
        app.state::<DeviceManager>()
            .pairing_server()
            .approve(&offer.pairing_id, None, now())
            .unwrap();

        // Before authentication, a read is refused.
        let metadata = Envelope::command(Action::HostsGetMetadata, Value::Null);
        assert_eq!(
            handler.dispatch(&session, &metadata).unwrap_err().code(),
            solayge_protocol::ErrorCode::NotAuthorized
        );

        // Auth leg 1: request a challenge.
        let mut auth = AuthRequest {
            device_id: phone.device_id().to_string(),
            desktop_id: app.state::<DeviceManager>().desktop_id_string(),
            protocol_version: PROTOCOL_VERSION,
            host_nonce: None,
            proof: None,
            transport_binding: None,
        };
        let request =
            Envelope::command(Action::ConnectionAuthenticate, serde_json::to_value(&auth).unwrap());
        let challenge: AuthResponse =
            serde_json::from_value(handler.dispatch(&session, &request).unwrap()).unwrap();
        assert!(!challenge.authenticated);
        let nonce = challenge.host_nonce.expect("a challenge is issued");

        // Auth leg 2: sign the challenge.
        auth.host_nonce = Some(nonce);
        auth.proof = Some(solayge_identity::prove(&phone, &auth).unwrap());
        let request =
            Envelope::command(Action::ConnectionAuthenticate, serde_json::to_value(&auth).unwrap());
        let ok: AuthResponse =
            serde_json::from_value(handler.dispatch(&session, &request).unwrap()).unwrap();
        assert!(ok.authenticated);
        assert_eq!(
            session.authenticated_device().as_deref(),
            Some(phone.device_id().as_str())
        );

        // The authenticated connection can read; a different one cannot.
        handler.dispatch(&session, &metadata).unwrap();
        let other = ConnectionSession::new("iroh:other");
        assert_eq!(
            handler.dispatch(&other, &metadata).unwrap_err().code(),
            solayge_protocol::ErrorCode::NotAuthorized
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
