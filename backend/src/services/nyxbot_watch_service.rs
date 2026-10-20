//! Report provenance for NyxBot watches (`nyxbot_watches`): taken from the
//! exact chat that caused the watch, never looked up again, so a replacement
//! turn is never captured.
use crate::models::{
    assistant_conversation::{AgentEvent, REPORT_PROVENANCE_V1, carried_report_to},
    nyxbot_channel::NyxbotWatch,
};
use mongodb::bson::{self, doc};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReportProvenance {
    pub invocation_id: Option<String>,
    /// The causing turn's `ActiveTurn::report_to`, meaningful only with
    /// `version == REPORT_PROVENANCE_V1`.
    pub report_to: Option<String>,
    /// 0 = legacy, nothing proven; 1 = validated report snapshot.
    pub version: u8,
}

impl ReportProvenance {
    pub const LEGACY: Self = Self {
        invocation_id: None,
        report_to: None,
        version: 0,
    };

    /// Never re-read from the conversation, so a replacement turn cannot be
    /// captured.
    pub fn from_chat(chat: &super::assistant_acknowledgement_service::ChatAuthority) -> Self {
        if chat.turn_live && !chat.turn_stopped {
            Self {
                invocation_id: chat.invocation_id.clone(),
                report_to: chat.report_to.clone(),
                version: REPORT_PROVENANCE_V1,
            }
        } else {
            Self::LEGACY
        }
    }

    pub fn from_watch(watch: &NyxbotWatch) -> Self {
        if watch.report_provenance == REPORT_PROVENANCE_V1 {
            Self {
                invocation_id: watch.invocation_id.clone(),
                report_to: watch.report_to.clone(),
                version: REPORT_PROVENANCE_V1,
            }
        } else {
            Self::LEGACY
        }
    }

    pub fn fields(&self) -> bson::Document {
        doc! {"invocation_id":self.invocation_id.clone(),"report_to": self.report_to.clone(), "report_provenance": i32::from(self.version)}
    }

    pub fn carried(&self) -> Option<String> {
        carried_report_to(self.report_to.as_deref(), self.version)
    }
}

pub fn watch_event(watch: &NyxbotWatch, kind: &str, text: String) -> AgentEvent {
    let event = super::assistant_team_service::event(kind, text, None)
        .for_invocation(watch.invocation_id.as_deref());
    let provenance = ReportProvenance::from_watch(watch);
    if provenance.version == REPORT_PROVENANCE_V1 {
        event.bound_to(provenance.carried().as_deref())
    } else {
        event
    }
}

/// Invocation watch creation shares the root fence with cancellation and
/// terminalization. Ordinary watch creation retains its existing fast path.
pub async fn upsert(
    db: &mongodb::Database,
    owner: &str,
    invocation: Option<&str>,
    mut filter: bson::Document,
    update: bson::Document,
) -> crate::errors::AppResult<()> {
    use super::{api_key_mutation_service as transactions, assistant_invocation_service};
    filter.insert("invocation_id", invocation);
    if invocation.is_none() {
        db.collection::<bson::Document>(crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME)
            .update_one(filter, update)
            .upsert(true)
            .await?;
        return Ok(());
    }
    let db = db.clone();
    let owner = owner.to_owned();
    let invocation = invocation.map(str::to_owned);
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (db, owner, invocation, filter, update),
            |session, (db, owner, invocation, filter, update)| {
                Box::pin(async move {
                    let result = async {
                        if !assistant_invocation_service::fence_in_session(
                            db,
                            session,
                            owner,
                            invocation.as_deref(),
                        )
                        .await?
                        {
                            return Err(crate::errors::AppError::AssistantTurnRequired);
                        }
                        db.collection::<bson::Document>(
                            crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME,
                        )
                        .update_one(filter.clone(), update.clone())
                        .upsert(true)
                        .session(&mut *session)
                        .await?;
                        Ok(())
                    }
                    .await;
                    transactions::transaction_result(result)
                })
            },
        )
        .await
        .map_err(transactions::map_transaction_error)
}
