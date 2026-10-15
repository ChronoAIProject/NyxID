//! Shared owner reply decisions. Transport-specific authority is admitted by
//! the caller; the decision body can join its binding and ingress transaction.
use mongodb::Database;

use crate::{
    errors::AppResult,
    models::assistant_acknowledgement::AssistantAcknowledgement,
    services::{
        api_key_mutation_service as transactions, assistant_acknowledgement_service as acks,
        assistant_nyxagent as engine, audit_service,
    },
};

/// Ordinary channel wrapper: preserve same-thread previous-message selection,
/// decision effects and metadata-only audit without constructing HTTP auth.
pub(crate) async fn decide_owner_reply(
    db: &Database,
    owner: &str,
    conversation_id: &str,
    text: &str,
) -> AppResult<Option<AssistantAcknowledgement>> {
    if acks::parse_reply(text).is_none() {
        return Ok(None);
    }
    let since = engine::previous_user_message(db, owner, conversation_id, None)
        .await?
        .map(|message| message.created_at);
    let mut session = db.client().start_session().await?;
    let decision_db = db.clone();
    let decision_owner = owner.to_owned();
    let conversation_ids = vec![conversation_id.to_owned()];
    let text = text.to_owned();
    let decided = session
        .start_transaction()
        .and_run2(async move |session| {
            let result = Box::pin(acks::decide_reply_in_session(
                &decision_db,
                session,
                &decision_owner,
                &conversation_ids,
                &text,
                since,
            ))
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    if let Some(decided) = &decided {
        acks::audit_decision(
            db,
            &audit_service::AuditActor {
                user_id: owner.to_owned(),
                ip_address: None,
                user_agent: None,
                api_key_id: None,
                api_key_name: None,
            },
            decided,
        )
        .await;
    }
    Ok(decided)
}
