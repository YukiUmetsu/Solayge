//! Host-side permission scopes (mobile spec section 5.4).
//!
//! Scopes are granted by the desktop when a phone is approved and checked by the
//! host on **every** request. A newly paired phone defaults to
//! [`Scope::default_phone`] — View (`sessions.read`) + Message
//! (`sessions.message`) — and is never silently granted `devices.manage`.

use serde::{Deserialize, Serialize};

use crate::error::{IdentityError, Result};

/// One capability a trusted device may exercise on the desktop host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Scope {
    /// Read projects, sessions, statuses, and filtered logs.
    #[serde(rename = "sessions.read")]
    SessionsRead,
    /// Send prompts and answer agent questions.
    #[serde(rename = "sessions.message")]
    SessionsMessage,
    /// Start or cancel supported operations.
    #[serde(rename = "sessions.control")]
    SessionsControl,
    /// Accept or deny one specific agent request.
    #[serde(rename = "approvals.respond")]
    ApprovalsRespond,
    /// Manage trusted devices (revoke). Never granted to a new phone by default.
    #[serde(rename = "devices.manage")]
    DevicesManage,
}

impl Scope {
    /// Every scope, in the order the mobile spec lists them.
    pub const ALL: [Scope; 5] = [
        Scope::SessionsRead,
        Scope::SessionsMessage,
        Scope::SessionsControl,
        Scope::ApprovalsRespond,
        Scope::DevicesManage,
    ];

    /// The dotted wire string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Scope::SessionsRead => "sessions.read",
            Scope::SessionsMessage => "sessions.message",
            Scope::SessionsControl => "sessions.control",
            Scope::ApprovalsRespond => "approvals.respond",
            Scope::DevicesManage => "devices.manage",
        }
    }

    /// Parse a dotted scope, rejecting unknown strings.
    pub fn parse(s: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|scope| scope.as_str() == s)
    }

    /// Parse a list, rejecting the first unknown scope.
    pub fn parse_all(items: &[String]) -> Result<Vec<Scope>> {
        items
            .iter()
            .map(|s| {
                Scope::parse(s)
                    .ok_or_else(|| IdentityError::InvalidField(format!("unknown scope: {s}")))
            })
            .collect()
    }

    /// The default grant for a newly paired phone: View + Message only.
    pub fn default_phone() -> Vec<Scope> {
        vec![Scope::SessionsRead, Scope::SessionsMessage]
    }

    /// Whether this scope confers device-administration authority.
    pub const fn is_admin(self) -> bool {
        matches!(self, Scope::DevicesManage)
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Normalize a requested grant into the scopes that will actually be stored.
///
/// An empty request becomes the safe default rather than "no access"; duplicates
/// are removed and the canonical spec order is used so persisted grants compare
/// and display consistently.
pub fn normalize_grant(requested: &[Scope]) -> Vec<Scope> {
    let source: Vec<Scope> = if requested.is_empty() {
        Scope::default_phone()
    } else {
        requested.to_vec()
    };
    Scope::ALL
        .into_iter()
        .filter(|scope| source.contains(scope))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scope_round_trips_through_its_wire_string() {
        for scope in Scope::ALL {
            assert_eq!(Scope::parse(scope.as_str()), Some(scope));
            let json = serde_json::to_string(&scope).unwrap();
            assert_eq!(json, format!("{:?}", scope.as_str()));
            let back: Scope = serde_json::from_str(&json).unwrap();
            assert_eq!(back, scope);
        }
        assert_eq!(Scope::parse("devices.sudo"), None);
    }

    #[test]
    fn the_default_phone_grant_is_view_and_message_only() {
        let defaults = Scope::default_phone();
        assert_eq!(defaults, vec![Scope::SessionsRead, Scope::SessionsMessage]);
        assert!(!defaults.iter().any(|s| s.is_admin()));
    }

    #[test]
    fn normalization_dedupes_orders_and_defaults_empty() {
        assert_eq!(normalize_grant(&[]), Scope::default_phone());
        assert_eq!(
            normalize_grant(&[
                Scope::SessionsMessage,
                Scope::SessionsRead,
                Scope::SessionsMessage,
            ]),
            vec![Scope::SessionsRead, Scope::SessionsMessage]
        );
        // An explicit admin grant is preserved when the user asks for it.
        assert_eq!(
            normalize_grant(&[Scope::DevicesManage]),
            vec![Scope::DevicesManage]
        );
    }
}
