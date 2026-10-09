//! Deterministic command ids and idempotent command handling.
//!
//! Idempotency is a correctness requirement, not an optimization: a retry after
//! a dropped response must not start a second agent run. Callers derive a stable
//! [`command_id_for`] from the command's scope and payload, and hosts record it
//! in a [`CommandLedger`] before dispatch. A repeated id with the same action is
//! a harmless duplicate; a repeated id with a *different* action is a conflict.

use std::collections::{HashMap, VecDeque};

use crate::actions::Action;
use crate::error::ProtocolError;
use crate::status::CommandStatus;

/// 128-bit FNV-1a. This is a stable, non-cryptographic fingerprint for deriving
/// idempotency keys; it is never used for authentication or secrets.
fn fnv1a_128(bytes: &[u8]) -> u128 {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= *byte as u128;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Derive a deterministic, stable command id from the command's scope and
/// payload. Retrying the exact same logical command yields the same id; changing
/// any input yields a different id. `serde_json`'s default map ordering is
/// sorted, so payload key order does not affect the result.
pub fn command_id_for(
    desktop_id: &str,
    session_id: &str,
    action: Action,
    payload: &serde_json::Value,
) -> String {
    let canonical_payload = serde_json::to_string(payload).unwrap_or_default();
    let canonical = format!(
        "{desktop_id}\u{1f}{session_id}\u{1f}{}\u{1f}{canonical_payload}",
        action.as_str()
    );
    format!("cmd_{:032x}", fnv1a_128(canonical.as_bytes()))
}

/// Add an explicit nonce to a deterministic command id, for deliberately
/// sending two otherwise-identical commands.
pub fn command_id_with_nonce(base: &str, nonce: impl std::fmt::Display) -> String {
    format!("{base}_{nonce}")
}

/// What a host learned when it registered an incoming command id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandObservation {
    /// First time this id was seen.
    New,
    /// Already seen with the same action; return the recorded status without
    /// re-executing.
    Duplicate { status: CommandStatus },
}

#[derive(Debug, Clone)]
struct Entry {
    action: Action,
    status: CommandStatus,
}

/// A bounded, in-memory record of command ids and their statuses.
///
/// It is the host-side idempotency primitive. It is intentionally small and
/// replaceable: persistent hosts can back the same API with their store.
pub struct CommandLedger {
    entries: HashMap<String, Entry>,
    /// Insertion order, used to evict the oldest terminal entry at capacity.
    order: VecDeque<String>,
    max_entries: usize,
}

impl Default for CommandLedger {
    fn default() -> Self {
        CommandLedger::new(1024)
    }
}

impl CommandLedger {
    pub fn new(max_entries: usize) -> Self {
        CommandLedger {
            entries: HashMap::new(),
            order: VecDeque::new(),
            max_entries: max_entries.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, command_id: &str) -> Option<CommandStatus> {
        self.entries.get(command_id).map(|e| e.status)
    }

    pub fn action_of(&self, command_id: &str) -> Option<Action> {
        self.entries.get(command_id).map(|e| e.action)
    }

    /// Remove a command id. Used to roll back a registration when the command
    /// is rejected before dispatch (for example, a decision for an approval
    /// that no longer exists), so a retry is retried rather than mistaken for a
    /// successful duplicate.
    pub fn forget(&mut self, command_id: &str) -> bool {
        if self.entries.remove(command_id).is_some() {
            self.order.retain(|id| id != command_id);
            true
        } else {
            false
        }
    }

    /// Register an incoming command. Idempotent by `command_id`.
    pub fn register(
        &mut self,
        command_id: &str,
        action: Action,
    ) -> Result<CommandObservation, ProtocolError> {
        if let Some(existing) = self.entries.get(command_id) {
            if existing.action != action {
                return Err(ProtocolError::idempotency_conflict(format!(
                    "command {command_id} was already used for {}",
                    existing.action.as_str()
                )));
            }
            return Ok(CommandObservation::Duplicate {
                status: existing.status,
            });
        }

        self.evict_for_room()?;
        self.entries.insert(
            command_id.to_string(),
            Entry {
                action,
                status: CommandStatus::Created,
            },
        );
        self.order.push_back(command_id.to_string());
        Ok(CommandObservation::New)
    }

    /// Advance a command's status, rejecting illegal transitions.
    pub fn advance(
        &mut self,
        command_id: &str,
        status: CommandStatus,
    ) -> Result<CommandStatus, ProtocolError> {
        let entry = self
            .entries
            .get_mut(command_id)
            .ok_or_else(|| ProtocolError::not_found(command_id))?;
        if !entry.status.can_transition_to(status) {
            return Err(ProtocolError::invalid_transition(
                entry.status.wire_name(),
                status.wire_name(),
            ));
        }
        entry.status = status;
        Ok(status)
    }

    /// Make room by dropping the oldest terminal entry. If every entry is still
    /// pending, the ledger is exhausted and refuses the new command rather than
    /// silently forgetting an in-flight one.
    fn evict_for_room(&mut self) -> Result<(), ProtocolError> {
        while self.entries.len() >= self.max_entries {
            let Some(candidate) = self.order.iter().find(|id| {
                self.entries
                    .get(*id)
                    .map(|e| e.status.is_terminal())
                    .unwrap_or(false)
            }) else {
                return Err(ProtocolError::resource_exhausted(format!(
                    "idempotency ledger is full ({} pending commands)",
                    self.entries.len()
                )));
            };
            let candidate = candidate.clone();
            self.order.retain(|id| id != &candidate);
            self.entries.remove(&candidate);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_ids_are_deterministic_and_sensitive_to_every_input() {
        let payload = json!({ "text": "run tests" });
        let base = command_id_for("desk", "ses", Action::SessionsSendPrompt, &payload);
        assert_eq!(
            base,
            command_id_for("desk", "ses", Action::SessionsSendPrompt, &payload)
        );
        assert_ne!(
            base,
            command_id_for("other", "ses", Action::SessionsSendPrompt, &payload)
        );
        assert_ne!(
            base,
            command_id_for("desk", "other", Action::SessionsSendPrompt, &payload)
        );
        assert_ne!(
            base,
            command_id_for("desk", "ses", Action::SessionsCancel, &payload)
        );
        assert_ne!(
            base,
            command_id_for(
                "desk",
                "ses",
                Action::SessionsSendPrompt,
                &json!({ "text": "x" })
            )
        );
        // Object key order must not matter.
        let reordered = json!({ "b": 2, "a": 1 });
        let a = command_id_for("d", "s", Action::SessionsStart, &json!({ "a": 1, "b": 2 }));
        let b = command_id_for("d", "s", Action::SessionsStart, &reordered);
        assert_eq!(a, b);
    }

    #[test]
    fn nonce_produces_a_distinct_id() {
        let base = command_id_for("d", "s", Action::SessionsSendPrompt, &json!({"text":"x"}));
        assert_ne!(base, command_id_with_nonce(&base, 2));
    }

    #[test]
    fn registering_the_same_command_twice_is_a_duplicate() {
        let mut ledger = CommandLedger::default();
        assert_eq!(
            ledger
                .register("cmd_1", Action::SessionsSendPrompt)
                .unwrap(),
            CommandObservation::New
        );
        ledger.advance("cmd_1", CommandStatus::Running).unwrap();
        assert_eq!(
            ledger
                .register("cmd_1", Action::SessionsSendPrompt)
                .unwrap(),
            CommandObservation::Duplicate {
                status: CommandStatus::Running
            }
        );
        assert_eq!(
            ledger.len(),
            1,
            "a duplicate must not create a second entry"
        );
    }

    #[test]
    fn reusing_a_command_id_for_another_action_is_a_conflict() {
        let mut ledger = CommandLedger::default();
        ledger
            .register("cmd_1", Action::SessionsSendPrompt)
            .unwrap();
        let err = ledger
            .register("cmd_1", Action::SessionsCancel)
            .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::IdempotencyConflict);
    }

    #[test]
    fn illegal_status_transitions_are_rejected() {
        let mut ledger = CommandLedger::default();
        ledger
            .register("cmd_1", Action::SessionsSendPrompt)
            .unwrap();
        ledger.advance("cmd_1", CommandStatus::Succeeded).unwrap();
        let err = ledger.advance("cmd_1", CommandStatus::Running).unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::InvalidTransition);
        // Unknown is missing entirely.
        assert_eq!(
            ledger
                .advance("missing", CommandStatus::Running)
                .unwrap_err()
                .code(),
            crate::ErrorCode::NotFound
        );
    }

    #[test]
    fn unknown_outcomes_can_be_resolved_but_not_resumed() {
        let mut ledger = CommandLedger::default();
        ledger
            .register("cmd_1", Action::SessionsSendPrompt)
            .unwrap();
        ledger.advance("cmd_1", CommandStatus::Unknown).unwrap();
        assert!(ledger.advance("cmd_1", CommandStatus::Running).is_err());
        ledger.advance("cmd_1", CommandStatus::Succeeded).unwrap();
        assert_eq!(ledger.get("cmd_1"), Some(CommandStatus::Succeeded));
    }

    #[test]
    fn forgetting_a_command_frees_its_slot_and_allows_a_retry() {
        let mut ledger = CommandLedger::new(2);
        ledger.register("a", Action::SessionsSendPrompt).unwrap();
        assert!(ledger.forget("a"));
        assert!(!ledger.forget("a"), "forgetting twice is a no-op");
        assert!(ledger.get("a").is_none());

        // The freed slot is reusable even though nothing was terminal.
        ledger.register("b", Action::SessionsCancel).unwrap();
        ledger.register("c", Action::SessionsCancel).unwrap();
        assert_eq!(ledger.len(), 2);
    }

    #[test]
    fn the_ledger_evicts_oldest_terminal_entries_and_refuses_when_full_of_pending() {
        let mut ledger = CommandLedger::new(2);
        ledger.register("a", Action::SessionsSendPrompt).unwrap();
        ledger.register("b", Action::SessionsSendPrompt).unwrap();
        ledger.advance("a", CommandStatus::Succeeded).unwrap();
        // Room is made by evicting the finished `a`.
        assert_eq!(
            ledger.register("c", Action::SessionsSendPrompt).unwrap(),
            CommandObservation::New
        );
        assert!(ledger.get("a").is_none());
        // Both remaining are pending; a fourth is refused.
        let err = ledger
            .register("d", Action::SessionsSendPrompt)
            .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::ResourceExhausted);
    }
}
