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
//! Reads are implemented end to end. The mobile UI's writes
//! (`sessions.send_prompt`, `sessions.answer`, `sessions.cancel`,
//! `approvals.respond`, `push.register`/`push.unregister`) are implemented for
//! a device that holds the matching scope; every other mutating action returns a
//! typed `unsupported_action` rather than silently pretending to succeed.

use serde_json::Value;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

use solayge_identity::{
    AuthRequest, AuthResponse, ChallengeStore, PairBeginRequest, PairBeginResponse,
    PairProveIdentityRequest, PairProveIdentityResponse, Scope,
};
use solayge_network::{BoxFuture, ConnectionSession, FrameHandler};
use solayge_protocol::{
    Action, AnswerPayload, ApprovalField, ApprovalFieldKind, ApprovalFieldOption, ApprovalKind,
    ApprovalResponsePayload, ApprovalRisk, ApprovalSummary, ApprovalsResponse, CommandLedger,
    CommandObservation, CommandReceipt, CommandStatus, ConnectionHealth, Envelope, HostMetadata,
    MessageKind, ProtocolError, PushRegisterPayload, PushRegisterResult, PushUnregisterPayload,
    SendPromptPayload, SessionDetail, SessionSummary,
    SessionsResponse, ProjectSummary, ProjectsResponse, PROTOCOL_VERSION,
};

use crate::devices::DeviceManager;
use crate::models::{AskField, AskFieldKind, AskKind, Provider, Task, TaskKind, TaskStatus};
use crate::state::now;

/// The Iroh-backed host handler shared by every incoming connection.
///
/// Generic over the Tauri runtime so it can be exercised in tests with
/// `tauri::test::mock_app` as well as the production Wry runtime.
pub struct HostHandler<R: tauri::Runtime> {
    app: AppHandle<R>,
    challenges: ChallengeStore,
    /// Host-side idempotency for remote commands, so a retried frame never runs
    /// an agent twice. Shared across connections for the process lifetime.
    commands: Mutex<CommandLedger>,
}

impl<R: tauri::Runtime> HostHandler<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        HostHandler {
            app,
            challenges: ChallengeStore::new(),
            commands: Mutex::new(CommandLedger::default()),
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

    async fn dispatch(
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

            // ---- push token registration (any authenticated paired device) ----
            Action::PushRegister => {
                let device_id = self.authorize(session, Scope::SessionsRead)?;
                self.push_register(&device_id, request)
            }
            Action::PushUnregister => {
                let device_id = self.authorize(session, Scope::SessionsRead)?;
                self.push_unregister(&device_id, request)
            }

            // ---- writes ----
            Action::SessionsSendPrompt => {
                self.authorize(session, Scope::SessionsMessage)?;
                self.send_prompt(request).await
            }
            Action::SessionsAnswer => {
                self.authorize(session, Scope::SessionsMessage)?;
                self.answer_question(request).await
            }
            Action::SessionsCancel => {
                self.authorize(session, Scope::SessionsControl)?;
                self.cancel_session(request).await
            }
            Action::ApprovalsRespond => {
                self.authorize(session, Scope::ApprovalsRespond)?;
                self.respond_approval(request).await
            }

            // ---- recognized but not implemented in this phase ----
            Action::SessionsStart
            | Action::SessionsGetCommandStatus
            | Action::DevicesRevoke
            | Action::InsightsSummary
            | Action::InsightsErrors
            | Action::InsightsAgentQuestions
            | Action::PairApprove
            | Action::PairReject => Err(ProtocolError::unsupported_action(request.action.as_str())),
        }
    }

    /// Register a mutating command id, returning the recorded status when the
    /// exact command has already been seen (a harmless retry).
    fn register_command(
        &self,
        command_id: &str,
        action: Action,
    ) -> Result<Option<CommandStatus>, ProtocolError> {
        let mut ledger = self
            .commands
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match ledger.register(command_id, action)? {
            CommandObservation::New => Ok(None),
            CommandObservation::Duplicate { status } => Ok(Some(status)),
        }
    }

    /// Record a command's terminal status. A failed advance is not fatal: the
    /// command still ran, and the ledger is only an idempotency aid.
    fn finish_command(&self, command_id: &str, status: CommandStatus) {
        let mut ledger = self
            .commands
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = ledger.advance(command_id, status);
    }

    /// Drop a command id when the command failed before doing anything, so a
    /// retry is retried rather than mistaken for a successful duplicate.
    fn forget_command(&self, command_id: &str) {
        let mut ledger = self
            .commands
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ledger.forget(command_id);
    }

    async fn send_prompt(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let payload: SendPromptPayload = request.payload_as()?;
        let session_id = require_session_id(request)?;
        let command_id = require_command_id(request)?;
        if let Some(status) = self.register_command(&command_id, Action::SessionsSendPrompt)? {
            return typed(&CommandReceipt {
                command_id,
                status,
                session_id: Some(session_id),
            });
        }
        // A remote message only reaches a task with a live agent session.
        let live_session = {
            let state = self.app.state::<crate::state::AppState>();
            let inner = crate::state::lock(&state.inner);
            let task = inner
                .tasks
                .iter()
                .find(|t| t.id == session_id)
                .ok_or_else(|| ProtocolError::not_found(&session_id))?;
            if task.status != TaskStatus::Running {
                None
            } else {
                task.session_id.clone().filter(|id| !id.is_empty())
            }
        };
        let Some(live_session) = live_session else {
            self.forget_command(&command_id);
            return Err(ProtocolError::invalid_payload(
                "this task is not running, so it cannot accept a message",
            ));
        };
        let connection = match crate::opencode_server::ensure_server().await {
            Ok(connection) => connection,
            Err(message) => {
                self.forget_command(&command_id);
                return Err(command_error(message));
            }
        };
        match crate::opencode_server::prompt(&connection, &live_session, &payload.text).await {
            Ok(()) => {
                self.finish_command(&command_id, CommandStatus::Accepted);
                typed(&CommandReceipt {
                    command_id,
                    status: CommandStatus::Accepted,
                    session_id: Some(session_id),
                })
            }
            Err(message) => {
                self.forget_command(&command_id);
                Err(command_error(message))
            }
        }
    }

    async fn answer_question(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let payload: AnswerPayload = request.payload_as()?;
        let session_id = require_session_id(request)?;
        let command_id = require_command_id(request)?;
        if let Some(status) = self.register_command(&command_id, Action::SessionsAnswer)? {
            return typed(&CommandReceipt {
                command_id,
                status,
                session_id: Some(session_id),
            });
        }
        let answer = Value::Object(payload.answers);
        match self
            .reply_to_ask(&payload.approval_id, &session_id, answer)
            .await
        {
            Ok(()) => {
                self.finish_command(&command_id, CommandStatus::Succeeded);
                typed(&CommandReceipt {
                    command_id,
                    status: CommandStatus::Succeeded,
                    session_id: Some(session_id),
                })
            }
            Err(error) => {
                self.forget_command(&command_id);
                Err(error)
            }
        }
    }

    async fn cancel_session(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let session_id = require_session_id(request)?;
        let command_id = require_command_id(request)?;
        if let Some(status) = self.register_command(&command_id, Action::SessionsCancel)? {
            return typed(&CommandReceipt {
                command_id,
                status,
                session_id: Some(session_id),
            });
        }
        let state = self.app.state::<crate::state::AppState>();
        let exists = {
            let inner = crate::state::lock(&state.inner);
            inner.tasks.iter().any(|task| task.id == session_id)
        };
        if !exists {
            self.forget_command(&command_id);
            return Err(ProtocolError::not_found(&session_id));
        }
        {
            let mut inner = crate::state::lock(&state.inner);
            if let Some(task) = inner.tasks.iter_mut().find(|task| task.id == session_id) {
                crate::commands::cancel_task_state(task);
            }
        }
        crate::commands::kill_task_children(&state, &session_id);
        state.save();
        self.finish_command(&command_id, CommandStatus::Canceled);
        typed(&CommandReceipt {
            command_id,
            status: CommandStatus::Canceled,
            session_id: Some(session_id),
        })
    }

    async fn respond_approval(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let payload: ApprovalResponsePayload = request.payload_as()?;
        let command_id = require_command_id(request)?;
        let session_id = request.session_id.clone().filter(|id| !id.is_empty());
        if let Some(status) = self.register_command(&command_id, Action::ApprovalsRespond)? {
            return typed(&CommandReceipt {
                command_id,
                status,
                session_id,
            });
        }
        let decision = match payload.decision {
            solayge_protocol::ApprovalDecision::ApproveOnce => "once",
            solayge_protocol::ApprovalDecision::Deny => "reject",
        };
        let answer = serde_json::json!({ "decision": decision });
        let task_id = match self.task_for_approval(&payload.approval_id) {
            Ok(task_id) => task_id,
            Err(error) => {
                self.forget_command(&command_id);
                return Err(error);
            }
        };
        match self
            .reply_to_ask(&payload.approval_id, &task_id, answer)
            .await
        {
            Ok(()) => {
                self.finish_command(&command_id, CommandStatus::Succeeded);
                typed(&CommandReceipt {
                    command_id,
                    status: CommandStatus::Succeeded,
                    session_id: Some(task_id),
                })
            }
            Err(error) => {
                self.forget_command(&command_id);
                Err(error)
            }
        }
    }

    /// Resolve the task backing an approval id.
    fn task_for_approval(&self, approval_id: &str) -> Result<String, ProtocolError> {
        let state = self.app.state::<crate::state::AppState>();
        let inner = crate::state::lock(&state.inner);
        inner
            .tasks
            .iter()
            .find(|task| task.ask.as_ref().is_some_and(|ask| ask.id == approval_id))
            .map(|task| task.id.clone())
            .ok_or_else(|| ProtocolError::not_found(approval_id))
    }

    /// Reply to the pending ask for `approval_id`, refusing a mismatched
    /// session. Mirrors the desktop command so a remote answer and a local one
    /// take the same path through the provider.
    async fn reply_to_ask(
        &self,
        approval_id: &str,
        session_id: &str,
        answer: Value,
    ) -> Result<(), ProtocolError> {
        let task_id = self.task_for_approval(approval_id)?;
        if task_id != session_id {
            return Err(ProtocolError::not_authorized(
                "the approval does not belong to this session",
            ));
        }
        let ask = {
            let state = self.app.state::<crate::state::AppState>();
            let inner = crate::state::lock(&state.inner);
            let task = inner
                .tasks
                .iter()
                .find(|t| t.id == task_id)
                .ok_or_else(|| ProtocolError::not_found(&task_id))?;
            if !task.can_answer_ask() {
                return Err(ProtocolError::invalid_payload(
                    "this task is no longer waiting for an answer",
                ));
            }
            task.ask.clone().ok_or_else(|| ProtocolError::not_found(approval_id))?
        };
        let provider_session = ask
            .session_id
            .clone()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                ProtocolError::invalid_payload("the question is not attached to a live session")
            })?;
        let connection = crate::opencode_server::ensure_server()
            .await
            .map_err(command_error)?;
        match ask.kind {
            AskKind::Question => {
                crate::opencode_server::reply_form(&connection, &provider_session, &ask.id, answer)
                    .await
                    .map_err(command_error)?;
            }
            AskKind::Permission => {
                let decision = answer
                    .get("decision")
                    .and_then(Value::as_str)
                    .unwrap_or("reject");
                crate::opencode_server::reply_permission(
                    &connection,
                    &provider_session,
                    &ask.id,
                    decision,
                    None,
                )
                .await
                .map_err(command_error)?;
            }
        }
        {
            let state = self.app.state::<crate::state::AppState>();
            let mut inner = crate::state::lock(&state.inner);
            if let Some(task) = inner.tasks.iter_mut().find(|t| t.id == task_id) {
                if task.ask.as_ref().map(|a| a.id.as_str()) == Some(ask.id.as_str()) {
                    task.ask = None;
                }
            }
        }
        self.app.state::<crate::state::AppState>().save();
        Ok(())
    }

    fn health(&self) -> Result<Value, ProtocolError> {
        let manager = self.app.state::<DeviceManager>();
        typed(&ConnectionHealth {
            desktop_id: manager
                .desktop_id_string()
                .map_err(|e| ProtocolError::internal(e.to_string()))?,
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
            .map_err(|e| ProtocolError::internal(e.to_string()))?
            .handle_begin(begin, now())
            .map_err(identity_error)?;
        typed(&response)
    }

    fn pair_prove(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let prove: PairProveIdentityRequest = request.payload_as()?;
        let manager = self.app.state::<DeviceManager>();
        let response: PairProveIdentityResponse = manager
            .pairing_server()
            .map_err(|e| ProtocolError::internal(e.to_string()))?
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
        let desktop_id = manager
            .desktop_id_string()
            .map_err(|e| ProtocolError::internal(e.to_string()))?;
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
            desktop_id: manager
                .desktop_id_string()
                .map_err(|e| ProtocolError::internal(e.to_string()))?,
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
        // Drafts are included: a task the user has planned but not released is
        // still something they want to see and review from the phone.
        let sessions = inner.tasks.iter().map(session_summary).collect();
        typed(&SessionsResponse { sessions })
    }

    fn session_detail(&self, request: &Envelope) -> Result<Value, ProtocolError> {
        let session_id = request
            .session_id
            .clone()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| ProtocolError::missing_field("session_id"))?;
        let state = self.app.state::<crate::state::AppState>();
        let log_path = state.log_path(&session_id);
        let inner = crate::state::lock(&state.inner);
        let task = inner
            .tasks
            .iter()
            .find(|t| t.id == session_id)
            .ok_or_else(|| ProtocolError::not_found(&session_id))?;
        typed(&session_detail(task, read_log_tail(&log_path)))
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

    /// Store the calling device's push token. The token is opaque here; it is
    /// only forwarded to the push gateway.
    fn push_register(
        &self,
        device_id: &str,
        request: &Envelope,
    ) -> Result<Value, ProtocolError> {
        let payload: PushRegisterPayload = request.payload_as()?;
        if payload.token.trim().is_empty() {
            return Err(ProtocolError::missing_field("token"));
        }
        let manager = self.app.state::<DeviceManager>();
        manager
            .set_push_token(device_id, payload.token, payload.platform)
            .map_err(|e| ProtocolError::internal(e.to_string()))?;
        typed(&PushRegisterResult { registered: true })
    }

    /// Forget the calling device's push token.
    fn push_unregister(
        &self,
        device_id: &str,
        request: &Envelope,
    ) -> Result<Value, ProtocolError> {
        let _: PushUnregisterPayload = request.payload_as()?;
        let manager = self.app.state::<DeviceManager>();
        manager
            .clear_push_token(device_id)
            .map_err(|e| ProtocolError::internal(e.to_string()))?;
        typed(&PushRegisterResult { registered: false })
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
            match self.dispatch(session, &request).await {
                Ok(payload) => Envelope::response_to(&request, payload),
                Err(error) => Envelope::error_to(&request, &error),
            }
        })
    }
}

fn typed<T: serde::Serialize>(value: &T) -> Result<Value, ProtocolError> {
    serde_json::to_value(value).map_err(|e| ProtocolError::internal(e.to_string()))
}

/// The session id a mutating command must name.
fn require_session_id(request: &Envelope) -> Result<String, ProtocolError> {
    request
        .session_id
        .clone()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ProtocolError::missing_field("session_id"))
}

/// The idempotency key every mutating command must carry.
fn require_command_id(request: &Envelope) -> Result<String, ProtocolError> {
    request
        .command_id
        .clone()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ProtocolError::missing_field("command_id"))
}

/// Map a desktop command failure onto a typed protocol error. The desktop's
/// messages are human-readable; classify the common cases so a client can react
/// without parsing prose.
fn command_error(message: String) -> ProtocolError {
    let lower = message.to_lowercase();
    if lower.contains("not found") {
        ProtocolError::not_found(&message)
    } else if lower.contains("no pending")
        || lower.contains("no longer")
        || lower.contains("already active")
        || lower.contains("empty")
        || lower.contains("invalid")
    {
        ProtocolError::invalid_payload(message)
    } else {
        ProtocolError::internal(message)
    }
}

/// Project a host question field onto the wire DTO the mobile client renders.
fn approval_field(field: &AskField) -> ApprovalField {
    ApprovalField {
        key: field.key.clone(),
        label: field.label.clone(),
        description: field.description.clone(),
        kind: match field.kind {
            AskFieldKind::String => ApprovalFieldKind::String,
            AskFieldKind::Number => ApprovalFieldKind::Number,
            AskFieldKind::Integer => ApprovalFieldKind::Integer,
            AskFieldKind::Boolean => ApprovalFieldKind::Boolean,
            AskFieldKind::Multiselect => ApprovalFieldKind::Multiselect,
            AskFieldKind::External => ApprovalFieldKind::External,
        },
        required: field.required,
        custom: field.custom,
        options: field
            .options
            .iter()
            .map(|option| ApprovalFieldOption {
                value: option.value.clone(),
                label: option.label.clone(),
                description: option.description.clone(),
            })
            .collect(),
        placeholder: field.placeholder.clone(),
        url: field.url.clone(),
    }
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

fn session_detail(task: &Task, log_tail: Option<String>) -> SessionDetail {
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
        prompt: Some(redact_long(&task.prompt)),
        log_tail,
        files_changed: 0,
    }
}

/// The last part of a task log, capped, for the mobile detail view. `None` when
/// the log does not exist yet. Reads at most `MAX` bytes so a huge log cannot
/// bloat a frame or the phone.
fn read_log_tail(path: &std::path::Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const MAX: u64 = 8 * 1024;
    let len = std::fs::metadata(path).ok()?.len();
    let mut file = std::fs::File::open(path).ok()?;
    if len > MAX {
        file.seek(SeekFrom::Start(len - MAX)).ok()?;
    }
    let mut buf = Vec::new();
    file.take(MAX).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).to_string();
    (!text.trim().is_empty()).then(|| redact_long(&text))
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
        fields: ask.fields.iter().map(approval_field).collect(),
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
                fields: vec![crate::models::AskField {
                    key: "api".into(),
                    label: "Which API should I call?".into(),
                    description: Some("Pick the backend".into()),
                    kind: AskFieldKind::String,
                    required: true,
                    custom: false,
                    options: vec![crate::models::AskOption {
                        value: "rest".into(),
                        label: "REST".into(),
                        description: None,
                    }],
                    default: None,
                    placeholder: Some("e.g. REST".into()),
                    format: None,
                    min: None,
                    max: None,
                    min_length: None,
                    max_length: None,
                    pattern: None,
                    min_items: None,
                    max_items: None,
                    url: None,
                }],
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
        // The question's structured content must reach the phone, not be dropped.
        assert_eq!(approval.fields.len(), 1);
        assert_eq!(approval.fields[0].key, "api");
        assert_eq!(approval.fields[0].kind, ApprovalFieldKind::String);
        assert_eq!(approval.fields[0].options[0].value, "rest");
        assert!(approval_of(&task(TaskStatus::Running, false)).is_none());
    }

    #[test]
    fn long_summaries_are_capped_on_a_char_boundary() {
        let long = "é".repeat(10_000);
        let capped = redact_long(&long);
        assert!(capped.len() <= 8 * 1024 + 4);
        assert!(capped.ends_with('…'));
    }

    #[test]
    fn session_detail_carries_the_prompt_and_log_tail_for_mobile() {
        let detail = session_detail(&task(TaskStatus::Running, false), Some("tail".into()));
        assert_eq!(detail.prompt.as_deref(), Some("prompt"));
        assert_eq!(detail.log_tail.as_deref(), Some("tail"));
    }

    #[test]
    fn log_tail_reads_the_end_and_handles_a_missing_file() {
        let dir =
            std::env::temp_dir().join(format!("solayge-logtail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_log_tail(&dir.join("missing.log")).is_none());

        let path = dir.join("t.log");
        std::fs::write(&path, format!("{}\nEND", "x".repeat(20_000))).unwrap();
        let tail = read_log_tail(&path).unwrap();
        assert!(tail.ends_with("END"));
        assert!(tail.len() <= 8 * 1024 + 4);

        let _ = std::fs::remove_dir_all(&dir);
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

    #[tokio::test]
    async fn a_phone_pairs_then_authenticates_and_binds_to_its_connection() {
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
            .unwrap()
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
            serde_json::from_value(handler.dispatch(&session, &request).await.unwrap()).unwrap();

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
        handler.dispatch(&session, &request).await.unwrap();

        // Desktop user approves with the default View + Message grant.
        app.state::<DeviceManager>()
            .pairing_server()
            .unwrap()
            .approve(&offer.pairing_id, None, now())
            .unwrap();

        // Before authentication, a read is refused.
        let metadata = Envelope::command(Action::HostsGetMetadata, Value::Null);
        assert_eq!(
            handler.dispatch(&session, &metadata).await.unwrap_err().code(),
            solayge_protocol::ErrorCode::NotAuthorized
        );

        // Auth leg 1: request a challenge.
        let mut auth = AuthRequest {
            device_id: phone.device_id().to_string(),
            desktop_id: app.state::<DeviceManager>().desktop_id_string().unwrap(),
            protocol_version: PROTOCOL_VERSION,
            host_nonce: None,
            proof: None,
            transport_binding: None,
        };
        let request =
            Envelope::command(Action::ConnectionAuthenticate, serde_json::to_value(&auth).unwrap());
        let challenge: AuthResponse =
            serde_json::from_value(handler.dispatch(&session, &request).await.unwrap()).unwrap();
        assert!(!challenge.authenticated);
        let nonce = challenge.host_nonce.expect("a challenge is issued");

        // Auth leg 2: sign the challenge.
        auth.host_nonce = Some(nonce);
        auth.proof = Some(solayge_identity::prove(&phone, &auth).unwrap());
        let request =
            Envelope::command(Action::ConnectionAuthenticate, serde_json::to_value(&auth).unwrap());
        let ok: AuthResponse =
            serde_json::from_value(handler.dispatch(&session, &request).await.unwrap()).unwrap();
        assert!(ok.authenticated);
        assert_eq!(
            session.authenticated_device().as_deref(),
            Some(phone.device_id().as_str())
        );

        // The authenticated connection can read; a different one cannot.
        handler.dispatch(&session, &metadata).await.unwrap();
        let other = ConnectionSession::new("iroh:other");
        assert_eq!(
            handler.dispatch(&other, &metadata).await.unwrap_err().code(),
            solayge_protocol::ErrorCode::NotAuthorized
        );

        // The paired device can register a push token, and the host stores it.
        let push = Envelope::command(
            Action::PushRegister,
            serde_json::json!({ "token": "fcm-1", "platform": "android" }),
        )
        .with_command_id("cmd_push_1");
        let registered: PushRegisterResult =
            serde_json::from_value(handler.dispatch(&session, &push).await.unwrap()).unwrap();
        assert!(registered.registered);
        assert_eq!(
            app.state::<DeviceManager>()
                .push_token(phone.device_id().as_str())
                .unwrap()
                .token,
            "fcm-1"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
