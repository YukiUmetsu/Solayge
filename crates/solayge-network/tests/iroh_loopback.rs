//! End-to-end tests over real local Iroh endpoints.
//!
//! These stand up two loopback Iroh endpoints (host + client), exchange
//! `solayge-protocol` envelopes, and prove the connection manager reports
//! `Connected` only after authenticate + sync over the real transport.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::Value;
use solayge_network::{
    BackoffPolicy, BoxFuture, ConnectionManager, ConnectionState, EndpointType, FrameHandler,
    IrohConfig, IrohRemoteClient, IrohTransport, IrohTransportFactory, NetworkError, PathKind,
    TokioDelay, Transport, TransportFactory,
};
use solayge_protocol::{Action, ConnectionHealth, Envelope, MockHost, RemoteClient};

/// Serves the in-memory protocol host (from `solayge-protocol`) over Iroh. This
/// is the same handler shape the desktop host wiring uses.
struct MockHostHandler {
    host: Arc<MockHost>,
}

impl FrameHandler for MockHostHandler {
    fn handle<'a>(
        &'a self,
        _session: &'a solayge_network::ConnectionSession,
        request: Envelope,
    ) -> BoxFuture<'a, Envelope> {
        Box::pin(async move {
            // Keep a copy so a failure can be reported as a correlated error
            // frame instead of dropping the connection.
            let request_for_error = request.clone();
            let client = self.host.client();
            match client.exchange(request) {
                Ok(response) => response,
                Err(error) => Envelope::error_to(&request_for_error, &error),
            }
        })
    }
}

async fn start_loopback_host(host: Arc<MockHost>) -> Arc<IrohTransport> {
    let transport = Arc::new(
        IrohTransport::bind(IrohConfig::loopback(EndpointType::DesktopHost))
            .await
            .expect("bind host endpoint"),
    );
    let handler = Arc::new(MockHostHandler { host });
    let server = transport.clone();
    // Serve every incoming connection (the revocation test opens two).
    tokio::spawn(async move {
        if let Err(error) = server.serve(handler).await {
            eprintln!("host listener stopped: {error}");
        }
    });
    transport
}

fn client_config() -> IrohConfig {
    IrohConfig::loopback(EndpointType::MobileClient)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_local_iroh_endpoints_exchange_protocol_envelopes() {
    let host = Arc::new(MockHost::new());
    let server = start_loopback_host(host).await;

    let client = IrohTransport::bind(client_config())
        .await
        .expect("bind client endpoint");
    let connection = client
        .dial(server.endpoint_addr())
        .await
        .expect("dial loopback server");

    // Health is pre-auth and proves the envelope round-trips.
    let health: ConnectionHealth = connection
        .exchange(&Envelope::command(Action::ConnectionHealth, Value::Null))
        .await
        .expect("health exchange")
        .payload_as()
        .expect("health payload");
    assert_eq!(health.desktop_id, "desk_mock");
    assert_eq!(health.protocol_version, solayge_protocol::PROTOCOL_VERSION);

    // Authenticate, then read a real session.
    let auth = connection
        .exchange(&Envelope::command(
            Action::ConnectionAuthenticate,
            serde_json::json!({
                "device_id": "dev_1",
                "desktop_id": "",
                "protocol_version": solayge_protocol::PROTOCOL_VERSION,
            }),
        ))
        .await
        .expect("authenticate exchange");
    assert_ne!(auth.kind, solayge_protocol::MessageKind::Error);

    let sessions = connection
        .exchange(&Envelope::command(Action::SessionsList, Value::Null))
        .await
        .expect("sessions exchange");
    let sessions: solayge_protocol::SessionsResponse =
        sessions.payload_as().expect("sessions payload");
    assert_eq!(sessions.sessions.len(), 1);
    assert_eq!(sessions.sessions[0].id, "ses_1");

    // Diagnostics are honest: loopback is a direct IP path, peer identity was
    // verified by QUIC, and this endpoint is the mobile client.
    let diagnostics = connection.diagnostics();
    assert_eq!(
        diagnostics.path,
        PathKind::Direct,
        "loopback is a direct path"
    );
    assert!(diagnostics.transport_identity_verified);
    assert_eq!(diagnostics.endpoint_type, EndpointType::MobileClient);
    assert!(diagnostics.rtt.is_some(), "RTT is reported for a live path");

    connection.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_blocking_remote_client_implements_the_protocol_interface() {
    let host = Arc::new(MockHost::new());
    let server = start_loopback_host(host).await;

    let client = IrohRemoteClient::connect(client_config(), server.endpoint_addr())
        .expect("start blocking Iroh client");

    let health = client.health().expect("health");
    assert_eq!(health.desktop_id, "desk_mock");

    let authenticated = client.authenticate("dev_1").expect("authenticate");
    assert!(authenticated.authenticated);
    assert_eq!(client.list_sessions().expect("sessions").len(), 1);

    drop(client);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_manager_reaches_connected_over_iroh_only_after_auth_and_sync() {
    let host = Arc::new(MockHost::new());
    let server = start_loopback_host(host).await;

    let factory = Arc::new(IrohTransportFactory::new(
        client_config(),
        server.endpoint_addr(),
    ));
    let manager = ConnectionManager::new(
        "dev_1",
        factory,
        None,
        Arc::new(TokioDelay),
        BackoffPolicy::fast(),
    );

    manager.connect().await.expect("connect over iroh");
    assert_eq!(manager.state(), ConnectionState::Connected);

    let diagnostics = manager.diagnostics();
    assert!(diagnostics.solayge_authenticated);
    assert_eq!(diagnostics.transport.as_deref(), Some("iroh"));
    assert_eq!(diagnostics.path, PathKind::Direct);
    assert!(!diagnostics.on_fallback);

    manager.disconnect().await;
    assert_eq!(manager.state(), ConnectionState::Offline);
}

/// A failed handshake (here: an ALPN mismatch, but also a stray/malformed packet
/// or an aborted dial) must never take down the host listener. The listener must
/// keep accepting and serve a subsequent well-formed client.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_incoming_handshake_does_not_stop_the_listener() {
    let host = Arc::new(MockHost::new());
    let server = start_loopback_host(host).await;

    // Attempt a handshake with an ALPN the host does not negotiate. This is
    // exactly the routine, remotely-triggerable failure iroh documents as safe
    // to log and ignore.
    let rogue = IrohTransport::bind(client_config())
        .await
        .expect("bind rogue endpoint");
    let _ = rogue
        .endpoint()
        .connect(server.endpoint_addr(), b"not-solayge/1")
        .await;

    // The listener must still accept a correct client.
    let client = IrohTransport::bind(client_config())
        .await
        .expect("bind client endpoint");
    let connection = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.dial(server.endpoint_addr()),
    )
    .await
    .expect("listener must not be dead after a rejected handshake")
    .expect("dial after rejected handshake");
    let health: ConnectionHealth = connection
        .exchange(&Envelope::command(Action::ConnectionHealth, Value::Null))
        .await
        .expect("health exchange after rejected handshake")
        .payload_as()
        .expect("health payload");
    assert_eq!(health.desktop_id, "desk_mock");
    connection.close().await;
}

/// The transport-identity diagnostic must be a real comparison, not a
/// restatement of the remote id. An outbound dial knows the expected endpoint
/// id and can verify it; an inbound connection has no pre-agreed identity and
/// must report "not verified" until the application authenticates the peer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identity_verification_is_meaningful_for_dials_and_unknown_for_inbound() {
    let host = IrohTransport::bind(IrohConfig::loopback(EndpointType::DesktopHost))
        .await
        .expect("bind host endpoint");
    let host_id = host.endpoint_id();
    let host_addr = host.endpoint_addr();
    let client = IrohTransport::bind(client_config())
        .await
        .expect("bind client endpoint");

    let dialing = tokio::spawn(async move { client.dial(host_addr).await });

    let server_side = host
        .accept()
        .await
        .expect("accept")
        .expect("inbound connection");
    assert_eq!(
        server_side.expected_remote_id(),
        None,
        "an inbound connection has no pre-agreed expected identity"
    );
    assert!(
        !server_side.transport_identity_verified(),
        "an unauthenticated inbound peer must not be reported as verified"
    );

    let client_side = dialing.await.expect("dial task").expect("dial");
    assert_eq!(client_side.expected_remote_id(), Some(host_id));
    assert!(
        client_side.transport_identity_verified(),
        "a dialed peer that matches the requested endpoint id is verified"
    );

    client_side.close().await;
    server_side.close().await;
}

/// A host that completes the QUIC handshake but never answers a request.
struct PendingHandler;

impl FrameHandler for PendingHandler {
    fn handle<'a>(
        &'a self,
        _session: &'a solayge_network::ConnectionSession,
        _request: Envelope,
    ) -> BoxFuture<'a, Envelope> {
        Box::pin(std::future::pending())
    }
}

/// A stalled peer must be bounded by a timeout and reported as a retryable
/// connectivity failure, so the connection manager eventually backs off or
/// falls back instead of hanging in `Authenticating` forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_peer_times_out_instead_of_hanging_forever() {
    let transport = Arc::new(
        IrohTransport::bind(IrohConfig::loopback(EndpointType::DesktopHost))
            .await
            .expect("bind host endpoint"),
    );
    let server = transport.clone();
    tokio::spawn(async move {
        let _ = server.serve(Arc::new(PendingHandler)).await;
    });

    let client = IrohTransport::bind(client_config())
        .await
        .expect("bind client endpoint");
    let connection = client
        .dial(transport.endpoint_addr())
        .await
        .expect("dial pending host");

    let error = connection
        .exchange_with_timeout(
            &Envelope::command(Action::ConnectionHealth, Value::Null),
            std::time::Duration::from_millis(150),
        )
        .await
        .expect_err("a stalled peer must time out");
    assert!(
        error.is_network_failure(),
        "a timeout is a retryable connectivity failure, got: {error}"
    );
    connection.close().await;
}

/// A fallback factory that must never be used. Records attempts.
struct CountingFactory {
    connects: AtomicUsize,
}

impl CountingFactory {
    fn new() -> Self {
        CountingFactory {
            connects: AtomicUsize::new(0),
        }
    }

    fn connect_count(&self) -> usize {
        self.connects.load(Ordering::SeqCst)
    }
}

impl TransportFactory for CountingFactory {
    fn connect<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Transport>, NetworkError>> {
        Box::pin(async move {
            self.connects.fetch_add(1, Ordering::SeqCst);
            Err(NetworkError::connectivity(
                "the fallback transport must not be used for a denial",
            ))
        })
    }

    fn label(&self) -> &'static str {
        "fallback"
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revocation_denial_over_iroh_never_triggers_fallback() {
    let host = Arc::new(MockHost::new());
    let server = start_loopback_host(host).await;

    // Pair device `dev_1`, then revoke it from the host.
    let revoker = IrohRemoteClient::connect(client_config(), server.endpoint_addr())
        .expect("start revoker client");
    revoker.authenticate("dev_1").expect("authenticate dev_1");
    revoker.revoke_device("dev_1").expect("revoke dev_1");
    drop(revoker);

    let fallback = Arc::new(CountingFactory::new());
    let manager = ConnectionManager::new(
        "dev_1",
        Arc::new(IrohTransportFactory::new(
            client_config(),
            server.endpoint_addr(),
        )),
        Some(fallback.clone()),
        Arc::new(TokioDelay),
        BackoffPolicy::fast(),
    );

    let error = manager
        .connect()
        .await
        .expect_err("revoked device is denied");
    assert!(error.is_auth_denied(), "got: {error}");
    assert_eq!(manager.state(), ConnectionState::AccessDenied);
    assert_eq!(
        fallback.connect_count(),
        0,
        "a revocation denial must never use the fallback transport"
    );
    assert!(!manager.diagnostics().solayge_authenticated);
}
