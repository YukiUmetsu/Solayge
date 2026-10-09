//! Desktop Iroh listener for Solayge Mobile remotes.
//!
//! Compiled only with the `mobile-network` cargo feature. The listener is a
//! second, separate opt-in at runtime (`SOLAYGE_IROH_HOST=1`) so a normal
//! desktop launch never opens a network listener.
//!
//! Configuration is explicit and environment-driven; there is no hidden relay
//! or discovery setup:
//!
//! * `SOLAYGE_IROH_HOST=1` — enable the listener.
//! * `SOLAYGE_IROH_BIND` — UDP bind address (default `127.0.0.1:0`; set
//!   `0.0.0.0:0` for LAN direct connectivity).
//! * `SOLAYGE_IROH_RELAY` — `disabled`, `default`/`n0`, `staging`, or a
//!   comma-separated list of relay URLs (default `n0`).
//! * `SOLAYGE_IROH_DISCOVERY` — `n0`/`n0_dns` or `disabled` (default `n0`).
//!
//! `RelayConfig::Disabled` is not a promise of direct connectivity: with no
//! relay and no discovery a peer must be dialed by an explicit address.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use solayge_network::solayge_protocol::{Envelope, MockHost, RemoteClient};
use solayge_network::{
    BoxFuture, DiscoveryConfig, EndpointType, FrameHandler, IrohConfig, IrohTransport, RelayConfig,
};

/// Serves the in-memory protocol host over Iroh. The real desktop authorization
/// layer replaces this in the host phase; the transport wiring is identical.
struct ProtocolHost {
    host: Arc<MockHost>,
}

impl FrameHandler for ProtocolHost {
    fn handle<'a>(&'a self, request: Envelope) -> BoxFuture<'a, Envelope> {
        Box::pin(async move {
            let request_for_error = request.clone();
            let client = self.host.client();
            match client.exchange(request) {
                Ok(response) => response,
                Err(error) => Envelope::error_to(&request_for_error, &error),
            }
        })
    }
}

/// Read the listener configuration from the environment, or `Err` with a
/// user-facing reason.
pub fn config_from_env() -> Result<IrohConfig, String> {
    let mut config = IrohConfig::production(EndpointType::DesktopHost);
    // A desktop host binds loopback by default; remote reachability over a
    // relay still works, and LAN direct requires an explicit bind address.
    config.bind_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));

    if let Ok(spec) = std::env::var("SOLAYGE_IROH_RELAY") {
        config.relay = RelayConfig::parse(&spec)?;
    }
    if let Ok(spec) = std::env::var("SOLAYGE_IROH_DISCOVERY") {
        config.discovery = DiscoveryConfig::parse(&spec)?;
    }
    if let Ok(bind) = std::env::var("SOLAYGE_IROH_BIND") {
        config.bind_addr = bind
            .parse()
            .map_err(|e| format!("invalid SOLAYGE_IROH_BIND '{bind}': {e}"))?;
    }
    Ok(config)
}

/// Start the listener when `SOLAYGE_IROH_HOST=1`. No-op otherwise.
pub fn maybe_spawn() {
    if std::env::var("SOLAYGE_IROH_HOST").ok().as_deref() != Some("1") {
        return;
    }
    let config = match config_from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("solayge: Iroh host not started: {error}");
            return;
        }
    };
    let relay = config.relay.as_str();
    let discovery = config.discovery.as_str();
    let bind = config.bind_addr;
    let handler = Arc::new(ProtocolHost {
        host: Arc::new(MockHost::new()),
    });

    tauri::async_runtime::spawn(async move {
        match IrohTransport::bind(config).await {
            Ok(transport) => {
                let transport = Arc::new(transport);
                eprintln!(
                    "solayge: Iroh host listening (endpoint {}, bind {}, relay {}, discovery {})",
                    transport.endpoint_id(),
                    bind,
                    relay,
                    discovery
                );
                if let Err(error) = transport.serve(handler).await {
                    eprintln!("solayge: Iroh host listener stopped: {error}");
                }
            }
            Err(error) => eprintln!("solayge: failed to start Iroh host: {error}"),
        }
    });
}
