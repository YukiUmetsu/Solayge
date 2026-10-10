//! The standalone push gateway binary.
//!
//! Configuration (all via environment, so no secret is ever compiled in):
//!
//! * `PUSH_GATEWAY_TOKEN` — the bearer secret a desktop presents. Required.
//! * `GOOGLE_APPLICATION_CREDENTIALS` — path to the Firebase service-account
//!   JSON. Required; stays on the gateway only.
//! * `PUSH_GATEWAY_BIND` — listen address (default `127.0.0.1:8787`).
//! * `PUSH_GATEWAY_MAX_BODY` — max request size in bytes (default 16384).
//!
//! The desktop normally embeds the same server (see `solayge`'s `push_gateway`
//! module); this binary remains for a remote/shared deployment or CI.
#![deny(unsafe_code)]

use std::sync::Arc;

use solayge_push_gateway::fcm::{GoogleFcmSender, ServiceAccount};
use solayge_push_gateway::server::{self, DEFAULT_BIND, DEFAULT_MAX_BODY};
use solayge_push_gateway::FcmSender;

fn main() {
    let token = match std::env::var("PUSH_GATEWAY_TOKEN") {
        Ok(token) if !token.trim().is_empty() => token,
        _ => {
            eprintln!("PUSH_GATEWAY_TOKEN is required");
            std::process::exit(1);
        }
    };
    let credentials = match std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
        Ok(path) if !path.trim().is_empty() => path,
        _ => {
            eprintln!("GOOGLE_APPLICATION_CREDENTIALS is required");
            std::process::exit(1);
        }
    };
    let account = match ServiceAccount::from_file(std::path::Path::new(&credentials)) {
        Ok(account) => account,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let sender: Arc<dyn FcmSender> = match GoogleFcmSender::new(account) {
        Ok(sender) => Arc::new(sender),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    let bind = std::env::var("PUSH_GATEWAY_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
    let max_body: usize = std::env::var("PUSH_GATEWAY_MAX_BODY")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_MAX_BODY);

    let server = match tiny_http::Server::http(&bind) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("could not bind {bind}: {error}");
            std::process::exit(1);
        }
    };
    eprintln!("solayge push gateway listening on {bind}");

    server::serve(server, &token, sender.as_ref(), max_body, || false);
}
