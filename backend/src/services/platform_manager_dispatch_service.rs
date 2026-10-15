//! Verified native ingress chooses one durable audience before any execution.
use mongodb::{ClientSession, Database};
use serde_json::Value;

use super::{
    api_key_mutation_service::{map_transaction_error, transaction_result},
    platform_bot_runtime_control_service as controls,
    platform_manager_contract::*,
    platform_manager_ingress_service as ingress,
};
use crate::errors::{AppError, AppResult};

/// Only the verified native receiver supplies manager identity and observation.
/// Forwarding changes command eligibility, never the authenticated sender.
pub(crate) fn verified_message(
    profile_id: &str,
    manager_bot_id: &str,
    telegram_bot_id: i64,
    observation_id: &str,
    update: &Value,
) -> Option<VerifiedManagerUpdate> {
    if profile_id.is_empty()
        || observation_id.is_empty()
        || manager_bot_id.parse::<i64>().ok() != Some(telegram_bot_id)
        || telegram_bot_id <= 0
    {
        return None;
    }
    // Edited/channel/callback/service messages cannot become new owner input.
    let message = update.get("message")?;
    if message["from"]["is_bot"] != false || message.get("sender_chat").is_some() {
        return None;
    }
    let sender_id = message["from"]["id"].as_i64().filter(|id| *id > 0)?;
    let chat_id = message["chat"]["id"].as_i64().filter(|id| *id != 0)?;
    let chat_kind = match message["chat"]["type"].as_str()? {
        "private" if sender_id == chat_id => ManagerChatKind::Private,
        "group" => ManagerChatKind::Group,
        "supergroup" => ManagerChatKind::Supergroup,
        _ => return None,
    };
    let original = super::telegram_new_service::private_sender(message) == Some(sender_id);
    Some(VerifiedManagerUpdate {
        profile_id: profile_id.into(),
        manager_bot_id: manager_bot_id.into(),
        telegram_bot_id,
        observation_id: observation_id.into(),
        sender_id,
        chat_id,
        chat_kind,
        update_id: update["update_id"].as_i64().filter(|id| *id >= 0)?,
        message_id: message["message_id"].as_i64().filter(|id| *id > 0)?,
        command_eligibility: if original {
            ManagerCommandEligibility::OriginalPrivateMessage
        } else {
            ManagerCommandEligibility::Ineligible
        },
    })
}

pub(crate) async fn admit_in_session(
    db: &Database,
    session: &mut ClientSession,
    identity: &dyn ManagerIdentityProducer,
    update: &VerifiedManagerUpdate,
    claim: &IngressClaim,
) -> AppResult<IngressAdmission> {
    // Deliberately precedes current authority resolution: retries keep history.
    if let Some(receipt) = ingress::find_in_session(db, session, update).await? {
        return Ok(IngressAdmission::Duplicate {
            receipt_id: receipt.id,
        });
    }
    let row = controls::fence_manager_in_session(
        db,
        session,
        &update.profile_id,
        &update.manager_bot_id,
        update.telegram_bot_id,
        Some(&update.observation_id),
    )
    .await?
    .ok_or_else(|| AppError::Conflict("Manager transport is unavailable".into()))?;
    let policy = controls::policy(&row);
    let audience = if row.retired {
        ManagerAudience::Fixed(UnavailableReason::ManagerUnavailable)
    } else if policy.emergency_stopped {
        ManagerAudience::Fixed(UnavailableReason::EmergencyStopped)
    } else if update.chat_kind != ManagerChatKind::Private {
        ManagerAudience::PrivateChatHandoff
    } else {
        match identity.resolve_sender(db, session, update).await? {
            SenderResolution::Personal { authority, .. } => {
                if authority.manager != policy.manager {
                    return Err(AppError::Conflict("Manager authority changed".into()));
                }
                ManagerAudience::Personal(AdmittedPersonalUpdate {
                    ingress: claim.clone(),
                    authority,
                })
            }
            SenderResolution::Unlinked if policy.public_enabled => {
                ManagerAudience::Public(PublicIngress {
                    ingress: claim.clone(),
                    manager: policy.manager.clone(),
                    sender_id: update.sender_id,
                    chat_id: update.chat_id,
                    public_policy_revision: policy.public_policy_revision,
                })
            }
            SenderResolution::Unlinked => ManagerAudience::Fixed(UnavailableReason::PublicDisabled),
            SenderResolution::Unavailable(reason) => ManagerAudience::Fixed(reason),
        }
    };
    ingress::record_admission_in_session(db, session, update, &policy.manager, claim, &audience)
        .await
}

pub async fn admit_manager_update(
    db: &Database,
    identity: &dyn ManagerIdentityProducer,
    update: &VerifiedManagerUpdate,
) -> AppResult<IngressAdmission> {
    let claim = IngressClaim {
        receipt_id: uuid::Uuid::new_v4().to_string(),
        claim_id: uuid::Uuid::new_v4().to_string(),
    };
    let mut session = db.client().start_session().await?;
    let result = session
        .start_transaction()
        .and_run(
            (db, identity, update, claim),
            |session, (db, identity, update, claim)| {
                Box::pin(async move {
                    transaction_result(
                        admit_in_session(db, session, *identity, update, claim).await,
                    )
                })
            },
        )
        .await
        .map_err(map_transaction_error);
    if result.is_err() {
        // Covers duplicate-index races and unknown commit outcomes after abort.
        // Existing durable state is the only acceptable recovery, never dispatch.
        let mut reread = db.client().start_session().await?;
        if let Some(receipt) = ingress::find_in_session(db, &mut reread, update).await? {
            return Ok(IngressAdmission::Duplicate {
                receipt_id: receipt.id,
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::platform_personal_binding_service::{IdentityProducer, tests::fixture};
    use mongodb::bson::doc;

    #[test]
    fn forwarded_text_uses_real_sender_but_never_controls() {
        let mut update = serde_json::json!({"update_id":5,"message":{"message_id":7,"chat":{"id":123,"type":"private"},"from":{"id":123,"is_bot":false},"text":"yes"}});
        let original = verified_message("profile", "99", 99, "obs", &update).unwrap();
        assert_eq!(
            original.command_eligibility,
            ManagerCommandEligibility::OriginalPrivateMessage
        );
        for field in ["forward_origin", "forward_date", "via_bot"] {
            update["message"][field] = serde_json::json!({"id":999});
            let forwarded = verified_message("profile", "99", 99, "obs", &update).unwrap();
            assert_eq!(forwarded.sender_id, 123);
            assert_eq!(
                forwarded.command_eligibility,
                ManagerCommandEligibility::Ineligible
            );
            assert!(
                OriginalManagerCommand::from_verified(forwarded, TurnInput { text: "yes".into() })
                    .is_none()
            );
            update["message"].as_object_mut().unwrap().remove(field);
        }
        update["message"]["sender_chat"] = serde_json::json!({"id":999});
        assert!(verified_message("profile", "99", 99, "obs", &update).is_none());
    }

    #[test]
    fn group_handoff_and_edited_or_foreign_private_input() {
        let mut update = serde_json::json!({"update_id":5,"message":{"message_id":7,"chat":{"id":-123,"type":"group"},"from":{"id":123,"is_bot":false},"text":"/link"}});
        let group = verified_message("profile", "99", 99, "obs", &update).unwrap();
        assert_eq!(group.chat_kind, ManagerChatKind::Group);
        assert_eq!(
            group.command_eligibility,
            ManagerCommandEligibility::Ineligible
        );
        update["message"]["chat"]["type"] = serde_json::json!("private");
        assert!(verified_message("profile", "99", 99, "obs", &update).is_none());
        update["edited_message"] = update["message"].clone();
        update.as_object_mut().unwrap().remove("message");
        assert!(verified_message("profile", "99", 99, "obs", &update).is_none());
    }

    #[tokio::test]
    async fn audience_remains_public_after_link_and_duplicate_does_not_resolve_owner() {
        let (db, update, owner) = fixture("manager_ingress_audience").await;
        ingress::ensure_indexes(&db).await.unwrap();
        db.collection::<mongodb::bson::Document>(
            crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
        )
        .update_one(
            doc! {"platform_config_id":&update.profile_id},
            doc! {"$set":{"public_enabled":true}},
        )
        .await
        .unwrap();
        let first = admit_manager_update(&db, &IdentityProducer, &update)
            .await
            .unwrap();
        let IngressAdmission::Claimed {
            ingress: original,
            audience: ManagerAudience::Public(_),
        } = first
        else {
            panic!("unlinked sender should be public")
        };
        let binding =
            crate::services::platform_personal_binding_service::tests::binding(&update, &owner);
        db.collection::<crate::models::platform_personal_binding::PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        )
        .insert_one(&binding)
        .await
        .unwrap();
        assert!(
            matches!(admit_manager_update(&db,&IdentityProducer,&update).await.unwrap(),IngressAdmission::Duplicate {receipt_id} if receipt_id==original.receipt_id)
        );
        let mut next = update.clone();
        next.update_id += 1;
        assert!(matches!(
            admit_manager_update(&db, &IdentityProducer, &next)
                .await
                .unwrap(),
            IngressAdmission::Claimed {
                audience: ManagerAudience::Personal(_),
                ..
            }
        ));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn paused_personal_never_falls_back_to_public_and_concurrent_receipt_has_one_winner() {
        let (db, update, owner) = fixture("manager_ingress_paused").await;
        ingress::ensure_indexes(&db).await.unwrap();
        let binding =
            crate::services::platform_personal_binding_service::tests::binding(&update, &owner);
        db.collection::<crate::models::platform_personal_binding::PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        )
        .insert_one(&binding)
        .await
        .unwrap();
        db.collection::<mongodb::bson::Document>(
            crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
        )
        .update_one(
            doc! {"platform_config_id":&update.profile_id},
            doc! {"$set":{"personal_enabled":false,"public_enabled":true}},
        )
        .await
        .unwrap();
        let (a, b) = tokio::join!(
            admit_manager_update(&db, &IdentityProducer, &update),
            admit_manager_update(&db, &IdentityProducer, &update)
        );
        let results = [a.unwrap(), b.unwrap()];
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(
                    r,
                    IngressAdmission::Claimed {
                        audience: ManagerAudience::Fixed(UnavailableReason::PersonalDisabled),
                        ..
                    }
                ))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, IngressAdmission::Duplicate { .. }))
                .count(),
            1
        );
        assert_eq!(
            db.collection::<mongodb::bson::Document>(
                crate::models::platform_manager_ingress::COLLECTION_NAME
            )
            .count_documents(doc! {})
            .await
            .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }
}
