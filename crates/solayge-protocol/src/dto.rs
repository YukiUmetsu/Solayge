//! Typed payloads exchanged over the envelope.
//!
//! Fields are `snake_case` on the wire and mirror
//! `packages/shared-types/src/index.ts`; a cross-language fixture test keeps the
//! two in sync. Payloads are deliberately small and never carry credentials,
//! private keys, environment variables, or file contents.

use serde::{Deserialize, Serialize};

use crate::status::CommandStatus;

/// A serializable error carried in a payload (`WireError` because it is the
/// wire projection of [`crate::ProtocolError`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireError {
    pub code: String,
    pub message: String,
}

impl From<&crate::ProtocolError> for WireError {
    fn from(e: &crate::ProtocolError) -> Self {
        WireError {
            code: e.code().as_str().to_string(),
            message: e.message().to_string(),
        }
    }
}

// ---- connection / hosts ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionHealth {
    pub desktop_id: String,
    pub protocol_version: u32,
    pub app_version: String,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionAuthenticatePayload {
    pub device_id: String,
    pub desktop_id: String,
    pub protocol_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionAuthenticateResult {
    pub authenticated: bool,
    pub desktop_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostMetadata {
    pub desktop_id: String,
    pub name: String,
    pub platform: String,
    pub app_version: String,
    pub protocol_version: u32,
}

// ---- projects ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub active_sessions: u32,
    pub pending_approvals: u32,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectsResponse {
    pub projects: Vec<ProjectSummary>,
}

// ---- sessions ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub agent: String,
    pub status: String,
    pub pending_decisions: u32,
    pub started_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDetail {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub agent: String,
    pub status: String,
    pub pending_decisions: u32,
    pub started_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub summary: Option<String>,
    /// The instruction the agent was given (capped), for the mobile detail view.
    #[serde(default)]
    pub prompt: Option<String>,
    /// A capped tail of the task log, so the phone can show the same run detail
    /// as the desktop without shipping the whole transcript.
    #[serde(default)]
    pub log_tail: Option<String>,
    pub files_changed: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionsResponse {
    pub sessions: Vec<SessionSummary>,
}

/// One immutable event in a session's append-only stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub seq: u64,
    pub event_id: String,
    /// Coarse event kind (e.g. `message`, `tool`, `status`).
    pub kind: String,
    pub timestamp: i64,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsSubscribePayload {
    #[serde(default)]
    pub from_seq: u64,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsBatch {
    #[serde(default)]
    pub session_id: Option<String>,
    pub events: Vec<SessionEvent>,
    pub next_seq: u64,
    /// True when history was pruned and `events` replaces the client's cursor.
    pub resync: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendPromptPayload {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandStatusQuery {
    pub command_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandReceipt {
    pub command_id: String,
    pub status: CommandStatus,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandStatusPayload {
    pub command_id: String,
    pub status: CommandStatus,
    #[serde(default)]
    pub error: Option<WireError>,
    pub updated_at: i64,
}

// ---- approvals ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    Question,
    Permission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    ApproveOnce,
    Deny,
}

/// The widget type for one agent-question field, mirroring the host's
/// `AskFieldKind`. A remote client renders it without knowing the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalFieldKind {
    String,
    Number,
    Integer,
    Boolean,
    Multiselect,
    /// Answered outside the form (for example, an OAuth URL the agent hands over).
    External,
}

/// One selectable answer for a question field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalFieldOption {
    pub value: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// One field of an agent question. For a permission request the list is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalField {
    /// The provider's key for this field; the answer payload echoes it back.
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    pub kind: ApprovalFieldKind,
    #[serde(default)]
    pub required: bool,
    /// Whether the user may answer with something other than `options`.
    #[serde(default)]
    pub custom: bool,
    #[serde(default)]
    pub options: Vec<ApprovalFieldOption>,
    #[serde(default)]
    pub placeholder: Option<String>,
    /// For an `external` field: where the user completes it.
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalSummary {
    pub id: String,
    pub session_id: String,
    pub project_id: String,
    pub kind: ApprovalKind,
    pub title: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    pub risk: ApprovalRisk,
    pub read_only: bool,
    pub requested_at: i64,
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// The structured content of an agent question. Empty for permissions.
    #[serde(default)]
    pub fields: Vec<ApprovalField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalsResponse {
    pub approvals: Vec<ApprovalSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalQuery {
    pub approval_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalResponsePayload {
    pub approval_id: String,
    pub decision: ApprovalDecision,
}

/// Answers to an agent question, keyed by field. The values are exactly what the
/// provider's form expects, so the host can forward them without reinterpretation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerPayload {
    pub approval_id: String,
    pub answers: serde_json::Map<String, serde_json::Value>,
}

/// Selects a project for a read-only insight query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectQuery {
    pub project_id: String,
}

// ---- files ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFileSummary {
    pub path: String,
    /// Short status such as `modified`, `added`, `deleted`.
    pub status: String,
    #[serde(default)]
    pub additions: Option<u32>,
    #[serde(default)]
    pub deletions: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesChangedSummary {
    pub project_id: String,
    pub session_id: String,
    pub total: u32,
    pub files: Vec<ChangedFileSummary>,
}

// ---- insights ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightsSummary {
    pub project_id: String,
    pub completed_tasks: u32,
    pub failed_tasks: u32,
    pub running_tasks: u32,
    #[serde(default)]
    pub avg_duration_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInsight {
    pub kind: String,
    pub count: u32,
    pub last_seen: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightsErrors {
    pub project_id: String,
    pub errors: Vec<ErrorInsight>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionInsight {
    pub normalized_intent: String,
    pub count: u32,
    pub last_seen: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightsAgentQuestions {
    pub project_id: String,
    pub questions: Vec<QuestionInsight>,
}

// ---- devices ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSummary {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub scopes: Vec<String>,
    pub paired_at: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicesResponse {
    pub devices: Vec<DeviceSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRevokePayload {
    pub device_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRevokeResult {
    pub revoked: bool,
}

// ---- push ----

/// Per-kind notification preferences mirrored from the phone at registration.
/// The desktop uses these to decide whether a given event kind may wake the
/// device; they are the phone's settings expressed where the push originates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushPreferences {
    /// Agent questions and permission requests.
    pub on_questions: bool,
    /// Tasks that finished successfully.
    pub on_completed: bool,
    /// Tasks that failed or errored.
    pub on_failed: bool,
}

impl Default for PushPreferences {
    /// Without an explicit choice every kind is allowed, matching the phone's
    /// defaults and preserving behavior for clients that never send them.
    fn default() -> Self {
        PushPreferences {
            on_questions: true,
            on_completed: true,
            on_failed: true,
        }
    }
}

impl PushPreferences {
    /// Whether an event of wire `kind` may be pushed to a device with these
    /// preferences. Unknown kinds are allowed so a future event is not silently
    /// dropped by an old desktop.
    pub fn allows(&self, kind: &str) -> bool {
        match kind {
            "question" | "approval" => self.on_questions,
            "task_completed" => self.on_completed,
            "task_failed" => self.on_failed,
            _ => true,
        }
    }
}

/// Register a device's push token with the desktop host. The token is opaque to
/// the host; it is forwarded to the push gateway as an addressing hint only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRegisterPayload {
    pub token: String,
    /// `android` | `ios`.
    pub platform: String,
    /// The phone's per-kind notification settings. Absent on older clients,
    /// which are treated as allowing every kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferences: Option<PushPreferences>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRegisterResult {
    pub registered: bool,
}

/// Remove a previously registered push token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushUnregisterPayload {
    pub token: String,
}

// ---- pairing (bootstrap metadata only; no reusable secrets) ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairBeginPayload {
    pub pairing_id: String,
    pub desktop_id: String,
    /// Short human-verifiable code bound to the pairing transcript.
    pub verification_code: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairIdentityProof {
    pub pairing_id: String,
    pub device_id: String,
    /// An opaque, single-use proof produced by the identity layer. The protocol
    /// crate carries it but never creates or verifies cryptographic material.
    pub proof: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairDecisionPayload {
    pub pairing_id: String,
    pub device_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairAck {
    pub pairing_id: String,
    /// `approved`, `rejected`, or `pending`.
    pub state: String,
}
