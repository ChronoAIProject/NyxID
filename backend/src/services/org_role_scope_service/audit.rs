use crate::{
    errors::AppResult,
    models::{
        org_membership::OrgRole,
        org_role_scope::{COLLECTION_NAME, OrgRoleScope},
    },
    services::{
        audit_service::{
            AuditActor,
            required::{self, TextField},
        },
        org_membership_authority, security_effect_service,
    },
};
use bson::{Bson, doc};
use mongodb::{ClientSession, Database, options::ReturnDocument};

pub(super) fn set_intent(
    actor: &AuditActor,
    row: &OrgRoleScope,
) -> AppResult<crate::models::security_effect_intent::SecurityEffectIntent> {
    let mut data = serde_json::json!({
        "org_user_id": required::text(TextField::Identifier, &row.org_user_id),
        "role": row.role,
    });
    if let Some(ids) = row.allowed_service_ids.as_ref() {
        for (key, value) in required::identifier_list("allowed_service_ids", ids) {
            data[&key] = value;
        }
    } else {
        data["allowed_service_ids"] = serde_json::Value::Null;
    }
    required::intent_preserving_actor(actor, "org_role_scope_set", Some(data))
}
pub(super) fn clear_intent(
    actor: &AuditActor,
    org: &str,
    role: OrgRole,
) -> AppResult<crate::models::security_effect_intent::SecurityEffectIntent> {
    required::intent_preserving_actor(
        actor,
        "org_role_scope_cleared",
        Some(
            serde_json::json!({"org_user_id":required::text(TextField::Identifier,org),"role":role}),
        ),
    )
}

pub(super) async fn set(
    db: &Database,
    org: &str,
    role: OrgRole,
    allowed_service_ids: Option<Vec<String>>,
    actor: &AuditActor,
) -> AppResult<OrgRoleScope> {
    let row = OrgRoleScope {
        id: uuid::Uuid::new_v4().to_string(),
        org_user_id: org.to_owned(),
        role,
        allowed_service_ids,
        updated_at: bson::DateTime::now().to_chrono(),
        updated_by: actor.user_id.clone(),
    };
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result = set_in_session(db, row, actor, &mut tx).await;
    org_membership_authority::finish(tx, result).await
}
async fn set_in_session(
    db: &Database,
    mut row: OrgRoleScope,
    actor: &AuditActor,
    tx: &mut ClientSession,
) -> AppResult<OrgRoleScope> {
    let allowed = match &row.allowed_service_ids {
        None => Bson::Null,
        Some(ids) => {
            bson::to_bson(ids).map_err(|e| crate::errors::AppError::Internal(e.to_string()))?
        }
    };
    // Return the atomic preimage, avoiding a separate comparison read. On an
    // equal save the pipeline preserves the whole row, including author/time.
    let equal = doc! {"$and":[{"$ne":[{"$type":"$updated_at"},"missing"]},{"$eq":[{"$ifNull":["$allowed_service_ids",Bson::Null]},{"$literal":&allowed}]}]};
    let update = doc! {"$set":{
        "_id":{"$ifNull":["$_id",{"$literal":&row.id}]},
        "allowed_service_ids":{"$literal":&allowed},
        "updated_at":{"$cond":[&equal,"$updated_at",bson::DateTime::from_chrono(row.updated_at)]},
        "updated_by":{"$cond":[&equal,"$updated_by",{"$literal":&row.updated_by}]},
    }};
    let previous = db
        .collection::<OrgRoleScope>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {"org_user_id":&row.org_user_id,"role":row.role.as_str()},
            vec![update],
        )
        .upsert(true)
        .return_document(ReturnDocument::Before)
        .session(&mut *tx)
        .await?;
    if let Some(previous) = previous {
        if previous.allowed_service_ids == row.allowed_service_ids {
            return Ok(previous);
        }
        row.id = previous.id;
    }
    if row.role == OrgRole::Admin {
        db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.org_user_id},
                doc! {"$inc":{"approval_decision_fence":1_i64}},
            )
            .session(&mut *tx)
            .await?;
    }
    let intent = set_intent(actor, &row)?;
    security_effect_service::enqueue_in_session(db, tx, &[intent]).await?;
    Ok(row)
}
pub(super) async fn clear(
    db: &Database,
    org: &str,
    role: OrgRole,
    actor: &AuditActor,
) -> AppResult<()> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result = clear_in_session(db, org, role, actor, &mut tx).await;
    org_membership_authority::finish(tx, result).await
}
async fn clear_in_session(
    db: &Database,
    org: &str,
    role: OrgRole,
    actor: &AuditActor,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let deleted = db
        .collection::<OrgRoleScope>(COLLECTION_NAME)
        .delete_one(doc! {"org_user_id":org,"role":role.as_str()})
        .session(&mut *tx)
        .await?;
    if deleted.deleted_count == 1 {
        let intent = clear_intent(actor, org, role)?;
        security_effect_service::enqueue_in_session(db, tx, &[intent]).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::security_effect_intent::SecurityEffect;

    #[tokio::test]
    async fn role_scope_maximum_occurrences_fit_backstop() {
        let raw = "x".repeat(17000);
        let actor = AuditActor {
            user_id: raw.clone(),
            ip_address: Some(raw.clone()),
            user_agent: Some(raw.clone()),
            api_key_id: Some(raw.clone()),
            api_key_name: Some(raw.clone()),
        };
        // 5 + 10*(7+1+256) + 20*(7+2+256) + (7+2+238) = 8192.
        let mut ids = vec!["x".repeat(256); 30];
        ids.push("x".repeat(238));
        ids.push("omitted".into());
        let row = OrgRoleScope {
            id: uuid::Uuid::new_v4().to_string(),
            org_user_id: raw,
            role: OrgRole::Member,
            allowed_service_ids: Some(ids),
            updated_at: chrono::Utc::now(),
            updated_by: actor.user_id.clone(),
        };
        let turn = uuid::Uuid::new_v4().to_string();
        for intent in
            crate::turn_trace::scope(crate::turn_trace::TurnContext::for_turn(&turn), async {
                [
                    set_intent(&actor, &row).unwrap(),
                    clear_intent(&actor, &row.org_user_id, row.role).unwrap(),
                ]
            })
            .await
        {
            let SecurityEffect::Audit { event } = intent.effect else {
                panic!("audit")
            };
            let bytes = bson::to_vec(&event).unwrap().len();
            println!(
                "ROLE_SCOPE_MAX {} bytes={bytes} margin={}",
                event.event_type,
                16384 - bytes
            );
            assert!(bytes < 16384);
            if event.event_type == "org_role_scope_set" {
                let data = event.event_data.as_ref().unwrap();
                let ids = data["allowed_service_ids"].as_array().unwrap();
                let encoded = 5 + ids
                    .iter()
                    .enumerate()
                    .map(|(index, id)| 7 + index.to_string().len() + id.as_str().unwrap().len())
                    .sum::<usize>();
                assert_eq!(encoded, required::MAX_IDENTIFIER_LIST_BYTES);
                assert_eq!(data["allowed_service_ids_count"], 32);
            }
        }
    }
}
