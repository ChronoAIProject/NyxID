//! Required evidence for the administrator service-account lifecycle.
use super::{SERVICE_ACCOUNTS, ServiceAccount, require_managed_match};
use crate::{
    errors::{AppError, AppResult},
    models::security_effect_intent::SecurityEffectIntent,
    services::{
        audit_service::{
            AuditActor,
            required::{self, TextField},
        },
        security_effect_service, transaction_service,
    },
};
use bson::Document;
use mongodb::{ClientSession, Database, results::UpdateResult};

pub(super) enum Event<'a> {
    Created { client_id: &'a str, name: &'a str },
    Updated,
    Deleted,
    Rotated { client_id: &'a str },
    TokensRevoked { count: u64 },
}
impl Event<'_> {
    pub(super) fn intent(&self, actor: &AuditActor, id: &str) -> AppResult<SecurityEffectIntent> {
        let mut data = serde_json::json!({"target_sa_id":required::text(TextField::Identifier,id)});
        let event = match self {
            Self::Created { client_id, name } => {
                data["client_id"] = required::text(TextField::Identifier, client_id).into();
                data["name"] = required::text(TextField::DisplayName, name).into();
                "admin.sa.created"
            }
            Self::Updated => "admin.sa.updated",
            Self::Deleted => "admin.sa.deleted",
            Self::Rotated { client_id } => {
                data["client_id"] = required::text(TextField::Identifier, client_id).into();
                "admin.sa.secret_rotated"
            }
            Self::TokensRevoked { count } => {
                data["revoked_count"] = (*count).into();
                "admin.sa.tokens_revoked"
            }
        };
        required::intent_preserving_actor(actor, event, Some(data))
    }
}

pub(super) struct Mutation {
    pub db: Database,
    pub filter: Document,
    pub update: Document,
    pub expected_access: bool,
}
impl Mutation {
    async fn write(&self, session: Option<&mut ClientSession>) -> AppResult<UpdateResult> {
        let collection = self.db.collection::<ServiceAccount>(SERVICE_ACCOUNTS);
        let mut write = collection.update_one(self.filter.clone(), self.update.clone());
        if let Some(session) = session {
            write = write.session(session);
        }
        let result = write.await?;
        if self.expected_access && result.matched_count == 0 {
            return Err(AppError::Conflict(
                "Service account access changed; reload before applying catalog access".into(),
            ));
        }
        require_managed_match(result.matched_count)?;
        Ok(result)
    }
    pub(super) async fn apply(&self, intent: Option<&SecurityEffectIntent>) -> AppResult<()> {
        if let Some(intent) = intent {
            let context = (self, intent);
            transaction_service::run_db_only(&self.db, &context, |tx, context| {
                Box::pin(apply_in_session(tx, context))
            })
            .await
        } else {
            self.write(None).await?;
            Ok(())
        }
    }
}
async fn apply_in_session(
    tx: &mut ClientSession,
    context: &(&Mutation, &SecurityEffectIntent),
) -> AppResult<()> {
    let (source, intent) = context;
    let result = source.write(Some(&mut *tx)).await?;
    if result.modified_count != 0 {
        security_effect_service::enqueue_in_session(&source.db, tx, std::slice::from_ref(*intent))
            .await?;
    }
    Ok(())
}

pub(super) async fn revoke(source: &Mutation, id: &str, actor: &AuditActor) -> AppResult<u64> {
    let context = (source, id, actor);
    transaction_service::run_db_only(&source.db, &context, |tx, context| {
        Box::pin(revoke_in_session(tx, context))
    })
    .await
}
async fn revoke_in_session(
    tx: &mut ClientSession,
    context: &(&Mutation, &str, &AuditActor),
) -> AppResult<u64> {
    let (source, id, actor) = context;
    source.write(Some(&mut *tx)).await?;
    let count = super::revoke_token_rows_in_session(&source.db, id, Some(&mut *tx)).await?;
    let intent = Event::TokensRevoked { count }.intent(actor, id)?;
    security_effect_service::enqueue_in_session(&source.db, tx, &[intent]).await?;
    Ok(count)
}
