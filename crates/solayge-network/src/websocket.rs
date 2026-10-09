//! WebSocket transport for the optional Tailscale-served host listener.
//!
//! Tailscale terminates TLS for the tailnet and reverse-proxies an HTTPS URL to
//! a **loopback-only** plain WebSocket listener. This module provides both ends:
//!
//! * [`WsHost`] binds the loopback listener, upgrades each connection, and hands
//!   every frame to the same [`FrameHandler`] the Iroh host uses. Authorization is
//!   therefore identical across transports: a connection is only authorized after
//!   the application-level challenge-response binds a device id to it.
//! * [`WsConnection`] / [`WsRemoteClient`] are the mobile-side client used for
//!   the Tailscale fallback. The `wss://` certificate is verified with the Web PKI
//!   roots; the Solayge peer identity is verified at the application layer.
//!
//! Frames are the unchanged [`Envelope`] JSON carried as binary WebSocket
//! messages. Incremental sync and command deduplication are properties of the
//! protocol/host handler, not of the transport, so every transport shares them.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use solayge_protocol::{
    Action, ConnectionAuthenticatePayload, ConnectionAuthenticateResult, Envelope,
    EventsBatch, EventsSubscribePayload, MAX_FRAME_BYTES, PROTOCOL_VERSION,
};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

use crate::diagnostics::{EndpointType, PathKind, TransportDiagnostics};
use crate::error::NetworkError;
use crate::iroh_adapter::{parse_response, typed, ConnectionSession, FrameHandler};
use crate::manager::{SyncReport, Transport, TransportFactory};
use crate::BoxFuture;

/// The default deadline for one request/response exchange over WebSocket.
pub const DEFAULT_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);

/// Where the loopback WebSocket listener binds. Only `127.0.0.1`/`::1` are
/// accepted; the listener is never exposed directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WsHostConfig {
    pub bind_addr: SocketAddr,
}

impl WsHostConfig {
    /// Bind an ephemeral port on IPv4 loopback.
    pub fn loopback() -> Self {
        WsHostConfig {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        }
    }
}

impl Default for WsHostConfig {
    fn default() -> Self {
        WsHostConfig::loopback()
    }
}

/// The split stream type for an accepted connection.
type WsStream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Decrements the live-client counter when a connection task ends.
struct ClientGuard(Arc<AtomicUsize>);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A running loopback WebSocket listener. Dropping it does not stop the accept
/// loop; call [`WsHost::shutdown`].
#[derive(Debug)]
pub struct WsHost {
    local_addr: SocketAddr,
    clients: Arc<AtomicUsize>,
    shutdown: watch::Sender<bool>,
    accept_task: tokio::task::JoinHandle<()>,
}

impl WsHost {
    /// Bind and start accepting connections. Every accepted connection is served
    /// on its own task by `handler`.
    pub async fn bind(
        config: WsHostConfig,
        handler: Arc<dyn FrameHandler>,
    ) -> Result<Self, NetworkError> {
        if !config.bind_addr.ip().is_loopback() {
            return Err(NetworkError::internal(format!(
                "the WebSocket listener must bind loopback, not {}",
                config.bind_addr
            )));
        }
        let listener = TcpListener::bind(config.bind_addr)
            .await
            .map_err(|e| NetworkError::connectivity(format!("failed to bind WebSocket listener: {e}")))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| NetworkError::internal(format!("WebSocket listener has no address: {e}")))?;

        let clients = Arc::new(AtomicUsize::new(0));
        let (shutdown, shutdown_rx) = watch::channel(false);
        let accept_clients = clients.clone();
        let accept_task = tokio::spawn(accept_loop(listener, handler, accept_clients, shutdown_rx));

        Ok(WsHost {
            local_addr,
            clients,
            shutdown,
            accept_task,
        })
    }

    /// The bound loopback address, including the ephemeral port.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The number of currently connected clients (including unauthenticated
    /// sockets that have completed the WebSocket handshake).
    pub fn client_count(&self) -> usize {
        self.clients.load(Ordering::SeqCst)
    }

    /// A shareable handle to the live-client counter, so an owner can read the
    /// count without holding this host behind a lock.
    pub fn client_counter(&self) -> Arc<AtomicUsize> {
        self.clients.clone()
    }

    /// Stop accepting connections, ask every live connection to close, and wait
    /// for the accept loop to finish. Idempotent.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        // Wake the select so the loop notices even if it is parked on accept.
        let _ = self.accept_task.await;
    }
}

async fn accept_loop(
    listener: TcpListener,
    handler: Arc<dyn FrameHandler>,
    clients: Arc<AtomicUsize>,
    mut shutdown: watch::Receiver<bool>,
) {
    if *shutdown.borrow() {
        return;
    }
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer)) => {
                        // Reject anything that arrived after shutdown.
                        if *shutdown.borrow() {
                            break;
                        }
                        clients.fetch_add(1, Ordering::SeqCst);
                        let handler = handler.clone();
                        let clients = clients.clone();
                        let conn_shutdown = shutdown.clone();
                        tokio::spawn(async move {
                            let _guard = ClientGuard(clients);
                            if let Err(error) =
                                serve_connection(stream, peer, handler, conn_shutdown).await
                            {
                                // A single bad connection must never stop the listener.
                                eprintln!(
                                    "solayge-network: websocket connection from {peer} ended: {error}"
                                );
                            }
                        });
                    }
                    Err(error) => {
                        eprintln!("solayge-network: websocket accept failed: {error}");
                        // A persistent error (for example fd exhaustion) must
                        // not turn the accept loop into a hot spin.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
        }
    }
}

async fn serve_connection(
    stream: tokio::net::TcpStream,
    peer: SocketAddr,
    handler: Arc<dyn FrameHandler>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), NetworkError> {
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(|e| NetworkError::connectivity(format!("websocket handshake failed: {e}")))?;
    let (mut sink, mut source) = ws.split();
    // Bind the app session to the peer socket; the host's handler replaces it
    // with the authenticated device id after the challenge-response.
    let session = ConnectionSession::new(format!("ws:{peer}"));

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            }
            message = source.next() => {
                let Some(message) = message else { break };
                let message = message.map_err(|e| {
                    NetworkError::connectivity(format!("websocket read failed: {e}"))
                })?;
                match message {
                    Message::Binary(bytes) => {
                        if bytes.len() > MAX_FRAME_BYTES {
                            send_error(&mut sink, "frame exceeds the maximum size").await?;
                            continue;
                        }
                        let request = match Envelope::decode(&bytes) {
                            Ok(request) => request,
                            Err(error) => {
                                send_error(&mut sink, &error.to_string()).await?;
                                continue;
                            }
                        };
                        let response = handler.handle(&session, request).await;
                        let encoded = response.encode().map_err(NetworkError::Protocol)?;
                        sink.send(Message::Binary(encoded))
                            .await
                            .map_err(|e| NetworkError::connectivity(format!("websocket write failed: {e}")))?;
                    }
                    Message::Text(_) => {
                        send_error(&mut sink, "the host accepts only binary frames").await?;
                    }
                    Message::Ping(payload) => {
                        let _ = sink.send(Message::Pong(payload)).await;
                    }
                    Message::Pong(_) | Message::Frame(_) => {}
                    Message::Close(_) => break,
                }
            }
        }
    }
    Ok(())
}

/// Send a typed error frame that does not depend on a well-formed request.
async fn send_error<S>(sink: &mut S, message: &str) -> Result<(), NetworkError>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let request = Envelope::command(Action::ConnectionHealth, serde_json::Value::Null);
    let error = solayge_protocol::ProtocolError::invalid_frame(message);
    let response = Envelope::error_to(&request, &error);
    let encoded = response.encode().map_err(NetworkError::Protocol)?;
    sink.send(Message::Binary(encoded))
        .await
        .map_err(|e| NetworkError::connectivity(format!("websocket write failed: {e}")))
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Client configuration for the Tailscale WebSocket fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsClientConfig {
    /// `wss://host.tailnet.ts.net[:port]/` in production.
    pub url: String,
    pub role: EndpointType,
    /// Allow a plaintext `ws://` endpoint. Only permitted for loopback hosts;
    /// used by tests and never for a remote Tailscale endpoint.
    pub allow_insecure_loopback: bool,
}

impl WsClientConfig {
    /// A secure `wss://` client for `url`.
    pub fn wss(url: impl Into<String>) -> Self {
        WsClientConfig {
            url: url.into(),
            role: EndpointType::MobileClient,
            allow_insecure_loopback: false,
        }
    }

    /// A plaintext loopback client, for tests and same-machine tooling.
    pub fn loopback(url: impl Into<String>) -> Self {
        WsClientConfig {
            url: url.into(),
            role: EndpointType::MobileClient,
            allow_insecure_loopback: true,
        }
    }
}

/// Ensure a rustls crypto provider is installed before the first TLS handshake.
/// Iroh installs the ring provider; this is idempotent and tolerates an
/// already-installed provider.
fn install_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Validate the endpoint URL: `wss://` always, or `ws://` only for loopback.
pub fn validate_endpoint_url(url: &str, allow_insecure_loopback: bool) -> Result<(), NetworkError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| NetworkError::internal(format!("invalid WebSocket URL '{url}'")))?;
    if rest.is_empty() {
        return Err(NetworkError::internal(format!("invalid WebSocket URL '{url}'")));
    }
    match scheme {
        "wss" => Ok(()),
        "ws" => {
            if !allow_insecure_loopback {
                return Err(NetworkError::internal(
                    "plaintext ws:// is only allowed for loopback endpoints",
                ));
            }
            let host = url_host(rest);
            if is_loopback_host(host) {
                Ok(())
            } else {
                Err(NetworkError::internal(format!(
                    "plaintext ws:// is only allowed for loopback endpoints, not '{host}'"
                )))
            }
        }
        other => Err(NetworkError::internal(format!(
            "unsupported WebSocket scheme '{other}'"
        ))),
    }
}

fn url_host(rest: &str) -> &str {
    if let Some(stripped) = rest.strip_prefix('[') {
        return stripped.split(']').next().unwrap_or("");
    }
    rest.split(['/', ':']).next().unwrap_or("")
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// The default deadline for establishing the WebSocket (and TLS) connection.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// One live WebSocket connection.
pub struct WsConnection {
    stream: tokio::sync::Mutex<WsStream>,
    endpoint: String,
    role: EndpointType,
    connected: AtomicBool,
    last_error: Mutex<Option<String>>,
}

impl WsConnection {
    /// Validate the URL, connect, and return a live connection. Bounded by
    /// [`DEFAULT_CONNECT_TIMEOUT`] so an unreachable fallback endpoint cannot
    /// hang a dial indefinitely.
    pub async fn connect(config: &WsClientConfig) -> Result<Self, NetworkError> {
        validate_endpoint_url(&config.url, config.allow_insecure_loopback)?;
        install_crypto_provider();
        let (stream, _response) = tokio::time::timeout(
            DEFAULT_CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async(config.url.as_str()),
        )
        .await
        .map_err(|_| {
            NetworkError::connectivity(format!(
                "websocket connect timed out after {DEFAULT_CONNECT_TIMEOUT:?}"
            ))
        })?
        .map_err(|e| NetworkError::connectivity(format!("websocket connect failed: {e}")))?;
        Ok(WsConnection {
            stream: tokio::sync::Mutex::new(stream),
            endpoint: config.url.clone(),
            role: config.role,
            connected: AtomicBool::new(true),
            last_error: Mutex::new(None),
        })
    }

    /// Send one request and read one reply, bounded by [`DEFAULT_EXCHANGE_TIMEOUT`].
    pub async fn exchange(&self, request: &Envelope) -> Result<Envelope, NetworkError> {
        match tokio::time::timeout(DEFAULT_EXCHANGE_TIMEOUT, self.exchange_inner(request)).await {
            Ok(result) => result,
            Err(_) => Err(self.fail(format!(
                "request timed out after {DEFAULT_EXCHANGE_TIMEOUT:?}"
            ))),
        }
    }

    async fn exchange_inner(&self, request: &Envelope) -> Result<Envelope, NetworkError> {
        let bytes = request.encode().map_err(NetworkError::Protocol)?;
        let mut stream = self.stream.lock().await;
        stream
            .send(Message::Binary(bytes))
            .await
            .map_err(|e| self.fail(format!("websocket write failed: {e}")))?;
        loop {
            match stream.next().await {
                Some(Ok(Message::Binary(bytes))) => {
                    return Envelope::decode(&bytes).map_err(NetworkError::Protocol);
                }
                Some(Ok(Message::Ping(payload))) => {
                    let _ = stream.send(Message::Pong(payload)).await;
                }
                Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                Some(Ok(Message::Text(_))) => continue,
                Some(Ok(Message::Close(_))) | None => {
                    self.connected.store(false, Ordering::SeqCst);
                    return Err(self.fail("the WebSocket connection closed".to_string()));
                }
                Some(Err(error)) => {
                    self.connected.store(false, Ordering::SeqCst);
                    return Err(self.fail(format!("websocket read failed: {error}")));
                }
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub async fn close(&self) {
        self.connected.store(false, Ordering::SeqCst);
        let mut stream = self.stream.lock().await;
        let _ = stream.send(Message::Close(None)).await;
    }

    pub fn diagnostics(&self) -> TransportDiagnostics {
        TransportDiagnostics {
            endpoint_id: self.endpoint.clone(),
            remote_id: Some(self.endpoint.clone()),
            endpoint_type: self.role,
            // A single WebSocket hop carries a direct, usually-loopback path;
            // Tailscale's own relay choice is not visible to Solayge.
            path: PathKind::Direct,
            rtt: None,
            // The TLS certificate is verified, but the *Solayge* peer identity
            // is established by the application handshake that follows.
            transport_identity_verified: false,
            last_error: self.last_error.lock().ok().and_then(|guard| guard.clone()),
        }
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

impl Transport for WsConnection {
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
        WsConnection::is_connected(self)
    }

    fn close<'a>(&'a self) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            WsConnection::close(self).await;
        })
    }

    fn diagnostics(&self) -> TransportDiagnostics {
        WsConnection::diagnostics(self)
    }
}

/// Creates a fresh [`WsConnection`] per connection attempt.
pub struct WsTransportFactory {
    config: WsClientConfig,
}

impl WsTransportFactory {
    pub fn new(config: WsClientConfig) -> Self {
        WsTransportFactory { config }
    }
}

impl TransportFactory for WsTransportFactory {
    fn connect<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Transport>, NetworkError>> {
        Box::pin(async move {
            let connection = WsConnection::connect(&self.config).await?;
            Ok(Arc::new(connection) as Arc<dyn Transport>)
        })
    }

    fn label(&self) -> &'static str {
        "tailscale_ws"
    }
}

// ---------------------------------------------------------------------------
// Sync `RemoteClient` bridge
// ---------------------------------------------------------------------------

struct Job {
    request: Envelope,
    reply: std::sync::mpsc::Sender<Result<Envelope, solayge_protocol::ProtocolError>>,
}

/// A blocking [`solayge_protocol::RemoteClient`] over a WebSocket connection,
/// mirroring [`crate::iroh_adapter::IrohRemoteClient`]: the async I/O runs on a
/// dedicated OS thread with its own single-threaded Tokio runtime, while
/// [`RemoteClient::exchange`] blocks on a plain channel.
pub struct WsRemoteClient {
    tx: Option<tokio::sync::mpsc::UnboundedSender<Job>>,
    worker: Option<std::thread::JoinHandle<()>>,
    endpoint: String,
    desktop_id: Option<String>,
}

impl WsRemoteClient {
    /// Connect the worker to `config.url` and return once the handshake succeeds.
    pub fn connect(config: WsClientConfig) -> Result<Self, NetworkError> {
        let endpoint = config.url.clone();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), NetworkError>>();
        let worker = std::thread::Builder::new()
            .name("solayge-ws-client".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        let _ = ready_tx.send(Err(NetworkError::internal(format!(
                            "failed to start WebSocket client runtime: {e}"
                        ))));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let connection = match WsConnection::connect(&config).await {
                        Ok(connection) => connection,
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(()));
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
            .map_err(|e| NetworkError::internal(format!("failed to spawn WebSocket client: {e}")))?;

        ready_rx
            .recv()
            .map_err(|_| NetworkError::internal("WebSocket client worker exited early"))??;
        Ok(WsRemoteClient {
            tx: Some(tx),
            worker: Some(worker),
            endpoint,
            desktop_id: None,
        })
    }

    /// The endpoint URL this client dialed.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

impl solayge_protocol::RemoteClient for WsRemoteClient {
    fn exchange(&self, request: Envelope) -> Result<Envelope, solayge_protocol::ProtocolError> {
        let Some(tx) = self.tx.as_ref() else {
            return Err(solayge_protocol::ProtocolError::transport(
                "WebSocket client worker is stopped",
            ));
        };
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        tx.send(Job {
            request,
            reply: reply_tx,
        })
        .map_err(|_| solayge_protocol::ProtocolError::transport("WebSocket client worker is stopped"))?;
        reply_rx.recv().unwrap_or_else(|_| {
            Err(solayge_protocol::ProtocolError::transport(
                "WebSocket client dropped the reply",
            ))
        })
    }

    fn desktop_id(&self) -> Option<&str> {
        self.desktop_id.as_deref()
    }

    fn is_connected(&self) -> bool {
        self.tx.is_some()
    }
}

impl Drop for WsRemoteClient {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    use solayge_protocol::{ErrorCode, HostMetadata, MessageKind};

    /// A handler that answers health, requires authentication for metadata, and
    /// tracks how many connections authenticated.
    struct TestHandler {
        authenticated: AtomicU32,
    }

    impl TestHandler {
        fn new() -> Self {
            TestHandler {
                authenticated: AtomicU32::new(0),
            }
        }
    }

    impl FrameHandler for TestHandler {
        fn handle<'a>(
            &'a self,
            session: &'a ConnectionSession,
            request: Envelope,
        ) -> BoxFuture<'a, Envelope> {
            Box::pin(async move {
                if request.kind != MessageKind::Command {
                    return Envelope::error_to(
                        &request,
                        &solayge_protocol::ProtocolError::invalid_frame("command only"),
                    );
                }
                match request.action {
                    Action::ConnectionHealth => {
                        Envelope::response_to(&request, serde_json::json!({ "ok": true }))
                    }
                    Action::ConnectionAuthenticate => {
                        session.set_authenticated_device("dev_test");
                        self.authenticated.fetch_add(1, Ordering::SeqCst);
                        Envelope::response_to(
                            &request,
                            serde_json::to_value(ConnectionAuthenticateResult {
                                authenticated: true,
                                desktop_id: "desk_test".to_string(),
                            })
                            .unwrap(),
                        )
                    }
                    Action::HostsGetMetadata => match session.authenticated_device() {
                        Some(_) => Envelope::response_to(
                            &request,
                            serde_json::to_value(HostMetadata {
                                desktop_id: "desk_test".to_string(),
                                name: "Test".to_string(),
                                platform: "macos".to_string(),
                                app_version: "0".to_string(),
                                protocol_version: PROTOCOL_VERSION,
                            })
                            .unwrap(),
                        ),
                        None => Envelope::error_to(
                            &request,
                            &solayge_protocol::ProtocolError::not_authorized("authenticate first"),
                        ),
                    },
                    _ => Envelope::error_to(
                        &request,
                        &solayge_protocol::ProtocolError::unsupported_action("nope"),
                    ),
                }
            })
        }
    }

    async fn host() -> (WsHost, Arc<TestHandler>) {
        let handler = Arc::new(TestHandler::new());
        let host = WsHost::bind(WsHostConfig::loopback(), handler.clone())
            .await
            .expect("bind");
        (host, handler)
    }

    fn loopback_url(host: &WsHost) -> String {
        format!("ws://{}", host.local_addr())
    }

    #[tokio::test]
    async fn host_exchange_round_trips_an_envelope() {
        let (host, _handler) = host().await;
        let connection = WsConnection::connect(&WsClientConfig::loopback(loopback_url(&host)))
            .await
            .unwrap();
        let request = Envelope::command(Action::ConnectionHealth, serde_json::Value::Null);
        let response = connection.exchange(&request).await.unwrap();
        assert_eq!(response.kind, MessageKind::Response);
        assert_eq!(response.message_id, request.message_id);
        connection.close().await;
        host.shutdown().await;
    }

    #[tokio::test]
    async fn the_listener_refuses_a_non_loopback_bind() {
        let handler = Arc::new(TestHandler::new());
        let config = WsHostConfig {
            bind_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
        };
        let error = WsHost::bind(config, handler).await.unwrap_err();
        assert!(matches!(error, NetworkError::Internal(_)));
    }

    #[tokio::test]
    async fn the_host_counts_multiple_clients_and_forgets_them_on_close() {
        let (host, _handler) = host().await;
        let url = loopback_url(&host);
        let a = WsConnection::connect(&WsClientConfig::loopback(&url)).await.unwrap();
        let b = WsConnection::connect(&WsClientConfig::loopback(&url)).await.unwrap();
        // Allow the accept loop to observe both connections.
        for _ in 0..50 {
            if host.client_count() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(host.client_count(), 2);
        a.close().await;
        for _ in 0..50 {
            if host.client_count() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(host.client_count(), 1);
        drop(b);
        host.shutdown().await;
    }

    #[tokio::test]
    async fn authenticate_binds_the_device_and_unauthorized_reads_are_denied() {
        let (host, handler) = host().await;
        let url = loopback_url(&host);

        // Two independent connections: authentication is per-connection.
        let authed = WsConnection::connect(&WsClientConfig::loopback(&url)).await.unwrap();
        let anonymous = WsConnection::connect(&WsClientConfig::loopback(&url)).await.unwrap();

        let metadata = Envelope::command(Action::HostsGetMetadata, serde_json::Value::Null);
        let denied = anonymous.exchange(&metadata).await.unwrap();
        assert_eq!(denied.kind, MessageKind::Error);

        <WsConnection as Transport>::authenticate(&authed, "dev_test")
            .await
            .unwrap();
        let allowed = authed.exchange(&metadata).await.unwrap();
        assert_eq!(allowed.kind, MessageKind::Response);
        assert_eq!(handler.authenticated.load(Ordering::SeqCst), 1);

        // Closing and reconnecting yields a fresh, unauthenticated session.
        authed.close().await;
        let reconnect = WsConnection::connect(&WsClientConfig::loopback(&url)).await.unwrap();
        let denied_again = reconnect.exchange(&metadata).await.unwrap();
        assert_eq!(denied_again.kind, MessageKind::Error);

        host.shutdown().await;
    }

    #[tokio::test]
    async fn a_malformed_frame_gets_a_typed_error_instead_of_closing_the_listener() {
        let (host, _handler) = host().await;
        let (mut raw, _) = tokio_tungstenite::connect_async(loopback_url(&host))
            .await
            .unwrap();
        raw.send(Message::Binary(b"{not json".to_vec())).await.unwrap();
        let reply = raw.next().await.unwrap().unwrap();
        let Message::Binary(bytes) = reply else {
            panic!("expected a binary error frame, got {reply:?}");
        };
        let envelope = Envelope::decode(&bytes).unwrap();
        assert_eq!(envelope.kind, MessageKind::Error);
        let wire: solayge_protocol::WireError = envelope.payload_as().unwrap();
        assert_eq!(wire.code, ErrorCode::InvalidFrame.as_str());

        // The listener still serves a fresh, valid connection.
        let ok = WsConnection::connect(&WsClientConfig::loopback(loopback_url(&host)))
            .await
            .unwrap();
        ok.exchange(&Envelope::command(Action::ConnectionHealth, serde_json::Value::Null))
            .await
            .unwrap();
        host.shutdown().await;
    }

    #[tokio::test]
    async fn plaintext_ws_is_rejected_for_non_loopback_hosts() {
        assert!(validate_endpoint_url("ws://example.com:443", false).is_err());
        assert!(validate_endpoint_url("ws://127.0.0.1:1", false).is_err());
        assert!(validate_endpoint_url("ws://127.0.0.1:1", true).is_ok());
        assert!(validate_endpoint_url("ws://192.168.1.5:1", true).is_err());
        assert!(validate_endpoint_url("wss://host.tailnet.ts.net/", false).is_ok());
        assert!(validate_endpoint_url("http://host/", false).is_err());
        assert!(validate_endpoint_url("not-a-url", false).is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_sync_remote_client_bridge_drives_the_same_protocol() {
        use solayge_protocol::RemoteClient as _;
        let (host, _handler) = host().await;
        let client =
            WsRemoteClient::connect(WsClientConfig::loopback(loopback_url(&host))).unwrap();
        let response = client
            .exchange(Envelope::command(
                Action::ConnectionHealth,
                serde_json::Value::Null,
            ))
            .unwrap();
        assert_eq!(response.kind, MessageKind::Response);
        assert!(solayge_protocol::RemoteClient::is_connected(&client));
        drop(client);
        host.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_closes_live_connections_and_stops_accepting() {
        let (host, _handler) = host().await;
        let url = loopback_url(&host);
        let connection = WsConnection::connect(&WsClientConfig::loopback(&url)).await.unwrap();
        host.shutdown().await;
        // The next exchange on the closed socket fails rather than hanging.
        let result = connection
            .exchange(&Envelope::command(
                Action::ConnectionHealth,
                serde_json::Value::Null,
            ))
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn factory_label_is_stable() {
        let factory = WsTransportFactory::new(WsClientConfig::wss("wss://h/"));
        assert_eq!(factory.label(), "tailscale_ws");
    }
}
