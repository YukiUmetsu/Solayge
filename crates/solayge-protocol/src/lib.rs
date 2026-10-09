//! `solayge-protocol` — the versioned, transport-neutral contract between a
//! Solayge desktop host and remote clients (mobile).
//!
//! This crate is intentionally small and dependency-light (only `serde` and
//! `serde_json`). It defines:
//!
//! * [`Envelope`] — one validated frame carried over any transport.
//! * [`Action`] — the versioned action namespace (section 6.1).
//! * [`CommandStatus`] — the command lifecycle, including the distinct
//!   `unknown` state.
//! * [`ProtocolError`] / [`ErrorCode`] — typed, wire-stable errors.
//! * [`command_id_for`] / [`CommandLedger`] — deterministic ids and host-side
//!   idempotency.
//! * [`RemoteClient`] plus the in-memory [`MockHost`] / [`LoopbackClient`] used
//!   by tests and local UI development.
//!
//! The crate never implements cryptography: pairing payloads carry opaque proof
//! material produced elsewhere.
//!
//! # Example
//!
//! ```
//! use solayge_protocol::{Action, Envelope, MessageKind};
//! use serde_json::json;
//!
//! let frame = Envelope::command(Action::ConnectionHealth, json!({}))
//!     .with_message_id("msg_1");
//! let bytes = frame.encode().unwrap();
//! let decoded = Envelope::decode(&bytes).unwrap();
//! assert_eq!(decoded.kind, MessageKind::Command);
//! assert_eq!(decoded.action, Action::ConnectionHealth);
//! ```
#![deny(unsafe_code)]

pub mod actions;
pub mod client;
pub mod dto;
pub mod envelope;
pub mod error;
pub mod idempotency;
pub mod status;

pub use actions::Action;
pub use client::{LoopbackClient, MockHost, RemoteClient};
pub use dto::*;
pub use envelope::{
    next_message_id, Envelope, MessageKind, MAX_FRAME_BYTES, MAX_ID_LEN, MAX_PAYLOAD_BYTES,
    PROTOCOL_VERSION,
};
pub use error::{ErrorCode, ProtocolError};
pub use idempotency::{command_id_for, command_id_with_nonce, CommandLedger, CommandObservation};
pub use status::CommandStatus;
