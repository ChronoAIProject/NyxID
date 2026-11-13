//! Push-only automation delivery. Availability and provider refusal are distinct.
use super::*;

#[derive(Clone, Copy, Debug)]
pub struct Failure {
    pub code: &'static str,
    pub http_status: Option<u16>,
    pub retry_after: Option<u64>,
}
impl Failure {
    pub fn new(code: &'static str) -> Self {
        Self {
            code,
            http_status: None,
            retry_after: None,
        }
    }
}

pub enum Outcome {
    Accepted,
    Refused(Failure),
    Unknown,
}

fn configured(device: &DeviceToken, config: &AppConfig, fcm: bool, apns: bool) -> bool {
    let nonempty = |v: Option<&str>| v.is_some_and(|v| !v.trim().is_empty());
    !device.token.trim().is_empty()
        && match device.platform.as_str() {
            "fcm" => fcm && nonempty(config.fcm_project_id.as_deref()),
            "apns" => apns && nonempty(device.app_id.as_deref().or(config.apns_topic.as_deref())),
            _ => false,
        }
}

/// Re-read preferences on every attempt, before the durable sending barrier.
pub async fn prepare(
    db: &Database,
    config: &AppConfig,
    fcm: bool,
    apns: bool,
    owner: &str,
) -> AppResult<Result<NotificationChannel, Failure>> {
    let Some(channel) = db
        .collection::<NotificationChannel>(COLLECTION_NAME)
        .find_one(doc! {"user_id":owner})
        .await?
    else {
        return Ok(Err(Failure::new("push_not_set_up")));
    };
    let reason = if !channel.push_enabled {
        Some("push_disabled")
    } else if channel.push_devices.is_empty() {
        Some("push_no_devices")
    } else if !channel
        .push_devices
        .iter()
        .any(|d| configured(d, config, fcm, apns))
    {
        Some("push_provider_unavailable")
    } else {
        None
    };
    Ok(match reason {
        Some(reason) => Err(Failure::new(reason)),
        None => Ok(channel),
    })
}

fn refused(status: u16, retry_after: Option<u64>) -> Outcome {
    // Only explicit rate-limit/service-unavailable responses authorize retry.
    // Transport failures and timeouts do not establish non-acceptance.
    Outcome::Refused(Failure {
        code: if matches!(status, 429 | 503) {
            "push_temporarily_refused"
        } else {
            "push_provider_rejected"
        },
        http_status: Some(status),
        retry_after: matches!(status, 429 | 503).then_some(retry_after.unwrap_or(60).max(60)),
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn send(
    db: &Database,
    config: &AppConfig,
    http: &Client,
    fcm: Option<&FcmAuth>,
    apns: Option<&ApnsAuth>,
    channel: &NotificationChannel,
    label: &str,
    text: &str,
    thread: &str,
) -> Outcome {
    let body: String = text.chars().take(200).collect();
    let data = HashMap::from([
        ("type".into(), "automation_result".into()),
        ("conversation_id".into(), thread.into()),
        (
            "deeplink".into(),
            crate::services::assistant_links::AssistantPage::Conversation {
                conversation: thread,
            }
            .url(&config.frontend_url),
        ),
    ]);
    send_data(db, config, http, fcm, apns, channel, label, &body, data).await
}

/// Same device fan-out and outcome rules with a caller-built payload
/// (NyxBot Notifications push).
#[allow(clippy::too_many_arguments)]
pub async fn send_data(
    db: &Database,
    config: &AppConfig,
    http: &Client,
    fcm: Option<&FcmAuth>,
    apns: Option<&ApnsAuth>,
    channel: &NotificationChannel,
    label: &str,
    body: &str,
    data: HashMap<String, String>,
) -> Outcome {
    let mut failure = Failure::new("push_devices_invalid");
    let mut accepted = false;
    for device in unique_devices_by_token(&channel.push_devices) {
        if !configured(device, config, fcm.is_some(), apns.is_some()) {
            continue;
        }
        let result = match device.platform.as_str() {
            "fcm" => push_service::send_fcm_notification(
                http,
                fcm.expect("configured"),
                config.fcm_project_id.as_deref().expect("configured"),
                &device.token,
                label,
                body,
                &data,
            )
            .await
            .map(|result| match result {
                FcmSendResult::Success { .. } => Outcome::Accepted,
                FcmSendResult::TokenInvalid => {
                    Outcome::Refused(Failure::new("push_devices_invalid"))
                }
                FcmSendResult::Failed {
                    status,
                    retry_after,
                    ..
                } => refused(status, retry_after),
            }),
            "apns" => push_service::send_apns_notification(
                http,
                apns.expect("configured"),
                &device.token,
                device
                    .app_id
                    .as_deref()
                    .or(config.apns_topic.as_deref())
                    .expect("configured"),
                config.apns_sandbox,
                label,
                body,
                &data,
            )
            .await
            .map(|result| match result {
                ApnsSendResult::Success => Outcome::Accepted,
                ApnsSendResult::TokenInvalid => {
                    Outcome::Refused(Failure::new("push_devices_invalid"))
                }
                ApnsSendResult::Failed {
                    status,
                    retry_after,
                    ..
                } => refused(status, retry_after),
            }),
            _ => unreachable!("configured platform"),
        };
        match result {
            // Preserve device fanout. Any acceptance satisfies the destination;
            // an uncertain send ends the batch without authorizing a retry.
            Ok(Outcome::Accepted) => accepted = true,
            Err(_) | Ok(Outcome::Unknown) => {
                return if accepted {
                    Outcome::Accepted
                } else {
                    Outcome::Unknown
                };
            }
            Ok(Outcome::Refused(next)) => {
                if next.code == "push_devices_invalid" {
                    // A concurrently refreshed token must survive cleanup.
                    let _ = db.collection::<NotificationChannel>(COLLECTION_NAME).update_one(
                        doc! {"_id": &channel.id, "push_devices": {"$elemMatch":{"device_id":&device.device_id,"token":&device.token}}},
                        doc! {"$pull":{"push_devices":{"device_id":&device.device_id,"token":&device.token}}, "$set":{"updated_at":bson::DateTime::now()}},
                    ).await;
                }
                if next.retry_after.is_some() {
                    let delay = failure
                        .retry_after
                        .unwrap_or(0)
                        .max(next.retry_after.unwrap_or(0));
                    failure = Failure {
                        retry_after: Some(delay),
                        ..next
                    };
                } else if failure.retry_after.is_none() {
                    failure = next;
                }
            }
        }
    }
    if accepted {
        Outcome::Accepted
    } else {
        Outcome::Refused(failure)
    }
}

/// Fixed owner-safe messages shared by history, tools and the durable notice.
pub fn message(code: &str) -> &'static str {
    match code {
        "push_not_set_up" => {
            "Mobile push is not set up. Sign in to the NyxID mobile app and enable notifications."
        }
        "push_disabled" => {
            "Mobile push is disabled. Enable push notifications in notification settings and allow notifications in the NyxID mobile app."
        }
        "push_no_devices" => {
            "No mobile device is registered. Sign in to the NyxID mobile app and enable notifications."
        }
        "push_provider_unavailable" => {
            "Mobile push is unavailable on this server. Ask the administrator to configure the push provider, or choose a channel chat for this automation."
        }
        "push_devices_invalid" => {
            "The registered mobile devices are no longer reachable. Open the NyxID mobile app and enable notifications to register again."
        }
        "push_provider_rejected" => {
            "The push provider rejected the notification. Ask the administrator to check push credentials and app configuration, or choose a channel chat."
        }
        "push_temporarily_refused" => {
            "The push provider temporarily refused the notification. NyxID will retry the saved report without running the task again."
        }
        "push_retries_exhausted" => {
            "The push provider kept refusing the notification and automatic retries have ended. Read the saved report and ask the administrator to check the push provider, or choose a channel chat."
        }
        "push_delivery_unknown" => {
            "Push acceptance could not be confirmed. To avoid a duplicate notification, NyxID will not resend it automatically. Read the saved report in this thread."
        }
        _ => {
            "Push delivery could not finish. Read the saved report and check notification settings or choose a channel chat."
        }
    }
}
