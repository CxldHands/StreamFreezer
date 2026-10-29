// ─────────────────────────────────────────────
// Imports
// ─────────────────────────────────────────────

use keyring::Entry;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

// ─────────────────────────────────────────────
// Twitch OAuth Configuration
// ─────────────────────────────────────────────

pub const CLIENT_ID: &str = "evukj98bxv530lwircj1te458ypd01";

const EXPECTED_LOGIN: &str = "cxldhands";
const REQUIRED_SCOPE: &str = "moderator:read:followers";

const DEVICE_URL: &str = "https://id.twitch.tv/oauth2/device";
const TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
const VALIDATE_URL: &str = "https://id.twitch.tv/oauth2/validate";

const KEYRING_SERVICE: &str = "StreamFreezer";
const KEYRING_USER: &str = "twitch-oauth";

// ─────────────────────────────────────────────
// Authentication State Types
// ─────────────────────────────────────────────

#[derive(Clone)]
pub struct TwitchAuth {
    client: Client,
    status: Arc<Mutex<AuthStatus>>,
}

#[derive(Clone, Debug)]
pub struct EventSubCredentials {
    pub access_token: String,
    pub user_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuthStatus {
    pub state: String,
    pub message: String,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
    pub expires_in: Option<u64>,
    pub login: Option<String>,
    pub scopes: Vec<String>,
}

// ─────────────────────────────────────────────
// Default Authentication Status
// ─────────────────────────────────────────────

impl Default for AuthStatus {
    fn default() -> Self {
        Self {
            state: "disconnected".to_owned(),
            message: "Connect your Twitch account to enable live follow events.".to_owned(),
            user_code: None,
            verification_uri: None,
            expires_in: None,
            login: None,
            scopes: Vec::new(),
        }
    }
}

// ─────────────────────────────────────────────
// Twitch API Response Types
// ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct ValidatedToken {
    login: String,
    user_id: String,
    scopes: Vec<String>,
    expires_in: u64,
}

#[derive(Serialize, Deserialize)]
struct StoredTokens {
    access_token: String,
    refresh_token: String,
}

// ─────────────────────────────────────────────
// Authentication Manager
// ─────────────────────────────────────────────

impl TwitchAuth {
    // ─────────────────────────────────────────
    // Initialization and Status
    // ─────────────────────────────────────────

    pub fn new() -> Self {
        Self {
            client: Client::new(),
            status: Arc::new(Mutex::new(AuthStatus::default())),
        }
    }

    pub async fn status(&self) -> AuthStatus {
        self.status.lock().await.clone()
    }

    // ─────────────────────────────────────────
    // Restore Saved Session
    // ─────────────────────────────────────────

    pub async fn restore_saved_session(&self) {
        let entry = match Entry::new(KEYRING_SERVICE, KEYRING_USER) {
            Ok(entry) => entry,
            Err(error) => {
                set_error(
                    &self.status,
                    format!("Could not access Windows Credential Manager: {error}"),
                )
                .await;
                return;
            }
        };

        let serialized = match entry.get_password() {
            Ok(serialized) => serialized,
            Err(keyring::Error::NoEntry) => return,
            Err(error) => {
                set_error(
                    &self.status,
                    format!("Could not read saved Twitch authorization: {error}"),
                )
                .await;
                return;
            }
        };

        let saved: StoredTokens = match serde_json::from_str(&serialized) {
            Ok(saved) => saved,
            Err(error) => {
                set_error(
                    &self.status,
                    format!("Saved Twitch authorization data is invalid: {error}"),
                )
                .await;
                return;
            }
        };

        match validate_token(&self.client, &saved.access_token).await {
            Ok(validated)
                if validated.login.eq_ignore_ascii_case(EXPECTED_LOGIN)
                    && validated.scopes.iter().any(|scope| scope == REQUIRED_SCOPE) =>
            {
                let mut current = self.status.lock().await;
                current.state = "connected".to_owned();
                current.message = format!("Connected to Twitch as @{}.", validated.login);
                current.login = Some(validated.login);
                current.scopes = validated.scopes;
                current.expires_in = Some(validated.expires_in);
            }

            _ => {
                // Attempt to refresh the saved token if validation fails.
                let refresh_response = self
                    .client
                    .post(TOKEN_URL)
                    .form(&[
                        ("client_id", CLIENT_ID),
                        ("grant_type", "refresh_token"),
                        ("refresh_token", saved.refresh_token.as_str()),
                    ])
                    .send()
                    .await;

                let refreshed = match refresh_response {
                    Ok(response) if response.status().is_success() => {
                        match response.json::<TokenResponse>().await {
                            Ok(token) => token,
                            Err(error) => {
                                set_error(
                                    &self.status,
                                    format!("Twitch token refresh response was invalid: {error}"),
                                )
                                .await;
                                return;
                            }
                        }
                    }

                    Ok(response) => {
                        set_error(
                            &self.status,
                            format!(
                                "Saved Twitch authorization needs reconnecting (refresh returned HTTP {}).",
                                response.status()
                            ),
                        )
                        .await;
                        return;
                    }

                    Err(error) => {
                        set_error(
                            &self.status,
                            format!("Could not refresh Twitch authorization: {error}"),
                        )
                        .await;
                        return;
                    }
                };

                match validate_and_store(&self.client, refreshed).await {
                    Ok(validated) => {
                        let mut current = self.status.lock().await;
                        current.state = "connected".to_owned();
                        current.message = format!("Connected to Twitch as @{}.", validated.login);
                        current.login = Some(validated.login);
                        current.scopes = validated.scopes;
                        current.expires_in = Some(validated.expires_in);
                    }
                    Err(error) => set_error(&self.status, error).await,
                }
            }
        }
    }

    // ─────────────────────────────────────────
    // EventSub Credentials
    // ─────────────────────────────────────────

    pub async fn eventsub_credentials(&self) -> Result<EventSubCredentials, String> {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|error| format!("Could not access saved Twitch credentials: {error}"))?;

        let serialized = entry
            .get_password()
            .map_err(|error| format!("Could not read saved Twitch credentials: {error}"))?;

        let saved: StoredTokens = serde_json::from_str(&serialized)
            .map_err(|error| format!("Saved Twitch credentials are invalid: {error}"))?;

        let validated = validate_token(&self.client, &saved.access_token).await?;

        if !validated.login.eq_ignore_ascii_case(EXPECTED_LOGIN) {
            return Err(format!(
                "Expected Twitch account @{EXPECTED_LOGIN}, but the token belongs to @{}.",
                validated.login
            ));
        }

        if !validated.scopes.iter().any(|scope| scope == REQUIRED_SCOPE) {
            return Err(format!(
                "The Twitch token is missing the required scope: {REQUIRED_SCOPE}"
            ));
        }

        Ok(EventSubCredentials {
            access_token: saved.access_token,
            user_id: validated.user_id,
        })
    }

    // ─────────────────────────────────────────
    // Start Device Code Authorization
    // ─────────────────────────────────────────

    pub async fn start(&self) -> Result<AuthStatus, String> {
        {
            let status = self.status.lock().await;

            if status.state == "authorizing" || status.state == "connected" {
                return Ok(status.clone());
            }
        }

        let response = self
            .client
            .post(DEVICE_URL)
            .form(&[("client_id", CLIENT_ID), ("scopes", REQUIRED_SCOPE)])
            .send()
            .await
            .map_err(|error| format!("Could not contact Twitch device authorization: {error}"))?;

        if !response.status().is_success() {
            let status_code = response.status();
            let body = response.text().await.unwrap_or_default();

            return Err(format!(
                "Twitch rejected device authorization ({status_code}): {body}"
            ));
        }

        let device: DeviceCodeResponse = response.json().await.map_err(|error| {
            format!("Twitch returned an invalid device authorization response: {error}")
        })?;

        let new_status = AuthStatus {
            state: "authorizing".to_owned(),
            message: "Open the verification page and enter the code. Waiting for authorization."
                .to_owned(),
            user_code: Some(device.user_code.clone()),
            verification_uri: Some(device.verification_uri.clone()),
            expires_in: Some(device.expires_in),
            login: None,
            scopes: Vec::new(),
        };

        *self.status.lock().await = new_status.clone();

        let client = self.client.clone();
        let status = Arc::clone(&self.status);

        tokio::spawn(async move {
            poll_device_authorization(client, status, device).await;
        });

        Ok(new_status)
    }
}

// ─────────────────────────────────────────────
// Device Code Authorization Polling
// ─────────────────────────────────────────────

async fn poll_device_authorization(
    client: Client,
    status: Arc<Mutex<AuthStatus>>,
    device: DeviceCodeResponse,
) {
    let mut interval = device.interval.max(1);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(device.expires_in);

    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(interval)).await;

        let response = match client
            .post(TOKEN_URL)
            .form(&[
                ("client_id", CLIENT_ID),
                ("device_code", device.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("scopes", REQUIRED_SCOPE),
            ])
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                set_error(
                    &status,
                    format!("Could not poll Twitch authorization: {error}"),
                )
                .await;
                return;
            }
        };

        let status_code = response.status();

        let body = match response.text().await {
            Ok(body) => body,
            Err(error) => {
                set_error(
                    &status,
                    format!("Could not read Twitch authorization response: {error}"),
                )
                .await;
                return;
            }
        };

        if status_code.is_success() {
            let token: TokenResponse = match serde_json::from_str(&body) {
                Ok(token) => token,
                Err(error) => {
                    set_error(
                        &status,
                        format!("Twitch returned an invalid token response: {error}"),
                    )
                    .await;
                    return;
                }
            };

            match validate_and_store(&client, token).await {
                Ok(validated) => {
                    let mut current = status.lock().await;
                    current.state = "connected".to_owned();
                    current.message = format!("Connected to Twitch as @{}.", validated.login);
                    current.user_code = None;
                    current.verification_uri = None;
                    current.expires_in = Some(validated.expires_in);
                    current.login = Some(validated.login);
                    current.scopes = validated.scopes;
                }
                Err(error) => set_error(&status, error).await,
            }

            return;
        }

        let error_body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);

        match error_body
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error_body.get("error").and_then(Value::as_str))
        {
            Some("authorization_pending") => continue,

            Some("slow_down") => {
                interval = interval.saturating_add(5);
            }

            Some("access_denied") => {
                set_error(&status, "Twitch authorization was denied.".to_owned()).await;
                return;
            }

            Some("expired_token") => {
                set_error(
                    &status,
                    "The Twitch authorization code expired. Start again to get a new code."
                        .to_owned(),
                )
                .await;
                return;
            }

            Some(message) => {
                set_error(&status, format!("Twitch authorization failed: {message}")).await;
                return;
            }

            None => {
                set_error(
                    &status,
                    format!("Twitch authorization failed (HTTP {status_code})."),
                )
                .await;
                return;
            }
        }
    }

    set_error(
        &status,
        "The Twitch authorization code expired. Start again to get a new code.".to_owned(),
    )
    .await;
}

// ─────────────────────────────────────────────
// Token Validation
// ─────────────────────────────────────────────

async fn validate_token(client: &Client, access_token: &str) -> Result<ValidatedToken, String> {
    let response = client
        .get(VALIDATE_URL)
        .header("Authorization", format!("OAuth {access_token}"))
        .send()
        .await
        .map_err(|error| format!("Could not validate the Twitch token: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Twitch token validation failed (HTTP {}).",
            response.status()
        ));
    }

    response
        .json::<ValidatedToken>()
        .await
        .map_err(|error| format!("Twitch returned an invalid token validation response: {error}"))
}

// ─────────────────────────────────────────────
// Validate and Persist Credentials
// ─────────────────────────────────────────────

async fn validate_and_store(
    client: &Client,
    token: TokenResponse,
) -> Result<ValidatedToken, String> {
    let validated = validate_token(client, &token.access_token).await?;

    if !validated.login.eq_ignore_ascii_case(EXPECTED_LOGIN) {
        return Err(format!(
            "Authorized as @{}, but StreamFreezer is currently configured for @{}. No tokens were saved.",
            validated.login, EXPECTED_LOGIN
        ));
    }

    if !validated.scopes.iter().any(|scope| scope == REQUIRED_SCOPE) {
        return Err(format!(
            "Twitch authorization is missing the required scope: {REQUIRED_SCOPE}. No tokens were saved."
        ));
    }

    let stored = StoredTokens {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
    };

    let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|error| format!("Could not access Windows Credential Manager: {error}"))?;

    let serialized = serde_json::to_string(&stored)
        .map_err(|error| format!("Could not prepare Twitch credentials for storage: {error}"))?;

    entry.set_password(&serialized).map_err(|error| {
        format!("Could not save Twitch credentials to Windows Credential Manager: {error}")
    })?;

    // Verify that the credentials can be read back accurately.
    let saved_check = entry.get_password().map_err(|error| {
        format!("Twitch credentials were saved but could not be read back: {error}")
    })?;

    if saved_check != serialized {
        return Err("Saved Twitch credentials did not match the stored data.".to_owned());
    }

    println!("Twitch credentials saved and verified in Windows Credential Manager.");

    Ok(validated)
}

// ─────────────────────────────────────────────
// Authentication Error State
// ─────────────────────────────────────────────

async fn set_error(status: &Arc<Mutex<AuthStatus>>, message: String) {
    let mut current = status.lock().await;

    current.state = "error".to_owned();
    current.message = message;
    current.user_code = None;
    current.verification_uri = None;
    current.expires_in = None;
}

// ─────────────────────────────────────────────
// Authentication Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::AuthStatus;

    #[test]
    fn default_status_is_disconnected_and_does_not_expose_tokens() {
        let status = AuthStatus::default();

        assert_eq!(status.state, "disconnected");

        let json = serde_json::to_value(status).expect("status should serialize");

        assert!(json.get("access_token").is_none());
        assert!(json.get("refresh_token").is_none());
    }
}
