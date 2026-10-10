//! Reusable HTTP surface for the push gateway.
//!
//! Both delivery paths run this exact code: the standalone binary
//! ([`crate::main`]) and the gateway the desktop embeds. Sharing it means the
//! two cannot drift — one authenticated `POST /v1/push` endpoint, one
//! `GET /healthz` probe, one request-size limit.
//!
//! The embedded path ([`spawn`]) binds a loopback listener, serves it on a
//! dedicated thread, and returns a [`GatewayHandle`] whose `Drop` stops and
//! joins the thread. Nothing here holds the service-account key except the FCM
//! sender it is given.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::fcm::{GoogleFcmSender, ServiceAccount};
use crate::{handle_push, FcmSender};

/// Default loopback bind for an embedded gateway.
pub const DEFAULT_BIND: &str = "127.0.0.1:8787";
/// Default maximum accepted request body. A push carries identifiers only, so
/// this is generous while still bounding what an unauthenticated peer can send.
pub const DEFAULT_MAX_BODY: usize = 16 * 1024;

/// How long the serve loop blocks before it re-checks the shutdown flag.
const SHUTDOWN_POLL: Duration = Duration::from_millis(250);

/// A running gateway owned by whoever spawned it. Dropping the handle requests
/// shutdown and joins the serve thread, so a process exit cannot leave a thread
/// holding the listener.
pub struct GatewayHandle {
    bind: String,
    /// The desktop-side `POST /v1/push` URL (carries the actual bound port).
    pub local_url: String,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl GatewayHandle {
    /// The address the listener actually bound (resolves an ephemeral port).
    pub fn bind(&self) -> &str {
        &self.bind
    }

    /// Ask the serve thread to stop and wait for it. Idempotent.
    pub fn stop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for GatewayHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Bind and start an embedded gateway from a loaded service account.
pub fn spawn(
    bind: &str,
    token: String,
    service_account: ServiceAccount,
    max_body: usize,
) -> Result<GatewayHandle, String> {
    let sender: Arc<dyn FcmSender> = Arc::new(GoogleFcmSender::new(service_account)?);
    spawn_with_sender(bind, token, sender, max_body)
}

/// Bind and start an embedded gateway with an explicit sender. Split out so
/// tests can inject a recording sender and never touch the network.
pub fn spawn_with_sender(
    bind: &str,
    token: String,
    sender: Arc<dyn FcmSender>,
    max_body: usize,
) -> Result<GatewayHandle, String> {
    let server =
        tiny_http::Server::http(bind).map_err(|e| format!("could not bind {bind}: {e}"))?;
    let actual_bind = server
        .server_addr()
        .to_ip()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|| bind.to_string());
    let local_url = format!("http://{actual_bind}/v1/push");

    let shutdown = Arc::new(AtomicBool::new(false));
    let flag = shutdown.clone();
    let join = std::thread::Builder::new()
        .name("solayge-push-gateway".to_string())
        .spawn(move || {
            serve(server, &token, sender.as_ref(), max_body, || {
                flag.load(Ordering::SeqCst)
            });
        })
        .map_err(|e| format!("could not start the gateway thread: {e}"))?;

    Ok(GatewayHandle {
        bind: actual_bind,
        local_url,
        shutdown,
        join: Some(join),
    })
}

/// Serve requests until `shutdown` returns true. Blocking; call from a thread.
pub fn serve(
    server: tiny_http::Server,
    token: &str,
    sender: &dyn FcmSender,
    max_body: usize,
    shutdown: impl Fn() -> bool,
) {
    while !shutdown() {
        match server.recv_timeout(SHUTDOWN_POLL) {
            Ok(Some(mut request)) => {
                let response = handle_request(&mut request, token, sender, max_body);
                let _ = request.respond(response);
            }
            Ok(None) => {}
            // A listener error is fatal for this process; the caller logs the
            // exit and a restart retries the bind.
            Err(_) => break,
        }
    }
}

/// Route and handle one HTTP request.
pub fn handle_request(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FcmError, FcmMessage};
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Default)]
    struct RecordingSender {
        messages: Mutex<Vec<FcmMessage>>,
    }

    impl FcmSender for RecordingSender {
        fn send(&self, message: &FcmMessage) -> Result<(), FcmError> {
            self.messages
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(message.clone());
            Ok(())
        }
    }

    #[test]
    fn the_embedded_server_serves_health_and_push() {
        let sender = Arc::new(RecordingSender::default());
        // Port 0 gets an ephemeral loopback port, so parallel tests never clash.
        let handle = spawn_with_sender("127.0.0.1:0", "secret".into(), sender.clone(), DEFAULT_MAX_BODY)
            .expect("gateway binds");
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        let health = client
            .get(format!("http://{}/healthz", handle.bind()))
            .send()
            .expect("health request");
        assert!(health.status().is_success());

        let body = serde_json::json!({
            "token": "fcm",
            "event": {
                "event_id": "desk:se:ses:task_completed",
                "kind": "task_completed",
                "desktop_id": "desk",
                "session_id": "ses"
            }
        });
        let unauthorized = client
            .post(&handle.local_url)
            .json(&body)
            .send()
            .expect("push request");
        assert_eq!(unauthorized.status(), 401);

        let delivered = client
            .post(&handle.local_url)
            .bearer_auth("secret")
            .json(&body)
            .send()
            .expect("push request");
        assert_eq!(delivered.status(), 202);
        assert_eq!(sender.messages.lock().unwrap().len(), 1);
    }
}
