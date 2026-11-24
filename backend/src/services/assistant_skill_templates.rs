//! Start a curated template in a private NyxBot thread with one durable opening event.
use mongodb::{Database, bson::doc};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::LazyLock};

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult, skill_read::SkillReadReason},
    models::{
        assistant_agent::{AgentSkillMetadata, AssistantAgent, COLLECTION_NAME as AGENTS},
        assistant_conversation::{
            AssistantConversation, COLLECTION_NAME as THREADS, GallerySkillLaunch,
            GallerySkillState, TurnOrigin,
        },
        catalog_skill_revision::SkillReference,
    },
};

use super::{agent_skill_service as skills, assistant_team_service as team, transaction_service};

pub(crate) const EVENT: &str = "gallery_skill_started";
pub(crate) const NOTICE: &str = "Started a skill from the NyxBot template gallery.";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Template {
    pub skill_id: String,
    pub version: String,
    pub sha256: String,
    title: String,
}

static TEMPLATES: LazyLock<BTreeMap<String, Template>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../shared/nyxbot-skill-templates.json"))
        .expect("checked-in skill template catalog")
});

pub(crate) fn get(slug: &str) -> AppResult<&'static Template> {
    TEMPLATES
        .get(slug)
        .ok_or_else(|| AppError::NotFound("Unknown skill template".into()))
}

pub(crate) fn conversation_id(owner: &str, slug: &str, setup_id: &str) -> AppResult<String> {
    if setup_id.len() != 32
        || !setup_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(AppError::ValidationError(
            "Invalid template setup ID".into(),
        ));
    }
    let id = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("nyxbot-gallery:{owner}:{slug}:{setup_id}").as_bytes(),
    );
    Ok(format!("nyxa-{}", id.simple()))
}

impl Template {
    pub(crate) fn verify(&self, reference: &SkillReference) -> AppResult<()> {
        if reference.source != "ornn"
            || reference.skill_id != self.skill_id
            || reference.version != self.version
            || reference.sha256 != self.sha256
            || reference.disabled
        {
            return Err(AppError::SkillRead(SkillReadReason::PinnedVersionMismatch));
        }
        Ok(())
    }
}

pub(crate) async fn prepare(
    db: &Database,
    keys: &EncryptionKeys,
    actor: &super::audit_service::AuditActor,
    id: &str,
    slug: &str,
    template: &'static Template,
    preview: skills::Preview,
) -> AppResult<()> {
    template.verify(&preview.reference)?;
    let agent = team::ensure_nyxbot(db, &actor.user_id).await?;
    let data = (db, keys, actor, id, slug, template, preview, agent.id);
    transaction_service::run_db_only(db, &data, |session, data| Box::pin(async move {
        let (db, keys, actor, id, slug, template, preview, agent_id) = data;
        let owner = actor.user_id.as_str();
        if db.collection::<AssistantConversation>(THREADS)
            .find_one(doc! {"_id": *id, "user_id": owner})
            .session(&mut *session).await?.is_some() {
            return Ok(());
        }
        let agent = db.collection::<AssistantAgent>(AGENTS)
            .find_one(doc! {"_id": agent_id, "user_id": owner, "kind": "nyxbot", "destroyed_at": null})
            .session(&mut *session).await?
            .ok_or_else(|| AppError::NotFound("Personal NyxBot is unavailable".into()))?;
        match agent.skills.iter().find(|pin| pin.skill_id == preview.reference.skill_id) {
            Some(pin) if pin.disabled => return Err(AppError::Conflict("The template skill is disabled".into())),
            Some(pin) if !matches_pin(pin, &preview.reference) => return Err(AppError::Conflict("NyxBot has a different pin for this template".into())),
            Some(_) => {},
            None => {
                if agent.skills.len() >= 16 {
                    return Err(AppError::Conflict("NyxBot already has 16 skills".into()));
                }
                let mut selection = skills::Selection { expected_revision: agent.skills_revision, skills: agent.skills.clone() };
                selection.skills.push(preview.reference.clone());
                let mut metadata = agent.skill_metadata.clone();
                metadata.insert(preview.reference.skill_id.clone(), AgentSkillMetadata {
                    description: preview.description.clone(),
                    size_bytes: preview.size_bytes,
                    ..Default::default()
                });
                skills::apply_verified_in_session(db, &agent, &selection, &metadata, session).await?;
            }
        }
        let launch = GallerySkillLaunch {
            slug: (*slug).into(),
            reference: preview.reference.clone(),
            state: GallerySkillState::Queued,
        };
        team::create_gallery_thread(db, keys, &agent, id, &template.title, launch, session).await?;
        super::audit_service::log_actor_event_in_transaction(
            db, session, actor, "assistant_gallery_skill_launched",
            serde_json::json!({
                "conversation_id": id, "template": slug, "skill_id": template.skill_id,
                "version": template.version
            }),
        ).await
    })).await
}

fn matches_pin(live: &SkillReference, expected: &SkillReference) -> bool {
    !live.disabled
        && live.source == expected.source
        && live.skill_id == expected.skill_id
        && live.version == expected.version
        && live.sha256 == expected.sha256
        && live.dependencies.len() == expected.dependencies.len()
        && live.dependencies.iter().all(|pin| {
            expected.dependencies.iter().any(|entry| {
                pin.source == entry.source
                    && pin.skill_id == entry.skill_id
                    && pin.version == entry.version
                    && pin.sha256 == entry.sha256
            })
        })
}

pub(crate) fn matches_agent(launch: &GallerySkillLaunch, agent: &AssistantAgent) -> bool {
    get(&launch.slug).is_ok_and(|template| template.verify(&launch.reference).is_ok())
        && agent.destroyed_at.is_none()
        && agent
            .skills
            .iter()
            .any(|pin| matches_pin(pin, &launch.reference))
}

pub(crate) fn queued_opening(row: &AssistantConversation) -> bool {
    row.gallery_skill
        .as_ref()
        .is_some_and(|launch| launch.state == GallerySkillState::Queued)
        && row.pending_events.iter().any(|event| event.kind == EVENT)
}

pub(crate) fn opening_turn(row: &AssistantConversation) -> Option<&GallerySkillLaunch> {
    let turn = row.active_turn.as_ref()?;
    turn.events
        .iter()
        .any(|event| event.kind == EVENT)
        .then_some(())?;
    row.gallery_skill.as_ref()
}

/// Recheck the frozen dependency closure when the opening turn reads its skill.
pub(crate) async fn verify_opening_pin(
    db: &Database,
    id: &str,
    turn_id: &str,
    agent: &AssistantAgent,
    skill: &str,
) -> AppResult<()> {
    if !TEMPLATES
        .values()
        .any(|template| template.skill_id == skill)
        && !agent.skills.iter().any(|pin| {
            pin.name == skill
                && TEMPLATES
                    .values()
                    .any(|template| template.skill_id == pin.skill_id)
        })
    {
        return Ok(());
    }
    let row = db
        .collection::<AssistantConversation>(THREADS)
        .find_one(doc! {
            "_id": id, "user_id": &agent.user_id, "agent_id": &agent.id,
            "active_turn.turn_id": turn_id, "active_turn.events.kind": EVENT,
        })
        .await?;
    if let Some(launch) = row.as_ref().and_then(opening_turn)
        && !matches_agent(launch, agent)
    {
        return Err(AppError::SkillRead(SkillReadReason::PinnedVersionMismatch));
    }
    Ok(())
}

pub(crate) async fn admit(
    db: &Database,
    session: &mut mongodb::ClientSession,
    row: &AssistantConversation,
    origin: TurnOrigin,
) -> AppResult<bool> {
    let Some(launch) = row.gallery_skill.as_ref() else {
        return Ok(false);
    };
    if launch.state != GallerySkillState::Queued
        || origin != TurnOrigin::Event
        || row.channel.is_some()
        || row.platform_thread.is_some()
        || row.group_id.is_some()
        || row.guest_turn
        || row.shared_agent_thread
        || row.agent_owner_id.is_some()
    {
        return Ok(false);
    }
    let agent = db.collection::<AssistantAgent>(AGENTS)
        .find_one(doc! {"_id": &row.agent_id, "user_id": &row.user_id, "kind": "nyxbot", "destroyed_at": null})
        .session(&mut *session).await?;
    Ok(agent.is_some_and(|agent| matches_agent(launch, &agent)))
}

/// Cancel only the queued gallery event. The conditional write races on the
/// same thread as admission. If admission wins, Stop handles the active turn.
pub(crate) async fn cancel_queued(db: &Database, row: &AssistantConversation) -> AppResult<bool> {
    let result = db
        .collection::<AssistantConversation>(THREADS)
        .update_one(
            doc! {"_id": &row.id, "user_id": &row.user_id, "gallery_skill.state": "queued"},
            doc! {"$set": {"gallery_skill.state": "cancelled"},
            "$pull": {"pending_events": {"kind": EVENT}}},
        )
        .await?;
    Ok(result.modified_count == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLUG: &str = "research-paper-writing-rigor";
    const SETUP_ID: &str = "0123456789abcdef0123456789abcdef";

    fn reference(template: &Template) -> SkillReference {
        SkillReference {
            source: "ornn".into(),
            skill_id: template.skill_id.clone(),
            name: SLUG.into(),
            version: template.version.clone(),
            sha256: template.sha256.clone(),
            dependencies: Vec::new(),
            disabled: false,
        }
    }

    #[test]
    fn exact_and_enabled_pin_is_required() {
        let template = get(SLUG).unwrap();
        let expected = reference(template);
        assert!(template.verify(&expected).is_ok());

        let mut updated = expected.clone();
        updated.sha256 = "a".repeat(64);
        assert!(template.verify(&updated).is_err());
        updated = expected.clone();
        updated.version = "99.0".into();
        assert!(template.verify(&updated).is_err());
        updated = expected.clone();
        updated.disabled = true;
        assert!(!matches_pin(&updated, &expected));
        updated = expected.clone();
        updated
            .dependencies
            .push(crate::models::catalog_skill_revision::SkillPin {
                source: "ornn".into(),
                skill_id: "another-skill".into(),
                name: "Another skill".into(),
                version: "1.0".into(),
                sha256: "b".repeat(64),
            });
        assert!(!matches_pin(&updated, &expected));
    }

    #[test]
    fn setup_ids_are_bounded_and_bound_to_account_and_template() {
        let id = conversation_id("owner", SLUG, SETUP_ID).unwrap();
        assert_eq!(id, conversation_id("owner", SLUG, SETUP_ID).unwrap());
        assert_ne!(id, conversation_id("other-owner", SLUG, SETUP_ID).unwrap());
        assert_ne!(
            id,
            conversation_id("owner", "release-readiness-review", SETUP_ID).unwrap()
        );
        assert!(conversation_id("owner", SLUG, &SETUP_ID.to_uppercase()).is_err());
    }

    #[tokio::test]
    async fn simultaneous_starts_create_one_private_thread_and_replay_after_cancel() {
        let fixture =
            crate::services::assistant_authority_tests::orchestrator_fixture("gallery_launch")
                .await;
        let db = &fixture.state.db;
        let actor = super::super::audit_service::AuditActor::from_auth_user(
            &crate::test_utils::test_auth_user(&fixture.owner),
        );
        let id = conversation_id(&fixture.owner, SLUG, SETUP_ID).unwrap();
        let template = get(SLUG).unwrap();
        let prepare_start = || {
            prepare(
                db,
                &fixture.state.encryption_keys,
                &actor,
                &id,
                SLUG,
                template,
                skills::Preview {
                    reference: reference(template),
                    description: "Research writing".into(),
                    size_bytes: 1,
                    skill_md: None,
                    author: None,
                },
            )
        };
        let (first, second) = tokio::join!(prepare_start(), prepare_start());
        assert!(first.is_ok() || second.is_ok());
        prepare_start().await.unwrap();

        let row = super::super::assistant_nyxagent::get(db, &fixture.owner, &id)
            .await
            .unwrap();
        assert_ne!(row.id, fixture.nyxbot_thread);
        assert!(row.channel.is_none());
        assert!(row.active_turn.is_none());
        assert_eq!(row.message_count, 0);
        assert_eq!(row.pending_events.len(), 1);
        assert_eq!(row.pending_events[0].kind, EVENT);
        let agent = team::agent_for_conversation(db, &row).await.unwrap();
        assert!(matches_agent(row.gallery_skill.as_ref().unwrap(), &agent));

        assert!(cancel_queued(db, &row).await.unwrap());
        assert!(!cancel_queued(db, &row).await.unwrap());
        prepare_start().await.unwrap();
        let replay = super::super::assistant_nyxagent::get(db, &fixture.owner, &id)
            .await
            .unwrap();
        assert!(replay.pending_events.is_empty());
        assert_eq!(
            replay.gallery_skill.unwrap().state,
            GallerySkillState::Cancelled
        );
        crate::test_utils::drop_test_database(db).await;
    }
}
