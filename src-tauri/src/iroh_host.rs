//! Desktop Iroh listener for Solayge Mobile remotes.
//!
//! Compiled only with the `mobile-network` cargo feature (on by default). The
//! listener is enabled unless `SOLAYGE_IROH_HOST=0`, so a normal desktop launch
//! is reachable by a phone the user chooses to pair. Data still requires
//! authentication: only pairing and health are allowed before the phone proves
//! possession of its paired key (see [`crate::host`]).
//!
//! Configuration is explicit and environment-driven; there is no hidden setup:
//!
//! * `SOLAYGE_IROH_HOST=0` — disable the listener (any other value or unset
//!   enables it).
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

use tauri::Manager;

use solayge_network::{
    secret_key_from_seed, DiscoveryConfig, EndpointType, IrohConfig, IrohTransport, RelayConfig,
    SecretKey,
};

use crate::host::HostHandler;

/// The secure-store namespace for the desktop's transport key. Kept separate
/// from the app identity so the QUIC endpoint key is not the same secret.
const TRANSPORT_KEY_APP_ID: &str = "com.solayge.desktop/iroh";

/// Whether the user explicitly disabled the listener.
fn disabled_by_env() -> bool {
    matches!(std::env::var("SOLAYGE_IROH_HOST").ok().as_deref(), Some("0"))
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

/// Load (or create) the desktop's stable transport key. Keeping it in the OS
/// credential store means the Iroh endpoint id survives a restart, so a phone
/// paired once can keep reconnecting.
fn transport_secret_key(app: &tauri::AppHandle) -> Result<SecretKey, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    let identity = match solayge_identity::load_or_create_identity(
        solayge_identity::default_store(TRANSPORT_KEY_APP_ID, &data_dir).as_ref(),
    ) {
        Ok(identity) => identity,
        Err(_) => solayge_identity::load_or_create_identity(&solayge_identity::FileStore::new(
            &data_dir,
            TRANSPORT_KEY_APP_ID,
        ))
        .map_err(|e| e.to_string())?,
    };
    let seed = identity.secret_bytes();
    Ok(secret_key_from_seed(&seed))
}

/// Start the listener unless `SOLAYGE_IROH_HOST=0`.
pub fn maybe_spawn(app: tauri::AppHandle) {
    if disabled_by_env() {
        return;
    }
    let mut config = match config_from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("solayge: Iroh host not started: {error}");
            return;
        }
    };
    match transport_secret_key(&app) {
        Ok(key) => config.secret_key = Some(key),
        Err(error) => eprintln!("solayge: using an ephemeral Iroh identity: {error}"),
    }
    let relay = config.relay.as_str();
    let discovery = config.discovery.as_str();
    let bind = config.bind_addr;
    let handler = Arc::new(HostHandler::new(app));

    tauri::async_runtime::spawn(async move {
        match IrohTransport::bind(config).await {
            Ok(transport) => {
                let transport = Arc::new(transport);
                let endpoint_id = transport.endpoint_id().to_string();
                // Publish the endpoint id so the Add-phone QR can advertise a
                // direct dial target; the hint is public.
                crate::host_runtime::set_endpoint_id(endpoint_id.clone());
                eprintln!(
                    "solayge: Iroh host listening (endpoint {}, bind {}, relay {}, discovery {})",
                    endpoint_id, bind, relay, discovery
                );
                if let Err(error) = transport.serve(handler).await {
                    eprintln!("solayge: Iroh host listener stopped: {error}");
                }
            }
            Err(error) => eprintln!("solayge: failed to start Iroh host: {error}"),
        }
    });
}
