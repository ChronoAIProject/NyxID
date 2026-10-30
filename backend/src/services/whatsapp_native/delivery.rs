//! CMA receipt projections. Never synthesize NyxID relay receipt rows.
use crate::models::channel_whatsapp_native::NativeWhatsapp;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

#[derive(Default, Serialize)]
pub struct ReceiptSummary {
    pub source: &'static str,
    /// Per-component observations, never a whole-reply delivery claim.
    pub observations: Vec<ReceiptObservation>,
}
#[derive(Serialize)]
pub struct ReceiptObservation {
    pub platform_message_id: String,
    pub status: String,
    pub confirmed_at: DateTime<Utc>,
}

/// Input is one event from the authenticated, exact-channel Activity endpoint.
/// Consumer completion and Graph acceptance alone never imply a platform receipt.
pub fn summarize(config: &NativeWhatsapp, activity: &Value) -> Option<ReceiptSummary> {
    let source = &activity["source"];
    let recipient = activity["conversation"]["id"].as_str()?;
    if source["type"] != "whatsapp"
        || source["app_id"].as_str() != Some(&config.meta_app_id)
        || source["waba_id"].as_str() != Some(&config.waba_id)
        || source["phone_number_id"].as_str() != Some(&config.phone_number_id)
        || activity["conversation"]["kind"] != "private"
        || !super::numeric_id(recipient)
    {
        return None;
    }
    let mut summary = ReceiptSummary {
        source: "cma",
        ..Default::default()
    };
    for consumer in activity["consumers"].as_array()?.iter().take(8) {
        if consumer["consumer_id"] != "default" || consumer["provider"] != "nyxbot" {
            continue;
        }
        for delivery in consumer["deliveries"]
            .as_array()
            .into_iter()
            .flatten()
            .take(16)
        {
            let Some(at) = delivery["confirmed_at"]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|at| at.with_timezone(&Utc))
            else {
                continue;
            };
            let status = match (delivery["step"].as_str(), delivery["outcome"].as_str()) {
                (
                    Some("platform_status"),
                    Some(status @ ("sent" | "delivered" | "read" | "played")),
                ) => status,
                (Some("platform_failure"), Some("failed")) => "failed",
                _ => continue,
            };
            let Some(message_id) = delivery["platform_message_id"]
                .as_str()
                .filter(|id| crate::services::channel_delivery_service::valid_message_id(id))
            else {
                continue;
            };
            let exact = delivery["destination"] == "origin"
                && delivery["recipients"].as_array().is_some_and(|recipients| {
                    recipients.len() == 1
                        && recipients[0]["kind"] == "whatsapp_chat"
                        && recipients[0]["id"].as_str() == Some(recipient)
                        && recipients[0]["thread_id"].is_null()
                });
            if !exact {
                continue;
            }
            summary.observations.push(ReceiptObservation {
                platform_message_id: message_id.to_owned(),
                status: status.to_owned(),
                confirmed_at: at,
            });
        }
    }
    Some(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cma_read_does_not_invent_local_or_delivered_receipts() {
        let config = super::super::tests::config();
        let mut activity = super::super::tests::context()["activity"].clone();
        activity["consumers"] = json!([{"consumer_id":"default", "provider":"nyxbot", "state":"completed",
            "deliveries":[{"step":"platform_status", "outcome":"read", "confirmed_at":"2026-10-28T00:00:00Z"},
                {"step":"platform_failure", "outcome":"failed", "confirmed_at":"2026-10-28T00:01:00Z"}]}]);
        for item in activity["consumers"][0]["deliveries"]
            .as_array_mut()
            .unwrap()
        {
            item["destination"] = json!("origin");
            item["platform_message_id"] = json!("wamid.one");
            item["recipients"] = json!([{"kind":"whatsapp_chat", "id":"0015551234567"}]);
        }
        let result = summarize(&config, &activity).unwrap();
        assert_eq!(result.source, "cma");
        assert_eq!(
            result
                .observations
                .iter()
                .map(|v| v.status.as_str())
                .collect::<Vec<_>>(),
            ["read", "failed"]
        );
        activity["consumers"][0]["deliveries"][0]["step"] = json!("send");
        assert_eq!(summarize(&config, &activity).unwrap().observations.len(), 1);
        activity["consumers"][0]["deliveries"][1]["recipients"][0]["id"] = json!("other");
        assert!(
            summarize(&config, &activity)
                .unwrap()
                .observations
                .is_empty()
        );
        activity["source"]["phone_number_id"] = json!("999");
        assert!(summarize(&config, &activity).is_none());
    }
}
