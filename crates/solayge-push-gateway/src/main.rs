//! The push gateway binary.
//!
//! Configuration (all via environment, so no secret is ever compiled in):
//!
//! * `PUSH_GATEWAY_TOKEN` — the bearer secret a desktop presents. Required.
//! * `GOOGLE_APPLICATION_CREDENTIALS` — path to the Firebase service-account
//!   JSON. Required; stays on the gateway only.
//! * `PUSH_GATEWAY_BIND` — listen address (default `127.0.0.1:8787`).
//! * `PUSH_GATEWAY_MAX_BODY` — max request size in bytes (default 16384).
#![deny(unsafe_code)]

use std::io::Read;
use std::sync::Arc;

use solayge_push_gateway::fcm::{GoogleFcmSender, ServiceAccount};
use solayge_push_gateway::{handle_push, FcmSender};

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

    let bind = std::env::var("PUSH_GATEWAY_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
    let max_body: usize = std::env::var("PUSH_GATEWAY_MAX_BODY")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(16 * 1024);

    let server = match tiny_http::Server::http(&bind) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("could not bind {bind}: {error}");
            std::process::exit(1);
        }
    };
    eprintln!("solayge push gateway listening on {bind}");

    for request in server.incoming_requests() {
        let mut request = request;
        let response = handle_request(&mut request, &token, sender.as_ref(), max_body);
        let _ = request.respond(response);
    }
}

fn handle_request(
    request: &mut tiny_http::Request,
    token: &str,
    sender: &dyn FcmSender,
    max_body: usize,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let url = request.url().to_string();
    if request.method() == &tiny_http::Method::Get && url == "/healthz" {
        return json(200, "{\"ok\":true}");
    }
    if request.method() != &tiny_http::Method::Post || url != "/v1/push" {
        return json(404, "{\"error\":\"not found\"}");
    }
    let authorization = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Authorization"))
        .map(|header| header.value.as_str().to_string());

    let mut body = Vec::new();
    let reader = request.as_reader();
    let mut limited = reader.take(max_body as u64 + 1);
    if limited.read_to_end(&mut body).is_err() {
        return json(400, "{\"error\":\"could not read body\"}");
    }
    if body.len() > max_body {
        return json(413, "{\"error\":\"payload too large\"}");
    }

    let result = handle_push(&body, authorization.as_deref(), token, sender);
    json(result.status, &result.body)
}

fn json(status: u16, body: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let mut response = tiny_http::Response::from_string(body.to_string());
    response = response.with_status_code(status);
    response.with_header(
        tiny_http::Header::from_bytes("Content-Type", "application/json").expect("valid header"),
    )
}
