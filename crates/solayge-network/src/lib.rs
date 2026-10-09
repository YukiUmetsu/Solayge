//! `solayge-network` — the Iroh-encrypted transport and the connection manager
//! state machine for Solayge remote clients (mobile spec sections 4, 5.1, 5.2).
//!
//! The crate is split into two cooperating layers:
//!
//! * [`iroh_adapter`] owns every Iroh detail. It binds endpoints with explicit
//!   discovery and relay configuration, exchanges length-bounded
//!   [`solayge_protocol::Envelope`] frames over QUIC, and reports honest
//!   diagnostics. It also provides [`iroh_adapter::IrohRemoteClient`], a
//!   blocking implementation of [`solayge_protocol::RemoteClient`].
//! * [`manager`] implements the section 5.2 state machine over an injectable
//!   [`manager::Transport`], with capped exponential backoff, jitter, bounded
//!   retry budgets, a single active session, and the hard security rule that an
//!   authentication/revocation denial is terminal and never falls back.
//!
//! No cryptography or NAT traversal is implemented here: both are provided by
//! the maintained `iroh` crate.
//!
//! # Iroh version and platform support
//!
//! This crate targets **iroh 1.3** (released 2026-09-28) and matches its 1.x
//! `Endpoint`/`Connection`/`presets` API. Iroh 1.3 supports macOS, Windows, and
//! Linux as well as Android and iOS, but each platform has a caveat worth
//! recording before shipping:
//!
//! * **Android:** the default DNS resolver reads the system configuration over
//!   JNI and needs a JVM context published to `ndk_context` before the endpoint
//!   is bound (`iroh::dns::install_android_jni_context`). Without it, iroh
//!   falls back to fallback nameservers — and panics if the app is built with
//!   `panic = "abort"`. Android also does not surface network changes natively;
//!   call `Endpoint::network_change` when the app observes one.
//! * **iOS:** no extra setup is required, but iOS suspends background apps, so
//!   the caller must reconnect on foreground rather than relying on a live
//!   QUIC socket (mobile spec section 9).
//!
//! Tailscale is intentionally **out of scope** here. The connection manager
//! models the `Fallback` state and accepts a fallback [`manager::TransportFactory`],
//! but no Tailscale adapter is implemented in this crate.
#![deny(unsafe_code)]

// Re-exported so downstream host/client crates can name protocol types without
// adding a second dependency on `solayge-protocol`.
pub use solayge_protocol;
// The endpoint secret-key type, so callers can persist a stable transport key
// without depending on `iroh` themselves.
pub use iroh::SecretKey;

pub mod diagnostics;
pub mod error;
pub mod frame;
pub mod iroh_adapter;
pub mod manager;
pub mod tailscale;
pub mod websocket;

pub use diagnostics::{ConnectionDiagnostics, EndpointType, PathKind, TransportDiagnostics};
pub use error::{ErrorClass, NetworkError};
pub use iroh_adapter::{
    connect_by_endpoint_id, endpoint_addr_from_id, secret_key_from_seed, DiscoveryConfig,
    FrameHandler, IrohConfig, IrohConnection, IrohRemoteClient, IrohTransport, IrohTransportFactory,
    RelayConfig, DEFAULT_EXCHANGE_TIMEOUT, SOLAYGE_ALPN,
};
pub use iroh_adapter::ConnectionSession;
pub use manager::{
    Backoff, BackoffPolicy, BoxFuture, ConnectionManager, ConnectionState, Delay, Jitter,
    SyncReport, TokioDelay, Transport, TransportFactory,
};
pub use tailscale::{
    CommandOutput, CommandRunner, ServeStatus, ServeStopOutcome, SystemCommandRunner, TailscaleCli,
    TailscaleError, TailscaleStatus,
};
pub use websocket::{WsClientConfig, WsConnection, WsHost, WsHostConfig, WsRemoteClient, WsTransportFactory};
