// Cross-language contract test.
//
// `packages/shared-types/fixtures/protocol-sample.json` is the shared fixture:
// this test proves it deserializes into these Rust DTOs *and* re-serializes back
// to exactly the same JSON. The TypeScript package has a mirror test that
// asserts the same fixture's field names. Together they catch drift in either
// language (added/removed/renamed fields).
use serde::Deserialize;
use solayge_protocol::*;

#[derive(Debug, Deserialize, serde::Serialize)]
struct Sample {
    envelope: Envelope,
    connection_health: ConnectionHealth,
    connection_authenticate_payload: ConnectionAuthenticatePayload,
    connection_authenticate_result: ConnectionAuthenticateResult,
    host_metadata: HostMetadata,
    project_summary: ProjectSummary,
    session_summary: SessionSummary,
    session_detail: SessionDetail,
    session_event: SessionEvent,
    events_subscribe_payload: EventsSubscribePayload,
    events_batch: EventsBatch,
    send_prompt_payload: SendPromptPayload,
    command_status_query: CommandStatusQuery,
    command_receipt: CommandReceipt,
    command_status: CommandStatusPayload,
    wire_error: WireError,
    approval_summary: ApprovalSummary,
    approval_field: ApprovalField,
    approval_query: ApprovalQuery,
    approval_response_payload: ApprovalResponsePayload,
    answer_payload: AnswerPayload,
    project_query: ProjectQuery,
    changed_file_summary: ChangedFileSummary,
    files_changed_summary: FilesChangedSummary,
    insights_summary: InsightsSummary,
    error_insight: ErrorInsight,
    insights_errors: InsightsErrors,
    question_insight: QuestionInsight,
    insights_agent_questions: InsightsAgentQuestions,
    device_summary: DeviceSummary,
    devices_response: DevicesResponse,
    device_revoke_payload: DeviceRevokePayload,
    device_revoke_result: DeviceRevokeResult,
    push_register_payload: PushRegisterPayload,
    push_register_result: PushRegisterResult,
    push_unregister_payload: PushUnregisterPayload,
    pair_begin_payload: PairBeginPayload,
    pair_identity_proof: PairIdentityProof,
    pair_decision_payload: PairDecisionPayload,
    pair_ack: PairAck,
}

const FIXTURE: &str = include_str!("../../../packages/shared-types/fixtures/protocol-sample.json");

#[test]
fn the_shared_fixture_matches_the_rust_dtos_exactly() {
    let original: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture is valid JSON");
    let sample: Sample = serde_json::from_str(FIXTURE).expect("fixture matches the Rust DTOs");

    // Spot-check that the fixture exercises non-trivial values.
    assert_eq!(sample.envelope.action, Action::SessionsSendPrompt);
    assert_eq!(sample.envelope.protocol_version, PROTOCOL_VERSION);
    assert_eq!(sample.command_receipt.status, CommandStatus::Running);
    assert_eq!(sample.command_status.status, CommandStatus::Succeeded);
    assert_eq!(sample.approval_summary.kind, ApprovalKind::Permission);
    assert!(sample.approval_summary.fields.is_empty());
    assert_eq!(sample.approval_field.kind, ApprovalFieldKind::String);
    assert_eq!(sample.approval_field.options.len(), 2);
    assert_eq!(
        sample
            .answer_payload
            .answers
            .get("color")
            .and_then(|v| v.as_str()),
        Some("blue")
    );
    assert_eq!(
        sample.approval_response_payload.decision,
        ApprovalDecision::ApproveOnce
    );
    assert_eq!(sample.events_batch.events.len(), 1);
    assert_eq!(sample.session_detail.files_changed, 2);
    assert_eq!(sample.devices_response.devices.len(), 1);

    // Re-serializing must produce exactly the fixture: a removed or renamed
    // field would show up as a difference.
    let round_tripped = serde_json::to_value(&sample).expect("sample serializes");
    assert_eq!(
        round_tripped, original,
        "the fixture and the DTOs have drifted"
    );
}
