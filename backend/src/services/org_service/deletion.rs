//! Audit boundaries only. The marker is evidence, not deletion admission or a lease.
use bson::{Bson, Document, doc};
use mongodb::{ClientSession, Database, options::ReturnDocument};

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

const REQUESTED: &str = "org_deletion_requested";
const COMPLETED: &str = "org_deleted";

pub(super) fn occurrence(
    actor: &AuditActor,
    event: &str,
    org: &str,
) -> AppResult<SecurityEffectIntent> {
    required::intent_preserving_actor(
        actor,
        event,
        Some(serde_json::json!({
            "org_user_id": required::text(TextField::Identifier, org),
        })),
    )
}

struct Start<'a> {
    db: &'a Database,
    org: &'a str,
    intent: SecurityEffectIntent,
}

pub(super) async fn start(db: &Database, org: &str, actor: &AuditActor) -> AppResult<String> {
    let context = Start {
        db,
        org,
        intent: occurrence(actor, REQUESTED, org)?,
    };
    transaction_service::run_db_only(db, &context, |tx, context| {
        Box::pin(start_in_session(context, tx))
    })
    .await
}

async fn start_in_session(context: &Start<'_>, tx: &mut ClientSession) -> AppResult<String> {
    let Start { db, org, intent } = context;
    // Return the preimage in the same command that creates the marker. A
    // transaction retry sees a competing start's marker and emits no duplicate.
    let previous = db
        .collection::<Document>(super::USERS)
        .find_one_and_update(
            doc! {"_id": *org, "user_type": "org"},
            vec![
                doc! {"$set": {"deletion_audit": {"$ifNull": ["$deletion_audit", {
                    "event_id": {"$literal": &intent.id}, "requested_at": "$$NOW",
                }]}}},
            ],
        )
        .projection(doc! {"deletion_audit": 1})
        .return_document(ReturnDocument::Before)
        .session(&mut *tx)
        .await?
        .ok_or_else(|| AppError::OrgNotFound((*org).to_owned()))?;
    match previous.get("deletion_audit") {
        None | Some(Bson::Null) => {
            security_effect_service::enqueue_in_session(db, tx, std::slice::from_ref(intent))
                .await?;
            Ok(intent.id.clone())
        }
        Some(Bson::Document(marker)) => marker
            .get_str("event_id")
            .ok()
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
            .map(str::to_owned)
            .ok_or_else(|| AppError::Internal("Organization deletion evidence is invalid".into())),
        Some(_) => Err(AppError::Internal(
            "Organization deletion evidence is invalid".into(),
        )),
    }
}

struct Completion<'a> {
    db: &'a Database,
    org: &'a str,
    marker: &'a str,
    intent: SecurityEffectIntent,
}
pub(super) async fn complete(
    db: &Database,
    org: &str,
    marker: &str,
    actor: &AuditActor,
) -> AppResult<()> {
    let context = Completion {
        db,
        org,
        marker,
        intent: occurrence(actor, COMPLETED, org)?,
    };
    transaction_service::run_db_only(db, &context, |tx, context| {
        Box::pin(complete_in_session(context, tx))
    })
    .await
}
async fn complete_in_session(context: &Completion<'_>, tx: &mut ClientSession) -> AppResult<()> {
    let Completion {
        db,
        org,
        marker,
        intent,
    } = context;
    super::super::execution_permission_enrollment::jobs::delete_memberships_in_session(
        db, org, false, tx,
    )
    .await?;
    let deleted = db
        .collection::<Document>(super::USERS)
        .delete_one(doc! {
            "_id": *org, "user_type": "org", "deletion_audit.event_id": *marker,
        })
        .session(&mut *tx)
        .await?;
    if deleted.deleted_count != 1 {
        return Err(AppError::OrgNotFound((*org).to_owned()));
    }
    security_effect_service::enqueue_in_session(db, tx, std::slice::from_ref(intent)).await
}
