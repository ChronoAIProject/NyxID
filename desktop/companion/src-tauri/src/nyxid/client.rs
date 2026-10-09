use std::fmt;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use reqwest::{Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use super::chat::{
    NyxIdChatAdmission, NyxIdChatAdmissionWire, NyxIdChatHistory, NyxIdChatHistoryWire,
    NyxIdChatRequest, valid_conversation_id, valid_request_id,
};
use super::model::{
    CredentialBundle, NyxIdCapabilities, NyxIdService, NyxIdServiceState, NyxIdUser,
    valid_device_code,
};

pub(crate) const API_ORIGIN: &str = "https://nyx-api.chrono-ai.fun";
const LOGIN_ORIGIN: &str = "nyx.chrono-ai.fun";
const LOGIN_PATH: &str = "/login/device";
const ERROR_BODY_LIMIT: usize = 64 * 1024;
const PROFILE_BODY_LIMIT: usize = 256 * 1024;
const KEYS_BODY_LIMIT: usize = 8 * 1024 * 1024;
const ASSISTANT_HISTORY_BODY_LIMIT: usize = 8 * 1024 * 1024;
const ASSISTANT_ADMISSION_BODY_LIMIT: usize = 64 * 1024;
const ASSISTANT_TURN_TIMEOUT: Duration = Duration::from_secs(660);

pub(crate) enum AssistantAdmissionLookup {
    Found(NyxIdChatAdmission),
    ExplicitlyAbsent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssistantStartErrorKind {
    Unauthorized,
    Network,
    Rejected,
}

#[derive(Clone, Debug)]
pub(crate) struct AssistantStartError {
    pub(crate) kind: AssistantStartErrorKind,
    pub(crate) message: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientErrorKind {
    Unauthorized,
    InvalidResponse,
    Transient,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefreshStatusClass {
    Success,
    Transient,
    Reauthenticate,
    Invalid,
}

#[derive(Clone)]
pub(crate) struct ClientError {
    pub(crate) kind: ClientErrorKind,
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
    pub(crate) retryable: bool,
}

impl ClientError {
    fn new(
        kind: ClientErrorKind,
        code: &'static str,
        message: &'static str,
        retryable: bool,
    ) -> Self {
        Self {
            kind,
            code,
            message,
            retryable,
        }
    }

    fn network() -> Self {
        Self::new(
            ClientErrorKind::Transient,
            "nyxid_unreachable",
            "暂时无法连接 NyxID，请稍后重试。",
            true,
        )
    }

    fn invalid_response() -> Self {
        Self::new(
            ClientErrorKind::InvalidResponse,
            "nyxid_invalid_response",
            "NyxID 返回了无法识别的数据，请稍后重试。",
            true,
        )
    }

    fn unauthorized() -> Self {
        Self::new(
            ClientErrorKind::Unauthorized,
            "nyxid_session_expired",
            "NyxID 登录已失效，请重新连接。",
            false,
        )
    }
}

impl fmt::Debug for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientError")
            .field("kind", &self.kind)
            .field("code", &self.code)
            .field("retryable", &self.retryable)
            .finish()
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for ClientError {}

pub(crate) struct LoginChallenge {
    pub(crate) device_code: Zeroizing<String>,
    pub(crate) recovery_secret: Zeroizing<String>,
    pub(crate) user_code: String,
    pub(crate) verification_url: Url,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) interval: Duration,
}

impl fmt::Debug for LoginChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginChallenge")
            .field("device_code", &"[REDACTED]")
            .field("recovery_secret", &"[REDACTED]")
            .field("user_code", &self.user_code)
            .field("verification_url", &self.verification_url)
            .field("expires_at", &self.expires_at)
            .field("interval", &self.interval)
            .finish()
    }
}

pub(crate) enum PollOutcome {
    Pending,
    SlowDown(Duration),
    RateLimited(Duration),
    Denied,
    Expired,
    AlreadyDelivered,
    Delivered(CredentialBundle),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogoutOutcome {
    Complete,
    Unauthorized,
    Retryable,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum CancelProbeOutcome {
    Complete,
    Retryable,
}

#[cfg(test)]
#[derive(Clone)]
struct CancelProbe {
    expected_device_code: Zeroizing<String>,
    expected_recovery_secret: Zeroizing<String>,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    outcome: CancelProbeOutcome,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum AccountProbeMode {
    ProfileAlwaysUnauthorized,
    CapabilitiesAlwaysUnauthorized,
}

#[cfg(test)]
#[derive(Clone)]
struct AccountProbe {
    mode: AccountProbeMode,
    profile_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    capabilities_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(test)]
pub(crate) struct AccountProbeHandles {
    pub(crate) logout_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) refresh_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) profile_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) capabilities_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum AssistantAdmissionProbeOutcome {
    ExplicitlyAbsent,
    Network,
    Found {
        active: bool,
        conversation_deleted: bool,
    },
}

#[cfg(test)]
#[derive(Clone)]
struct AssistantRecoveryProbe {
    admission: AssistantAdmissionProbeOutcome,
    history: Option<NyxIdChatHistory>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PollErrorClass {
    Pending,
    SlowDown,
    Denied,
    Expired,
    AlreadyDelivered,
    RateLimited,
    Transient,
    Invalid,
}

#[derive(Clone)]
pub(crate) struct NyxIdClient {
    http: reqwest::Client,
    #[cfg(test)]
    logout_probe: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    #[cfg(test)]
    logout_probe_outcome: LogoutOutcome,
    #[cfg(test)]
    refresh_probe: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    #[cfg(test)]
    refresh_probe_unauthorized: bool,
    #[cfg(test)]
    cancel_probe: Option<CancelProbe>,
    #[cfg(test)]
    account_probe: Option<AccountProbe>,
    #[cfg(test)]
    assistant_recovery_probe: Option<AssistantRecoveryProbe>,
}

impl NyxIdClient {
    pub(crate) fn new() -> Result<Self, reqwest::Error> {
        reqwest::Client::builder()
            .user_agent("NyxID-Companion/0.1")
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map(|http| Self {
                http,
                #[cfg(test)]
                logout_probe: None,
                #[cfg(test)]
                logout_probe_outcome: LogoutOutcome::Complete,
                #[cfg(test)]
                refresh_probe: None,
                #[cfg(test)]
                refresh_probe_unauthorized: false,
                #[cfg(test)]
                cancel_probe: None,
                #[cfg(test)]
                account_probe: None,
                #[cfg(test)]
                assistant_recovery_probe: None,
            })
    }

    #[cfg(test)]
    pub(crate) fn with_logout_probe() -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        Self::with_logout_probe_outcome(LogoutOutcome::Complete)
    }

    #[cfg(test)]
    pub(crate) fn with_logout_probe_outcome(
        outcome: LogoutOutcome,
    ) -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let probe = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut client = Self::new().expect("test HTTP client");
        client.logout_probe = Some(probe.clone());
        client.logout_probe_outcome = outcome;
        (client, probe)
    }

    #[cfg(test)]
    pub(crate) fn with_logout_and_refresh_probe() -> (
        Self,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        Self::with_logout_outcome_and_refresh_probe(LogoutOutcome::Complete)
    }

    #[cfg(test)]
    pub(crate) fn with_logout_outcome_and_refresh_probe(
        outcome: LogoutOutcome,
    ) -> (
        Self,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let (mut client, logout_probe) = Self::with_logout_probe_outcome(outcome);
        let refresh_probe = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        client.refresh_probe = Some(refresh_probe.clone());
        (client, logout_probe, refresh_probe)
    }

    #[cfg(test)]
    pub(crate) fn with_unauthorized_refresh_probe()
    -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let probe = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut client = Self::new().expect("test HTTP client");
        client.refresh_probe = Some(probe.clone());
        client.refresh_probe_unauthorized = true;
        (client, probe)
    }

    #[cfg(test)]
    pub(crate) fn with_cancel_probe(
        expected_device_code: String,
        expected_recovery_secret: String,
        outcome: CancelProbeOutcome,
    ) -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut client = Self::new().expect("test HTTP client");
        client.cancel_probe = Some(CancelProbe {
            expected_device_code: Zeroizing::new(expected_device_code),
            expected_recovery_secret: Zeroizing::new(expected_recovery_secret),
            calls: calls.clone(),
            outcome,
        });
        (client, calls)
    }

    #[cfg(test)]
    pub(crate) fn with_repeated_account_unauthorized_probe(
        mode: AccountProbeMode,
    ) -> (Self, AccountProbeHandles) {
        let (mut client, logout_calls, refresh_calls) = Self::with_logout_and_refresh_probe();
        let profile_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let capabilities_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        client.account_probe = Some(AccountProbe {
            mode,
            profile_calls: profile_calls.clone(),
            capabilities_calls: capabilities_calls.clone(),
        });
        (
            client,
            AccountProbeHandles {
                logout_calls,
                refresh_calls,
                profile_calls,
                capabilities_calls,
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn with_assistant_recovery_probe(
        admission: AssistantAdmissionProbeOutcome,
        history: Option<NyxIdChatHistory>,
    ) -> Self {
        let mut client = Self::new().expect("test HTTP client");
        client.assistant_recovery_probe = Some(AssistantRecoveryProbe { admission, history });
        client
    }

    pub(crate) async fn request_login(&self) -> Result<LoginChallenge, ClientError> {
        #[derive(Serialize)]
        struct Request<'a> {
            recovery_secret: &'a str,
            client_label: &'a str,
            client_app: &'a str,
            client_platform: &'a str,
            client_timezone: Option<&'a str>,
        }

        #[derive(Deserialize, zeroize::Zeroize)]
        #[zeroize(drop)]
        struct Challenge {
            device_code: String,
            user_code: String,
            verification_uri_complete: String,
            expires_in: i64,
            interval: u64,
        }

        let timezone = iana_time_zone::get_timezone().ok();
        let mut recovery_secret = generate_recovery_secret()?;
        let response = self
            .http
            .post(format!("{API_ORIGIN}/api/v1/auth/device/request"))
            .json(&Request {
                recovery_secret: recovery_secret.as_str(),
                client_label: "NyxID Companion",
                client_app: "nyxid-companion",
                client_platform: std::env::consts::OS,
                client_timezone: timezone.as_deref(),
            })
            .send()
            .await
            .map_err(|_| ClientError::network())?;
        if response.status() != StatusCode::OK {
            return Err(error_for_response(response).await);
        }
        let mut challenge: Challenge = read_bounded_json(response, PROFILE_BODY_LIMIT).await?;
        if !valid_device_code(&challenge.device_code)
            || !(4..=32).contains(&challenge.user_code.len())
            || challenge.expires_in <= 0
            || challenge.expires_in > 3600
        {
            return Err(ClientError::invalid_response());
        }
        let verification_url =
            validate_verification_url(&challenge.verification_uri_complete, &challenge.user_code)?;
        let device_code = std::mem::take(&mut challenge.device_code);
        let user_code = std::mem::take(&mut challenge.user_code);
        let recovery_secret = std::mem::take(&mut *recovery_secret);
        Ok(LoginChallenge {
            device_code: Zeroizing::new(device_code),
            recovery_secret: Zeroizing::new(recovery_secret),
            user_code,
            verification_url,
            expires_at: Utc::now() + chrono::Duration::seconds(challenge.expires_in),
            interval: Duration::from_secs(challenge.interval.clamp(5, 60)),
        })
    }

    pub(crate) async fn cancel_login(
        &self,
        device_code: &str,
        recovery_secret: &str,
    ) -> Result<(), ClientError> {
        #[cfg(test)]
        if let Some(probe) = self.cancel_probe.as_ref() {
            probe
                .calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if probe.expected_device_code.as_str() != device_code
                || probe.expected_recovery_secret.as_str() != recovery_secret
            {
                return Err(ClientError::invalid_response());
            }
            return match probe.outcome {
                CancelProbeOutcome::Complete => Ok(()),
                CancelProbeOutcome::Retryable => Err(ClientError::network()),
            };
        }

        #[derive(Serialize)]
        struct CancelRequest<'a> {
            device_code: &'a str,
            recovery_secret: &'a str,
        }
        #[derive(Deserialize)]
        struct CancelResponse {
            ok: bool,
        }

        let response = self
            .http
            .post(format!("{API_ORIGIN}/api/v1/auth/device/cancel"))
            .timeout(Duration::from_secs(5))
            .json(&CancelRequest {
                device_code,
                recovery_secret,
            })
            .send()
            .await
            .map_err(|_| ClientError::network())?;
        if response.status() != StatusCode::OK {
            return Err(error_for_response(response).await);
        }
        let completion: CancelResponse = read_bounded_json(response, ERROR_BODY_LIMIT).await?;
        completion
            .ok
            .then_some(())
            .ok_or_else(ClientError::invalid_response)
    }

    pub(crate) async fn poll_login(&self, device_code: &str) -> Result<PollOutcome, ClientError> {
        #[derive(Serialize)]
        struct PollRequest<'a> {
            device_code: &'a str,
        }
        #[derive(Deserialize, zeroize::Zeroize)]
        #[zeroize(drop)]
        struct Delivery {
            auth_kind: String,
            access_token: Option<String>,
            refresh_token: Option<String>,
            expires_in: Option<i64>,
        }

        let response = self
            .http
            .post(format!("{API_ORIGIN}/api/v1/auth/device/poll"))
            .json(&PollRequest { device_code })
            .send()
            .await
            .map_err(|_| ClientError::network())?;
        if response.status().is_success() {
            let mut delivery: Delivery = read_bounded_json(response, PROFILE_BODY_LIMIT).await?;
            if delivery.auth_kind != "account_session" {
                return Err(ClientError::invalid_response());
            }
            let expires_in = delivery
                .expires_in
                .filter(|seconds| *seconds > 0 && *seconds <= 86_400)
                .ok_or_else(ClientError::invalid_response)?;
            let bundle = CredentialBundle::new(
                delivery
                    .access_token
                    .take()
                    .ok_or_else(ClientError::invalid_response)?,
                delivery
                    .refresh_token
                    .take()
                    .ok_or_else(ClientError::invalid_response)?,
                Utc::now() + chrono::Duration::seconds(expires_in),
            )
            .ok_or_else(ClientError::invalid_response)?;
            return Ok(PollOutcome::Delivered(bundle));
        }

        let retry_after = retry_after(&response);
        let status = response.status();
        let envelope: ErrorEnvelope = read_bounded_json(response, ERROR_BODY_LIMIT)
            .await
            .unwrap_or_default();
        match classify_poll_error(envelope.error_code, status) {
            PollErrorClass::Pending => Ok(PollOutcome::Pending),
            PollErrorClass::SlowDown => Ok(PollOutcome::SlowDown(
                envelope
                    .interval
                    .map(Duration::from_secs)
                    .unwrap_or(Duration::ZERO),
            )),
            PollErrorClass::Denied => Ok(PollOutcome::Denied),
            PollErrorClass::Expired => Ok(PollOutcome::Expired),
            PollErrorClass::AlreadyDelivered => Ok(PollOutcome::AlreadyDelivered),
            PollErrorClass::RateLimited => Ok(PollOutcome::RateLimited(
                retry_after.unwrap_or(Duration::from_secs(10)),
            )),
            PollErrorClass::Transient => Err(ClientError::network()),
            PollErrorClass::Invalid => Err(ClientError::invalid_response()),
        }
    }

    pub(crate) async fn refresh(
        &self,
        refresh_token: &str,
    ) -> Result<CredentialBundle, ClientError> {
        #[cfg(test)]
        if let Some(probe) = self.refresh_probe.as_ref() {
            let _ = refresh_token;
            probe.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.refresh_probe_unauthorized {
                return Err(ClientError::unauthorized());
            }
            return Ok(CredentialBundle::new(
                "refreshed-access".into(),
                "refreshed-refresh".into(),
                Utc::now() + chrono::Duration::hours(1),
            )
            .expect("test refresh bundle"));
        }
        #[derive(Serialize)]
        struct RefreshRequest<'a> {
            refresh_token: &'a str,
        }
        #[derive(Deserialize, zeroize::Zeroize)]
        #[zeroize(drop)]
        struct RefreshResponse {
            access_token: String,
            refresh_token: String,
            expires_in: i64,
        }

        let response = self
            .http
            .post(format!("{API_ORIGIN}/api/v1/auth/refresh"))
            .json(&RefreshRequest { refresh_token })
            .send()
            .await
            .map_err(|_| ClientError::network())?;
        match classify_refresh_status(response.status()) {
            RefreshStatusClass::Success => {}
            RefreshStatusClass::Transient => {
                let _ = read_bounded_json::<ErrorEnvelope>(response, ERROR_BODY_LIMIT).await;
                return Err(ClientError::network());
            }
            RefreshStatusClass::Reauthenticate => {
                let _ = read_bounded_json::<ErrorEnvelope>(response, ERROR_BODY_LIMIT).await;
                return Err(ClientError::unauthorized());
            }
            RefreshStatusClass::Invalid => {
                let _ = read_bounded_json::<ErrorEnvelope>(response, ERROR_BODY_LIMIT).await;
                return Err(ClientError::invalid_response());
            }
        }
        let mut refreshed: RefreshResponse =
            read_bounded_json(response, PROFILE_BODY_LIMIT).await?;
        let expires_in = refreshed.expires_in;
        if expires_in <= 0 || expires_in > 86_400 {
            return Err(ClientError::invalid_response());
        }
        CredentialBundle::new(
            std::mem::take(&mut refreshed.access_token),
            std::mem::take(&mut refreshed.refresh_token),
            Utc::now() + chrono::Duration::seconds(expires_in),
        )
        .ok_or_else(ClientError::invalid_response)
    }

    pub(crate) async fn get_me(&self, access_token: &str) -> Result<NyxIdUser, ClientError> {
        #[cfg(test)]
        if let Some(probe) = self.account_probe.as_ref() {
            let _ = access_token;
            probe
                .profile_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return match probe.mode {
                AccountProbeMode::ProfileAlwaysUnauthorized => Err(ClientError::unauthorized()),
                AccountProbeMode::CapabilitiesAlwaysUnauthorized => Ok(NyxIdUser {
                    id: "user-1".into(),
                    email: "user@example.com".into(),
                    display_name: None,
                    avatar_url: None,
                }),
            };
        }

        #[derive(Deserialize)]
        struct Profile {
            id: String,
            email: String,
            display_name: Option<String>,
            avatar_url: Option<String>,
        }

        let response = self
            .http
            .get(format!("{API_ORIGIN}/api/v1/users/me"))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| ClientError::network())?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(ClientError::unauthorized());
        }
        if !response.status().is_success() {
            return Err(error_for_response(response).await);
        }
        let profile: Profile = read_bounded_json(response, PROFILE_BODY_LIMIT).await?;
        if !valid_short_text(&profile.id)
            || profile.email.trim().is_empty()
            || profile.email.len() > 320
            || profile
                .display_name
                .as_ref()
                .is_some_and(|value| value.encode_utf16().count() > 256)
            || profile
                .avatar_url
                .as_ref()
                .is_some_and(|value| !valid_avatar_url(value))
        {
            return Err(ClientError::invalid_response());
        }
        Ok(NyxIdUser {
            id: profile.id,
            email: profile.email,
            display_name: profile.display_name,
            avatar_url: profile.avatar_url,
        })
    }

    pub(crate) async fn get_capabilities(
        &self,
        access_token: &str,
    ) -> Result<NyxIdCapabilities, ClientError> {
        #[cfg(test)]
        if let Some(probe) = self.account_probe.as_ref() {
            let _ = access_token;
            probe
                .capabilities_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return match probe.mode {
                AccountProbeMode::ProfileAlwaysUnauthorized => {
                    Ok(NyxIdCapabilities::from_services(Vec::new(), Utc::now()))
                }
                AccountProbeMode::CapabilitiesAlwaysUnauthorized => {
                    Err(ClientError::unauthorized())
                }
            };
        }

        #[derive(Deserialize)]
        struct KeyList {
            keys: Vec<Key>,
        }
        #[derive(Deserialize)]
        struct Key {
            id: String,
            slug: String,
            label: String,
            is_active: bool,
            #[serde(default)]
            credential_missing: bool,
            #[serde(default)]
            connected: bool,
            #[serde(default)]
            requires_connection: bool,
            #[serde(default)]
            has_node_binding: bool,
            node_status: Option<String>,
            status: String,
            connection_status: Option<String>,
        }

        let response = self
            .http
            .get(format!("{API_ORIGIN}/api/v1/keys"))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| ClientError::network())?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(ClientError::unauthorized());
        }
        if !response.status().is_success() {
            return Err(error_for_response(response).await);
        }
        let list: KeyList = read_bounded_json(response, KEYS_BODY_LIMIT).await?;
        let services = list
            .keys
            .into_iter()
            .map(|key| -> Result<NyxIdService, ClientError> {
                if !valid_short_text(&key.id)
                    || !valid_short_text(&key.slug)
                    || !valid_short_text(&key.label)
                {
                    return Err(ClientError::invalid_response());
                }
                let unhealthy_connection = (key.requires_connection && !key.connected)
                    || is_unhealthy_connection_status(key.connection_status.as_deref());
                let unhealthy_node = key.has_node_binding
                    && key
                        .node_status
                        .as_deref()
                        .is_none_or(|status| !status.eq_ignore_ascii_case("online"));
                let state = classify_service_state(
                    key.is_active,
                    key.credential_missing,
                    unhealthy_connection,
                    unhealthy_node,
                    is_unusable_credential_status(&key.status),
                );
                Ok(NyxIdService {
                    id: key.id,
                    slug: key.slug,
                    label: key.label,
                    state,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(NyxIdCapabilities::from_services(services, Utc::now()))
    }

    pub(crate) async fn start_assistant_turn(
        &self,
        access_token: &str,
        request: &NyxIdChatRequest,
    ) -> Result<Response, AssistantStartError> {
        #[derive(Serialize)]
        struct TurnBody<'a> {
            conversation_id: Option<&'a str>,
            text: &'a str,
        }

        let response = self
            .assistant_turn_request(
                access_token,
                &request.request_id,
                &TurnBody {
                    conversation_id: request.conversation_id.as_deref(),
                    text: &request.text,
                },
            )
            .send()
            .await
            .map_err(|_| AssistantStartError {
                kind: AssistantStartErrorKind::Network,
                message: "暂时无法连接 NyxID 助手，请稍后重试。",
            })?;
        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status();
        let _ = read_bounded_json::<serde_json::Value>(response, ERROR_BODY_LIMIT).await;
        Err(classify_assistant_turn_error(status))
    }

    fn assistant_turn_request<T: Serialize + ?Sized>(
        &self,
        access_token: &str,
        request_id: &str,
        body: &T,
    ) -> reqwest::RequestBuilder {
        self.http
            .post(format!(
                "{API_ORIGIN}/api/v1/assistant/nyxagent/turns/idempotent"
            ))
            .bearer_auth(access_token)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .header("Idempotency-Key", request_id)
            .timeout(ASSISTANT_TURN_TIMEOUT)
            .json(body)
    }

    pub(crate) async fn resolve_assistant_admission(
        &self,
        access_token: &str,
        request_id: &str,
    ) -> Result<AssistantAdmissionLookup, AssistantStartError> {
        if !valid_request_id(request_id) {
            return Err(AssistantStartError {
                kind: AssistantStartErrorKind::Rejected,
                message: "NyxID 对话请求标识无效。",
            });
        }
        #[cfg(test)]
        if let Some(probe) = self.assistant_recovery_probe.as_ref() {
            let _ = access_token;
            return match probe.admission {
                AssistantAdmissionProbeOutcome::ExplicitlyAbsent => {
                    Ok(AssistantAdmissionLookup::ExplicitlyAbsent)
                }
                AssistantAdmissionProbeOutcome::Network => Err(AssistantStartError {
                    kind: AssistantStartErrorKind::Network,
                    message: "暂时无法确认 NyxID 对话状态，请稍后重试。",
                }),
                AssistantAdmissionProbeOutcome::Found {
                    active,
                    conversation_deleted,
                } => Ok(AssistantAdmissionLookup::Found(NyxIdChatAdmission {
                    client_request_id: request_id.to_owned(),
                    conversation_id: "nyxa-0123456789abcdef0123456789abcdef".to_owned(),
                    turn_id: "turn-recovery-probe".to_owned(),
                    active,
                    conversation_deleted,
                })),
            };
        }
        let response = self
            .assistant_admission_request(access_token, request_id)
            .send()
            .await
            .map_err(|_| AssistantStartError {
                kind: AssistantStartErrorKind::Network,
                message: "暂时无法确认 NyxID 对话状态，请稍后重试。",
            })?;
        if assistant_admission_explicitly_absent(response.status()) {
            return Ok(AssistantAdmissionLookup::ExplicitlyAbsent);
        }
        if response.status() == StatusCode::OK {
            let admission: NyxIdChatAdmissionWire =
                read_bounded_json(response, ASSISTANT_ADMISSION_BODY_LIMIT)
                    .await
                    .map_err(|_| AssistantStartError {
                        kind: AssistantStartErrorKind::Network,
                        message: "NyxID 返回了无法识别的对话受理记录，请稍后重试。",
                    })?;
            return admission
                .into_public(request_id)
                .map(AssistantAdmissionLookup::Found)
                .map_err(|_| AssistantStartError {
                    kind: AssistantStartErrorKind::Network,
                    message: "NyxID 返回了无法识别的对话受理记录，请稍后重试。",
                });
        }

        let status = response.status();
        let _ = read_bounded_json::<serde_json::Value>(response, ERROR_BODY_LIMIT).await;
        let (kind, message) = match status {
            StatusCode::UNAUTHORIZED => (
                AssistantStartErrorKind::Unauthorized,
                "NyxID 登录已失效，请重新连接。",
            ),
            _ => (
                AssistantStartErrorKind::Network,
                "暂时无法确认 NyxID 对话状态，请稍后重试。",
            ),
        };
        Err(AssistantStartError { kind, message })
    }

    fn assistant_admission_request(
        &self,
        access_token: &str,
        request_id: &str,
    ) -> reqwest::RequestBuilder {
        self.http
            .post(format!(
                "{API_ORIGIN}/api/v1/assistant/nyxagent/turn-admissions/{request_id}/resolve"
            ))
            .bearer_auth(access_token)
    }

    pub(crate) async fn get_assistant_history(
        &self,
        access_token: &str,
        conversation_id: &str,
    ) -> Result<NyxIdChatHistory, AssistantStartError> {
        if !valid_conversation_id(conversation_id) {
            return Err(AssistantStartError {
                kind: AssistantStartErrorKind::Rejected,
                message: "NyxID 会话标识无效，请重新打开对话。",
            });
        }

        #[cfg(test)]
        if let Some(probe) = self.assistant_recovery_probe.as_ref() {
            let _ = access_token;
            return probe.history.clone().ok_or(AssistantStartError {
                kind: AssistantStartErrorKind::Network,
                message: "暂时无法读取 NyxID 对话，请稍后重试。",
            });
        }

        let response = self
            .http
            .get(format!(
                "{API_ORIGIN}/api/v1/assistant/nyxagent/conversations/{conversation_id}"
            ))
            .query(&[("limit", 50_u8)])
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| AssistantStartError {
                kind: AssistantStartErrorKind::Network,
                message: "暂时无法读取 NyxID 对话，请稍后重试。",
            })?;
        if response.status() == StatusCode::OK {
            let history: NyxIdChatHistoryWire =
                read_bounded_json(response, ASSISTANT_HISTORY_BODY_LIMIT)
                    .await
                    .map_err(|error| {
                        if error.kind == ClientErrorKind::Transient {
                            AssistantStartError {
                                kind: AssistantStartErrorKind::Network,
                                message: "读取 NyxID 对话时连接中断，请稍后重试。",
                            }
                        } else {
                            AssistantStartError {
                                kind: AssistantStartErrorKind::Rejected,
                                message: "NyxID 返回了无法识别的对话记录。",
                            }
                        }
                    })?;
            return history
                .into_public(conversation_id)
                .map_err(|_| AssistantStartError {
                    kind: AssistantStartErrorKind::Rejected,
                    message: "NyxID 返回了无法识别的对话记录。",
                });
        }

        let status = response.status();
        let _ = read_bounded_json::<serde_json::Value>(response, ERROR_BODY_LIMIT).await;
        let (kind, message) = match status {
            StatusCode::UNAUTHORIZED => (
                AssistantStartErrorKind::Unauthorized,
                "NyxID 登录已失效，请重新连接。",
            ),
            StatusCode::NOT_FOUND => (AssistantStartErrorKind::Rejected, "找不到这段 NyxID 对话。"),
            _ => (
                AssistantStartErrorKind::Rejected,
                "NyxID 暂时无法读取这段对话。",
            ),
        };
        Err(AssistantStartError { kind, message })
    }

    pub(crate) async fn stop_assistant_turn(
        &self,
        access_token: &str,
        conversation_id: &str,
    ) -> Result<(), AssistantStartError> {
        if !valid_conversation_id(conversation_id) {
            return Err(AssistantStartError {
                kind: AssistantStartErrorKind::Rejected,
                message: "NyxID 会话标识无效，请重新打开对话。",
            });
        }

        let response = self
            .http
            .post(format!(
                "{API_ORIGIN}/api/v1/assistant/nyxagent/conversations/{conversation_id}/stop"
            ))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| AssistantStartError {
                kind: AssistantStartErrorKind::Network,
                message: "暂时无法停止 NyxID 对话，请稍后重试。",
            })?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(());
        }

        let status = response.status();
        let _ = read_bounded_json::<serde_json::Value>(response, ERROR_BODY_LIMIT).await;
        let (kind, message) = match status {
            StatusCode::UNAUTHORIZED => (
                AssistantStartErrorKind::Unauthorized,
                "NyxID 登录已失效，请重新连接。",
            ),
            StatusCode::NOT_FOUND => (AssistantStartErrorKind::Rejected, "找不到这段 NyxID 对话。"),
            _ => (
                AssistantStartErrorKind::Rejected,
                "NyxID 暂时无法停止这段对话。",
            ),
        };
        Err(AssistantStartError { kind, message })
    }

    pub(crate) async fn logout(&self, access_token: &str) -> LogoutOutcome {
        #[cfg(test)]
        if let Some(probe) = self.logout_probe.as_ref() {
            let _ = access_token;
            probe.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return self.logout_probe_outcome;
        }
        match self
            .http
            .post(format!("{API_ORIGIN}/api/v1/auth/logout"))
            .bearer_auth(access_token)
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => LogoutOutcome::Complete,
            Ok(response) if response.status() == StatusCode::UNAUTHORIZED => {
                LogoutOutcome::Unauthorized
            }
            Ok(_) | Err(_) => LogoutOutcome::Retryable,
        }
    }
}

fn assistant_admission_explicitly_absent(status: StatusCode) -> bool {
    status == StatusCode::NO_CONTENT
}

#[derive(Default, Deserialize)]
struct ErrorEnvelope {
    error_code: Option<u32>,
    interval: Option<u64>,
}

fn generate_recovery_secret() -> Result<Zeroizing<String>, ClientError> {
    let mut bytes = Zeroizing::new([0_u8; 32]);
    getrandom::fill(&mut *bytes).map_err(|_| {
        ClientError::new(
            ClientErrorKind::Transient,
            "secure_random_unavailable",
            "无法安全创建 NyxID 登录，请重试。",
            true,
        )
    })?;
    Ok(Zeroizing::new(URL_SAFE_NO_PAD.encode(*bytes)))
}

fn retry_after(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.clamp(1, 300)))
}

fn classify_refresh_status(status: StatusCode) -> RefreshStatusClass {
    if status.is_success() {
        RefreshStatusClass::Success
    } else if status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::CONFLICT
        || status.is_server_error()
    {
        RefreshStatusClass::Transient
    } else if status.is_client_error() {
        RefreshStatusClass::Reauthenticate
    } else {
        RefreshStatusClass::Invalid
    }
}

fn classify_assistant_turn_error(status: StatusCode) -> AssistantStartError {
    let (kind, message) = match status {
        StatusCode::UNAUTHORIZED => (
            AssistantStartErrorKind::Unauthorized,
            "NyxID 登录已失效，请重新连接。",
        ),
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => (
            AssistantStartErrorKind::Rejected,
            "这条消息未通过 NyxID 校验，没有执行。",
        ),
        StatusCode::FORBIDDEN => (
            AssistantStartErrorKind::Rejected,
            "当前账号没有权限发送这条消息。",
        ),
        StatusCode::PAYMENT_REQUIRED => (
            AssistantStartErrorKind::Rejected,
            "NyxID 余额不足，这次对话没有执行。",
        ),
        StatusCode::CONFLICT => (
            AssistantStartErrorKind::Rejected,
            "上一条消息仍在处理中，请稍后再发。",
        ),
        StatusCode::TOO_MANY_REQUESTS => (
            AssistantStartErrorKind::Rejected,
            "消息发送得太快了，请稍后再试。",
        ),
        _ => (
            AssistantStartErrorKind::Network,
            "无法确认 NyxID 是否已经接收这条消息，请不要重复发送。",
        ),
    };
    AssistantStartError { kind, message }
}

fn classify_poll_error(error_code: Option<u32>, status: StatusCode) -> PollErrorClass {
    match error_code {
        Some(11202) => PollErrorClass::Pending,
        Some(11203) => PollErrorClass::SlowDown,
        Some(11204) => PollErrorClass::Denied,
        Some(11200 | 11201) => PollErrorClass::Expired,
        Some(11205) => PollErrorClass::AlreadyDelivered,
        Some(11206) => PollErrorClass::RateLimited,
        _ if status == StatusCode::TOO_MANY_REQUESTS => PollErrorClass::RateLimited,
        _ if status == StatusCode::REQUEST_TIMEOUT || status.is_server_error() => {
            PollErrorClass::Transient
        }
        _ => PollErrorClass::Invalid,
    }
}

fn valid_short_text(value: &str) -> bool {
    !value.trim().is_empty() && value.encode_utf16().count() <= 256
}

fn valid_avatar_url(value: &str) -> bool {
    value.len() <= 2048
        && Url::parse(value).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
}

fn classify_service_state(
    is_active: bool,
    credential_missing: bool,
    unhealthy_connection: bool,
    unhealthy_node: bool,
    unusable_credential: bool,
) -> NyxIdServiceState {
    if !is_active {
        NyxIdServiceState::Disabled
    } else if credential_missing || unhealthy_connection || unhealthy_node || unusable_credential {
        NyxIdServiceState::Attention
    } else {
        NyxIdServiceState::Enabled
    }
}

fn is_unhealthy_connection_status(status: Option<&str>) -> bool {
    status.is_some_and(|status| {
        matches!(
            status.to_ascii_lowercase().as_str(),
            "expired" | "revoked" | "failed" | "refresh_failed" | "disconnected" | "error"
        )
    })
}

fn is_unusable_credential_status(status: &str) -> bool {
    status != "active"
}

async fn error_for_response(response: Response) -> ClientError {
    let status = response.status();
    let _ = read_bounded_json::<ErrorEnvelope>(response, ERROR_BODY_LIMIT).await;
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        ClientError::unauthorized()
    } else if status.is_server_error()
        || status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
    {
        ClientError::network()
    } else {
        ClientError::invalid_response()
    }
}

async fn read_bounded_json<T: DeserializeOwned>(
    mut response: Response,
    limit: usize,
) -> Result<T, ClientError> {
    if response
        .content_length()
        .is_some_and(|content_length| content_length > limit as u64)
    {
        return Err(ClientError::invalid_response());
    }
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| ClientError::network())? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(ClientError::invalid_response());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| ClientError::invalid_response())
}

pub(crate) fn validate_verification_url(
    value: &str,
    expected_user_code: &str,
) -> Result<Url, ClientError> {
    let url = Url::parse(value).map_err(|_| ClientError::invalid_response())?;
    let valid_origin = url.scheme() == "https"
        && url.host_str() == Some(LOGIN_ORIGIN)
        && matches!(url.port(), None | Some(443))
        && url.username().is_empty()
        && url.password().is_none();
    let pairs = url.query_pairs().collect::<Vec<_>>();
    let valid_query =
        pairs.len() == 1 && pairs[0].0 == "user_code" && pairs[0].1.as_ref() == expected_user_code;
    if !valid_origin || url.path() != LOGIN_PATH || url.fragment().is_some() || !valid_query {
        return Err(ClientError::invalid_response());
    }
    Ok(url)
}

pub(crate) fn next_retry_delay(current: Duration, hint: Option<Duration>) -> Duration {
    let doubled = current
        .saturating_mul(2)
        .clamp(Duration::from_secs(5), Duration::from_secs(60));
    doubled
        .max(hint.unwrap_or(Duration::ZERO))
        .max(Duration::from_secs(5))
        .min(Duration::from_secs(300))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_exact_companion_verification_url() {
        let code = "ABCD-EFGH";
        assert!(
            validate_verification_url(
                "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-EFGH",
                code
            )
            .is_ok()
        );
        for invalid in [
            "http://nyx.chrono-ai.fun/login/device?user_code=ABCD-EFGH",
            "https://evil.example/login/device?user_code=ABCD-EFGH",
            "https://nyx.chrono-ai.fun/login/device?user_code=WRONG",
            "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-EFGH&next=evil",
            "https://user@nyx.chrono-ai.fun/login/device?user_code=ABCD-EFGH",
            "https://nyx.chrono-ai.fun/assistant?user_code=ABCD-EFGH",
        ] {
            assert!(
                validate_verification_url(invalid, code).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn recovery_secret_is_exact_canonical_base64url_for_32_random_bytes() {
        let secret = generate_recovery_secret().unwrap();
        let decoded = URL_SAFE_NO_PAD.decode(secret.as_bytes()).unwrap();
        assert_eq!(secret.len(), 43);
        assert_eq!(decoded.len(), 32);
        assert_eq!(URL_SAFE_NO_PAD.encode(decoded), secret.as_str());
    }

    #[test]
    fn login_challenge_debug_redacts_device_and_recovery_capabilities() {
        let challenge = LoginChallenge {
            device_code: Zeroizing::new("device-capability".into()),
            recovery_secret: Zeroizing::new("recovery-capability".into()),
            user_code: "ABCD-EFGH".into(),
            verification_url: Url::parse(
                "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-EFGH",
            )
            .unwrap(),
            expires_at: Utc::now(),
            interval: Duration::from_secs(5),
        };

        let debug = format!("{challenge:?}");
        assert!(!debug.contains("device-capability"));
        assert!(!debug.contains("recovery-capability"));
        assert_eq!(debug.matches("[REDACTED]").count(), 2);
    }

    #[tokio::test]
    async fn cancel_probe_is_bound_to_both_capabilities() {
        let (client, calls) = NyxIdClient::with_cancel_probe(
            "expected-device".into(),
            "expected-recovery".into(),
            CancelProbeOutcome::Complete,
        );

        assert!(
            client
                .cancel_login("wrong-device", "expected-recovery")
                .await
                .is_err()
        );
        assert!(
            client
                .cancel_login("expected-device", "wrong-recovery")
                .await
                .is_err()
        );
        assert!(
            client
                .cancel_login("expected-device", "expected-recovery")
                .await
                .is_ok()
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn retry_backoff_is_bounded_and_honors_server_hint() {
        assert_eq!(
            next_retry_delay(Duration::from_secs(5), None),
            Duration::from_secs(10)
        );
        assert_eq!(
            next_retry_delay(Duration::from_secs(40), None),
            Duration::from_secs(60)
        );
        assert_eq!(
            next_retry_delay(Duration::from_secs(5), Some(Duration::from_secs(75))),
            Duration::from_secs(75)
        );
        assert_eq!(
            next_retry_delay(Duration::from_secs(40), Some(Duration::from_secs(10))),
            Duration::from_secs(60)
        );
        assert_eq!(
            next_retry_delay(Duration::from_secs(5), Some(Duration::from_secs(999))),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn active_bound_service_without_node_health_needs_attention() {
        assert_eq!(
            classify_service_state(true, false, false, true, false),
            NyxIdServiceState::Attention
        );
        assert_eq!(
            classify_service_state(false, true, true, true, true),
            NyxIdServiceState::Disabled
        );
    }

    #[test]
    fn unhealthy_credential_evidence_never_disables_an_active_service() {
        assert!(is_unhealthy_connection_status(Some("expired")));
        assert_eq!(
            classify_service_state(true, false, true, false, false),
            NyxIdServiceState::Attention
        );
        assert_eq!(
            classify_service_state(true, false, false, false, true),
            NyxIdServiceState::Attention
        );
        assert_eq!(
            classify_service_state(false, false, false, false, false),
            NyxIdServiceState::Disabled
        );
    }

    #[test]
    fn only_active_credential_status_is_usable() {
        assert!(!is_unusable_credential_status("active"));
        for status in ["disabled", "suspended", "unknown-future-status", "Active"] {
            assert!(is_unusable_credential_status(status), "{status}");
            assert_eq!(
                classify_service_state(
                    true,
                    false,
                    false,
                    false,
                    is_unusable_credential_status(status),
                ),
                NyxIdServiceState::Attention,
                "{status}",
            );
        }
        assert_eq!(
            classify_service_state(
                true,
                false,
                false,
                false,
                is_unusable_credential_status("active"),
            ),
            NyxIdServiceState::Enabled,
        );
    }

    #[test]
    fn validates_frontend_text_and_avatar_contracts() {
        assert!(valid_short_text("service"));
        assert!(!valid_short_text(""));
        assert!(!valid_short_text(&"a".repeat(257)));
        assert!(valid_avatar_url("https://example.com/avatar.png"));
        assert!(!valid_avatar_url("data:image/png;base64,abc"));
        assert!(!valid_avatar_url("https://user@example.com/avatar.png"));
    }

    #[test]
    fn refresh_status_classification_matches_session_contract() {
        for status in [
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::CONFLICT,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert_eq!(
                classify_refresh_status(status),
                RefreshStatusClass::Transient
            );
        }
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::UNPROCESSABLE_ENTITY,
        ] {
            assert_eq!(
                classify_refresh_status(status),
                RefreshStatusClass::Reauthenticate
            );
        }
        assert_eq!(
            classify_refresh_status(StatusCode::OK),
            RefreshStatusClass::Success
        );
    }

    #[test]
    fn assistant_turn_status_only_unlocks_proven_pre_admission_rejections() {
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::FORBIDDEN,
            StatusCode::PAYMENT_REQUIRED,
            StatusCode::CONFLICT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::UNPROCESSABLE_ENTITY,
        ] {
            assert_eq!(
                classify_assistant_turn_error(status).kind,
                AssistantStartErrorKind::Rejected,
                "{status}",
            );
        }
        assert_eq!(
            classify_assistant_turn_error(StatusCode::UNAUTHORIZED).kind,
            AssistantStartErrorKind::Unauthorized,
        );

        for status in [
            StatusCode::NOT_FOUND,
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let error = classify_assistant_turn_error(status);
            assert_eq!(error.kind, AssistantStartErrorKind::Network, "{status}");
            assert!(error.message.contains("不要重复发送"), "{status}");
        }
    }

    #[test]
    fn assistant_admission_absence_requires_the_explicit_no_content_status() {
        assert!(assistant_admission_explicitly_absent(
            StatusCode::NO_CONTENT
        ));
        for ambiguous in [
            StatusCode::NOT_FOUND,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::BAD_GATEWAY,
        ] {
            assert!(!assistant_admission_explicitly_absent(ambiguous));
        }
    }

    #[test]
    fn assistant_admission_resolution_uses_the_state_changing_post_route() {
        let client = NyxIdClient::new().unwrap();
        let request_id = uuid::Uuid::new_v4().to_string();
        let request = client
            .assistant_admission_request("test-access-token", &request_id)
            .build()
            .unwrap();

        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().path(),
            format!("/api/v1/assistant/nyxagent/turn-admissions/{request_id}/resolve")
        );
    }

    #[test]
    fn assistant_turn_request_uses_the_rollout_safe_route_and_uuid_v4_key() {
        let client = NyxIdClient::new().unwrap();
        let request_id = uuid::Uuid::new_v4().to_string();
        let request = client
            .assistant_turn_request(
                "test-access-token",
                &request_id,
                &serde_json::json!({"conversation_id": null, "text": "hello"}),
            )
            .build()
            .unwrap();

        assert_eq!(
            request
                .headers()
                .get("Idempotency-Key")
                .and_then(|value| value.to_str().ok()),
            Some(request_id.as_str())
        );
        assert_eq!(
            request.url().path(),
            "/api/v1/assistant/nyxagent/turns/idempotent"
        );
    }

    #[test]
    fn device_poll_error_codes_keep_distinct_terminal_meanings() {
        assert_eq!(
            classify_poll_error(Some(11202), StatusCode::BAD_REQUEST),
            PollErrorClass::Pending
        );
        assert_eq!(
            classify_poll_error(Some(11203), StatusCode::TOO_MANY_REQUESTS),
            PollErrorClass::SlowDown
        );
        assert_eq!(
            classify_poll_error(Some(11204), StatusCode::FORBIDDEN),
            PollErrorClass::Denied
        );
        assert_eq!(
            classify_poll_error(Some(11201), StatusCode::GONE),
            PollErrorClass::Expired
        );
        assert_eq!(
            classify_poll_error(Some(11205), StatusCode::GONE),
            PollErrorClass::AlreadyDelivered
        );
        assert_eq!(
            classify_poll_error(Some(11206), StatusCode::TOO_MANY_REQUESTS),
            PollErrorClass::RateLimited
        );
        assert_eq!(
            classify_poll_error(None, StatusCode::INTERNAL_SERVER_ERROR),
            PollErrorClass::Transient
        );
    }
}
