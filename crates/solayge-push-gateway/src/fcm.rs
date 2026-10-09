//! The real FCM HTTP v1 client and its service-account token exchange.
//!
//! The service-account JSON is loaded from disk on the gateway only. The
//! distributed desktop/mobile apps never contain it; they authenticate to the
//! gateway with a separate bearer secret.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::json;

use crate::{FcmMessage, FcmSender};

const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const FCM_BASE: &str = "https://fcm.googleapis.com/v1/projects";

/// The fields the gateway needs from a Firebase service-account JSON file.
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceAccount {
    pub project_id: String,
    pub client_email: String,
    pub private_key: String,
    #[serde(default = "default_token_uri")]
    pub token_uri: String,
}

fn default_token_uri() -> String {
    TOKEN_URI.to_string()
}

impl ServiceAccount {
    /// Load a service-account JSON file. Returns an error without echoing the
    /// key material.
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("could not read service account: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("invalid service account JSON: {e}"))
    }
}

#[derive(Debug, serde::Serialize)]
struct Claims {
    iss: String,
    scope: String,
    aud: String,
    iat: u64,
    exp: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

/// Sends messages through the FCM HTTP v1 API using a cached OAuth token.
pub struct GoogleFcmSender {
    account: ServiceAccount,
    http: reqwest::blocking::Client,
    cached: Mutex<Option<(String, Instant)>>,
}

impl GoogleFcmSender {
    pub fn new(account: ServiceAccount) -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| format!("could not build HTTP client: {e}"))?;
        Ok(GoogleFcmSender {
            account,
            http,
            cached: Mutex::new(None),
        })
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Exchange a signed JWT assertion for an OAuth access token, caching it
    /// until shortly before it expires.
    fn access_token(&self) -> Result<String, String> {
        if let Some((token, expires_at)) = self
            .cached
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            if Instant::now() < *expires_at {
                return Ok(token.clone());
            }
        }

        let issued = Self::now();
        let claims = Claims {
            iss: self.account.client_email.clone(),
            scope: SCOPE.to_string(),
            aud: self.account.token_uri.clone(),
            iat: issued,
            exp: issued + 3600,
        };
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(self.account.private_key.as_bytes())
            .map_err(|e| format!("invalid service-account key: {e}"))?;
        let assertion = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &key,
        )
        .map_err(|e| format!("could not sign assertion: {e}"))?;

        let response = self
            .http
            .post(&self.account.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .map_err(|e| format!("token request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("token request rejected: {}", response.status()));
        }
        let parsed: TokenResponse = response
            .json()
            .map_err(|e| format!("invalid token response: {e}"))?;
        let lifetime = parsed.expires_in.max(60);
        let expires_at = Instant::now() + Duration::from_secs(lifetime.saturating_sub(60));
        let token = parsed.access_token;
        *self.cached.lock().unwrap_or_else(|p| p.into_inner()) = Some((token.clone(), expires_at));
        Ok(token)
    }
}

impl FcmSender for GoogleFcmSender {
    fn send(&self, message: &FcmMessage) -> Result<(), String> {
        let token = self.access_token()?;
        let priority = if message.high_priority {
            "high"
        } else {
            "normal"
        };
        let apns_priority = if message.high_priority { "10" } else { "5" };
        let body = json!({
            "message": {
                "token": message.token,
                "notification": { "title": message.title, "body": message.body },
                "data": message.data,
                "android": {
                    "priority": priority,
                    "ttl": "3600s",
                    "notification": {
                        "channel_id": FcmMessage::CHANNEL_ID,
                        "tag": message.tag(),
                        "sound": "default",
                    },
                },
                "apns": {
                    "headers": { "apns-priority": apns_priority },
                    "payload": { "aps": { "sound": "default" } },
                },
            }
        });
        let url = format!("{FCM_BASE}/{}/messages:send", self.account.project_id);
        let response = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .map_err(|e| format!("FCM request failed: {e}"))?;
        if response.status().is_success() {
            Ok(())
        } else {
            // Do not echo the body: it can contain the registration token.
            Err(format!("FCM rejected the message: {}", response.status()))
        }
    }
}
