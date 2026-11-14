//! Read accepted delivery metadata after an automatic reply has settled.
//! Caller validates the live actor and exact destination; this service keeps
//! every query bound to that destination. It never retries a provider effect.
use crate::{
    errors::{AppError, AppResult},
    models::{
        channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
        channel_reply_delivery::{COLLECTION_NAME as REPLIES, ChannelReplyDelivery, PartStatus},
        channel_text_delivery::{COLLECTION_NAME as TEXT, ChannelTextDelivery, TextPartState},
        nyxbot_channel::{EVENTS_COLLECTION_NAME as EVENTS, NyxbotChannel, NyxbotThread},
        trigger_run::{COLLECTION_NAME as RUNS, RunOutcome, TriggerRun},
    },
};
use bson::{Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::Database;
use serde_json::{Value, json};

fn unavailable() -> AppError {
    AppError::NotFound("Delivery evidence unavailable for this destination".into())
}

fn scope(channel: &NyxbotChannel, target: &NyxbotThread) -> AppResult<Document> {
    Ok(doc! {
        "channel_bot_id":&channel.channel_bot_id,
        "user_id":channel.bot_owner_id.as_deref().unwrap_or(&channel.user_id),
        "conversation_id":channel.route_id.as_deref().ok_or_else(unavailable)?,
        "agent_api_key_id":&channel.route_api_key_id,"platform":&channel.platform,
        "platform_conversation_id":target.platform_chat_id.as_deref().ok_or_else(unavailable)?,
    })
}

pub(crate) async fn read(
    db: &Database,
    channel: &NyxbotChannel,
    target: &NyxbotThread,
    actor: &str,
    agent: Option<&str>,
    args: &Value,
) -> AppResult<Value> {
    let source = args["source_message_id"].as_str();
    let schedule = args["schedule_id"].as_str();
    if source.is_some() == schedule.is_some() {
        return Err(AppError::ValidationError(
            "Supply exactly one of source_message_id or schedule_id".into(),
        ));
    }
    if let Some(source) = source {
        let mut filter = scope(channel, target)?;
        filter.extend(doc! {"_id":source,"direction":"inbound"});
        let inbound = db
            .collection::<ChannelMessage>(MESSAGES)
            .find_one(filter)
            .await?
            .ok_or_else(unavailable)?;
        let mut filter = scope(channel, target)?;
        filter.extend(doc! {"reply_to_message_id":source,"direction":"outbound"});
        let accepted: Vec<ChannelMessage> = db
            .collection(MESSAGES)
            .find(filter)
            .sort(doc! {"created_at":1,"_id":1})
            .limit(100)
            .await?
            .try_collect()
            .await?;
        let mut receipts: Vec<_> = accepted.iter().filter(|m| m.platform_message_id.as_deref().is_some_and(|id| !id.is_empty()))
            .map(|m| json!({"message_id":m.id,"platform_message_id":m.platform_message_id,"accepted_at":m.created_at.to_rfc3339()})).collect();
        let mut state = if receipts.is_empty() {
            "unconfirmed"
        } else {
            "confirmed"
        };
        if let Some(plan)=db.collection::<ChannelReplyDelivery>(REPLIES).find_one(doc! {
            "user_id":&inbound.user_id,"channel_bot_id":&channel.channel_bot_id,"inbound_message_id":source,
        }).sort(doc! {"created_at":-1,"_id":-1}).await? {
            (state,receipts)=reply_status(&plan);
        }
        let event = db
            .collection::<Document>(EVENTS)
            .find_one(doc! {
                "channel_id":&channel.id,"user_id":&channel.user_id,"partition":&target.partition,
                "$or":[{"relay_message_id":source},{"event_id":source}],
            })
            .sort(doc! {"created_at":-1,"_id":-1})
            .projection(doc! {"status":1})
            .await?;
        let execution_state = event
            .as_ref()
            .and_then(|e| e.get_str("status").ok())
            .unwrap_or("unknown");
        return Ok(json!({"chat_id":target.id,"source_message_id":source,
            "source_platform_message_id":inbound.platform_message_id,
            "state":state,
            "receipts":receipts,"execution_state":execution_state,
            "answer_succeeded":match execution_state {"completed"=>Some(true),"failed"=>Some(false),_=>None},
            "checked_at":Utc::now().to_rfc3339(),
            "meaning":"platform acceptance; not a read receipt"}));
    }
    let mut filter = doc! {"trigger_id":schedule.unwrap(),"user_id":actor,
    "deliver_to.type":"chat","deliver_to.chat_id":&target.id};
    if let Some(agent) = agent {
        filter.insert("agent_id", agent);
    }
    let run = db
        .collection::<TriggerRun>(RUNS)
        .find_one(filter)
        .sort(doc! {"scheduled_at":-1,"_id":-1})
        .await?
        .ok_or_else(unavailable)?;
    let request = format!("automation:{}", run.id);
    let key = super::mcp_service::canonical_sha256(json!([
        "nyxbot-post-v1",
        channel.user_id,
        target.id,
        request
    ]));
    let delivery = db
        .collection::<ChannelTextDelivery>(TEXT)
        .find_one(doc! {"logical_key":key,"user_id":&channel.user_id,"source_id":request})
        .await?;
    let (state, receipts) = match delivery {
        Some(delivery) => plan_status(&delivery),
        None => (
            if run.result_claimed {
                "unknown"
            } else {
                "pending"
            },
            Vec::new(),
        ),
    };
    Ok(
        json!({"chat_id":target.id,"schedule_id":schedule,"run_id":run.id,
        "scheduled_at":run.scheduled_at.to_rfc3339(),"finished_at":run.finished_at.map(|v| v.to_rfc3339()),
        "execution_outcome":run.execution_outcome,"run_outcome":run.outcome,
        "state":state,"receipts":receipts,"checked_at":Utc::now().to_rfc3339(),
        "task_completed":run.outcome==RunOutcome::Completed,
        "meaning":"platform acceptance; not a read receipt"}),
    )
}

fn reply_status(plan: &ChannelReplyDelivery) -> (&'static str, Vec<Value>) {
    let receipts = plan
        .parts
        .iter()
        .filter(|p| p.status == PartStatus::Accepted)
        .flat_map(|p| {
            p.platform_message_ids
                .iter()
                .filter(|id| !id.is_empty())
                .map(|id| json!({"message_id":p.outbound_message_id,"platform_message_id":id}))
        })
        .collect();
    let all_accepted = plan.expected_parts > 0
        && plan.parts.len() == plan.expected_parts as usize
        && plan.parts.iter().all(|p| {
            p.status == PartStatus::Accepted
                && !p.platform_message_ids.is_empty()
                && p.platform_message_ids.iter().all(|id| !id.is_empty())
        });
    let state = if all_accepted && plan.complete {
        "confirmed"
    } else if plan.parts.iter().any(|p| p.status == PartStatus::Uncertain) {
        "unknown"
    } else if plan.parts.iter().any(|p| p.status == PartStatus::Failed) {
        "refused"
    } else {
        "pending"
    };
    (state, receipts)
}

fn plan_status(delivery: &ChannelTextDelivery) -> (&'static str, Vec<Value>) {
    let receipts = delivery
        .parts
        .iter()
        .filter(|p| p.state == TextPartState::Sent)
        .filter_map(|p| {
            p.platform_message_id
                .as_ref()
                .filter(|id| !id.is_empty())
                .map(|id| json!({"message_id":p.outbound_message_id,"platform_message_id":id}))
        })
        .collect::<Vec<_>>();
    let state = if !delivery.parts.is_empty() && receipts.len() == delivery.parts.len() {
        "confirmed"
    } else if delivery
        .parts
        .iter()
        .any(|p| p.state == TextPartState::Sending)
    {
        "unknown"
    } else if delivery
        .parts
        .iter()
        .any(|p| p.state == TextPartState::Refused)
    {
        "refused"
    } else {
        "pending"
    };
    (state, receipts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::channel_text_delivery::TextPart;

    #[test]
    fn an_accepted_prefix_never_confirms_the_whole_reply() {
        use crate::models::channel_reply_delivery::ReplyPart;
        let part = ReplyPart {
            formatting_fallback: None,
            status: PartStatus::Accepted,
            platform_message_ids: vec!["om_prefix".into()],
            outbound_message_id: Some("outbound".into()),
            failure_code: None,
        };
        let mut p = ChannelReplyDelivery {
            id: "delivery".into(),
            user_id: "owner".into(),
            channel_bot_id: "bot".into(),
            inbound_message_id: "source".into(),
            expected_parts: 2,
            complete: false,
            created_at: Utc::now(),
            parts: vec![part.clone(), part],
        };
        p.parts[1].status = PartStatus::Uncertain;
        assert_eq!(reply_status(&p).0, "unknown");
        p.parts[1].status = PartStatus::Failed;
        assert_eq!(reply_status(&p).0, "refused");
        p.parts[1].status = PartStatus::Accepted;
        assert_eq!(reply_status(&p).0, "pending");
        p.complete = true;
        assert_eq!(reply_status(&p).0, "confirmed");
        p.parts[1].platform_message_ids.clear();
        assert_ne!(reply_status(&p).0, "confirmed");
    }

    #[test]
    fn only_all_accepted_parts_confirm_a_delivery() {
        let mut d = ChannelTextDelivery {
            id: "receipt".into(),
            logical_key: "key".into(),
            user_id: "owner".into(),
            source_id: "source".into(),
            fingerprint: "fingerprint".into(),
            notice: None,
            retry_at: None,
            created_at: Utc::now(),
            parts: vec![],
        };
        assert_eq!(plan_status(&d).0, "pending");
        d.parts.push(TextPart {
            formatting_fallback: None,
            state: TextPartState::Sent,
            attempt_id: None,
            platform_message_id: Some("om_one".into()),
            outbound_message_id: Some("outbound".into()),
        });
        assert_eq!(plan_status(&d).0, "confirmed");
        let mut second = d.parts[0].clone();
        second.state = TextPartState::Pending;
        second.platform_message_id = None;
        d.parts.push(second);
        assert_eq!(plan_status(&d).0, "pending");
        d.parts[1].state = TextPartState::Sending;
        assert_eq!(plan_status(&d).0, "unknown");
        d.parts[1].state = TextPartState::Refused;
        assert_eq!(plan_status(&d).0, "refused");
        d.parts[1].state = TextPartState::Sent;
        assert_eq!(
            plan_status(&d).0,
            "pending",
            "missing platform IDs are never delivery proof"
        );
    }
}
