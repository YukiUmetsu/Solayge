//! Transport-neutral client trait and an in-memory loopback host.
//!
//! [`RemoteClient`] is the seam a real transport (Iroh, Tailscale WSS) plugs
//! into: it moves validated [`Envelope`] frames and knows nothing else. The
//! provided methods turn that one primitive into typed requests.
//!
//! [`MockHost`] / [`LoopbackClient`] implement the same contract entirely in
//! memory. They exist for unit tests and for local UI development before a
//! transport is wired up, and they enforce the same idempotency and
//! authorization rules a real host must.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::actions::Action;
use crate::dto::*;
use crate::envelope::{Envelope, MessageKind, PROTOCOL_VERSION};
use crate::error::ProtocolError;
use crate::idempotency::{command_id_for, CommandLedger, CommandObservation};
use crate::status::CommandStatus;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn typed<T: Serialize>(value: &T) -> Result<Value, ProtocolError> {
    serde_json::to_value(value).map_err(|e| ProtocolError::invalid_payload(e.to_string()))
}

fn into_typed<T: DeserializeOwned>(response: Envelope) -> Result<T, ProtocolError> {
    let payload = match response.kind {
        MessageKind::Error => {
            let wire: WireError = response.payload_as()?;
            let code = crate::ErrorCode::parse(&wire.code).unwrap_or(crate::ErrorCode::Internal);
            return Err(ProtocolError::new(code, wire.message));
        }
        MessageKind::Response => response.payload,
        _ => return Err(ProtocolError::invalid_frame("expected a response frame")),
    };
    serde_json::from_value(payload).map_err(|e| ProtocolError::invalid_payload(e.to_string()))
}

fn require_command_id(request: &Envelope) -> Result<String, ProtocolError> {
    request
        .command_id
        .clone()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ProtocolError::missing_field("command_id"))
}

fn require_session_id(request: &Envelope) -> Result<String, ProtocolError> {
    request
        .session_id
        .clone()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ProtocolError::missing_field("session_id"))
}

/// A client that exchanges protocol frames with a Solayge host.
///
/// Implementations must validate frames and must treat `command_id` as an
/// idempotency key. They must not interpret payloads beyond this protocol.
pub trait RemoteClient: Send + Sync {
    /// Exchange one request frame for one response (or error) frame.
    fn exchange(&self, request: Envelope) -> Result<Envelope, ProtocolError>;

    /// The selected host's public id, when known.
    fn desktop_id(&self) -> Option<&str> {
        None
    }

    /// Whether a transport is currently established (not whether it is
    /// authenticated — call [`RemoteClient::health`] for that).
    fn is_connected(&self) -> bool {
        true
    }

    fn health(&self) -> Result<ConnectionHealth, ProtocolError> {
        into_typed(self.exchange(Envelope::command(Action::ConnectionHealth, Value::Null))?)
    }

    fn authenticate(&self, device_id: &str) -> Result<ConnectionAuthenticateResult, ProtocolError> {
        let payload = typed(&ConnectionAuthenticatePayload {
            device_id: device_id.to_string(),
            desktop_id: self.desktop_id().unwrap_or_default().to_string(),
            protocol_version: PROTOCOL_VERSION,
        })?;
        let mut request = Envelope::command(Action::ConnectionAuthenticate, payload);
        if let Some(desktop_id) = self.desktop_id() {
            request = request.with_desktop_id(desktop_id);
        }
        into_typed(self.exchange(request)?)
    }

    fn host_metadata(&self) -> Result<HostMetadata, ProtocolError> {
        into_typed(self.exchange(Envelope::command(Action::HostsGetMetadata, Value::Null))?)
    }

    fn list_projects(&self) -> Result<Vec<ProjectSummary>, ProtocolError> {
        let response: ProjectsResponse =
            into_typed(self.exchange(Envelope::command(Action::ProjectsList, Value::Null))?)?;
        Ok(response.projects)
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>, ProtocolError> {
        let response: SessionsResponse =
            into_typed(self.exchange(Envelope::command(Action::SessionsList, Value::Null))?)?;
        Ok(response.sessions)
    }

    fn get_session(&self, session_id: &str) -> Result<SessionDetail, ProtocolError> {
        let request =
            Envelope::command(Action::SessionsGet, Value::Null).with_session_id(session_id);
        into_typed(self.exchange(request)?)
    }

    fn subscribe_events(
        &self,
        session_id: Option<&str>,
        from_seq: u64,
    ) -> Result<EventsBatch, ProtocolError> {
        let payload = typed(&EventsSubscribePayload {
            from_seq,
            session_id: session_id.map(str::to_string),
        })?;
        into_typed(self.exchange(Envelope::command(Action::SessionsEventsSubscribe, payload))?)
    }

    /// Send a prompt. The command id is derived deterministically, so retrying
    /// the same logical prompt is a duplicate rather than a second agent run.
    fn send_prompt(&self, session_id: &str, text: &str) -> Result<CommandReceipt, ProtocolError> {
        let payload = typed(&SendPromptPayload {
            text: text.to_string(),
        })?;
        let command_id = command_id_for(
            self.desktop_id().unwrap_or_default(),
            session_id,
            Action::SessionsSendPrompt,
            &payload,
        );
        self.send_prompt_with_id(session_id, text, &command_id)
    }

    /// Send a prompt with a caller-supplied idempotency key.
    fn send_prompt_with_id(
        &self,
        session_id: &str,
        text: &str,
        command_id: &str,
    ) -> Result<CommandReceipt, ProtocolError> {
        let payload = typed(&SendPromptPayload {
            text: text.to_string(),
        })?;
        let request = Envelope::command(Action::SessionsSendPrompt, payload)
            .with_session_id(session_id)
            .with_command_id(command_id);
        into_typed(self.exchange(request)?)
    }

    fn get_command_status(&self, command_id: &str) -> Result<CommandStatusPayload, ProtocolError> {
        let payload = typed(&CommandStatusQuery {
            command_id: command_id.to_string(),
        })?;
        into_typed(self.exchange(Envelope::command(Action::SessionsGetCommandStatus, payload))?)
    }

    fn cancel_session(&self, session_id: &str) -> Result<CommandReceipt, ProtocolError> {
        let command_id = command_id_for(
            self.desktop_id().unwrap_or_default(),
            session_id,
            Action::SessionsCancel,
            &Value::Null,
        );
        self.cancel_session_with_id(session_id, &command_id)
    }

    fn cancel_session_with_id(
        &self,
        session_id: &str,
        command_id: &str,
    ) -> Result<CommandReceipt, ProtocolError> {
        let request = Envelope::command(Action::SessionsCancel, Value::Null)
            .with_session_id(session_id)
            .with_command_id(command_id);
        into_typed(self.exchange(request)?)
    }

    fn start_session(&self, session_id: &str) -> Result<CommandReceipt, ProtocolError> {
        let command_id = command_id_for(
            self.desktop_id().unwrap_or_default(),
            session_id,
            Action::SessionsStart,
            &Value::Null,
        );
        let request = Envelope::command(Action::SessionsStart, Value::Null)
            .with_session_id(session_id)
            .with_command_id(command_id);
        into_typed(self.exchange(request)?)
    }

    fn list_approvals(&self) -> Result<Vec<ApprovalSummary>, ProtocolError> {
        let response: ApprovalsResponse =
            into_typed(self.exchange(Envelope::command(Action::ApprovalsList, Value::Null))?)?;
        Ok(response.approvals)
    }

    fn get_approval(&self, approval_id: &str) -> Result<ApprovalSummary, ProtocolError> {
        let payload = typed(&ApprovalQuery {
            approval_id: approval_id.to_string(),
        })?;
        into_typed(self.exchange(Envelope::command(Action::ApprovalsGet, payload))?)
    }

    fn respond_approval(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> Result<CommandReceipt, ProtocolError> {
        let payload = typed(&ApprovalResponsePayload {
            approval_id: approval_id.to_string(),
            decision,
        })?;
        let command_id = command_id_for(
            self.desktop_id().unwrap_or_default(),
            "",
            Action::ApprovalsRespond,
            &payload,
        );
        let request =
            Envelope::command(Action::ApprovalsRespond, payload).with_command_id(command_id);
        into_typed(self.exchange(request)?)
    }

    fn files_changed_summary(
        &self,
        session_id: &str,
    ) -> Result<FilesChangedSummary, ProtocolError> {
        let request =
            Envelope::command(Action::FilesChangedSummary, Value::Null).with_session_id(session_id);
        into_typed(self.exchange(request)?)
    }

    fn insights_summary(&self, project_id: &str) -> Result<InsightsSummary, ProtocolError> {
        let payload = typed(&ProjectQuery {
            project_id: project_id.to_string(),
        })?;
        into_typed(self.exchange(Envelope::command(Action::InsightsSummary, payload))?)
    }

    fn insights_errors(&self, project_id: &str) -> Result<InsightsErrors, ProtocolError> {
        let payload = typed(&ProjectQuery {
            project_id: project_id.to_string(),
        })?;
        into_typed(self.exchange(Envelope::command(Action::InsightsErrors, payload))?)
    }

    fn insights_agent_questions(
        &self,
        project_id: &str,
    ) -> Result<InsightsAgentQuestions, ProtocolError> {
        let payload = typed(&ProjectQuery {
            project_id: project_id.to_string(),
        })?;
        into_typed(self.exchange(Envelope::command(Action::InsightsAgentQuestions, payload))?)
    }

    fn list_devices(&self) -> Result<Vec<DeviceSummary>, ProtocolError> {
        let response: DevicesResponse =
            into_typed(self.exchange(Envelope::command(Action::DevicesList, Value::Null))?)?;
        Ok(response.devices)
    }

    fn revoke_device(&self, device_id: &str) -> Result<DeviceRevokeResult, ProtocolError> {
        let payload = typed(&DeviceRevokePayload {
            device_id: device_id.to_string(),
        })?;
        let command_id = command_id_for(
            self.desktop_id().unwrap_or_default(),
            "",
            Action::DevicesRevoke,
            &payload,
        );
        let request = Envelope::command(Action::DevicesRevoke, payload).with_command_id(command_id);
        into_typed(self.exchange(request)?)
    }

    fn pair_begin(&self) -> Result<PairBeginPayload, ProtocolError> {
        into_typed(self.exchange(Envelope::command(Action::PairBegin, Value::Null))?)
    }

    fn pair_prove_identity(&self, proof: PairIdentityProof) -> Result<PairAck, ProtocolError> {
        into_typed(self.exchange(Envelope::command(Action::PairProveIdentity, typed(&proof)?))?)
    }

    fn pair_approve(&self, pairing_id: &str, device_id: &str) -> Result<PairAck, ProtocolError> {
        let payload = typed(&PairDecisionPayload {
            pairing_id: pairing_id.to_string(),
            device_id: device_id.to_string(),
        })?;
        let command_id = command_id_for(
            self.desktop_id().unwrap_or_default(),
            "",
            Action::PairApprove,
            &payload,
        );
        into_typed(self.exchange(
            Envelope::command(Action::PairApprove, payload).with_command_id(command_id),
        )?)
    }

    fn pair_reject(&self, pairing_id: &str, device_id: &str) -> Result<PairAck, ProtocolError> {
        let payload = typed(&PairDecisionPayload {
            pairing_id: pairing_id.to_string(),
            device_id: device_id.to_string(),
        })?;
        let command_id = command_id_for(
            self.desktop_id().unwrap_or_default(),
            "",
            Action::PairReject,
            &payload,
        );
        into_typed(
            self.exchange(
                Envelope::command(Action::PairReject, payload).with_command_id(command_id),
            )?,
        )
    }
}

struct MockState {
    /// The device id of the current application session, if any. Revoking this
    /// device must drop the active session, not just future authentications.
    authenticated_device: Option<String>,
    host: HostMetadata,
    projects: Vec<ProjectSummary>,
    sessions: Vec<SessionDetail>,
    events: HashMap<String, Vec<SessionEvent>>,
    approvals: Vec<ApprovalSummary>,
    devices: Vec<DeviceSummary>,
    revoked: HashSet<String>,
    changed: HashMap<String, Vec<ChangedFileSummary>>,
    errors: Vec<ErrorInsight>,
    questions: Vec<QuestionInsight>,
    ledger: CommandLedger,
    /// Prompts the host actually executed, for asserting idempotency.
    prompts: Vec<(String, String, String)>,
}

/// An in-memory, deterministic Solayge host for tests and local UI dev.
pub struct MockHost {
    desktop_id: String,
    state: Mutex<MockState>,
}

impl Default for MockHost {
    fn default() -> Self {
        MockHost::with_desktop_id("desk_mock")
    }
}

impl MockHost {
    pub fn new() -> Self {
        MockHost::default()
    }

    /// A host seeded with one project, one running session, an event stream, a
    /// pending permission approval, and one paired device.
    pub fn with_desktop_id(desktop_id: &str) -> Self {
        let sessions = vec![SessionDetail {
            id: "ses_1".to_string(),
            project_id: "/repo/app".to_string(),
            title: "Refactor UI".to_string(),
            agent: "opencode".to_string(),
            status: "running".to_string(),
            pending_decisions: 1,
            started_at: 10,
            updated_at: 100,
            summary: Some("Editing components; running tests next.".to_string()),
            files_changed: 2,
        }];
        let host = HostMetadata {
            desktop_id: desktop_id.to_string(),
            name: "Mock Desktop".to_string(),
            platform: std::env::consts::OS.to_string(),
            app_version: "0.1.0".to_string(),
            protocol_version: PROTOCOL_VERSION,
        };
        let state = MockState {
            authenticated_device: None,
            host,
            projects: vec![ProjectSummary {
                id: "/repo/app".to_string(),
                name: "app".to_string(),
                active_sessions: 1,
                pending_approvals: 1,
                updated_at: 100,
            }],
            sessions,
            events: HashMap::from([(
                "ses_1".to_string(),
                vec![
                    SessionEvent {
                        seq: 1,
                        event_id: "ev_1".to_string(),
                        kind: "message".to_string(),
                        timestamp: 20,
                        payload: serde_json::json!({ "text": "Editing components" }),
                    },
                    SessionEvent {
                        seq: 2,
                        event_id: "ev_2".to_string(),
                        kind: "tool".to_string(),
                        timestamp: 30,
                        payload: serde_json::json!({ "tool": "read", "path": "src/App.tsx" }),
                    },
                ],
            )]),
            approvals: vec![ApprovalSummary {
                id: "apr_1".to_string(),
                session_id: "ses_1".to_string(),
                project_id: "/repo/app".to_string(),
                kind: ApprovalKind::Permission,
                title: "Run command?".to_string(),
                message: Some("The agent wants to run the test suite.".to_string()),
                command: Some("cargo test --workspace".to_string()),
                risk: ApprovalRisk::Medium,
                read_only: false,
                requested_at: 90,
                expires_at: Some(3_600),
            }],
            devices: vec![DeviceSummary {
                device_id: "dev_1".to_string(),
                name: "iPhone 15".to_string(),
                platform: "ios".to_string(),
                scopes: vec!["sessions.read".to_string(), "sessions.message".to_string()],
                paired_at: 1,
                last_seen: 100,
            }],
            revoked: HashSet::new(),
            changed: HashMap::from([(
                "ses_1".to_string(),
                vec![
                    ChangedFileSummary {
                        path: "src/App.tsx".to_string(),
                        status: "modified".to_string(),
                        additions: Some(10),
                        deletions: Some(2),
                    },
                    ChangedFileSummary {
                        path: "src/api.ts".to_string(),
                        status: "modified".to_string(),
                        additions: Some(4),
                        deletions: Some(1),
                    },
                ],
            )]),
            errors: vec![ErrorInsight {
                kind: "provider_timeout".to_string(),
                count: 1,
                last_seen: 80,
            }],
            questions: vec![QuestionInsight {
                normalized_intent: "choose implementation approach".to_string(),
                count: 2,
                last_seen: 70,
            }],
            ledger: CommandLedger::default(),
            prompts: Vec::new(),
        };
        MockHost {
            desktop_id: desktop_id.to_string(),
            state: Mutex::new(state),
        }
    }

    /// A client bound to this host.
    pub fn client(self: &Arc<Self>) -> LoopbackClient {
        LoopbackClient {
            host: Arc::clone(self),
        }
    }

    pub fn desktop_id(&self) -> &str {
        &self.desktop_id
    }

    /// How many prompts the host actually executed.
    pub fn prompt_count(&self) -> usize {
        lock(&self.state).prompts.len()
    }

    pub fn command_status(&self, command_id: &str) -> Option<CommandStatus> {
        lock(&self.state).ledger.get(command_id)
    }

    /// Replace the changed-file summary for a session.
    pub fn set_changed_files(&self, session_id: &str, files: Vec<ChangedFileSummary>) {
        lock(&self.state)
            .changed
            .insert(session_id.to_string(), files);
    }

    fn handle(&self, request: Envelope) -> Result<Envelope, ProtocolError> {
        // A malformed request is refused before it can be correlated.
        request.validate()?;
        if request.kind != MessageKind::Command {
            return Err(ProtocolError::invalid_frame(
                "the mock host accepts only command frames",
            ));
        }
        let mut state = lock(&self.state);
        match self.dispatch(&mut state, &request) {
            Ok(payload) => Ok(Envelope::response_to(&request, payload)),
            Err(error) => Ok(Envelope::error_to(&request, &error)),
        }
    }

    fn dispatch(&self, state: &mut MockState, request: &Envelope) -> Result<Value, ProtocolError> {
        let action = request.action;
        let authorized = match &state.authenticated_device {
            Some(device) => !state.revoked.contains(device),
            None => false,
        };
        if !action.is_pre_auth() && !authorized {
            return Err(ProtocolError::not_authorized(
                "authenticate before using this action",
            ));
        }

        match action {
            Action::ConnectionHealth => typed(&ConnectionHealth {
                desktop_id: state.host.desktop_id.clone(),
                protocol_version: PROTOCOL_VERSION,
                app_version: state.host.app_version.clone(),
                uptime_secs: 42,
            }),
            Action::ConnectionAuthenticate => {
                let payload: ConnectionAuthenticatePayload = request.payload_as()?;
                if state.revoked.contains(&payload.device_id) {
                    return Err(ProtocolError::not_authorized("device has been revoked"));
                }
                state.authenticated_device = Some(payload.device_id);
                typed(&ConnectionAuthenticateResult {
                    authenticated: true,
                    desktop_id: state.host.desktop_id.clone(),
                })
            }
            Action::PairBegin => typed(&PairBeginPayload {
                pairing_id: "pair_1".to_string(),
                desktop_id: state.host.desktop_id.clone(),
                verification_code: "4821".to_string(),
                expires_at: 60,
            }),
            Action::PairProveIdentity => {
                let proof: PairIdentityProof = request.payload_as()?;
                typed(&PairAck {
                    pairing_id: proof.pairing_id,
                    state: "pending".to_string(),
                })
            }
            Action::PairApprove | Action::PairReject => {
                let payload: PairDecisionPayload = request.payload_as()?;
                let state_name = if action == Action::PairApprove {
                    "approved"
                } else {
                    "rejected"
                };
                typed(&PairAck {
                    pairing_id: payload.pairing_id,
                    state: state_name.to_string(),
                })
            }
            Action::HostsGetMetadata => typed(&state.host),
            Action::ProjectsList => typed(&ProjectsResponse {
                projects: state.projects.clone(),
            }),
            Action::SessionsList => typed(&SessionsResponse {
                sessions: state.sessions.iter().map(summary_of).collect(),
            }),
            Action::SessionsGet => {
                let session_id = require_session_id(request)?;
                let session = find_session(state, &session_id)?;
                typed(&session)
            }
            Action::SessionsEventsSubscribe => {
                let payload: EventsSubscribePayload = request.payload_as()?;
                let mut events: Vec<SessionEvent> = match &payload.session_id {
                    Some(session_id) => state
                        .events
                        .get(session_id)
                        .map(|list| {
                            list.iter()
                                .filter(|e| e.seq >= payload.from_seq)
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default(),
                    None => {
                        let mut all: Vec<SessionEvent> = state
                            .events
                            .values()
                            .flatten()
                            .filter(|e| e.seq >= payload.from_seq)
                            .cloned()
                            .collect();
                        all.sort_by_key(|e| e.seq);
                        all
                    }
                };
                events.sort_by_key(|e| e.seq);
                let next_seq = events.last().map(|e| e.seq + 1).unwrap_or(payload.from_seq);
                typed(&EventsBatch {
                    session_id: payload.session_id,
                    events,
                    next_seq,
                    resync: false,
                })
            }
            Action::SessionsSendPrompt => {
                let payload: SendPromptPayload = request.payload_as()?;
                let session_id = require_session_id(request)?;
                find_session(state, &session_id)?;
                let command_id = require_command_id(request)?;
                let status = match state.ledger.register(&command_id, action)? {
                    CommandObservation::Duplicate { status } => status,
                    CommandObservation::New => {
                        // Execute exactly once.
                        state
                            .prompts
                            .push((session_id.clone(), payload.text, command_id.clone()));
                        state
                            .ledger
                            .advance(&command_id, CommandStatus::Delivered)?;
                        state.ledger.advance(&command_id, CommandStatus::Accepted)?;
                        state.ledger.advance(&command_id, CommandStatus::Running)?;
                        CommandStatus::Running
                    }
                };
                typed(&CommandReceipt {
                    command_id,
                    status,
                    session_id: Some(session_id),
                })
            }
            Action::SessionsGetCommandStatus => {
                let payload: CommandStatusQuery = request.payload_as()?;
                let status = state
                    .ledger
                    .get(&payload.command_id)
                    .ok_or_else(|| ProtocolError::not_found(&payload.command_id))?;
                typed(&CommandStatusPayload {
                    command_id: payload.command_id,
                    status,
                    error: None,
                    updated_at: 0,
                })
            }
            Action::SessionsCancel => {
                let session_id = require_session_id(request)?;
                find_session(state, &session_id)?;
                let command_id = require_command_id(request)?;
                let status = match state.ledger.register(&command_id, action)? {
                    CommandObservation::Duplicate { status } => status,
                    CommandObservation::New => {
                        state.ledger.advance(&command_id, CommandStatus::Canceled)?;
                        CommandStatus::Canceled
                    }
                };
                typed(&CommandReceipt {
                    command_id,
                    status,
                    session_id: Some(session_id),
                })
            }
            Action::SessionsStart => {
                let session_id = require_session_id(request)?;
                find_session(state, &session_id)?;
                let command_id = require_command_id(request)?;
                let status = match state.ledger.register(&command_id, action)? {
                    CommandObservation::Duplicate { status } => status,
                    CommandObservation::New => {
                        state
                            .ledger
                            .advance(&command_id, CommandStatus::Succeeded)?;
                        CommandStatus::Succeeded
                    }
                };
                typed(&CommandReceipt {
                    command_id,
                    status,
                    session_id: Some(session_id),
                })
            }
            Action::ApprovalsList => typed(&ApprovalsResponse {
                approvals: state.approvals.clone(),
            }),
            Action::ApprovalsGet => {
                let payload: ApprovalQuery = request.payload_as()?;
                let approval = state
                    .approvals
                    .iter()
                    .find(|a| a.id == payload.approval_id)
                    .cloned()
                    .ok_or_else(|| ProtocolError::not_found(&payload.approval_id))?;
                typed(&approval)
            }
            Action::ApprovalsRespond => {
                let payload: ApprovalResponsePayload = request.payload_as()?;
                let command_id = require_command_id(request)?;
                let status = match state.ledger.register(&command_id, action)? {
                    CommandObservation::Duplicate { status } => status,
                    CommandObservation::New => {
                        // A decision is final: a stale replay finds nothing.
                        // The approval must be resolved *before* we record the
                        // command as accepted; otherwise a rejected decision
                        // would linger as `Created` and a retry would look
                        // like a successful duplicate.
                        let position = match state
                            .approvals
                            .iter()
                            .position(|a| a.id == payload.approval_id)
                        {
                            Some(position) => position,
                            None => {
                                state.ledger.forget(&command_id);
                                return Err(ProtocolError::not_found(&payload.approval_id));
                            }
                        };
                        state.approvals.remove(position);
                        state
                            .ledger
                            .advance(&command_id, CommandStatus::Succeeded)?;
                        CommandStatus::Succeeded
                    }
                };
                typed(&CommandReceipt {
                    command_id,
                    status,
                    session_id: None,
                })
            }
            Action::FilesChangedSummary => {
                let session_id = require_session_id(request)?;
                let session = find_session(state, &session_id)?;
                let files = state.changed.get(&session_id).cloned().unwrap_or_default();
                typed(&FilesChangedSummary {
                    project_id: session.project_id,
                    session_id,
                    total: files.len() as u32,
                    files,
                })
            }
            Action::InsightsSummary => {
                let payload: ProjectQuery = request.payload_as()?;
                typed(&InsightsSummary {
                    project_id: payload.project_id,
                    completed_tasks: 4,
                    failed_tasks: 1,
                    running_tasks: 1,
                    avg_duration_secs: Some(360),
                })
            }
            Action::InsightsErrors => {
                let payload: ProjectQuery = request.payload_as()?;
                typed(&InsightsErrors {
                    project_id: payload.project_id,
                    errors: state.errors.clone(),
                })
            }
            Action::InsightsAgentQuestions => {
                let payload: ProjectQuery = request.payload_as()?;
                typed(&InsightsAgentQuestions {
                    project_id: payload.project_id,
                    questions: state.questions.clone(),
                })
            }
            Action::DevicesList => typed(&DevicesResponse {
                devices: state.devices.clone(),
            }),
            Action::DevicesRevoke => {
                let payload: DeviceRevokePayload = request.payload_as()?;
                let command_id = require_command_id(request)?;
                match state.ledger.register(&command_id, action)? {
                    CommandObservation::Duplicate { .. } => {}
                    CommandObservation::New => {
                        state.devices.retain(|d| d.device_id != payload.device_id);
                        state.revoked.insert(payload.device_id.clone());
                        // Revocation takes effect immediately: if the active
                        // session belongs to this device, drop it now.
                        if state.authenticated_device.as_deref() == Some(payload.device_id.as_str())
                        {
                            state.authenticated_device = None;
                        }
                        state
                            .ledger
                            .advance(&command_id, CommandStatus::Succeeded)?;
                    }
                }
                typed(&DeviceRevokeResult { revoked: true })
            }
        }
    }
}

fn summary_of(session: &SessionDetail) -> SessionSummary {
    SessionSummary {
        id: session.id.clone(),
        project_id: session.project_id.clone(),
        title: session.title.clone(),
        agent: session.agent.clone(),
        status: session.status.clone(),
        pending_decisions: session.pending_decisions,
        started_at: session.started_at,
        updated_at: session.updated_at,
    }
}

fn find_session(state: &MockState, session_id: &str) -> Result<SessionDetail, ProtocolError> {
    state
        .sessions
        .iter()
        .find(|s| s.id == session_id)
        .cloned()
        .ok_or_else(|| ProtocolError::not_found(session_id))
}

/// An in-memory [`RemoteClient`] backed by a [`MockHost`].
pub struct LoopbackClient {
    host: Arc<MockHost>,
}

impl LoopbackClient {
    pub fn new(host: Arc<MockHost>) -> Self {
        LoopbackClient { host }
    }
}

impl RemoteClient for LoopbackClient {
    fn exchange(&self, request: Envelope) -> Result<Envelope, ProtocolError> {
        self.host.handle(request)
    }

    fn desktop_id(&self) -> Option<&str> {
        Some(self.host.desktop_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn host_and_client() -> (Arc<MockHost>, LoopbackClient) {
        let host = Arc::new(MockHost::new());
        let client = host.client();
        (host, client)
    }

    fn authed_client() -> (Arc<MockHost>, LoopbackClient) {
        let (host, client) = host_and_client();
        client.authenticate("dev_1").expect("authenticate");
        (host, client)
    }

    #[test]
    fn health_works_before_authentication() {
        let (_host, client) = host_and_client();
        let health = client.health().unwrap();
        assert_eq!(health.protocol_version, PROTOCOL_VERSION);
        assert_eq!(health.desktop_id, "desk_mock");
    }

    #[test]
    fn actions_are_denied_before_authentication() {
        let (_host, client) = host_and_client();
        let err = client.list_sessions().unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::NotAuthorized);
    }

    #[test]
    fn authenticated_client_reads_the_seeded_session() {
        let (_host, client) = authed_client();
        let sessions = client.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "ses_1");
        let detail = client.get_session("ses_1").unwrap();
        assert_eq!(detail.title, "Refactor UI");
        assert_eq!(client.list_projects().unwrap()[0].name, "app");
    }

    #[test]
    fn unknown_sessions_return_a_typed_not_found() {
        let (_host, client) = authed_client();
        assert_eq!(
            client.get_session("missing").unwrap_err().code(),
            crate::ErrorCode::NotFound
        );
    }

    #[test]
    fn events_are_filtered_by_cursor_and_ordered() {
        let (_host, client) = authed_client();
        let batch = client.subscribe_events(Some("ses_1"), 0).unwrap();
        assert_eq!(batch.events.len(), 2);
        assert_eq!(batch.next_seq, 3);

        let later = client.subscribe_events(Some("ses_1"), 2).unwrap();
        assert_eq!(later.events.len(), 1);
        assert_eq!(later.events[0].event_id, "ev_2");
    }

    #[test]
    fn sending_the_same_prompt_twice_executes_once() {
        let (host, client) = authed_client();
        let first = client.send_prompt("ses_1", "run tests").unwrap();
        assert_eq!(first.status, CommandStatus::Running);
        assert_eq!(host.prompt_count(), 1);

        // Same logical prompt => same deterministic command id => duplicate.
        let second = client.send_prompt("ses_1", "run tests").unwrap();
        assert_eq!(second.command_id, first.command_id);
        assert_eq!(second.status, CommandStatus::Running);
        assert_eq!(host.prompt_count(), 1, "a retry must not run again");

        // A genuinely different prompt does run.
        client.send_prompt("ses_1", "now run clippy").unwrap();
        assert_eq!(host.prompt_count(), 2);
    }

    #[test]
    fn reusing_a_command_id_for_a_different_action_is_rejected() {
        let (_host, client) = authed_client();
        let receipt = client.send_prompt("ses_1", "run tests").unwrap();
        let err = client
            .cancel_session_with_id("ses_1", &receipt.command_id)
            .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::IdempotencyConflict);
    }

    #[test]
    fn an_error_frame_is_surfaced_as_a_typed_error() {
        let (host, _client) = host_and_client();
        let request = Envelope::command(Action::SessionsList, Value::Null)
            .with_message_id("msg_x")
            .with_desktop_id("desk_mock");
        // Bypass the client to make the raw exchange, which returns an error frame.
        let response = host.handle(request).unwrap();
        assert_eq!(response.kind, MessageKind::Error);
        let wire: WireError = response.payload_as().unwrap();
        assert_eq!(wire.code, "not_authorized");
    }

    #[test]
    fn malformed_requests_are_rejected_before_exchange_completes() {
        let (_host, client) = host_and_client();
        // A mutating action without a command id fails validation.
        let request = Envelope::command(Action::SessionsSendPrompt, json!({ "text": "x" }))
            .with_session_id("ses_1");
        let err = client.exchange(request).unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::MissingField);
    }

    #[test]
    fn revoking_the_active_device_terminates_its_session_and_blocks_reauthentication() {
        let (_host, client) = authed_client();
        assert_eq!(client.list_devices().unwrap().len(), 1);

        client.revoke_device("dev_1").unwrap();

        // Revocation is immediate: the active application session is dropped.
        assert_eq!(
            client.list_devices().unwrap_err().code(),
            crate::ErrorCode::NotAuthorized
        );

        // The revoked device cannot authenticate again.
        let err = client.authenticate("dev_1").unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::NotAuthorized);

        // A different device authenticates and sees the revoked one gone.
        client.authenticate("dev_2").unwrap();
        assert!(client.list_devices().unwrap().is_empty());
    }

    #[test]
    fn a_rejected_decision_is_not_remembered_as_a_successful_duplicate() {
        let (_host, client) = authed_client();
        let first = client
            .respond_approval("missing", ApprovalDecision::Deny)
            .unwrap_err();
        assert_eq!(first.code(), crate::ErrorCode::NotFound);
        // A retry of the same logical decision must fail the same way, not be
        // mistaken for a successful duplicate just because it was recorded.
        let second = client
            .respond_approval("missing", ApprovalDecision::Deny)
            .unwrap_err();
        assert_eq!(second.code(), crate::ErrorCode::NotFound);
    }

    #[test]
    fn starting_an_unknown_session_is_not_found() {
        let (_host, client) = authed_client();
        let first = client.start_session("missing").unwrap_err();
        assert_eq!(first.code(), crate::ErrorCode::NotFound);
        let second = client.start_session("missing").unwrap_err();
        assert_eq!(second.code(), crate::ErrorCode::NotFound);
    }

    #[test]
    fn approvals_flow_through_the_client_and_dedupe() {
        let (_host, client) = authed_client();
        let approvals = client.list_approvals().unwrap();
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0].id, "apr_1");

        let receipt = client
            .respond_approval("apr_1", ApprovalDecision::ApproveOnce)
            .unwrap();
        assert_eq!(receipt.status, CommandStatus::Succeeded);
        // A replay of the same decision is idempotent and does not error.
        let again = client
            .respond_approval("apr_1", ApprovalDecision::ApproveOnce)
            .unwrap();
        assert_eq!(again.command_id, receipt.command_id);
        assert_eq!(again.status, CommandStatus::Succeeded);
        assert!(client.list_approvals().unwrap().is_empty());
    }

    #[test]
    fn files_and_insights_are_available() {
        let (_host, client) = authed_client();
        let files = client.files_changed_summary("ses_1").unwrap();
        assert_eq!(files.total, 2);
        assert_eq!(files.files[0].additions, Some(10));
        let summary = client.insights_summary("/repo/app").unwrap();
        assert_eq!(summary.running_tasks, 1);
    }

    #[test]
    fn device_list_requires_authentication() {
        let (_host, client) = host_and_client();
        assert_eq!(
            client.list_devices().unwrap_err().code(),
            crate::ErrorCode::NotAuthorized
        );
    }
}
