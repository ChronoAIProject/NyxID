//! Single-recipient provider requests for the bound delivery executor.
//! Producer-owned resolvers retain live authorization, immutable versions and
//! receipt storage. Call send_once only from DeliveryConsumer::send inside the
//! executor's consumed run_bound closure. There is no transport retry authority.

use std::{collections::HashMap, fmt, net::SocketAddr, time::Duration};

use reqwest::{Client, Response, header::HeaderValue};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::execution::ResolvedDelivery;
use super::{Channel, DeliveryBinding, QueuedDelivery, SendOutcome, check_binding};
use crate::{
    errors::{AppError, AppResult},
    services::{
        url_validation,
        webhook_delivery_service::{self, SignatureContract},
    },
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_WEBHOOK_BYTES: usize = 16 * 1024;

fn unavailable() -> AppError {
    AppError::BadRequest("Notification destination unavailable".into())
}

fn client_builder() -> reqwest::ClientBuilder {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(REQUEST_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
}

fn sensitive(value: &str) -> AppResult<HeaderValue> {
    let mut header = HeaderValue::from_str(value).map_err(|_| unavailable())?;
    header.set_sensitive(true);
    Ok(header)
}

/// Exactly one independently resolved destination, immutable body and request.
/// All fields are private; the product's resolver supplies the binding.
pub struct PreparedSend {
    binding: DeliveryBinding,
    channel: Channel,
    client: Client,
    // The body is Bytes::from_owner(Zeroizing<Vec<u8>>): request clones share
    // its wiping owner instead of creating an unprotected body copy.
    request: reqwest::Request,
}

impl fmt::Debug for PreparedSend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PreparedSend([REDACTED])")
    }
}

/// Protected product receipt, never serialized into an audit event or logged.
pub enum ProviderReceipt {
    Webhook {
        status: u16,
    },
    Fcm {
        message_name: Option<Zeroizing<String>>,
    },
    Apns {
        request_id: Option<Zeroizing<String>>,
    },
    Telegram {
        message_id: i64,
    },
}

impl fmt::Debug for ProviderReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProviderReceipt([REDACTED])")
    }
}

impl PreparedSend {
    pub fn resolved(self) -> ResolvedDelivery<Self> {
        ResolvedDelivery {
            binding: DeliveryBinding {
                destination_id: self.binding.destination_id,
                payload: self.binding.payload.clone(),
            },
            channel: self.channel,
            prepared: self,
        }
    }

    /// The dispatcher has already checked this independent binding before the
    /// claim and while consuming run_bound. Refuse any misuse before I/O too.
    pub async fn send_once(self, queued: &QueuedDelivery) -> SendOutcome<ProviderReceipt> {
        if check_binding(queued, &self.binding, self.channel).is_err() {
            return SendOutcome::Rejected {
                status: None,
                invalid_destination: false,
            };
        }
        let response = match self.client.execute(self.request).await {
            Ok(response) => response,
            Err(_) => return SendOutcome::Uncertain,
        };
        classify(self.channel, response).await
    }

    fn finish(
        binding: DeliveryBinding,
        channel: Channel,
        client: Client,
        builder: reqwest::RequestBuilder,
    ) -> AppResult<Self> {
        // Fail before claim when a producer's protected store is corrupt.
        super::require_v4(binding.destination_id)?;
        let request = builder.build().map_err(|_| unavailable())?;
        Ok(Self {
            binding,
            channel,
            client,
            request,
        })
    }

    /// Prepare under the producer's immutable destination/payload revision.
    /// The public-address set is pinned into this private, nonredirecting client.
    #[allow(clippy::too_many_arguments)]
    pub async fn webhook(
        binding: DeliveryBinding,
        target: &str,
        secret: &Zeroizing<Vec<u8>>,
        event_type: &str,
        event_id: &str,
        body: Zeroizing<Vec<u8>>,
        contract: SignatureContract,
        key_id: Option<&str>,
    ) -> AppResult<Self> {
        if body.len() > MAX_WEBHOOK_BYTES || secret.is_empty() {
            return Err(unavailable());
        }
        let (client, target) = public_webhook_client(target).await?;
        Self::signed_webhook(
            binding, client, &target, secret, event_type, event_id, body, contract, key_id,
        )
    }

    // Only webhook() chooses this client in production, after public HTTPS
    // validation and DNS pinning. Keeping construction separate lets protocol
    // tests inspect signed bytes without trusting external DNS or sending data.
    #[allow(clippy::too_many_arguments)]
    fn signed_webhook(
        binding: DeliveryBinding,
        client: Client,
        target: &str,
        secret: &Zeroizing<Vec<u8>>,
        event_type: &str,
        event_id: &str,
        body: Zeroizing<Vec<u8>>,
        contract: SignatureContract,
        key_id: Option<&str>,
    ) -> AppResult<Self> {
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let signature = Zeroizing::new(match contract {
            SignatureContract::Timestamped => {
                webhook_delivery_service::compute_timestamped_signature(secret, &timestamp, &body)
            }
            SignatureContract::BodyOnly => {
                webhook_delivery_service::compute_body_signature(secret, &body)
            }
        });
        let signed = Zeroizing::new(format!("sha256={}", signature.as_str()));
        let mut request = client
            .post(target)
            .header("Content-Type", "application/json")
            .header("X-NyxID-Event", sensitive(event_type)?)
            .header("X-NyxID-Delivery-Id", sensitive(event_id)?)
            .header("X-NyxID-Signature", sensitive(&signed)?)
            .body(bytes::Bytes::from_owner(body));
        if contract == SignatureContract::Timestamped {
            request = request.header("X-NyxID-Timestamp", sensitive(&timestamp)?);
        }
        if let Some(key_id) = key_id {
            request = request.header("X-NyxID-Key-Id", sensitive(key_id)?);
        }
        Self::finish(binding, Channel::Webhook, client.clone(), request)
    }

    /// One FCM send. Refresh access_token while resolving, before a claim;
    /// neither 401 nor any other result can trigger another send here.
    pub fn fcm(
        binding: DeliveryBinding,
        project: &str,
        access_token: Zeroizing<String>,
        device_token: Zeroizing<String>,
        notification: PushNotification<'_>,
    ) -> AppResult<Self> {
        if access_token.is_empty()
            || device_token.is_empty()
            || project.is_empty()
            || !project
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(unavailable());
        }
        #[derive(Serialize)]
        struct Payload<'a> {
            message: Message<'a>,
        }
        #[derive(Serialize)]
        struct Message<'a> {
            token: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            notification: Option<Alert<'a>>,
            #[serde(skip_serializing_if = "HashMap::is_empty")]
            data: &'a HashMap<String, String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            android: Option<serde_json::Value>,
        }
        #[derive(Serialize)]
        struct Alert<'a> {
            title: &'a str,
            body: &'a str,
        }
        let payload = Payload {
            message: Message {
                token: &device_token,
                notification: notification
                    .alert
                    .map(|(title, body)| Alert { title, body }),
                data: notification.data,
                android: notification.alert.map(|_| {
                    serde_json::json!({"priority":"high", "notification": {
                        "channel_id":"approvals", "sound":"default", "default_sound":true,
                        "default_vibrate_timings":true, "notification_priority":"PRIORITY_MAX"
                    }})
                }),
            },
        };
        let body = Zeroizing::new(serde_json::to_vec(&payload).map_err(|_| unavailable())?);
        let client = client_builder().build().map_err(|_| unavailable())?;
        let request = client
            .post(format!(
                "https://fcm.googleapis.com/v1/projects/{project}/messages:send"
            ))
            .bearer_auth(access_token.as_str())
            .header("Content-Type", "application/json")
            .body(bytes::Bytes::from_owner(body));
        Self::finish(binding, Channel::Fcm, client.clone(), request)
    }

    /// One APNs send. ExpiredProviderToken is a rejection, without a refresh loop.
    pub fn apns(
        binding: DeliveryBinding,
        jwt: Zeroizing<String>,
        device_token: Zeroizing<String>,
        topic: &str,
        sandbox: bool,
        notification: PushNotification<'_>,
    ) -> AppResult<Self> {
        if jwt.is_empty()
            || topic.is_empty()
            || device_token.is_empty()
            || !device_token.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(unavailable());
        }
        let host = if sandbox {
            "api.sandbox.push.apple.com"
        } else {
            "api.push.apple.com"
        };
        let target = Zeroizing::new(format!("https://{host}/3/device/{}", device_token.as_str()));
        let aps = if let Some((title, body)) = notification.alert {
            serde_json::json!({"alert":{"title":title,"body":body}, "sound":"default",
                "mutable-content":1, "category":"APPROVAL_REQUEST"})
        } else {
            serde_json::json!({"content-available":1})
        };
        let mut payload = serde_json::Map::new();
        // Never let caller data replace the provider's aps envelope.
        for (name, value) in notification.data {
            if name == "aps" {
                return Err(unavailable());
            }
            payload.insert(name.clone(), serde_json::Value::String(value.clone()));
        }
        payload.insert("aps".into(), aps);
        let body = Zeroizing::new(serde_json::to_vec(&payload).map_err(|_| unavailable())?);
        let client = client_builder()
            .http2_prior_knowledge()
            .build()
            .map_err(|_| unavailable())?;
        let silent = notification.alert.is_none();
        let request = client
            .post(target.as_str())
            .bearer_auth(jwt.as_str())
            .header("apns-topic", sensitive(topic)?)
            .header(
                "apns-push-type",
                if silent { "background" } else { "alert" },
            )
            .header("apns-priority", if silent { "5" } else { "10" })
            .header(
                "apns-expiration",
                if silent {
                    "0".into()
                } else {
                    (chrono::Utc::now().timestamp() + 300).to_string()
                },
            )
            .header("Content-Type", "application/json")
            .body(bytes::Bytes::from_owner(body));
        Self::finish(binding, Channel::Apns, client.clone(), request)
    }

    /// Telegram success needs ok:true and the accepted message identity.
    /// reply_markup is producer-rendered protected content, never audit evidence.
    pub fn telegram(
        binding: DeliveryBinding,
        bot_token: Zeroizing<String>,
        chat_id: i64,
        text: Zeroizing<String>,
        reply_markup: Option<&serde_json::Value>,
    ) -> AppResult<Self> {
        if chat_id == 0
            || bot_token.is_empty()
            || !bot_token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
        {
            return Err(unavailable());
        }
        #[derive(Serialize)]
        struct Payload<'a> {
            chat_id: i64,
            text: &'a str,
            parse_mode: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            reply_markup: Option<&'a serde_json::Value>,
        }
        let payload = Payload {
            chat_id,
            text: &text,
            parse_mode: "HTML",
            reply_markup,
        };
        let body = Zeroizing::new(serde_json::to_vec(&payload).map_err(|_| unavailable())?);
        let target = Zeroizing::new(format!(
            "https://api.telegram.org/bot{}/sendMessage",
            bot_token.as_str()
        ));
        let client = client_builder().build().map_err(|_| unavailable())?;
        let request = client
            .post(target.as_str())
            .header("Content-Type", "application/json")
            .body(bytes::Bytes::from_owner(body));
        Self::finish(binding, Channel::Telegram, client.clone(), request)
    }
}

/// Alert and silent pushes share the original producer's protected data.
pub struct PushNotification<'a> {
    pub alert: Option<(&'a str, &'a str)>,
    pub data: &'a HashMap<String, String>,
}

async fn public_webhook_client(target: &str) -> AppResult<(Client, String)> {
    tokio::time::timeout(REQUEST_TIMEOUT, async {
        let target = webhook_delivery_service::validate_webhook_url(target, "Webhook destination")
            .await
            .map_err(|_| unavailable())?;
        let parsed = url::Url::parse(&target).map_err(|_| unavailable())?;
        let host = parsed.host_str().ok_or_else(unavailable)?;
        let port = parsed.port_or_known_default().ok_or_else(unavailable)?;
        let lookup_host = host.trim_start_matches('[').trim_end_matches(']');
        let addresses: Vec<SocketAddr> = if let Ok(ip) = lookup_host.parse() {
            vec![SocketAddr::new(ip, port)]
        } else {
            tokio::net::lookup_host((host, port))
                .await
                .map_err(|_| unavailable())?
                .collect()
        };
        // DNS may change between the shared validator and this lookup. Check
        // the exact set that the client will pin, refusing mixed public/private
        // answers before building any request.
        let client = pinned_webhook_client(host, &addresses)?;
        Ok((client, target))
    })
    .await
    .map_err(|_| unavailable())?
}

fn pinned_webhook_client(host: &str, addresses: &[SocketAddr]) -> AppResult<Client> {
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| url_validation::is_private_or_internal_ip(address.ip()))
    {
        return Err(unavailable());
    }
    client_builder()
        .resolve_to_addrs(host, addresses)
        .build()
        .map_err(|_| unavailable())
}

async fn body(response: Response) -> Option<Zeroizing<Vec<u8>>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return None;
    }
    let mut response = response;
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    Some(bytes)
}

fn uncertain(status: u16) -> SendOutcome<ProviderReceipt> {
    SendOutcome::uncertain_response(status).unwrap_or(SendOutcome::Uncertain)
}

async fn classify(channel: Channel, response: Response) -> SendOutcome<ProviderReceipt> {
    let status = response.status().as_u16();
    let success = response.status().is_success();
    match channel {
        Channel::Webhook if success => SendOutcome::Accepted(ProviderReceipt::Webhook { status }),
        Channel::Webhook => uncertain(status),
        Channel::Fcm if success => {
            #[derive(Deserialize)]
            struct Accepted {
                name: Option<Zeroizing<String>>,
            }
            let name = match body(response).await {
                Some(bytes) => serde_json::from_slice::<Accepted>(&bytes)
                    .ok()
                    .and_then(|v| v.name),
                None => None,
            };
            SendOutcome::Accepted(ProviderReceipt::Fcm { message_name: name })
        }
        Channel::Fcm if matches!(status, 400 | 401 | 403 | 404 | 429) => {
            let invalid = body(response).await.is_some_and(|bytes| {
                // Only the documented provider code proves an invalid token.
                // Neither arbitrary response text nor a generic 404 suffices.
                #[derive(Deserialize)]
                struct ErrorBody {
                    error: Error,
                }
                #[derive(Deserialize)]
                struct Error {
                    #[serde(default)]
                    details: Vec<Detail>,
                }
                #[derive(Deserialize)]
                struct Detail {
                    #[serde(rename = "@type")]
                    kind: Option<String>,
                    #[serde(rename = "errorCode")]
                    code: Option<String>,
                }
                serde_json::from_slice::<ErrorBody>(&bytes)
                    .ok()
                    .is_some_and(|b| {
                        b.error.details.iter().any(|d| {
                            d.kind.as_deref()
                                == Some("type.googleapis.com/google.firebase.fcm.v1.FcmError")
                                && d.code.as_deref() == Some("UNREGISTERED")
                        })
                    })
            });
            SendOutcome::Rejected {
                status: Some(status),
                invalid_destination: invalid,
            }
        }
        Channel::Apns if success => {
            let request_id = response
                .headers()
                .get("apns-id")
                .and_then(|v| v.to_str().ok())
                .map(|v| Zeroizing::new(v.to_string()));
            SendOutcome::Accepted(ProviderReceipt::Apns { request_id })
        }
        Channel::Apns if (400..500).contains(&status) => {
            #[derive(Deserialize)]
            struct Rejection {
                reason: String,
            }
            let invalid = body(response).await.is_some_and(|bytes| {
                serde_json::from_slice::<Rejection>(&bytes)
                    .ok()
                    .is_some_and(|v| {
                        matches!(
                            (status, v.reason.as_str()),
                            (400, "BadDeviceToken") | (410, "Unregistered")
                        )
                    })
            });
            SendOutcome::Rejected {
                status: Some(status),
                invalid_destination: invalid,
            }
        }
        Channel::Telegram => {
            #[derive(Deserialize)]
            struct Telegram {
                ok: bool,
                result: Option<Message>,
                error_code: Option<u16>,
            }
            #[derive(Deserialize)]
            struct Message {
                message_id: i64,
            }
            let parsed = match body(response).await {
                Some(bytes) => serde_json::from_slice::<Telegram>(&bytes).ok(),
                None => None,
            };
            match parsed {
                Some(Telegram {
                    ok: true,
                    result: Some(message),
                    ..
                }) if success => SendOutcome::Accepted(ProviderReceipt::Telegram {
                    message_id: message.message_id,
                }),
                Some(Telegram {
                    ok: false,
                    error_code: Some(400..=499),
                    ..
                }) if success || (400..500).contains(&status) => SendOutcome::Rejected {
                    status: Some(status),
                    invalid_destination: false,
                },
                _ => uncertain(status),
            }
        }
        // No email request can be prepared, and no provider response text leaks.
        _ => uncertain(status),
    }
}

#[cfg(test)]
mod tests;
