//! Deterministic call-end snapshot on the ordinary, transactionally sequenced timeline.
use crate::{
    errors::AppResult,
    models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as THREADS},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        assistant_voice::{REQUESTS, RequestState, VoiceRequest},
        assistant_voice_session::{COLLECTION_NAME as CALLS, VoiceSession},
        credits::Credits,
        service_billing::BillingMetric,
        usage_meter::{COLLECTION_NAME as USAGE_METER, UsageMeterRow},
    },
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
    options::ReturnDocument,
};
use std::collections::HashMap;

/// Called in the same transaction that releases the live slot. Recovery cannot
/// overwrite this snapshot or allocate a second timeline sequence.
pub async fn append(db: &Database, tx: &mut ClientSession, call: &VoiceSession) -> AppResult<()> {
    let Some(thread) = db
        .collection::<AssistantConversation>(THREADS)
        .find_one_and_update(
            doc! {"_id":&call.conversation_id,"user_id":&call.user_id},
            doc! {"$inc":{"message_count":1},"$set":{"updated_at":bson::DateTime::now()}},
        )
        .return_document(ReturnDocument::After)
        .session(&mut *tx)
        .await?
    else {
        return Ok(());
    };
    let mut request_cursor = db
        .collection::<VoiceRequest>(REQUESTS)
        .find(doc! {"session_id":&call.id,"user_id":&call.user_id})
        .sort(doc! {"message_seq":1})
        .session(&mut *tx)
        .await?;
    let requests: Vec<VoiceRequest> = request_cursor.stream(&mut *tx).try_collect().await?;
    let source_ids: Vec<_> = requests.iter().map(|r| r.message_id.clone()).collect();
    let mut source_cursor = db
        .collection::<AssistantMessage>(MESSAGES)
        .find(doc! {"_id":{"$in":&source_ids},"user_id":&call.user_id})
        .session(&mut *tx)
        .await?;
    let sources: Vec<AssistantMessage> = source_cursor.stream(&mut *tx).try_collect().await?;
    let source_map: HashMap<_, _> = sources.into_iter().map(|m| (m.id.clone(), m)).collect();
    let request_ids: Vec<_> = requests.iter().map(|r| r.id.clone()).collect();
    let mut card_cursor = db
        .collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .find(doc! {"voice_request_id":{"$in":&request_ids},"status":{"$in":["allowed","denied","used"]}})
        .session(&mut *tx)
        .await?;
    let cards: Vec<Document> = card_cursor.stream(&mut *tx).try_collect().await?;
    let mut meter_cursor = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find(usage_filter(&call.user_id, &call.id, call.created_at))
        .session(&mut *tx)
        .await?;
    let meters: Vec<UsageMeterRow> = meter_cursor.stream(&mut *tx).try_collect().await?;
    let pending_cost = !meters.is_empty()
        && meters.iter().any(|row| {
            row.funding
                .as_ref()
                .and_then(|funding| funding.total_charge)
                .is_none()
        });
    let total_cost = (!pending_cost)
        .then(|| {
            Credits::checked_sum(meters.iter().filter_map(|row| {
                row.funding
                    .as_ref()
                    .and_then(|funding| funding.total_charge)
            }))
            .ok()
        })
        .flatten();
    let voice_cost = (!pending_cost)
        .then(|| {
            Credits::checked_sum(
                meters
                    .iter()
                    .filter(|row| row.metric == BillingMetric::VoiceSeconds)
                    .filter_map(|row| {
                        row.funding
                            .as_ref()
                            .and_then(|funding| funding.total_charge)
                    }),
            )
            .ok()
        })
        .flatten();
    let seconds = (call.closed_at.unwrap_or_else(Utc::now) - call.created_at)
        .num_seconds()
        .clamp(0, 1800);
    let text = render(
        seconds,
        !meters.is_empty(),
        pending_cost,
        total_cost,
        voice_cost,
        &requests,
        &source_map,
        &cards,
    );
    let id = uuid::Uuid::new_v4().to_string();
    db.collection::<AssistantMessage>(MESSAGES)
        .insert_one(AssistantMessage {
            id: id.clone(),
            conversation_id: call.conversation_id.clone(),
            user_id: call.user_id.clone(),
            seq: thread.message_count,
            turn_id: call.id.clone(),
            role: "assistant".into(),
            text,
            status: "completed".into(),
            error_code: None,
            created_at: call.closed_at.unwrap_or_else(Utc::now),
            activities: vec![],
            attachments: vec![],
            origin: None,
            via: Some("voice".into()),
            voice: None,
            execution_pending: true,
        })
        .session(&mut *tx)
        .await?;
    db.collection::<Document>(CALLS)
        .update_one(
            doc! {"_id":&call.id},
            doc! {"$set":{"receipt_message_id":id}},
        )
        .session(&mut *tx)
        .await?;
    Ok(())
}

fn usage_filter(user_id: &str, call_id: &str, created_at: chrono::DateTime<Utc>) -> Document {
    let prefix = regex::escape(&format!("voice:{call_id}:"));
    doc! {
        "billing_owner_id": user_id,
        "created_at": {"$gte": bson::DateTime::from_chrono(created_at)},
        "billing_request_id": {"$regex": format!("^{prefix}")},
    }
}

#[allow(clippy::too_many_arguments)]
fn render(
    seconds: i64,
    has_meters: bool,
    pending_cost: bool,
    total_cost: Option<Credits>,
    voice_cost: Option<Credits>,
    requests: &[VoiceRequest],
    sources: &HashMap<String, AssistantMessage>,
    cards: &[Document],
) -> String {
    let mut text = format!("**Call receipt** · {}:{:02}", seconds / 60, seconds % 60);
    if has_meters {
        let cost_line = if pending_cost {
            "Cost: pending".to_owned()
        } else if let (Some(total), Some(voice)) = (total_cost, voice_cost)
            && total == voice
        {
            format!("Cost: {total}")
        } else if let Some(total) = total_cost {
            match voice_cost {
                Some(voice) => format!("Credits: {total} · Voice cost: {voice}"),
                None => format!("Cost: {total}"),
            }
        } else {
            "Cost: pending".to_owned()
        };
        text.push_str("\n\n");
        text.push_str(&cost_line);
    }
    let has_work = !requests.is_empty();
    text.push_str("\n\n");
    text.push_str(if has_work {
        "Handed to NyxBot:\n"
    } else {
        "Just a conversation, nothing handed off."
    });
    if !has_work {
        return text;
    }
    for request in requests {
        let title = sources
            .get(&request.message_id)
            .map(|m| m.text.replace(['\n', '\r'], " "))
            .unwrap_or_else(|| "Voice request".into());
        let title: String = title.chars().take(96).collect();
        let status = match &request.state {
            RequestState::Completed => "Done",
            RequestState::Claimed => "Running",
            RequestState::Queued => "Queued",
            RequestState::Cancelled => "Cancelled",
            RequestState::AwaitingConfirmation => "Needs your OK",
        };
        let link = request
            .result_message_id
            .as_deref()
            .map(|id| format!(" · [result](#message-{id})"))
            .unwrap_or_default();
        text.push_str(&format!("\n- **{title}** — {status}{link}"));
        for card in cards
            .iter()
            .filter(|card| card.get_str("voice_request_id").ok() == Some(request.id.as_str()))
        {
            let decision = match card.get_str("status").unwrap_or("unknown") {
                "allowed" | "used" => "approved",
                "denied" => "denied",
                _ => "decided",
            };
            text.push_str(&format!("\n  - Confirmation: {decision}"));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(state: RequestState, id: &str, message_id: &str) -> VoiceRequest {
        VoiceRequest {
            id: id.into(),
            user_id: "owner".into(),
            conversation_id: "thread".into(),
            task_conversation_id: Some("hidden".into()),
            session_id: "call".into(),
            source_id: id.into(),
            message_id: message_id.into(),
            message_seq: 1,
            turn_id: id.into(),
            state,
            pending_acknowledgement_ids: Vec::new(),
            acknowledgement_id: None,
            credential_api_key_id: None,
            parent_turn_id: None,
            root_request_id: None,
            result_message_id: Some("result-1".into()),
            recovery_replays: 0,
            created_at: Utc::now(),
            expires_at: Utc::now(),
        }
    }

    #[test]
    fn receipt_hides_empty_work_and_lists_human_statuses_and_results() {
        let empty = render(12, false, false, None, None, &[], &HashMap::new(), &[]);
        assert!(empty.contains("Just a conversation, nothing handed off"));
        assert!(!empty.contains("Cost:"));
        let done = request(RequestState::Completed, "r1", "m1");
        let queued = request(RequestState::Queued, "r2", "m2");
        let sources = HashMap::from([(
            "m1".into(),
            AssistantMessage {
                voice: None,
                execution_pending: true,
                id: "m1".into(),
                conversation_id: "thread".into(),
                user_id: "owner".into(),
                seq: 1,
                turn_id: "r1".into(),
                role: "user".into(),
                text: "Check my calendar".into(),
                status: "completed".into(),
                error_code: None,
                created_at: Utc::now(),
                activities: Vec::new(),
                attachments: Vec::new(),
                origin: None,
                via: Some("voice".into()),
            },
        )]);
        let text = render(
            72,
            true,
            false,
            Some(Credits::ZERO),
            Some(Credits::ZERO),
            &[done, queued],
            &sources,
            &[],
        );
        assert!(text.contains("Check my calendar"));
        assert!(text.contains("Done"));
        assert!(text.contains("Queued"));
        assert!(text.contains("#message-result-1"));
        let voice_only = render(
            72,
            true,
            false,
            Some(Credits::from_micros(7)),
            Some(Credits::from_micros(7)),
            &[],
            &HashMap::new(),
            &[],
        );
        assert!(voice_only.contains("Cost: 0.000007"));
        assert!(!voice_only.contains("Credits:"));
        let pending = render(72, true, true, None, None, &[], &HashMap::new(), &[]);
        assert!(pending.contains("Cost: pending"));
    }

    #[test]
    fn usage_filter_uses_the_indexed_billing_owner_prefix() {
        let filter = usage_filter("owner", "call", Utc::now());
        assert_eq!(filter.get_str("billing_owner_id").unwrap(), "owner");
        assert!(
            filter
                .get_document("created_at")
                .unwrap()
                .contains_key("$gte")
        );
        assert!(
            filter
                .get_document("billing_request_id")
                .unwrap()
                .get_str("$regex")
                .unwrap()
                .starts_with("^voice:call:")
        );
    }
}
