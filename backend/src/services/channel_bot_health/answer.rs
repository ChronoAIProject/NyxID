//! The latest retained primary answer on the selected NyxBot binding.
use super::{HealthFacts, Recommendation, advice};
use crate::models::channel_bot::ChannelBotError;
use crate::models::{nyxbot_channel::EVENTS_COLLECTION_NAME, nyxbot_gateway_response};
use bson::{Bson, DateTime, Document, doc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Prepared,
    Confirmed,
    Rejected,
    Unconfirmed,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Delivery {
    pub state: State,
    pub reason: &'static str,
}

const STREAM_WITHOUT_RECEIPT: &str = "gateway_does_not_confirm";
const SENDING: &str = "delivery_sending";
const RECEIPT_PENDING: &str = "gateway_receipt_pending";
const STALLED: &str = "delivery_stalled";
const DELIVERY_GRACE: chrono::Duration = chrono::Duration::minutes(10);

impl Delivery {
    pub(super) fn recommendation(self) -> Recommendation {
        let (action, text) = match self.state {
            State::Confirmed => (
                "none",
                "The platform confirmed delivery of the latest answer.",
            ),
            State::Prepared => ("none", "The latest answer is prepared for delivery."),
            State::Rejected => (
                "reconnect",
                "The answer could not be sent. Check the chat and reconnect the channel if the bot has lost access.",
            ),
            State::Unconfirmed => match self.reason {
                STREAM_WITHOUT_RECEIPT => (
                    "none",
                    "The gateway does not provide a delivery receipt for this answer.",
                ),
                SENDING => ("none", "NyxID is sending the latest answer."),
                RECEIPT_PENDING => ("none", "NyxID is waiting for a delivery receipt."),
                STALLED => (
                    "verify",
                    "Delivery is taking longer than expected. Check the chat. If the answer is missing, verify the connection before asking again.",
                ),
                _ => (
                    "verify",
                    "The answer may not have arrived. Check the chat before asking again.",
                ),
            },
        };
        advice(action, text.into())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Observation {
    state: String,
    created_at: DateTime,
    #[serde(default)]
    gateway_refused: bool,
    #[serde(default)]
    receipt_pending: bool,
    #[serde(default)]
    receipt_started_at: Option<DateTime>,
}

impl Observation {
    fn delivery(&self, now: chrono::DateTime<chrono::Utc>) -> Option<Delivery> {
        let (state, reason) = match self.state.as_str() {
            "preparing_text" | "prepared" => (State::Prepared, "answer_prepared"),
            "sent" => (State::Confirmed, "platform_delivery_confirmed"),
            "refused" if self.gateway_refused => (State::Rejected, "gateway_reply_rejected"),
            "refused" => (State::Rejected, "answer_delivery_refused"),
            // Neither the stream nor exposed stored output has a negotiated primary-answer
            // receipt today. Resolve the advertised receipt here when the gateway adds it.
            "streamed" | "unconfirmed" => (State::Unconfirmed, STREAM_WITHOUT_RECEIPT),
            "sending" | "unknown" if self.state == "sending" || self.receipt_pending => {
                let reason = if now - self.created_at.to_chrono() > DELIVERY_GRACE {
                    STALLED
                } else if self.receipt_pending {
                    RECEIPT_PENDING
                } else {
                    SENDING
                };
                (State::Unconfirmed, reason)
            }
            "unknown" => (State::Unconfirmed, "platform_delivery_unconfirmed"),
            _ => return None,
        };
        Some(Delivery { state, reason })
    }
}

pub(crate) fn apply_latest(
    facts: &mut HealthFacts<'_>,
    event: Option<&Observation>,
    response: Option<&Observation>,
    outbound_error: Option<&ChannelBotError>,
) {
    // The local dispatch barrier wins ties with its stored response.
    let Some(latest) = [response, event]
        .into_iter()
        .flatten()
        .max_by_key(|o| o.created_at)
    else {
        return;
    };
    facts.answer_delivery = latest.delivery(chrono::Utc::now());
    // The receipt writer timestamps its bot diagnostic with the same started_at.
    // Let the answer grade this wait; keep failures from other outbound operations.
    if latest.receipt_pending
        && let Some(error) = outbound_error
        && error
            .reason
            .as_deref()
            .is_some_and(|r| r.starts_with("gateway_reply_"))
        && latest.receipt_started_at == Some(DateTime::from_chrono(error.at))
    {
        facts.outbound_reason = None;
    }
}

const LOCAL_STATES: [&str; 6] = [
    "preparing_text",
    "sending",
    "sent",
    "refused",
    "unknown",
    "streamed",
];

pub(crate) fn event_filter() -> Document {
    doc! {"turn_id":{"$type":"string"},"delivery.version":1,"delivery.state":{"$in":LOCAL_STATES.to_vec()}}
}

pub(crate) fn response_filter() -> Document {
    doc! {"answer_delivery":{"$in":["prepared","unconfirmed"]}}
}

pub(crate) fn event_lookup() -> Document {
    let mut filter = event_filter();
    filter.insert("$expr", doc! {"$eq":["$user_id","$$owner"]});
    filter.insert("expires_at", doc! {"$gt":DateTime::now()});
    doc! {"$lookup":{
    "from":EVENTS_COLLECTION_NAME, "localField":"_id","foreignField":"channel_id",
    "let":{"owner":"$user_id"},
    "pipeline":[{"$match":filter},{"$sort":{"created_at":-1,"_id":-1}},{"$limit":1},
        {"$project":{"_id":0,"created_at":1,"state":"$delivery.state",
            "gateway_refused":{"$isNumber":"$delivery.gateway_observation.status"},
            "receipt_started_at":"$delivery.preparation.gateway_receipt.started_at",
            "receipt_pending":{"$eq":[{"$ifNull":["$delivery.preparation.gateway_receipt.pending",false]},true]}}}],
    "as":"answer_events"}}
}

pub(crate) fn response_lookup() -> Document {
    let mut filter = response_filter();
    filter.insert("expires_at", doc! {"$gt":DateTime::now()});
    filter.insert(
        "$expr",
        doc! {"$and":[{"$eq":["$user_id","$$owner"]},{"$eq":["$binding_id","$$binding"]}]},
    );
    doc! {"$lookup":{
    "from":nyxbot_gateway_response::COLLECTION_NAME,"localField":"_id","foreignField":"channel_id",
    "let":{"owner":"$user_id","binding":{"$ifNull":["$binding_id",Bson::Null]}},
    "pipeline":[{"$match":filter},{"$sort":{"created_at":-1,"_id":-1}},{"$limit":1},
        {"$project":{"_id":0,"created_at":1,"state":"$answer_delivery"}}],
    "as":"answer_responses"}}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sending_and_pending_receipts_stall_only_after_ten_minutes() {
        let created_at = DateTime::now();
        for (state, receipt_pending, reason) in [
            ("sending", false, SENDING),
            ("unknown", true, RECEIPT_PENDING),
        ] {
            let observation = Observation {
                state: state.into(),
                created_at,
                gateway_refused: false,
                receipt_pending,
                receipt_started_at: None,
            };
            let boundary = created_at.to_chrono() + DELIVERY_GRACE;
            let recent = observation.delivery(boundary).unwrap();
            assert_eq!(recent.state, State::Unconfirmed);
            assert_eq!(recent.reason, reason);
            assert_eq!(recent.recommendation().action, "none");
            let stalled = observation
                .delivery(boundary + chrono::Duration::milliseconds(1))
                .unwrap();
            assert_eq!(stalled.reason, STALLED);
            assert_eq!(stalled.recommendation().action, "verify");
        }
    }
}
