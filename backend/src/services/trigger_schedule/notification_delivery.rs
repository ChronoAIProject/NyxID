//! Durable push result state. Report text remains only in the original thread.
use super::result_delivery::{DeliveryClaim, claim_filter};
use super::*;
use crate::models::{trigger_run::NotificationDelivery, trigger_schedule::DeliverTo};
use crate::services::notification_service::automation;

pub const MAX_ATTEMPTS: u32 = 3;

pub fn status(
    state: &str,
    reason: Option<&str>,
    attempts: u32,
    http_status: Option<u16>,
) -> NotificationDelivery {
    NotificationDelivery {
        state: state.into(),
        reason: reason.map(str::to_owned),
        attempts,
        http_status,
        retry_at: None,
    }
}

pub async fn save(
    db: &Database,
    claim: &DeliveryClaim,
    status: &NotificationDelivery,
) -> AppResult<bool> {
    let encoded = bson::to_bson(status)
        .map_err(|_| AppError::Internal("Notification status is unavailable".into()))?;
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run((db, claim, encoded), |session, (db, claim, encoded)| {
            Box::pin(async move {
                let operation: AppResult<bool> = async {
                    let Some(run) = db
                        .collection::<TriggerRun>(RUNS)
                        .find_one_and_update(
                            claim_filter(claim),
                            doc! {"$set":{"notification_delivery":encoded.clone()}},
                        )
                        .return_document(ReturnDocument::After)
                        .session(&mut *session)
                        .await?
                    else {
                        return Ok(false);
                    };
                    super::project_last_run(db, &run, Some(session)).await?;
                    Ok(true)
                }
                .await;
                transactions::transaction_result(operation)
            })
        })
        .await
        .map_err(transactions::map_transaction_error)
}

/// Legacy claimed sends and expired attempts are never replayed. This runs in
/// the terminal transaction, so a notice cannot be lost between state and finish.
pub(super) async fn unrecoverable_in_session(
    db: &Database,
    filter: &Document,
    session: &mut ClientSession,
) -> AppResult<()> {
    let Some(run) = db
        .collection::<TriggerRun>(RUNS)
        .find_one(filter.clone())
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    if !matches!(run.deliver_to, DeliverTo::Notification) {
        return Ok(());
    }
    let previous = run.notification_delivery.as_ref();
    if previous.is_some_and(|s| matches!(s.state.as_str(), "accepted" | "failed" | "unknown")) {
        return Ok(());
    }
    let reason = if previous.is_some_and(|s| s.state == "retrying") {
        "push_retries_exhausted"
    } else {
        "push_delivery_unknown"
    };
    let status = status(
        if reason == "push_delivery_unknown" {
            "unknown"
        } else {
            "failed"
        },
        Some(reason),
        previous.map_or(0, |s| s.attempts),
        previous.and_then(|s| s.http_status),
    );
    let encoded = bson::to_bson(&status)
        .map_err(|_| AppError::Internal("Notification status is unavailable".into()))?;
    db.collection::<Document>(RUNS)
        .update_one(
            filter.clone(),
            doc! {"$set":{"notification_delivery":encoded}},
        )
        .session(session)
        .await?;
    Ok(())
}

pub(super) async fn notice_in_session(
    db: &Database,
    run: &TriggerRun,
    frontend: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let Some(delivery) = run
        .notification_delivery
        .as_ref()
        .filter(|s| matches!(s.state.as_str(), "failed" | "unknown"))
    else {
        return Ok(());
    };
    let Some(thread) = &run.thread_id else {
        return Ok(());
    };
    use crate::models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    };
    use crate::services::assistant_links::AssistantPage;
    // The terminal run CAS and this insert share one transaction. A competing
    // finalizer cannot append a second notice, including transaction retries.
    let notice_id = Uuid::new_v4().to_string();
    let execution = if run.execution_outcome == Some(RunOutcome::Completed) {
        "Task completed; its report is saved in this thread."
    } else {
        "The task's result is saved in this thread."
    };
    let text = format!(
        "{execution} Mobile push delivery needs attention. {} A linked Telegram notification account does not enable mobile push; select a Telegram channel chat as the result destination to receive reports there. Review this automation: {}",
        automation::message(delivery.reason.as_deref().unwrap_or_default()),
        AssistantPage::Automations {
            setup: None,
            agent: run.agent_id.as_deref()
        }
        .url(frontend)
    );
    let now = Utc::now();
    if let Some(conversation) = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one_and_update(
            doc! {"_id":thread,"user_id":&run.user_id},
            doc! {"$inc":{"message_count":1},"$set":{"updated_at":date(now)}},
        )
        .return_document(ReturnDocument::After)
        .session(&mut *session)
        .await?
    {
        let message = AssistantMessage {
            automation_work: None,
            execution_task: None,
            delegated_images: Default::default(),
            media_event_ids: Vec::new(),
            media_queue_notice_id: None,
            media_pending: None,
            media_bound: None,
            platform_delivery_recovery_queued: false,
            platform_reply: None,
            platform_origin: None,
            author: None,
            retry_claimed_by: None,
            activity_count: None,
            activity_total: 0,
            report_to: None,
            id: notice_id.clone(),
            conversation_id: thread.clone(),
            user_id: run.user_id.clone(),
            seq: conversation.message_count,
            turn_id: notice_id.clone(),
            role: "event".into(),
            text,
            status: "completed".into(),
            error_code: None,
            upstream_error_code: None,
            error_detail: None,
            turn_metadata: None,
            created_at: now,
            activities: vec![],
            attachments: vec![],
            origin: None,
            via: None,
            continuation_of: None,
            retry_of: None,
            specialist_id: None,
            steering: None,
            voice: None,
            execution_pending: false,
        };
        db.collection::<AssistantMessage>(MESSAGES)
            .insert_one(message)
            .session(session)
            .await?;
    }
    Ok(())
}
