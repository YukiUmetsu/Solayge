//! The connection manager state machine (mobile spec section 5.2).
//!
//! States: `Offline → Connecting → Authenticating → Syncing → Connected`, with
//! `Reconnecting`, `Fallback`, and the terminal `AccessDenied`.
//!
//! Non-negotiable rules encoded here:
//!
//! * **Connected means authenticated *and* synced.** The state only becomes
//!   [`ConnectionState::Connected`] after `connection.authenticate` and the
//!   initial event sync both succeed.
//! * **Authorization is never bypassed by connectivity.** An authentication or
//!   revocation denial is terminal ([`ConnectionState::AccessDenied`]); it is
//!   never retried and never triggers a fallback transport.
//! * **At most one active application session.** `connect` is serialized and
//!   installs at most one transport, closing the previous one first.
//! * **Bounded retries.** Capped exponential backoff with jitter and a maximum
//!   attempt budget; when the budget is exhausted the manager goes
//!   [`ConnectionState::Offline`].

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use solayge_protocol::ConnectionAuthenticateResult;

use crate::diagnostics::{ConnectionDiagnostics, PathKind, TransportDiagnostics};
use crate::error::{ErrorClass, NetworkError};

/// A boxed, `Send` future used to keep the transport traits object-safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// The connection manager state machine (section 5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    /// No transport, no session.
    Offline,
    /// Establishing a primary transport.
    Connecting,
    /// Transport established; proving the application identity.
    Authenticating,
    /// Authenticated; reconciling the event cursor.
    Syncing,
    /// Authenticated and synced. The only "working" state.
    Connected,
    /// A previously connected session lost its network path.
    Reconnecting,
    /// Trying a configured fallback transport after a network failure.
    Fallback,
    /// Authentication or authorization was denied. Terminal until `reset`.
    AccessDenied,
}

impl ConnectionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            ConnectionState::Offline => "offline",
            ConnectionState::Connecting => "connecting",
            ConnectionState::Authenticating => "authenticating",
            ConnectionState::Syncing => "syncing",
            ConnectionState::Connected => "connected",
            ConnectionState::Reconnecting => "reconnecting",
            ConnectionState::Fallback => "fallback",
            ConnectionState::AccessDenied => "access_denied",
        }
    }

    fn to_code(self) -> u8 {
        match self {
            ConnectionState::Offline => 0,
            ConnectionState::Connecting => 1,
            ConnectionState::Authenticating => 2,
            ConnectionState::Syncing => 3,
            ConnectionState::Connected => 4,
            ConnectionState::Reconnecting => 5,
            ConnectionState::Fallback => 6,
            ConnectionState::AccessDenied => 7,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => ConnectionState::Connecting,
            2 => ConnectionState::Authenticating,
            3 => ConnectionState::Syncing,
            4 => ConnectionState::Connected,
            5 => ConnectionState::Reconnecting,
            6 => ConnectionState::Fallback,
            7 => ConnectionState::AccessDenied,
            _ => ConnectionState::Offline,
        }
    }
}

/// What the manager learned from the initial sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncReport {
    /// The next event-sequence cursor to resume from.
    pub cursor: u64,
    /// True when the host replaced our cursor with a fresh snapshot.
    pub resync: bool,
}

/// One established transport. Object-safe so tests can substitute fakes.
pub trait Transport: Send + Sync {
    /// `connection.authenticate`. A `NotAuthorized` outcome must surface as
    /// [`NetworkError::AuthDenied`].
    fn authenticate<'a>(
        &'a self,
        device_id: &'a str,
    ) -> BoxFuture<'a, Result<ConnectionAuthenticateResult, NetworkError>>;

    /// Initial event synchronization. A `NotAuthorized` outcome must surface as
    /// [`NetworkError::AuthDenied`].
    fn sync<'a>(&'a self) -> BoxFuture<'a, Result<SyncReport, NetworkError>>;

    fn is_connected(&self) -> bool;

    fn close<'a>(&'a self) -> BoxFuture<'a, ()>;

    fn diagnostics(&self) -> TransportDiagnostics;
}

/// Creates a fresh transport for one connection attempt.
pub trait TransportFactory: Send + Sync + 'static {
    fn connect<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Transport>, NetworkError>>;
    fn label(&self) -> &'static str;
}

/// Injectable timer so tests never sleep in wall-clock time.
pub trait Delay: Send + Sync + 'static {
    fn sleep<'a>(&'a self, duration: Duration) -> BoxFuture<'a, ()>;
}

/// Real timer.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioDelay;

impl Delay for TokioDelay {
    fn sleep<'a>(&'a self, duration: Duration) -> BoxFuture<'a, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// Source of jitter in `[0.0, 1.0]`.
pub trait Jitter: Send + 'static {
    fn unit(&mut self) -> f64;
}

/// Deterministic jitter for tests.
#[derive(Debug, Clone, Copy)]
pub struct FixedJitter(pub f64);

impl Jitter for FixedJitter {
    fn unit(&mut self) -> f64 {
        self.0
    }
}

/// A small splitmix64 PRNG. This is jitter, not security material.
pub struct SystemJitter {
    state: u64,
}

impl Default for SystemJitter {
    fn default() -> Self {
        SystemJitter::new()
    }
}

impl SystemJitter {
    pub fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let seed = nanos
            ^ COUNTER
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_mul(0x9E37_79B9);
        SystemJitter { state: seed | 1 }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl Jitter for SystemJitter {
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Capped exponential backoff with jitter and a retry budget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackoffPolicy {
    pub initial: Duration,
    pub max: Duration,
    pub multiplier: f64,
    /// Jitter fraction in `[0, 1]`: a delay is scaled by
    /// `1 + jitter_ratio * (2 * unit - 1)`.
    pub jitter_ratio: f64,
    /// Maximum number of retries before the budget is exhausted.
    pub max_attempts: u32,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        BackoffPolicy {
            initial: Duration::from_millis(500),
            max: Duration::from_secs(30),
            multiplier: 2.0,
            jitter_ratio: 0.2,
            max_attempts: 6,
        }
    }
}

impl BackoffPolicy {
    /// A compact policy for tests.
    pub fn fast() -> Self {
        BackoffPolicy {
            initial: Duration::from_millis(10),
            max: Duration::from_millis(80),
            multiplier: 2.0,
            jitter_ratio: 0.0,
            max_attempts: 4,
        }
    }
}

/// Stateful backoff. `next_delay` returns `None` once the budget is spent.
pub struct Backoff {
    policy: BackoffPolicy,
    attempts: u32,
    jitter: Box<dyn Jitter>,
}

impl Backoff {
    pub fn new(policy: BackoffPolicy, jitter: Box<dyn Jitter>) -> Self {
        Backoff {
            policy,
            attempts: 0,
            jitter,
        }
    }

    pub fn reset(&mut self) {
        self.attempts = 0;
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// The next `(attempt_number, delay)`, or `None` when the budget is spent.
    pub fn next_delay(&mut self) -> Option<(u32, Duration)> {
        if self.attempts >= self.policy.max_attempts {
            return None;
        }
        self.attempts += 1;
        let exponent = self.attempts.saturating_sub(1);
        let capped = (self.policy.initial.as_secs_f64()
            * self
                .policy
                .multiplier
                .powi(exponent.min(i32::MAX as u32) as i32))
        .min(self.policy.max.as_secs_f64());
        let unit = self.jitter.unit().clamp(0.0, 1.0);
        let factor = 1.0 + self.policy.jitter_ratio * (2.0 * unit - 1.0);
        let secs = (capped * factor).max(0.0);
        Some((self.attempts, Duration::from_secs_f64(secs)))
    }
}

#[derive(Clone)]
struct ActiveSession {
    transport: Arc<dyn Transport>,
    fallback: bool,
    label: String,
}

/// The connection manager. Cheap to share behind an `Arc`.
pub struct ConnectionManager {
    device_id: String,
    primary: Arc<dyn TransportFactory>,
    fallback: Option<Arc<dyn TransportFactory>>,
    delay: Arc<dyn Delay>,
    state_code: AtomicU8,
    needs_reconnect: AtomicBool,
    cancel: AtomicBool,
    attempts: AtomicU32,
    active: Mutex<Option<ActiveSession>>,
    last_error: Mutex<Option<String>>,
    next_retry_in: Mutex<Option<Duration>>,
    backoff: Mutex<Backoff>,
    transitions: Mutex<Vec<ConnectionState>>,
    connect_lock: tokio::sync::Mutex<()>,
}

impl ConnectionManager {
    pub fn new(
        device_id: impl Into<String>,
        primary: Arc<dyn TransportFactory>,
        fallback: Option<Arc<dyn TransportFactory>>,
        delay: Arc<dyn Delay>,
        policy: BackoffPolicy,
    ) -> Self {
        ConnectionManager::with_jitter(
            device_id,
            primary,
            fallback,
            delay,
            policy,
            Box::new(SystemJitter::new()),
        )
    }

    pub fn with_jitter(
        device_id: impl Into<String>,
        primary: Arc<dyn TransportFactory>,
        fallback: Option<Arc<dyn TransportFactory>>,
        delay: Arc<dyn Delay>,
        policy: BackoffPolicy,
        jitter: Box<dyn Jitter>,
    ) -> Self {
        ConnectionManager {
            device_id: device_id.into(),
            primary,
            fallback,
            delay,
            state_code: AtomicU8::new(ConnectionState::Offline.to_code()),
            needs_reconnect: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            attempts: AtomicU32::new(0),
            active: Mutex::new(None),
            last_error: Mutex::new(None),
            next_retry_in: Mutex::new(None),
            backoff: Mutex::new(Backoff::new(policy, jitter)),
            transitions: Mutex::new(Vec::new()),
            connect_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn state(&self) -> ConnectionState {
        ConnectionState::from_code(self.state_code.load(Ordering::SeqCst))
    }

    /// Every state transition observed, oldest first. Useful for diagnostics
    /// and for asserting the handshake ordering in tests.
    pub fn transitions(&self) -> Vec<ConnectionState> {
        self.transitions
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Connect (or reconnect) until [`ConnectionState::Connected`].
    ///
    /// Serialized: concurrent calls share one attempt and never create a second
    /// application session.
    pub async fn connect(&self) -> Result<(), NetworkError> {
        let _guard = self.connect_lock.lock().await;
        if self.state() == ConnectionState::Connected
            && self.active.lock().map(|a| a.is_some()).unwrap_or(false)
        {
            return Ok(());
        }
        if self.state() == ConnectionState::AccessDenied {
            // A denial is terminal until `reset`. Connectivity must never
            // engineer a way around it.
            return Err(NetworkError::auth_denied(
                "access was previously denied; the device must re-pair",
            ));
        }

        self.cancel.store(false, Ordering::SeqCst);
        self.attempts.store(0, Ordering::SeqCst);
        if let Ok(mut backoff) = self.backoff.lock() {
            backoff.reset();
        }
        *self.last_error.lock().unwrap() = None;

        let mut using_fallback = false;
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                self.set_state(ConnectionState::Offline);
                return Err(NetworkError::internal("connect canceled"));
            }
            let factory = if using_fallback {
                self.fallback
                    .clone()
                    .expect("fallback selected without a configured factory")
            } else {
                self.primary.clone()
            };

            match self.establish(&factory, using_fallback).await {
                Ok(()) => {
                    if self.cancel.load(Ordering::SeqCst) {
                        // An explicit disconnect raced our success. The user's
                        // disconnect wins: never leave the manager Connected.
                        self.clear_active().await;
                        self.set_state(ConnectionState::Offline);
                        return Err(NetworkError::internal("connect canceled"));
                    }
                    return Ok(());
                }
                Err(error) => {
                    self.record_error(&error);
                    self.clear_active().await;
                    match error.classify() {
                        ErrorClass::AuthDenied => {
                            // Terminal. Never fall back, never retry.
                            self.set_state(ConnectionState::AccessDenied);
                            return Err(error);
                        }
                        ErrorClass::NetworkFailure
                            if !using_fallback && self.fallback.is_some() =>
                        {
                            self.set_state(ConnectionState::Fallback);
                            using_fallback = true;
                            continue;
                        }
                        _ => {}
                    }

                    let next = self.backoff.lock().ok().and_then(|mut b| b.next_delay());
                    let Some((attempt, delay)) = next else {
                        self.set_state(ConnectionState::Offline);
                        return Err(error);
                    };
                    self.attempts.store(attempt, Ordering::SeqCst);
                    *self.next_retry_in.lock().unwrap() = Some(delay);
                    self.set_state(if using_fallback {
                        ConnectionState::Fallback
                    } else if self.needs_reconnect.load(Ordering::SeqCst) {
                        ConnectionState::Reconnecting
                    } else {
                        ConnectionState::Connecting
                    });
                    self.delay.sleep(delay).await;
                    *self.next_retry_in.lock().unwrap() = None;
                    if self.cancel.load(Ordering::SeqCst) {
                        self.set_state(ConnectionState::Offline);
                        return Err(NetworkError::internal("connect canceled"));
                    }
                }
            }
        }
    }

    async fn establish(
        &self,
        factory: &Arc<dyn TransportFactory>,
        using_fallback: bool,
    ) -> Result<(), NetworkError> {
        self.set_state(if using_fallback {
            ConnectionState::Fallback
        } else if self.needs_reconnect.load(Ordering::SeqCst) {
            ConnectionState::Reconnecting
        } else {
            ConnectionState::Connecting
        });

        let transport = factory.connect().await?;
        if self.cancel.load(Ordering::SeqCst) {
            let _ = transport.close().await;
            return Err(NetworkError::internal("connect canceled"));
        }
        // Exactly one active session: the previous transport is closed before
        // this one is installed.
        self.install_active(transport.clone(), using_fallback, factory.label())
            .await;

        self.set_state(ConnectionState::Authenticating);
        transport.authenticate(&self.device_id).await?;

        self.set_state(ConnectionState::Syncing);
        let _sync = transport.sync().await?;

        self.set_state(ConnectionState::Connected);
        self.attempts.store(0, Ordering::SeqCst);
        self.needs_reconnect.store(false, Ordering::SeqCst);
        *self.last_error.lock().unwrap() = None;
        Ok(())
    }

    /// Mark the active session as lost. The caller then reconnects.
    pub async fn notify_network_loss(&self) {
        if self.state() == ConnectionState::Connected {
            self.needs_reconnect.store(true, Ordering::SeqCst);
            self.set_state(ConnectionState::Reconnecting);
        }
    }

    /// Convenience: mark the session lost and reconnect.
    pub async fn reconnect(&self) -> Result<(), NetworkError> {
        self.notify_network_loss().await;
        self.connect().await
    }

    /// Explicit user disconnect. Always returns to `Offline`.
    pub async fn disconnect(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.clear_active().await;
        self.set_state(ConnectionState::Offline);
    }

    /// Clear a terminal `AccessDenied` after the user re-pairs.
    pub async fn reset(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.clear_active().await;
        self.set_state(ConnectionState::Offline);
        self.cancel.store(false, Ordering::SeqCst);
        self.needs_reconnect.store(false, Ordering::SeqCst);
    }

    pub fn diagnostics(&self) -> ConnectionDiagnostics {
        let state = self.state();
        let active = self.active.lock().ok().and_then(|guard| guard.clone());
        let (transport_label, transport_diagnostics, on_fallback) = match &active {
            Some(session) => (
                Some(session.label.clone()),
                Some(session.transport.diagnostics()),
                session.fallback,
            ),
            None => (None, None, false),
        };
        ConnectionDiagnostics {
            state,
            transport: transport_label,
            endpoint_id: transport_diagnostics
                .as_ref()
                .map(|d| d.endpoint_id.clone()),
            remote_id: transport_diagnostics
                .as_ref()
                .and_then(|d| d.remote_id.clone()),
            endpoint_type: transport_diagnostics.as_ref().map(|d| d.endpoint_type),
            path: transport_diagnostics
                .as_ref()
                .map(|d| d.path)
                .unwrap_or(PathKind::Unknown),
            rtt: transport_diagnostics.as_ref().and_then(|d| d.rtt),
            transport_identity_verified: transport_diagnostics
                .as_ref()
                .map(|d| d.transport_identity_verified)
                .unwrap_or(false),
            // The application checkmark is earned only by a completed handshake.
            solayge_authenticated: state == ConnectionState::Connected && active.is_some(),
            on_fallback,
            attempts: self.attempts.load(Ordering::SeqCst),
            next_retry_in: *self.next_retry_in.lock().unwrap(),
            last_error: self.last_error.lock().unwrap().clone(),
        }
    }

    async fn install_active(&self, transport: Arc<dyn Transport>, fallback: bool, label: &str) {
        let previous = {
            let mut guard = self.active.lock().unwrap();
            guard.replace(ActiveSession {
                transport,
                fallback,
                label: label.to_string(),
            })
        };
        if let Some(previous) = previous {
            previous.transport.close().await;
        }
    }

    async fn clear_active(&self) {
        let previous = self.active.lock().unwrap().take();
        if let Some(previous) = previous {
            previous.transport.close().await;
        }
    }

    fn record_error(&self, error: &NetworkError) {
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = Some(error.to_string());
        }
    }

    fn set_state(&self, state: ConnectionState) {
        let previous = self.state_code.swap(state.to_code(), Ordering::SeqCst);
        if ConnectionState::from_code(previous) == state {
            return;
        }
        if let Ok(mut guard) = self.transitions.lock() {
            guard.push(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use solayge_protocol::ConnectionAuthenticateResult;

    use crate::diagnostics::EndpointType;

    fn auth_ok() -> ConnectionAuthenticateResult {
        ConnectionAuthenticateResult {
            authenticated: true,
            desktop_id: "desk_test".to_string(),
        }
    }

    fn diagnostics_for(label: EndpointType) -> TransportDiagnostics {
        TransportDiagnostics {
            endpoint_id: "endpoint".to_string(),
            remote_id: Some("remote".to_string()),
            endpoint_type: label,
            path: PathKind::Direct,
            rtt: Some(Duration::from_millis(1)),
            transport_identity_verified: true,
            last_error: None,
        }
    }

    struct FakeTransport {
        calls: Mutex<Vec<&'static str>>,
        auth_result: Mutex<Result<ConnectionAuthenticateResult, NetworkError>>,
        sync_result: Mutex<Result<SyncReport, NetworkError>>,
        auth_gate: Option<Arc<tokio::sync::Semaphore>>,
        sync_gate: Option<Arc<tokio::sync::Semaphore>>,
        auth_started: Arc<tokio::sync::Notify>,
        sync_started: Arc<tokio::sync::Notify>,
    }

    impl FakeTransport {
        fn ok() -> Self {
            FakeTransport {
                calls: Mutex::new(Vec::new()),
                auth_result: Mutex::new(Ok(auth_ok())),
                sync_result: Mutex::new(Ok(SyncReport {
                    cursor: 7,
                    resync: false,
                })),
                auth_gate: None,
                sync_gate: None,
                auth_started: Arc::new(tokio::sync::Notify::new()),
                sync_started: Arc::new(tokio::sync::Notify::new()),
            }
        }

        fn deny(reason: &str) -> Self {
            let mut transport = FakeTransport::ok();
            transport.auth_result = Mutex::new(Err(NetworkError::auth_denied(reason)));
            transport
        }

        fn gated() -> Self {
            FakeTransport {
                auth_gate: Some(Arc::new(tokio::sync::Semaphore::new(0))),
                sync_gate: Some(Arc::new(tokio::sync::Semaphore::new(0))),
                ..FakeTransport::ok()
            }
        }

        fn call_count(&self, name: &str) -> usize {
            self.calls
                .lock()
                .map(|guard| guard.iter().filter(|c| **c == name).count())
                .unwrap_or(0)
        }
    }

    impl Transport for FakeTransport {
        fn authenticate<'a>(
            &'a self,
            _device_id: &'a str,
        ) -> BoxFuture<'a, Result<ConnectionAuthenticateResult, NetworkError>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push("authenticate");
                self.auth_started.notify_one();
                if let Some(gate) = &self.auth_gate {
                    let _permit = gate.acquire().await.expect("auth gate");
                }
                self.auth_result.lock().unwrap().clone()
            })
        }

        fn sync<'a>(&'a self) -> BoxFuture<'a, Result<SyncReport, NetworkError>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push("sync");
                self.sync_started.notify_one();
                if let Some(gate) = &self.sync_gate {
                    let _permit = gate.acquire().await.expect("sync gate");
                }
                self.sync_result.lock().unwrap().clone()
            })
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn close<'a>(&'a self) -> BoxFuture<'a, ()> {
            Box::pin(async {
                self.calls.lock().unwrap().push("close");
            })
        }

        fn diagnostics(&self) -> TransportDiagnostics {
            diagnostics_for(EndpointType::DesktopHost)
        }
    }

    struct FakeFactory {
        transport: Arc<dyn Transport>,
        connects: AtomicUsize,
    }

    impl FakeFactory {
        fn new(transport: Arc<dyn Transport>) -> Self {
            FakeFactory {
                transport,
                connects: AtomicUsize::new(0),
            }
        }

        fn connect_count(&self) -> usize {
            self.connects.load(Ordering::SeqCst)
        }
    }

    impl TransportFactory for FakeFactory {
        fn connect<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Transport>, NetworkError>> {
            Box::pin(async move {
                self.connects.fetch_add(1, Ordering::SeqCst);
                Ok(self.transport.clone())
            })
        }

        fn label(&self) -> &'static str {
            "fake"
        }
    }

    /// Fails a fixed number of `connect` calls, then yields a working transport.
    struct FlakyFactory {
        remaining_failures: AtomicUsize,
        transport: Arc<dyn Transport>,
        connects: AtomicUsize,
    }

    impl FlakyFactory {
        fn new(failures: usize, transport: Arc<dyn Transport>) -> Self {
            FlakyFactory {
                remaining_failures: AtomicUsize::new(failures),
                transport,
                connects: AtomicUsize::new(0),
            }
        }

        fn connect_count(&self) -> usize {
            self.connects.load(Ordering::SeqCst)
        }
    }

    impl TransportFactory for FlakyFactory {
        fn connect<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Transport>, NetworkError>> {
            Box::pin(async move {
                self.connects.fetch_add(1, Ordering::SeqCst);
                let remaining = self.remaining_failures.load(Ordering::SeqCst);
                if remaining > 0 {
                    self.remaining_failures.fetch_sub(1, Ordering::SeqCst);
                    return Err(NetworkError::connectivity("simulated path loss"));
                }
                Ok(self.transport.clone())
            })
        }

        fn label(&self) -> &'static str {
            "fake"
        }
    }

    struct AlwaysFailFactory {
        connects: AtomicUsize,
    }

    impl TransportFactory for AlwaysFailFactory {
        fn connect<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Transport>, NetworkError>> {
            Box::pin(async move {
                self.connects.fetch_add(1, Ordering::SeqCst);
                Err(NetworkError::connectivity("down"))
            })
        }

        fn label(&self) -> &'static str {
            "fake"
        }
    }

    #[derive(Default)]
    struct RecordingDelay {
        durations: Mutex<Vec<Duration>>,
    }

    impl Delay for RecordingDelay {
        fn sleep<'a>(&'a self, duration: Duration) -> BoxFuture<'a, ()> {
            self.durations.lock().unwrap().push(duration);
            Box::pin(async {})
        }
    }

    struct NoDelay;

    impl Delay for NoDelay {
        fn sleep<'a>(&'a self, _duration: Duration) -> BoxFuture<'a, ()> {
            Box::pin(async {})
        }
    }

    fn policy(max_attempts: u32) -> BackoffPolicy {
        BackoffPolicy {
            initial: Duration::from_millis(100),
            max: Duration::from_secs(1),
            multiplier: 2.0,
            jitter_ratio: 0.0,
            max_attempts,
        }
    }

    #[test]
    fn backoff_is_capped_exponential_with_a_bounded_budget() {
        let mut backoff = Backoff::new(policy(5), Box::new(FixedJitter(0.5)));
        let delays: Vec<Duration> = std::iter::from_fn(|| backoff.next_delay())
            .map(|(_, delay)| delay)
            .collect();
        assert_eq!(
            delays,
            vec![
                Duration::from_millis(100),
                Duration::from_millis(200),
                Duration::from_millis(400),
                Duration::from_millis(800),
                Duration::from_millis(1000),
            ]
        );
        assert!(backoff.next_delay().is_none(), "budget must be exhausted");
    }

    #[test]
    fn jitter_scales_the_delay_within_the_configured_fraction() {
        let mut policy = policy(1);
        policy.jitter_ratio = 0.5;
        let mut low = Backoff::new(policy, Box::new(FixedJitter(0.0)));
        let mut high = Backoff::new(policy, Box::new(FixedJitter(1.0)));
        assert_eq!(low.next_delay().unwrap().1, Duration::from_millis(50));
        assert_eq!(high.next_delay().unwrap().1, Duration::from_millis(150));
    }

    #[tokio::test]
    async fn connected_is_only_reached_after_authenticate_and_sync_succeed() {
        let transport = Arc::new(FakeTransport::gated());
        let factory = Arc::new(FakeFactory::new(transport.clone()));
        let manager = Arc::new(ConnectionManager::with_jitter(
            "dev_1",
            factory,
            None,
            Arc::new(NoDelay),
            policy(3),
            Box::new(FixedJitter(0.5)),
        ));

        let connecting = tokio::spawn({
            let manager = manager.clone();
            async move { manager.connect().await }
        });

        transport.auth_started.notified().await;
        assert_eq!(manager.state(), ConnectionState::Authenticating);
        assert!(!manager.diagnostics().solayge_authenticated);
        transport.auth_gate.as_ref().unwrap().add_permits(1);

        transport.sync_started.notified().await;
        assert_eq!(manager.state(), ConnectionState::Syncing);
        assert!(!manager.diagnostics().solayge_authenticated);
        transport.sync_gate.as_ref().unwrap().add_permits(1);

        connecting.await.unwrap().unwrap();
        assert_eq!(manager.state(), ConnectionState::Connected);
        assert!(manager.diagnostics().solayge_authenticated);
        assert_eq!(
            manager.transitions(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Authenticating,
                ConnectionState::Syncing,
                ConnectionState::Connected,
            ]
        );
    }

    #[tokio::test]
    async fn a_sync_failure_never_reports_connected() {
        let transport = Arc::new(FakeTransport::ok());
        *transport.sync_result.lock().unwrap() =
            Err(NetworkError::connectivity("cursor reconciliation failed"));
        let factory = Arc::new(FakeFactory::new(transport.clone()));
        let manager = ConnectionManager::with_jitter(
            "dev_1",
            factory,
            None,
            Arc::new(NoDelay),
            policy(1),
            Box::new(FixedJitter(0.5)),
        );

        let error = manager.connect().await.unwrap_err();
        assert!(error.is_network_failure());
        assert_eq!(manager.state(), ConnectionState::Offline);
        assert!(!manager.diagnostics().solayge_authenticated);
    }

    #[tokio::test]
    async fn a_network_failure_drives_backoff_and_reconnect() {
        let transport = Arc::new(FakeTransport::ok());
        let factory = Arc::new(FlakyFactory::new(2, transport.clone()));
        let delay = Arc::new(RecordingDelay::default());
        let manager = ConnectionManager::with_jitter(
            "dev_1",
            factory.clone(),
            None,
            delay.clone(),
            policy(5),
            Box::new(FixedJitter(0.5)),
        );

        manager.connect().await.unwrap();
        assert_eq!(manager.state(), ConnectionState::Connected);
        assert_eq!(factory.connect_count(), 3, "two failures then success");
        assert_eq!(
            delay.durations.lock().unwrap().clone(),
            vec![Duration::from_millis(100), Duration::from_millis(200)]
        );
        assert_eq!(transport.call_count("authenticate"), 1);
        assert_eq!(transport.call_count("sync"), 1);
    }

    #[tokio::test]
    async fn the_retry_budget_is_bounded_and_ends_offline() {
        let factory = Arc::new(AlwaysFailFactory {
            connects: AtomicUsize::new(0),
        });
        let delay = Arc::new(RecordingDelay::default());
        let manager = ConnectionManager::with_jitter(
            "dev_1",
            factory.clone(),
            None,
            delay.clone(),
            policy(2),
            Box::new(FixedJitter(0.5)),
        );

        let error = manager.connect().await.unwrap_err();
        assert!(error.is_network_failure());
        assert_eq!(manager.state(), ConnectionState::Offline);
        assert_eq!(
            delay.durations.lock().unwrap().clone(),
            vec![Duration::from_millis(100), Duration::from_millis(200)]
        );
        assert_eq!(factory.connects.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn an_authentication_or_revocation_denial_never_falls_back_or_retries() {
        for reason in ["invalid identity proof", "device has been revoked"] {
            let primary = Arc::new(FakeFactory::new(Arc::new(FakeTransport::deny(reason))));
            let fallback_transport = Arc::new(FakeTransport::ok());
            let fallback = Arc::new(FakeFactory::new(fallback_transport));
            let manager = ConnectionManager::with_jitter(
                "dev_1",
                primary,
                Some(fallback.clone()),
                Arc::new(NoDelay),
                policy(5),
                Box::new(FixedJitter(0.5)),
            );

            let error = manager.connect().await.unwrap_err();
            assert!(error.is_auth_denied(), "{reason}");
            assert_eq!(manager.state(), ConnectionState::AccessDenied);
            assert_eq!(
                fallback.connect_count(),
                0,
                "a denial must never touch the fallback transport ({reason})"
            );
            assert!(
                !manager.transitions().contains(&ConnectionState::Fallback),
                "a denial must never enter Fallback ({reason})"
            );
            assert!(!manager.diagnostics().solayge_authenticated);
        }
    }

    #[tokio::test]
    async fn a_network_failure_uses_a_configured_fallback() {
        let primary = Arc::new(AlwaysFailFactory {
            connects: AtomicUsize::new(0),
        });
        let fallback_transport = Arc::new(FakeTransport::ok());
        let fallback = Arc::new(FakeFactory::new(fallback_transport.clone()));
        let manager = ConnectionManager::with_jitter(
            "dev_1",
            primary,
            Some(fallback.clone()),
            Arc::new(NoDelay),
            policy(3),
            Box::new(FixedJitter(0.5)),
        );

        manager.connect().await.unwrap();
        assert_eq!(manager.state(), ConnectionState::Connected);
        assert_eq!(fallback.connect_count(), 1);
        assert!(manager.transitions().contains(&ConnectionState::Fallback));
        assert_eq!(manager.diagnostics().transport.as_deref(), Some("fake"));
    }

    #[tokio::test]
    async fn concurrent_connect_requests_create_only_one_active_session() {
        let transport = Arc::new(FakeTransport::ok());
        let factory = Arc::new(FakeFactory::new(transport.clone()));
        let manager = Arc::new(ConnectionManager::with_jitter(
            "dev_1",
            factory.clone(),
            None,
            Arc::new(NoDelay),
            policy(3),
            Box::new(FixedJitter(0.5)),
        ));

        let (first, second) = tokio::join!(manager.connect(), manager.connect());
        first.unwrap();
        second.unwrap();
        assert_eq!(factory.connect_count(), 1, "no transport racing");
        assert_eq!(transport.call_count("authenticate"), 1);
        assert_eq!(transport.call_count("sync"), 1);
    }

    #[tokio::test]
    async fn access_denied_is_terminal_until_reset() {
        let primary = Arc::new(FakeFactory::new(Arc::new(FakeTransport::deny("revoked"))));
        let manager = ConnectionManager::new("dev_1", primary, None, Arc::new(NoDelay), policy(3));

        assert!(manager.connect().await.unwrap_err().is_auth_denied());
        assert_eq!(manager.state(), ConnectionState::AccessDenied);
        // A second attempt is refused without touching the transport.
        assert!(manager.connect().await.unwrap_err().is_auth_denied());
        manager.reset().await;
        assert_eq!(manager.state(), ConnectionState::Offline);
    }

    #[tokio::test]
    async fn disconnect_closes_the_active_session_and_returns_offline() {
        let transport = Arc::new(FakeTransport::ok());
        let factory = Arc::new(FakeFactory::new(transport.clone()));
        let manager = ConnectionManager::with_jitter(
            "dev_1",
            factory,
            None,
            Arc::new(NoDelay),
            policy(3),
            Box::new(FixedJitter(0.5)),
        );
        manager.connect().await.unwrap();
        manager.disconnect().await;
        assert_eq!(manager.state(), ConnectionState::Offline);
        assert!(manager.diagnostics().transport.is_none());
    }

    #[tokio::test]
    async fn disconnect_during_authentication_wins_over_a_late_success() {
        let transport = Arc::new(FakeTransport::gated());
        let factory = Arc::new(FakeFactory::new(transport.clone()));
        let manager = Arc::new(ConnectionManager::with_jitter(
            "dev_1",
            factory,
            None,
            Arc::new(NoDelay),
            policy(3),
            Box::new(FixedJitter(0.5)),
        ));

        let connecting = tokio::spawn({
            let manager = manager.clone();
            async move { manager.connect().await }
        });

        // The handshake has started but not completed when the user disconnects.
        transport.auth_started.notified().await;
        manager.disconnect().await;
        assert_eq!(manager.state(), ConnectionState::Offline);

        // Let the in-flight handshake succeed; the disconnect must still win.
        transport.auth_gate.as_ref().unwrap().add_permits(1);
        transport.sync_gate.as_ref().unwrap().add_permits(1);
        let result = connecting.await.unwrap();
        assert!(
            result.is_err(),
            "a canceled connect must not report success"
        );
        assert_eq!(manager.state(), ConnectionState::Offline);
        assert!(!manager.diagnostics().solayge_authenticated);
    }

    #[tokio::test]
    async fn network_loss_marks_reconnecting_then_recovers() {
        let transport = Arc::new(FakeTransport::ok());
        let factory = Arc::new(FakeFactory::new(transport.clone()));
        let manager = ConnectionManager::with_jitter(
            "dev_1",
            factory,
            None,
            Arc::new(NoDelay),
            policy(3),
            Box::new(FixedJitter(0.5)),
        );
        manager.connect().await.unwrap();
        manager.notify_network_loss().await;
        assert_eq!(manager.state(), ConnectionState::Reconnecting);
        manager.connect().await.unwrap();
        assert_eq!(manager.state(), ConnectionState::Connected);
        assert_eq!(transport.call_count("authenticate"), 2);
    }
}
