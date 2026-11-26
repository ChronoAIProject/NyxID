use crate::{
    errors::AppResult,
    models::{org_membership::OrgMembership, security_effect_intent::SecurityEffectIntent},
    services::{
        audit_service::{
            AuditActor,
            required::{self, TextField},
        },
        security_effect_service,
    },
};
use mongodb::{ClientSession, Database};

pub(super) fn member_intent(
    actor: &AuditActor,
    event: &str,
    membership: &OrgMembership,
) -> AppResult<SecurityEffectIntent> {
    let mut data = serde_json::json!({
        "org_user_id":required::text(TextField::Identifier,&membership.org_user_id),
    });
    if event == "org_member_joined" {
        data["membership_id"] = required::text(TextField::Identifier, &membership.id).into();
    } else {
        data["member_user_id"] =
            required::text(TextField::Identifier, &membership.member_user_id).into();
    }
    if matches!(event, "org_member_added" | "org_member_joined") {
        data["role"] = membership.role.as_str().into();
    }
    required::intent_preserving_actor(actor, event, Some(data))
}
pub(crate) async fn member(
    db: &Database,
    actor: Option<&AuditActor>,
    event: &str,
    membership: &OrgMembership,
    tx: &mut ClientSession,
) -> AppResult<()> {
    if let Some(actor) = actor {
        let intent = member_intent(actor, event, membership)?;
        security_effect_service::enqueue_in_session(db, tx, &[intent]).await?;
    }
    Ok(())
}
pub(super) fn invite_intent(
    actor: &AuditActor,
    event: &str,
    org: &str,
    invite: &str,
) -> AppResult<SecurityEffectIntent> {
    required::intent_preserving_actor(
        actor,
        event,
        Some(serde_json::json!({
            "org_user_id":required::text(TextField::Identifier,org),
            "invite_id":required::text(TextField::Identifier,invite),
        })),
    )
}
pub(crate) async fn invite(
    db: &Database,
    actor: Option<&AuditActor>,
    event: &str,
    org: &str,
    invite: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    if let Some(actor) = actor {
        let intent = invite_intent(actor, event, org, invite)?;
        security_effect_service::enqueue_in_session(db, tx, &[intent]).await?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn fixture_actor() -> AuditActor {
    AuditActor {
        user_id: uuid::Uuid::nil().to_string(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    }
}
