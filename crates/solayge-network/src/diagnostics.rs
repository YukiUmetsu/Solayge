//! Honest connection diagnostics.
//!
//! The mobile spec (section 7.2.F) requires showing *what is actually true*:
//! endpoint type, direct vs relayed **when verifiable**, RTT, authentication
//! state, and the last error. It explicitly forbids a "fake encrypted
//! checkmark" inferred only from an attempted connection.
//!
//! To honor that, [`PathKind`] is [`PathKind::Unknown`] until Iroh reports a
//! selected path, and [`TransportDiagnostics::transport_identity_verified`]
//! reflects the QUIC/TLS peer identity only — never the Solayge application
//! authorization. The application-level flag lives on
//! [`ConnectionDiagnostics::solayge_authenticated`] and is set solely by the
//! manager after a successful `connection.authenticate`.

use std::time::Duration;

use crate::manager::ConnectionState;

/// Which side of the connection this endpoint plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointType {
    /// Desktop host that accepts remote clients.
    DesktopHost,
    /// Mobile client that dials a desktop host.
    MobileClient,
}

impl EndpointType {
    pub const fn as_str(self) -> &'static str {
        match self {
            EndpointType::DesktopHost => "desktop_host",
            EndpointType::MobileClient => "mobile_client",
        }
    }
}

/// How application traffic is currently carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// A direct IP path is selected.
    Direct,
    /// The selected path is carried by a relay.
    Relayed,
    /// Iroh has not reported a selected path; do not guess.
    Unknown,
}

impl PathKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            PathKind::Direct => "direct",
            PathKind::Relayed => "relayed",
            PathKind::Unknown => "unknown",
        }
    }
}

/// Transport-level diagnostics for one established Iroh connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportDiagnostics {
    /// This endpoint's public id.
    pub endpoint_id: String,
    /// The peer's QUIC-authenticated public id, when connected.
    pub remote_id: Option<String>,
    /// Whether this endpoint is the host or the client.
    pub endpoint_type: EndpointType,
    /// Direct, relayed, or honestly unknown.
    pub path: PathKind,
    /// Round-trip time of the selected path, when available.
    pub rtt: Option<Duration>,
    /// True only when the QUIC/TLS peer identity matched the expected endpoint
    /// id. This is transport authentication, not Solayge authorization.
    pub transport_identity_verified: bool,
    /// The last transport error observed on this connection, if any.
    pub last_error: Option<String>,
}

impl TransportDiagnostics {
    /// A conservative, all-unknown placeholder used before a connection exists.
    pub fn unknown(endpoint_type: EndpointType) -> Self {
        TransportDiagnostics {
            endpoint_id: String::new(),
            remote_id: None,
            endpoint_type,
            path: PathKind::Unknown,
            rtt: None,
            transport_identity_verified: false,
            last_error: None,
        }
    }
}

/// The full picture the UI may present, spanning transport and application auth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionDiagnostics {
    pub state: ConnectionState,
    /// Transport label (`"iroh"`) once a transport is active.
    pub transport: Option<String>,
    pub endpoint_id: Option<String>,
    pub remote_id: Option<String>,
    pub endpoint_type: Option<EndpointType>,
    pub path: PathKind,
    pub rtt: Option<Duration>,
    /// QUIC/TLS peer identity verified. Not an encryption badge by itself.
    pub transport_identity_verified: bool,
    /// Solayge application authentication succeeded. Only then may the UI show
    /// a trusted/authorized checkmark.
    pub solayge_authenticated: bool,
    /// True when the active session is carried by the configured fallback
    /// transport rather than the primary one.
    pub on_fallback: bool,
    pub attempts: u32,
    pub next_retry_in: Option<Duration>,
    pub last_error: Option<String>,
}

impl ConnectionDiagnostics {
    pub fn offline() -> Self {
        ConnectionDiagnostics {
            state: ConnectionState::Offline,
            transport: None,
            endpoint_id: None,
            remote_id: None,
            endpoint_type: None,
            path: PathKind::Unknown,
            rtt: None,
            transport_identity_verified: false,
            solayge_authenticated: false,
            on_fallback: false,
            attempts: 0,
            next_retry_in: None,
            last_error: None,
        }
    }
}
