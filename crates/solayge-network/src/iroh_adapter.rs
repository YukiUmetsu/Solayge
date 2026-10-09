//! Iroh adapter: endpoint lifecycle, framed request/response, and diagnostics.
//!
//! This module is the only place that touches Iroh. It:
//!
//! * creates endpoints for the desktop host and the mobile client with
//!   **explicit, configurable** discovery and relay settings — there is no
//!   hidden relay or discovery setup, and `RelayConfig::Disabled` means exactly
//!   that (no implied relay-free reachability guarantee);
//! * moves length-bounded [`Envelope`] frames over QUIC bi-directional streams,
//!   one exchange per stream so a slow or malformed request cannot block others;
//! * reports diagnostics honestly: direct vs relayed only when Iroh reports a
//!   selected path, plus RTT, endpoint type, peer-identity verification, and the
//!   last error.
//!
//! It never implements cryptography or NAT traversal: both come from Iroh.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{presets, RecvStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey};
use serde::de::DeserializeOwned;
use serde::Serialize;
use solayge_protocol::{
    Action, ConnectionAuthenticatePayload, ConnectionAuthenticateResult, Envelope, ErrorCode,
    EventsBatch, EventsSubscribePayload, MessageKind, ProtocolError, WireError, PROTOCOL_VERSION,
};

use crate::diagnostics::{EndpointType, PathKind, TransportDiagnostics};
use crate::error::NetworkError;
use crate::frame::{read_frame, write_frame};
use crate::manager::{SyncReport, Transport};
use crate::BoxFuture;

/// The ALPN both Solayge endpoints must negotiate.
pub const SOLAYGE_ALPN: &[u8] = b"solayge/1";

/// Default deadline for one request/response exchange. A peer that completes
/// the QUIC handshake but never answers must not stall the connection manager
/// past this bound; the failure is retryable and (when configured) eligible for
/// fallback, because it is a connectivity problem rather than an authorization
/// denial.
pub const DEFAULT_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);

/// Explicit address-discovery configuration. Off by default when the caller
/// constructs a loopback config; production configs opt in deliberately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryConfig {
    /// No address lookup service. Peers must be dialed with explicit addresses.
    Disabled,
    /// The public n0 DNS/pkarr discovery service. Free and rate-limited; it has
    /// no uptime guarantee (`docs/MOBILE_FEATURES.md` section 5.1).
    N0Dns,
}

impl DiscoveryConfig {
    pub const fn as_str(self) -> &'static str {
        match self {
            DiscoveryConfig::Disabled => "disabled",
            DiscoveryConfig::N0Dns => "n0_dns",
        }
    }

    /// Parse an explicit discovery spec. `""`/`n0` selects the public n0 DNS
    /// service; `disabled`/`none`/`off` disables it. Unknown values are an
    /// error rather than a silent default.
    pub fn parse(spec: &str) -> Result<DiscoveryConfig, String> {
        match spec.trim() {
            "" | "n0" | "n0_dns" => Ok(DiscoveryConfig::N0Dns),
            "disabled" | "none" | "off" => Ok(DiscoveryConfig::Disabled),
            other => Err(format!("unknown discovery mode '{other}'")),
        }
    }
}

/// Explicit relay configuration. `Disabled` is not a promise of direct
/// connectivity: it simply means no relay is used, so a direct path must exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayConfig {
    /// No relay servers at all.
    Disabled,
    /// The public n0 production relays (rate-limited, no SLA).
    N0Default,
    /// The n0 staging relays, for testing.
    Staging,
    /// A caller-provided relay list.
    Custom(Vec<iroh::RelayUrl>),
}

impl RelayConfig {
    pub fn as_str(&self) -> String {
        match self {
            RelayConfig::Disabled => "disabled".to_string(),
            RelayConfig::N0Default => "n0_default".to_string(),
            RelayConfig::Staging => "n0_staging".to_string(),
            RelayConfig::Custom(urls) => format!("custom({})", urls.len()),
        }
    }

    fn to_mode(&self) -> RelayMode {
        match self {
            RelayConfig::Disabled => RelayMode::Disabled,
            RelayConfig::N0Default => RelayMode::Default,
            RelayConfig::Staging => RelayMode::Staging,
            RelayConfig::Custom(urls) => RelayMode::custom(urls.clone()),
        }
    }

    /// Parse an explicit relay spec: `disabled`/`none`, `default`/`n0`,
    /// `staging`, or a comma-separated list of relay URLs.
    pub fn parse(spec: &str) -> Result<RelayConfig, String> {
        let spec = spec.trim();
        match spec {
            "disabled" | "none" => Ok(RelayConfig::Disabled),
            "default" | "n0" => Ok(RelayConfig::N0Default),
            "staging" => Ok(RelayConfig::Staging),
            other => {
                let mut urls = Vec::new();
                for part in other.split(',') {
                    let part = part.trim();
                    if part.is_empty() {
                        continue;
                    }
                    let url = part
                        .parse::<iroh::RelayUrl>()
                        .map_err(|e| format!("invalid relay URL '{part}': {e}"))?;
                    urls.push(url);
                }
                if urls.is_empty() {
                    return Err("no relay URLs provided".to_string());
                }
                Ok(RelayConfig::Custom(urls))
            }
        }
    }
}

/// Endpoint configuration. Every knob is explicit; nothing is inferred.
#[derive(Clone)]
pub struct IrohConfig {
    pub role: EndpointType,
    pub discovery: DiscoveryConfig,
    pub relay: RelayConfig,
    /// UDP bind address. Use `127.0.0.1:0` for tests.
    pub bind_addr: SocketAddr,
    pub alpns: Vec<Vec<u8>>,
    /// Optional stable endpoint identity. Generated per endpoint when absent.
    pub secret_key: Option<SecretKey>,
}

impl std::fmt::Debug for IrohConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IrohConfig")
            .field("role", &self.role)
            .field("discovery", &self.discovery)
            .field("relay", &self.relay)
            .field("bind_addr", &self.bind_addr)
            .field("alpns", &self.alpns.len())
            .field("secret_key", &self.secret_key.is_some())
            .finish()
    }
}

impl IrohConfig {
    /// A production configuration using the public n0 discovery and relay
    /// services. These are free and rate-limited with no uptime guarantee; a
    /// production deployment must select a funded relay/discovery plan.
    pub fn production(role: EndpointType) -> Self {
        IrohConfig {
            role,
            discovery: DiscoveryConfig::N0Dns,
            relay: RelayConfig::N0Default,
            bind_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            alpns: vec![SOLAYGE_ALPN.to_vec()],
            secret_key: None,
        }
    }

    /// A self-contained configuration for tests and local loopback: direct
    /// addresses only, no discovery, no relay.
    pub fn loopback(role: EndpointType) -> Self {
        IrohConfig {
            role,
            discovery: DiscoveryConfig::Disabled,
            relay: RelayConfig::Disabled,
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            alpns: vec![SOLAYGE_ALPN.to_vec()],
            secret_key: None,
        }
    }

    pub fn with_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.bind_addr = addr;
        self
    }

    pub fn with_secret_key(mut self, key: SecretKey) -> Self {
        self.secret_key = Some(key);
        self
    }
}

/// Build a bound Iroh endpoint from an explicit config.
async fn build_endpoint(config: &IrohConfig) -> Result<Endpoint, NetworkError> {
    let mut builder = Endpoint::builder(presets::Minimal);
    if let Some(key) = &config.secret_key {
        builder = builder.secret_key(key.clone());
    }
    builder = builder.relay_mode(config.relay.to_mode());
    builder = builder.alpns(config.alpns.clone());
    if config.discovery == DiscoveryConfig::N0Dns {
        // The same lookups the n0 preset installs, added explicitly so the
        // configuration above is the single source of truth.
        builder = builder
            .address_lookup(iroh::address_lookup::PkarrPublisher::n0_dns())
            .address_lookup(iroh::address_lookup::PkarrResolver::n0_dns())
            .address_lookup(iroh::address_lookup::DnsAddressLookup::n0_dns());
    }
    builder = builder
        .bind_addr(config.bind_addr)
        .map_err(|e| NetworkError::internal(format!("invalid Iroh bind address: {e}")))?;
    builder
        .bind()
        .await
        .map_err(|e| NetworkError::connectivity(format!("failed to bind Iroh endpoint: {e}")))
}

/// A bound Iroh endpoint that can dial and accept connections.
pub struct IrohTransport {
    endpoint: Endpoint,
    role: EndpointType,
    relay_label: String,
    discovery_label: &'static str,
}

impl IrohTransport {
    pub async fn bind(config: IrohConfig) -> Result<Self, NetworkError> {
        let endpoint = build_endpoint(&config).await?;
        Ok(IrohTransport {
            endpoint,
            role: config.role,
            relay_label: config.relay.as_str(),
            discovery_label: config.discovery.as_str(),
        })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint.id()
    }

    pub fn endpoint_addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    pub fn bound_sockets(&self) -> Vec<SocketAddr> {
        self.endpoint.bound_sockets()
    }

    /// The first loopback socket this endpoint is bound to, if any. Useful for
    /// local tests regardless of the configured bind address.
    pub fn loopback_socket(&self) -> Option<SocketAddr> {
        self.bound_sockets()
            .into_iter()
            .find(|addr| addr.ip().is_loopback())
    }

    pub fn role(&self) -> EndpointType {
        self.role
    }

    pub fn relay_label(&self) -> &str {
        &self.relay_label
    }

    pub fn discovery_label(&self) -> &'static str {
        self.discovery_label
    }

    /// Dial a peer with the Solayge ALPN. The expected endpoint id from `peer`
    /// is remembered so [`IrohConnection::transport_identity_verified`] can be a
    /// real comparison rather than a restatement of the remote id.
    pub async fn dial(&self, peer: EndpointAddr) -> Result<IrohConnection, NetworkError> {
        let expected = peer.id;
        let connection = self
            .endpoint
            .connect(peer, SOLAYGE_ALPN)
            .await
            .map_err(|e| NetworkError::connectivity(format!("dial failed: {e}")))?;
        Ok(self.wrap(connection, Some(expected)))
    }

    /// Accept the next incoming connection, or `None` when the endpoint closes.
    pub async fn accept(&self) -> Result<Option<IrohConnection>, NetworkError> {
        let incoming = match self.endpoint.accept().await {
            Some(incoming) => incoming,
            None => return Ok(None),
        };
        let connection = incoming
            .await
            .map_err(|e| NetworkError::connectivity(format!("accept failed: {e}")))?;
        // An inbound connection has no pre-agreed expected identity until the
        // application authenticates it, so verification is honestly unknown.
        Ok(Some(self.wrap(connection, None)))
    }

    fn wrap(
        &self,
        connection: iroh::endpoint::Connection,
        expected_remote: Option<EndpointId>,
    ) -> IrohConnection {
        // Bind the application session to the peer's transport endpoint id, so
        // pairing and auth proofs are scoped to this connection.
        let session = ConnectionSession::new(format!("iroh:{}", connection.remote_id()));
        IrohConnection {
            connection,
            endpoint: self.endpoint.clone(),
            role: self.role,
            expected_remote,
            session,
            last_error: Mutex::new(None),
        }
    }

    /// Accept connections forever, serving each on its own task. Each
    /// connection's application session is independent; authorization is
    /// enforced per-request by the [`FrameHandler`].
    ///
    /// A failed incoming handshake is routine over UDP (stray packets, ALPN
    /// mismatch, an aborted dial) and is logged and skipped: it must never tear
    /// down the listener. The loop only ends when the endpoint itself closes.
    pub async fn serve(
        self: &Arc<Self>,
        handler: Arc<dyn FrameHandler>,
    ) -> Result<(), NetworkError> {
        loop {
            let Some(incoming) = self.endpoint.accept().await else {
                return Ok(());
            };
            let connection = match incoming.await {
                Ok(connection) => connection,
                Err(error) => {
                    // Never propagate a per-handshake failure: doing so would
                    // let any reachable peer permanently stop the host.
                    eprintln!("solayge-network: rejected incoming Iroh connection: {error}");
                    continue;
                }
            };
            let connection = self.wrap(connection, None);
            let handler = handler.clone();
            tokio::spawn(async move {
                if let Err(error) = connection.serve(handler).await {
                    // A per-connection failure must not tear down the listener.
                    eprintln!("solayge-network: iroh connection ended: {error}");
                }
            });
        }
    }

    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}

/// Per-connection authentication state handed to a [`FrameHandler`].
///
/// The transport is deliberately identity-agnostic: it proves which *endpoint*
/// key is on the other side, not which Solayge device. This cell lets the
/// application layer bind an authenticated device id to exactly one connection,
/// so a request can never be authorized by a device that authenticated on a
/// different socket. The [`FrameHandler`] must clear or replace it if the
/// transport re-authenticates.
pub struct ConnectionSession {
    authenticated_device: Mutex<Option<String>>,
    transport_binding: String,
}

impl ConnectionSession {
    /// A fresh, unauthenticated session bound to a transport-level identifier
    /// (the remote endpoint id). The binding is signed into pairing and auth
    /// transcripts so a proof cannot be replayed onto another connection.
    pub fn new(transport_binding: impl Into<String>) -> Self {
        ConnectionSession {
            authenticated_device: Mutex::new(None),
            transport_binding: transport_binding.into(),
        }
    }

    /// The transport binding for this connection.
    pub fn transport_binding(&self) -> &str {
        &self.transport_binding
    }

    /// The authenticated device id, if the handshake has completed.
    pub fn authenticated_device(&self) -> Option<String> {
        self.authenticated_device
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Mark this connection as authenticated for `device_id`.
    pub fn set_authenticated_device(&self, device_id: impl Into<String>) {
        *self
            .authenticated_device
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(device_id.into());
    }

    /// Drop the authentication (for example when the device is revoked).
    pub fn clear_authenticated_device(&self) {
        *self
            .authenticated_device
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }
}

/// Authorization-agnostic frame handler used by the host listener and tests.
///
/// The [`ConnectionSession`] is per connection, so a handler can enforce its own
/// per-connection authorization without the transport knowing anything about
/// Solayge identities.
pub trait FrameHandler: Send + Sync + 'static {
    fn handle<'a>(
        &'a self,
        session: &'a ConnectionSession,
        request: Envelope,
    ) -> BoxFuture<'a, Envelope>;
}

/// One established, QUIC-authenticated Iroh connection.
pub struct IrohConnection {
    connection: iroh::endpoint::Connection,
    endpoint: Endpoint,
    role: EndpointType,
    /// The endpoint id we expected to reach, when we dialed with one. `None`
    /// for inbound connections, whose identity is only established by the
    /// application-level authentication that follows.
    expected_remote: Option<EndpointId>,
    /// The per-connection application session (authenticated device id).
    session: ConnectionSession,
    last_error: Mutex<Option<String>>,
}

impl IrohConnection {
    pub fn remote_id(&self) -> EndpointId {
        self.connection.remote_id()
    }

    pub fn expected_remote_id(&self) -> Option<EndpointId> {
        self.expected_remote
    }

    /// True only when we dialed an explicit endpoint id and the QUIC/TLS peer
    /// identity matched it. Inbound connections report `false` until the
    /// application authenticates the peer.
    pub fn transport_identity_verified(&self) -> bool {
        self.expected_remote == Some(self.connection.remote_id())
    }

    pub fn is_connected(&self) -> bool {
        self.connection.close_reason().is_none()
    }

    /// Send one request and read one reply on a fresh bi-directional stream,
    /// bounded by [`DEFAULT_EXCHANGE_TIMEOUT`] so a peer that completes the
    /// handshake and then stalls cannot hang the connection manager forever.
    pub async fn exchange(&self, request: &Envelope) -> Result<Envelope, NetworkError> {
        self.exchange_with_timeout(request, DEFAULT_EXCHANGE_TIMEOUT)
            .await
    }

    /// [`IrohConnection::exchange`] with an explicit deadline. A request that
    /// does not complete in time is reported as a retryable connectivity
    /// failure, never as an authorization outcome.
    pub async fn exchange_with_timeout(
        &self,
        request: &Envelope,
        timeout: Duration,
    ) -> Result<Envelope, NetworkError> {
        match tokio::time::timeout(timeout, self.exchange_inner(request)).await {
            Ok(result) => result,
            Err(_) => Err(self.fail(format!("request timed out after {timeout:?}"))),
        }
    }

    async fn exchange_inner(&self, request: &Envelope) -> Result<Envelope, NetworkError> {
        let bytes = request.encode().map_err(NetworkError::Protocol)?;
        let (mut send, mut recv) = self
            .connection
            .open_bi()
            .await
            .map_err(|e| self.fail(format!("open stream failed: {e}")))?;
        write_frame(&mut send, &bytes).await?;
        let response = read_frame(&mut recv).await?;
        Envelope::decode(&response).map_err(NetworkError::Protocol)
    }

    /// Serve request/response exchanges until the connection closes.
    pub async fn serve(&self, handler: Arc<dyn FrameHandler>) -> Result<(), NetworkError> {
        loop {
            let (mut send, mut recv) = match self.connection.accept_bi().await {
                Ok(stream) => stream,
                Err(e) => {
                    // A peer closing the connection is normal termination, not
                    // a failure to log.
                    if self.connection.close_reason().is_some() {
                        return Ok(());
                    }
                    return Err(self.fail(format!("connection closed: {e}")));
                }
            };
            let result = self.handle_stream(&mut send, &mut recv, &handler).await;
            if let Err(error) = result {
                // Record and keep serving other streams; one bad frame is not a
                // reason to drop a working connection.
                self.record(&error);
            }
        }
    }

    async fn handle_stream(
        &self,
        send: &mut iroh::endpoint::SendStream,
        recv: &mut RecvStream,
        handler: &Arc<dyn FrameHandler>,
    ) -> Result<(), NetworkError> {
        let request_bytes = read_frame(recv).await?;
        let request = Envelope::decode(&request_bytes).map_err(NetworkError::Protocol)?;
        let response = handler.handle(&self.session, request).await;
        let response_bytes = response.encode().map_err(NetworkError::Protocol)?;
        write_frame(send, &response_bytes).await
    }

    /// Honest diagnostics for this connection.
    pub fn diagnostics(&self) -> TransportDiagnostics {
        let remote = self.connection.remote_id();
        let mut path = PathKind::Unknown;
        let mut rtt = None;
        for candidate in self.connection.paths().iter() {
            if candidate.is_selected() {
                if candidate.is_ip() {
                    path = PathKind::Direct;
                } else if candidate.is_relay() {
                    path = PathKind::Relayed;
                }
                rtt = Some(candidate.rtt());
                break;
            }
        }
        TransportDiagnostics {
            endpoint_id: self.endpoint.id().to_string(),
            remote_id: Some(remote.to_string()),
            endpoint_type: self.role,
            path,
            rtt,
            transport_identity_verified: self.expected_remote == Some(self.connection.remote_id()),
            last_error: self.last_error.lock().ok().and_then(|guard| guard.clone()),
        }
    }

    pub async fn close(&self) {
        self.connection.close(0u8.into(), b"done");
    }

    pub async fn closed(&self) {
        let _ = self.connection.closed().await;
    }

    fn record(&self, error: &NetworkError) {
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = Some(error.to_string());
        }
    }

    fn fail(&self, message: String) -> NetworkError {
        let error = NetworkError::connectivity(message);
        self.record(&error);
        error
    }
}

/// A factory that binds a fresh endpoint and dials a fixed peer. A fresh
/// endpoint per attempt avoids reusing a torn-down socket on reconnect.
pub struct IrohTransportFactory {
    config: IrohConfig,
    peer: EndpointAddr,
}

impl IrohTransportFactory {
    pub fn new(config: IrohConfig, peer: EndpointAddr) -> Self {
        IrohTransportFactory { config, peer }
    }
}

impl super::manager::TransportFactory for IrohTransportFactory {
    fn connect<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Transport>, NetworkError>> {
        Box::pin(async move {
            let transport = IrohTransport::bind(self.config.clone()).await?;
            let connection = transport.dial(self.peer.clone()).await?;
            Ok(Arc::new(connection) as Arc<dyn Transport>)
        })
    }

    fn label(&self) -> &'static str {
        "iroh"
    }
}

fn typed<T: Serialize>(value: &T) -> Result<serde_json::Value, NetworkError> {
    serde_json::to_value(value).map_err(|e| NetworkError::internal(e.to_string()))
}

fn parse_response<T: DeserializeOwned>(response: Envelope) -> Result<T, NetworkError> {
    if response.kind == MessageKind::Error {
        let wire: WireError = response.payload_as().map_err(NetworkError::Protocol)?;
        if wire.code == ErrorCode::NotAuthorized.as_str() {
            return Err(NetworkError::auth_denied(wire.message));
        }
        let code = ErrorCode::parse(&wire.code).unwrap_or(ErrorCode::Internal);
        return Err(NetworkError::Protocol(ProtocolError::new(
            code,
            wire.message,
        )));
    }
    response.payload_as().map_err(NetworkError::Protocol)
}

impl Transport for IrohConnection {
    fn authenticate<'a>(
        &'a self,
        device_id: &'a str,
    ) -> BoxFuture<'a, Result<ConnectionAuthenticateResult, NetworkError>> {
        Box::pin(async move {
            let payload = typed(&ConnectionAuthenticatePayload {
                device_id: device_id.to_string(),
                desktop_id: String::new(),
                protocol_version: PROTOCOL_VERSION,
            })?;
            let request = Envelope::command(Action::ConnectionAuthenticate, payload);
            let result: ConnectionAuthenticateResult =
                parse_response(self.exchange(&request).await?)?;
            if !result.authenticated {
                return Err(NetworkError::auth_denied(
                    "host reported the device as not authenticated",
                ));
            }
            Ok(result)
        })
    }

    fn sync<'a>(&'a self) -> BoxFuture<'a, Result<SyncReport, NetworkError>> {
        Box::pin(async move {
            let payload = typed(&EventsSubscribePayload {
                from_seq: 0,
                session_id: None,
            })?;
            let request = Envelope::command(Action::SessionsEventsSubscribe, payload);
            let batch: EventsBatch = parse_response(self.exchange(&request).await?)?;
            Ok(SyncReport {
                cursor: batch.next_seq,
                resync: batch.resync,
            })
        })
    }

    fn is_connected(&self) -> bool {
        IrohConnection::is_connected(self)
    }

    fn close<'a>(&'a self) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            IrohConnection::close(self).await;
        })
    }

    fn diagnostics(&self) -> TransportDiagnostics {
        IrohConnection::diagnostics(self)
    }
}

// ---------------------------------------------------------------------------
// Sync `RemoteClient` bridge
// ---------------------------------------------------------------------------

struct Job {
    request: Envelope,
    reply: std::sync::mpsc::Sender<Result<Envelope, ProtocolError>>,
}

/// A blocking [`solayge_protocol::RemoteClient`] over an Iroh connection.
///
/// Iroh is asynchronous while the protocol's client trait is synchronous. This
/// bridge runs the endpoint and its I/O on a dedicated OS thread with its own
/// single-threaded Tokio runtime; [`RemoteClient::exchange`] hands the request
/// to that worker and blocks on a plain channel, so it is safe to call from a
/// synchronous context (and does not deadlock a caller's runtime).
pub struct IrohRemoteClient {
    tx: Option<tokio::sync::mpsc::UnboundedSender<Job>>,
    worker: Option<std::thread::JoinHandle<()>>,
    endpoint_id: String,
    desktop_id: Option<String>,
}

impl IrohRemoteClient {
    /// Bind an endpoint with `config`, dial `peer`, and start the worker.
    pub fn connect(config: IrohConfig, peer: EndpointAddr) -> Result<Self, NetworkError> {
        let endpoint_id_hint = peer.id;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<EndpointId, NetworkError>>();
        let worker = std::thread::Builder::new()
            .name("solayge-iroh-client".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        let _ = ready_tx.send(Err(NetworkError::internal(format!(
                            "failed to start Iroh client runtime: {e}"
                        ))));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let transport = match IrohTransport::bind(config).await {
                        Ok(transport) => transport,
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                            return;
                        }
                    };
                    let connection = match transport.dial(peer).await {
                        Ok(connection) => connection,
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(connection.remote_id()));
                    while let Some(job) = rx.recv().await {
                        let result = connection
                            .exchange(&job.request)
                            .await
                            .map_err(|e| e.to_protocol_error());
                        let _ = job.reply.send(result);
                    }
                    connection.close().await;
                });
            })
            .map_err(|e| NetworkError::internal(format!("failed to spawn Iroh client: {e}")))?;

        let remote = ready_rx
            .recv()
            .map_err(|_| NetworkError::internal("Iroh client worker exited early"))??;
        Ok(IrohRemoteClient {
            tx: Some(tx),
            worker: Some(worker),
            endpoint_id: remote.to_string(),
            desktop_id: Some(endpoint_id_hint.to_string()),
        })
    }

    /// The remote endpoint id this client dialed.
    pub fn remote_endpoint_id(&self) -> &str {
        &self.endpoint_id
    }
}

/// Parse a published endpoint id string into a dial target. Addresses are left
/// empty so the configured discovery service resolves them; a relay/direct hint
/// is not invented here.
pub fn endpoint_addr_from_id(id: &str) -> Result<EndpointAddr, String> {
    let id: EndpointId = id
        .parse()
        .map_err(|e| format!("invalid Iroh endpoint id '{id}': {e}"))?;
    Ok(EndpointAddr::new(id))
}

/// Build a stable endpoint secret key from 32 persisted bytes, so a host keeps
/// the same endpoint id across restarts and a paired phone can still dial it.
pub fn secret_key_from_seed(seed: &[u8; 32]) -> SecretKey {
    SecretKey::from_bytes(seed)
}

/// [`IrohRemoteClient::connect`] from a published endpoint id string, resolving
/// addresses through the configured discovery service.
pub fn connect_by_endpoint_id(config: IrohConfig, endpoint_id: &str) -> Result<IrohRemoteClient, NetworkError> {
    let peer = endpoint_addr_from_id(endpoint_id)
        .map_err(NetworkError::internal)?;
    IrohRemoteClient::connect(config, peer)
}

impl solayge_protocol::RemoteClient for IrohRemoteClient {
    fn exchange(&self, request: Envelope) -> Result<Envelope, ProtocolError> {
        let Some(tx) = self.tx.as_ref() else {
            return Err(ProtocolError::transport("Iroh client worker is stopped"));
        };
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        tx.send(Job {
            request,
            reply: reply_tx,
        })
        .map_err(|_| ProtocolError::transport("Iroh client worker is stopped"))?;
        reply_rx
            .recv()
            .unwrap_or_else(|_| Err(ProtocolError::transport("Iroh client dropped the reply")))
    }

    fn desktop_id(&self) -> Option<&str> {
        self.desktop_id.as_deref()
    }

    fn is_connected(&self) -> bool {
        self.tx.is_some()
    }
}

impl Drop for IrohRemoteClient {
    fn drop(&mut self) {
        // Close the request channel first so the worker's `recv` ends, then
        // wait for it to close the QUIC connection cleanly.
        self.tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_specs_are_parsed_explicitly_and_reject_unknown_values() {
        assert_eq!(
            RelayConfig::parse("disabled").unwrap(),
            RelayConfig::Disabled
        );
        assert_eq!(RelayConfig::parse("n0").unwrap(), RelayConfig::N0Default);
        assert_eq!(RelayConfig::parse("staging").unwrap(), RelayConfig::Staging);
        let custom = RelayConfig::parse("https://relay.example.com.").unwrap();
        match custom {
            RelayConfig::Custom(urls) => assert_eq!(urls.len(), 1),
            other => panic!("expected a custom relay list, got {other:?}"),
        }
        assert!(RelayConfig::parse("not a url at all").is_err());
        assert!(RelayConfig::parse("  ").is_err());
    }

    #[test]
    fn discovery_specs_are_parsed_explicitly_and_reject_unknown_values() {
        assert_eq!(
            DiscoveryConfig::parse("n0_dns").unwrap(),
            DiscoveryConfig::N0Dns
        );
        assert_eq!(
            DiscoveryConfig::parse("off").unwrap(),
            DiscoveryConfig::Disabled
        );
        assert!(DiscoveryConfig::parse("maybe").is_err());
    }

    #[test]
    fn loopback_config_is_direct_only_and_production_config_is_explicit() {
        let loopback = IrohConfig::loopback(EndpointType::MobileClient);
        assert_eq!(loopback.discovery, DiscoveryConfig::Disabled);
        assert_eq!(loopback.relay, RelayConfig::Disabled);
        assert!(loopback.bind_addr.ip().is_loopback());

        let production = IrohConfig::production(EndpointType::DesktopHost);
        assert_eq!(production.discovery, DiscoveryConfig::N0Dns);
        assert_eq!(production.relay, RelayConfig::N0Default);
    }
}
