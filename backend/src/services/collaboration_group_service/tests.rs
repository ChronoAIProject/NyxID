use super::identity::load;
use super::membership::{Change, IssuedInvite, Mutation, create, preview_invite};
use super::*;
use crate::{
    models::{
        assistant_agent::AssistantAgent,
        assistant_sharing::{ShareRole, ShareSubject},
    },
    services::{self as svc, org_agent_tests::Fixture},
};
use std::{collections::BTreeMap, sync::atomic::Ordering};

async fn setup(name: &str) -> (Fixture, AssistantAgent, AssistantGroup) {
    let f = Fixture::new(name).await;
    enable(&f).await;
    let agent = specialist(&f, &f.admin, "collab-one").await;
    let group = membership::create_ready(
        &f.state.db,
        &f.admin,
        "Research",
        std::slice::from_ref(&agent.id),
    )
    .await
    .unwrap();
    (f, agent, group)
}
async fn enable(f: &Fixture) {
    svc::shared_agent_service::ensure_indexes(&f.state.db)
        .await
        .unwrap();
    for flag in [
        svc::shared_agent_service::FLAG,
        svc::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
    ] {
        svc::feature_flag_service::set_platform_override(
            &f.state.db,
            flag,
            &svc::feature_flag_service::FlagTarget::Global,
            true,
            &f.admin,
        )
        .await
        .unwrap();
    }
}
async fn specialist(f: &Fixture, owner: &str, name: &str) -> AssistantAgent {
    svc::assistant_team_service::create_specialist(
        &f.state.db,
        &f.state.encryption_keys,
        owner,
        f.request(name),
    )
    .await
    .unwrap()
    .unwrap()
    .0
}
async fn change(f: &Fixture, actor: &str, group: &AssistantGroup, change: Change) -> Mutation {
    membership::apply_ready(
        &f.state.db,
        actor,
        &group.id,
        marker(group).unwrap().membership_revision,
        change,
        true,
    )
    .await
    .unwrap()
}
async fn invite(f: &Fixture, group: &AssistantGroup) -> IssuedInvite {
    change(
        f,
        &group.user_id,
        group,
        Change::Invite {
            intended_user_id: None,
        },
    )
    .await
    .invite
    .unwrap()
}
async fn join(f: &Fixture, group: &AssistantGroup, actor: &str) -> AssistantGroup {
    let invite = invite(f, group).await;
    membership::accept_ready(&f.state.db, actor, &invite.token, true)
        .await
        .unwrap()
}
async fn share(f: &Fixture, agent: &AssistantAgent, actors: &[&str]) {
    let previous = svc::shared_agent_service::policy(&f.state.db, agent)
        .await
        .unwrap();
    let audience = actors
        .iter()
        .map(|actor| svc::shared_agent_service::Audience {
            subject_kind: ShareSubject::Person,
            subject_id: format!("email:{actor}@example.com"),
            role: ShareRole::User,
        })
        .collect();
    svc::shared_agent_service::update(
        &f.state.db,
        &agent.user_id,
        &agent.id,
        svc::shared_agent_service::SharingUpdate {
            expected_version: previous.version,
            enabled: true,
            audience,
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap();
}
async fn admit_test(
    f: &Fixture,
    actor: &str,
    group: &str,
    request: &str,
    agent: &str,
) -> AppResult<requests::Admission> {
    let mut session = f.state.db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .and_run2(async |session| {
            svc::api_key_mutation_service::transaction_result(
                requests::admit_in_session(&f.state.db, actor, group, request, agent, session)
                    .await,
            )
        })
        .await
        .map_err(svc::api_key_mutation_service::map_transaction_error)
}

#[tokio::test]
async fn collaboration_reader_first_classification_and_readiness_preserve_legacy() {
    let (f, agent, group) = setup("collaboration_readers").await;
    use svc::assistant_group_kind::{GroupKind, classify};
    assert_eq!(classify(&group), GroupKind::Collaboration);
    assert!(
        svc::assistant_group_service::get(&f.state.db, &f.admin, &group.id)
            .await
            .is_err()
    );
    assert!(
        svc::org_group_service::get(&f.state.db, &f.admin, &group.id, None)
            .await
            .is_err()
    );
    assert!(
        svc::org_group_service::list(&f.state.db, &f.admin)
            .await
            .unwrap()
            .is_empty()
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::security_replica::COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$pull":{"capabilities":READINESS}})
        .await
        .unwrap();
    assert!(matches!(
        create(
            &f.state.db,
            &f.admin,
            "Not activated",
            std::slice::from_ref(&agent.id)
        )
        .await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    svc::security_readiness_service::ready_test_replica(&f.state.db).await;
    let legacy = svc::assistant_group_service::create(
        &f.state.db,
        &f.admin,
        "Legacy",
        std::slice::from_ref(&agent.id),
        "user",
    )
    .await
    .unwrap();
    assert_eq!(classify(&legacy), GroupKind::Personal);
    let mut doc = bson::to_document(&legacy).unwrap();
    doc.remove("collaboration");
    assert_eq!(
        classify(&bson::from_document(doc).unwrap()),
        GroupKind::Personal
    );
    let mut unknown = group.clone();
    unknown.collaboration.as_mut().unwrap().version = 2;
    assert_eq!(classify(&unknown), GroupKind::Unsupported);
    assert!(marker(&unknown).is_err());
    let mut conflicting = group.clone();
    conflicting.created_by_user_id = Some(f.admin.clone());
    assert_eq!(classify(&conflicting), GroupKind::Collaboration);
    assert!(marker(&conflicting).is_err());
    svc::feature_flag_service::set_platform_override(
        &f.state.db,
        svc::shared_agent_service::FLAG,
        &svc::feature_flag_service::FlagTarget::Global,
        false,
        &f.admin,
    )
    .await
    .unwrap();
    get(&f.state.db, &f.admin, &group.id).await.unwrap();
    assert!(load(&f.state.db, &f.admin, &group.id, None).await.is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_invites_are_hashed_single_use_and_disclose_full_history() {
    let (f, _, group) = setup("collaboration_invite_race").await;
    let invite = invite(&f, &group).await;
    assert!(!format!("{invite:?}").contains(invite.token.as_str()));
    let stored = f
        .state
        .db
        .collection::<bson::Document>(model::INVITES)
        .find_one(doc! {"_id":&invite.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!format!("{stored:?}").contains(invite.token.as_str()));
    assert_eq!(stored.get_str("token_hash").unwrap().len(), 64);
    assert_eq!(
        stored
            .get_datetime("expires_at")
            .unwrap()
            .timestamp_millis()
            - stored
                .get_datetime("created_at")
                .unwrap()
                .timestamp_millis(),
        7 * 24 * 3600 * 1000
    );
    let preview = preview_invite(&f.state.db, &f.outsider, &invite.token)
        .await
        .unwrap();
    assert_eq!(preview.history_disclosure, HISTORY_DISCLOSURE);
    assert!(
        messages(&f.state.db, &f.outsider, &group.id, None)
            .await
            .is_err()
    );
    assert!(
        membership::accept_ready(&f.state.db, &f.outsider, &invite.token, false)
            .await
            .is_err()
    );
    let (a, b) = tokio::join!(
        membership::accept_ready(&f.state.db, &f.outsider, &invite.token, true),
        membership::accept_ready(&f.state.db, &f.viewer, &invite.token, true)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let joined = a.or(b).unwrap();
    assert_eq!(joined.participant_user_ids.len(), 2);
    assert!(
        preview_invite(&f.state.db, &f.member, &invite.token)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_expired_cancelled_and_targeted_invites_fail_closed() {
    let (f, _, group) = setup("collaboration_invite_states").await;
    let invite = change(
        &f,
        &f.admin,
        &group,
        Change::Invite {
            intended_user_id: Some(f.member.clone()),
        },
    )
    .await
    .invite
    .unwrap();
    assert!(
        preview_invite(&f.state.db, &f.outsider, &invite.token)
            .await
            .is_err()
    );
    change(
        &f,
        &f.admin,
        &group,
        Change::CancelInvite {
            invite_id: invite.id.clone(),
        },
    )
    .await;
    assert!(
        membership::accept_ready(&f.state.db, &f.member, &invite.token, true)
            .await
            .is_err()
    );
    let expired = self::invite(&f, &group).await;
    f.state.db.collection::<bson::Document>(model::INVITES).update_one(doc! {"_id":expired.id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
    assert!(
        membership::accept_ready(&f.state.db, &f.member, &expired.token, true)
            .await
            .is_err()
    );
    assert_eq!(
        get(&f.state.db, &f.admin, &group.id)
            .await
            .unwrap()
            .participant_user_ids,
        vec![f.admin.clone()]
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_quick_add_includes_org_viewers_but_grants_no_agent_authority() {
    let (f, agent, group) = setup("collaboration_quickadd").await;
    assert!(
        membership::apply_ready(
            &f.state.db,
            &f.admin,
            &group.id,
            1,
            Change::QuickAdd {
                person_id: f.viewer.clone(),
                history_disclosed: false
            },
            false
        )
        .await
        .is_err()
    );
    assert!(
        membership::apply_ready(
            &f.state.db,
            &f.admin,
            &group.id,
            1,
            Change::QuickAdd {
                person_id: f.outsider.clone(),
                history_disclosed: true
            },
            false
        )
        .await
        .is_err()
    );
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::QuickAdd {
            person_id: f.viewer.clone(),
            history_disclosed: true,
        },
    )
    .await
    .group
    .unwrap();
    assert!(
        load(&f.state.db, &f.viewer, &group.id, None)
            .await
            .unwrap()
            .agent(&agent.id)
            .unwrap()
            .authority()
            .is_none()
    );
    assert!(
        membership::apply_ready(
            &f.state.db,
            &f.viewer,
            &group.id,
            2,
            Change::QuickAdd {
                person_id: f.member.clone(),
                history_disclosed: true
            },
            false
        )
        .await
        .is_err()
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_many(
            doc! {"member_user_id":&f.viewer},
            doc! {"$set":{"revoked_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    get(&f.state.db, &f.viewer, &group.id).await.unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_creator_transfer_requires_acceptance_then_allows_departure() {
    let (f, agent, group) = setup("collaboration_transfer").await;
    let group = join(&f, &group, &f.member).await;
    let bearer = invite(&f, &group).await;
    assert!(
        membership::apply_ready(&f.state.db, &f.admin, &group.id, 2, Change::Leave, false)
            .await
            .is_err()
    );
    change(
        &f,
        &f.admin,
        &group,
        Change::ProposeTransfer {
            person_id: f.member.clone(),
        },
    )
    .await;
    assert!(
        membership::apply_ready(
            &f.state.db,
            &f.admin,
            &group.id,
            2,
            Change::AcceptTransfer,
            false
        )
        .await
        .is_err()
    );
    assert_eq!(
        get(&f.state.db, &f.admin, &group.id).await.unwrap().user_id,
        f.admin
    );
    let group = change(&f, &f.member, &group, Change::AcceptTransfer)
        .await
        .group
        .unwrap();
    assert_eq!(group.user_id, f.member);
    assert!(
        membership::accept_ready(&f.state.db, &f.outsider, &bearer.token, true)
            .await
            .is_err()
    );
    let group = change(&f, &f.admin, &group, Change::Leave)
        .await
        .group
        .unwrap();
    assert_eq!(group.participant_user_ids, vec![f.member.clone()]);
    assert!(get(&f.state.db, &f.admin, &group.id).await.is_err());
    let stored = svc::assistant_team_service::agent(&f.state.db, &f.admin, &agent.id)
        .await
        .unwrap();
    assert_eq!(stored.user_id, f.admin);
    change(&f, &f.member, &group, Change::Leave).await;
    assert!(get(&f.state.db, &f.member, &group.id).await.is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_identity_prioritizes_use_rights_and_refuses_org_guests() {
    let (f, agent, group) = setup("collaboration_identity").await;
    let (org_agent, _) = f.create().await;
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::AddAgent {
            agent_id: org_agent.id.clone(),
            proposed_by: f.admin.clone(),
        },
    )
    .await
    .group
    .unwrap();
    let group = join(&f, &group, &f.member).await;
    let group = join(&f, &group, &f.viewer).await;
    share(&f, &agent, &[&f.member]).await;
    let owner = load(&f.state.db, &f.admin, &group.id, None).await.unwrap();
    assert_eq!(
        owner.agent(&agent.id).unwrap().authority().unwrap().lane,
        model::Lane::Owner
    );
    let member = load(&f.state.db, &f.member, &group.id, None).await.unwrap();
    assert!(member.matches(&f.member, &group.id, None));
    assert!(!member.matches(&f.admin, &group.id, None));
    assert!(!member.matches(&f.member, &group.id, Some("different-request")));
    assert_eq!(
        member.agent(&agent.id).unwrap().authority().unwrap().lane,
        model::Lane::Shared
    );
    assert_eq!(
        member
            .agent(&org_agent.id)
            .unwrap()
            .authority()
            .unwrap()
            .lane,
        model::Lane::OrgMember
    );
    let viewer = load(&f.state.db, &f.viewer, &group.id, None).await.unwrap();
    assert!(viewer.agent(&org_agent.id).unwrap().authority().is_none());
    assert!(viewer.agent(&agent.id).unwrap().authority().is_none());
    assert!(
        membership::apply_ready(
            &f.state.db,
            &f.member,
            &group.id,
            marker(&group).unwrap().membership_revision,
            Change::GuestAccess {
                agent_id: agent.id.clone(),
                enabled: true,
                expected_revision: 1
            },
            false
        )
        .await
        .is_err()
    );
    assert!(
        membership::apply_ready(
            &f.state.db,
            &f.admin,
            &group.id,
            marker(&group).unwrap().membership_revision,
            Change::GuestAccess {
                agent_id: org_agent.id.clone(),
                enabled: true,
                expected_revision: 1
            },
            false
        )
        .await
        .is_err()
    );
    change(
        &f,
        &f.admin,
        &group,
        Change::GuestAccess {
            agent_id: agent.id.clone(),
            enabled: true,
            expected_revision: 1,
        },
    )
    .await;
    assert_eq!(
        load(&f.state.db, &f.member, &group.id, None)
            .await
            .unwrap()
            .agent(&agent.id)
            .unwrap()
            .authority()
            .unwrap()
            .lane,
        model::Lane::Shared
    );
    assert_eq!(
        load(&f.state.db, &f.viewer, &group.id, None)
            .await
            .unwrap()
            .agent(&agent.id)
            .unwrap()
            .authority()
            .unwrap()
            .lane,
        model::Lane::Guest
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_queue_keeps_requesters_and_slots_separate() {
    let (f, agent, group) = setup("collaboration_requesters").await;
    let group = join(&f, &group, &f.member).await;
    share(&f, &agent, &[&f.member]).await;
    let (a, b) = tokio::join!(
        requests::enqueue_ready(&f.state.db, &f.admin, &group.id, "Owner request", None),
        requests::enqueue_ready(&f.state.db, &f.member, &group.id, "Recipient request", None)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.actor_user_id, f.admin);
    assert_eq!(b.actor_user_id, f.member);
    let aa = admit_test(&f, &f.admin, &group.id, &a.id, &agent.id)
        .await
        .unwrap();
    let bb = admit_test(&f, &f.member, &group.id, &b.id, &agent.id)
        .await
        .unwrap();
    assert_ne!(aa.slot().id, bb.slot().id);
    assert_eq!(aa.slot().authority.lane, model::Lane::Owner);
    assert_eq!(bb.slot().authority.lane, model::Lane::Shared);
    assert_eq!(bb.requester(), f.member);
    assert!(
        admit_test(&f, &f.admin, &group.id, &b.id, &agent.id)
            .await
            .is_err()
    );
    let history = messages(&f.state.db, &f.member, &group.id, None)
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert!(history.iter().all(|m| m.collaboration.is_some()));
    assert!(
        svc::assistant_group_service::messages(&f.state.db, &f.admin, &group.id, 100, None)
            .await
            .unwrap()
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_revoked_shared_queue_never_downgrades_to_guest() {
    let (f, agent, group) = setup("collaboration_no_downgrade").await;
    let group = join(&f, &group, &f.member).await;
    change(
        &f,
        &f.admin,
        &group,
        Change::GuestAccess {
            agent_id: agent.id.clone(),
            enabled: true,
            expected_revision: 1,
        },
    )
    .await;
    share(&f, &agent, &[&f.member]).await;
    let request =
        requests::enqueue_ready(&f.state.db, &f.member, &group.id, "Before revocation", None)
            .await
            .unwrap();
    share(&f, &agent, &[]).await;
    assert!(
        admit_test(&f, &f.member, &group.id, &request.id, &agent.id)
            .await
            .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(model::SLOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let request =
        requests::enqueue_ready(&f.state.db, &f.member, &group.id, "New guest request", None)
            .await
            .unwrap();
    let admission = admit_test(&f, &f.member, &group.id, &request.id, &agent.id)
        .await
        .unwrap();
    assert_eq!(admission.slot().authority.lane, model::Lane::Guest);
    assert_eq!(admission.slot().original_requester_id, f.member);
    assert_eq!(admission.slot().authority.owner_id, f.admin);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_unauthorized_recipient_gets_only_fixed_notice() {
    let (f, agent, group) = setup("collaboration_refusal").await;
    let group = join(&f, &group, &f.outsider).await;
    let before = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    let request = requests::enqueue_ready(
        &f.state.db,
        &f.outsider,
        &group.id,
        "Hello",
        Some(std::slice::from_ref(&agent.id)),
    )
    .await
    .unwrap();
    assert!(request.pending_agent_ids.is_empty());
    let history = messages(&f.state.db, &f.outsider, &group.id, None)
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].text, REFUSAL);
    assert_eq!(history[1].agent_id.as_deref(), Some(agent.id.as_str()));
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(model::SLOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_handoffs_keep_original_actor_and_dedupe_budget() {
    let (f, first, group) = setup("collaboration_handoffs").await;
    let second = specialist(&f, &f.member, "second").await;
    let group = join(&f, &group, &f.member).await;
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::AddAgent {
            agent_id: second.id.clone(),
            proposed_by: f.member.clone(),
        },
    )
    .await
    .group
    .unwrap();
    share(&f, &first, &[&f.member]).await;
    let request = requests::enqueue_ready(
        &f.state.db,
        &f.member,
        &group.id,
        "Delegate",
        Some(std::slice::from_ref(&first.id)),
    )
    .await
    .unwrap();
    let source = admit_test(&f, &f.member, &group.id, &request.id, &first.id)
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        requests::handoff_ready(&f.state.db, &source, &second.id),
        requests::handoff_ready(&f.state.db, &source, &second.id)
    );
    assert_ne!(a.unwrap(), b.unwrap());
    let next = admit_test(&f, &f.member, &group.id, &request.id, &second.id)
        .await
        .unwrap();
    assert_eq!(next.requester(), f.member);
    assert_eq!(next.slot().authority.lane, model::Lane::Owner);
    let row = f
        .state
        .db
        .collection::<crate::models::assistant_group::GroupRequest>(
            crate::models::assistant_group::REQUESTS_COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&request.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.hops_remaining, request.hops_remaining - 1);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_removal_revokes_requests_and_live_bindings_atomically() {
    let (f, agent, group) = setup("collaboration_revoke").await;
    let group = join(&f, &group, &f.member).await;
    share(&f, &agent, &[&f.member]).await;
    let request = requests::enqueue_ready(&f.state.db, &f.member, &group.id, "Work", None)
        .await
        .unwrap();
    let admission = admit_test(&f, &f.member, &group.id, &request.id, &agent.id)
        .await
        .unwrap();
    let thread = svc::assistant_team_service::home_thread_for(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &agent,
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<bson::Document>(model::SLOTS)
        .update_one(
            doc! {"_id":&admission.slot().id},
            doc! {"$set":{"conversation_id":&thread.id}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&thread.id},
            doc! {"$set":{"active_turn":{"turn_id":"test-turn","stop_requested":false}}},
        )
        .await
        .unwrap();
    change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await;
    assert!(get(&f.state.db, &f.member, &group.id).await.is_err());
    let chat = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id":&thread.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        chat.get_document("active_turn")
            .unwrap()
            .get_bool("stop_requested")
            .unwrap()
    );
    let key = f
        .state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":&thread.credential_api_key_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!key.get_bool("is_active").unwrap());
    assert!(
        requests::handoff_ready(&f.state.db, &admission, &agent.id)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_snapshot_read_count_is_constant_for_eight_agents() {
    let (f, _, reads) = Fixture::observed("collaboration_query_count").await;
    enable(&f).await;
    let mut agents = vec![];
    for i in 0..8 {
        agents.push(specialist(&f, &f.admin, &format!("collab-{i}")).await.id);
    }
    let single = membership::create_ready(&f.state.db, &f.admin, "Single", &agents[..1])
        .await
        .unwrap();
    let eight = membership::create_ready(&f.state.db, &f.admin, "Eight", &agents)
        .await
        .unwrap();
    reads.store(0, Ordering::SeqCst);
    load(&f.state.db, &f.admin, &single.id, None).await.unwrap();
    let single_reads = reads.load(Ordering::SeqCst);
    reads.store(0, Ordering::SeqCst);
    load(&f.state.db, &f.admin, &eight.id, None).await.unwrap();
    let eight_reads = reads.load(Ordering::SeqCst);
    assert_eq!(single_reads, eight_reads);
    assert!(eight_reads <= 12, "{eight_reads}");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_external_owner_can_only_opt_in_and_remove_readd_resets_guests() {
    let (f, first, group) = setup("collaboration_guest_owner").await;
    let foreign = specialist(&f, &f.outsider, "foreign").await;
    share(&f, &foreign, &[&f.admin]).await;
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::AddAgent {
            agent_id: foreign.id.clone(),
            proposed_by: f.admin.clone(),
        },
    )
    .await
    .group
    .unwrap();
    assert!(!group.participant_user_ids.contains(&f.outsider));
    assert!(get(&f.state.db, &f.outsider, &group.id).await.is_err());
    change(
        &f,
        &f.outsider,
        &group,
        Change::GuestAccess {
            agent_id: foreign.id.clone(),
            enabled: true,
            expected_revision: 1,
        },
    )
    .await;
    assert!(
        messages(&f.state.db, &f.outsider, &group.id, None)
            .await
            .is_err()
    );
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::RemoveAgent {
            agent_id: foreign.id.clone(),
        },
    )
    .await
    .group
    .unwrap();
    assert_eq!(group.member_agent_ids, vec![first.id]);
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::AddAgent {
            agent_id: foreign.id.clone(),
            proposed_by: f.admin.clone(),
        },
    )
    .await
    .group
    .unwrap();
    let settings = f
        .state
        .db
        .collection::<model::AgentAccess>(model::AGENT_ACCESS)
        .find_one(doc! {"group_id":&group.id,"agent_id":&foreign.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!settings.guest_enabled);
    assert!(settings.consenting_owner_id.is_none());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_colliding_handles_refuse_and_exact_ids_preserve_selection() {
    let (f, first, group) = setup("collaboration_names").await;
    let other = specialist(&f, &f.member, &first.name).await;
    share(&f, &other, &[&f.admin]).await;
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::AddAgent {
            agent_id: other.id.clone(),
            proposed_by: f.admin.clone(),
        },
    )
    .await
    .group
    .unwrap();
    assert!(
        requests::enqueue_ready(
            &f.state.db,
            &f.admin,
            &group.id,
            &format!("@{} hello", first.name),
            None
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_group::MESSAGES_COLLECTION_NAME)
            .count_documents(doc! {"group_id":&group.id})
            .await
            .unwrap(),
        0
    );
    let exact = requests::enqueue_ready(
        &f.state.db,
        &f.admin,
        &group.id,
        &format!("@{} hello", other.id),
        None,
    )
    .await
    .unwrap();
    assert_eq!(exact.pending_agent_ids, vec![other.id.clone()]);
    let sticky = requests::enqueue_ready(&f.state.db, &f.admin, &group.id, "Continue", None)
        .await
        .unwrap();
    assert_eq!(sticky.pending_agent_ids, vec![other.id]);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_participant_cap_and_stale_membership_writes_are_atomic() {
    let (f, _, group) = setup("collaboration_cap").await;
    let invite = invite(&f, &group).await;
    let mut people = vec![f.admin.clone()];
    for _ in 0..15 {
        let id = uuid::Uuid::new_v4().to_string();
        f.state
            .db
            .collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &id,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        people.push(id);
    }
    f.state
        .db
        .collection::<bson::Document>(GROUPS)
        .update_one(
            doc! {"_id":&group.id},
            doc! {"$set":{"participant_user_ids":people,"collaboration.membership_revision":2}},
        )
        .await
        .unwrap();
    assert!(
        membership::accept_ready(&f.state.db, &f.member, &invite.token, true)
            .await
            .is_err()
    );
    let saved = f
        .state
        .db
        .collection::<model::Invite>(model::INVITES)
        .find_one(doc! {"_id":&invite.id})
        .await
        .unwrap()
        .unwrap();
    assert!(saved.accepted_at.is_none());
    assert!(matches!(
        membership::apply_ready(
            &f.state.db,
            &f.admin,
            &group.id,
            1,
            Change::Remove {
                person_id: f.member.clone()
            },
            false
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(
        get(&f.state.db, &f.admin, &group.id)
            .await
            .unwrap()
            .participant_user_ids
            .len(),
        16
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_new_participant_sees_retained_group_history_only() {
    let (f, agent, group) = setup("collaboration_history").await;
    requests::enqueue_ready(
        &f.state.db,
        &f.admin,
        &group.id,
        "Visible retained message",
        None,
    )
    .await
    .unwrap();
    let private = svc::assistant_team_service::home_thread_for(
        &f.state.db,
        &f.state.encryption_keys,
        &f.admin,
        &agent,
    )
    .await
    .unwrap();
    let group = join(&f, &group, &f.member).await;
    let history = messages(&f.state.db, &f.member, &group.id, None)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].text, "Visible retained message");
    assert!(
        svc::assistant_nyxagent::get(&f.state.db, &f.member, &private.id)
            .await
            .is_err()
    );
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await
    .group
    .unwrap();
    assert!(
        messages(&f.state.db, &f.member, &group.id, None)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

async fn runtime_start(
    f: &Fixture,
    actor: &str,
    group: &AssistantGroup,
    agent: &AssistantAgent,
) -> crate::services::assistant_nyxagent::OwnerTurnClaim {
    let request = requests::enqueue_ready(
        &f.state.db,
        actor,
        &group.id,
        "Answer this group",
        Some(std::slice::from_ref(&agent.id)),
    )
    .await
    .unwrap();
    runtime::begin_ready(
        &f.state.db,
        &f.state.encryption_keys,
        actor,
        &group.id,
        &request.id,
        &agent.id,
    )
    .await
    .unwrap()
    .claim
}
async fn runtime_key(
    f: &Fixture,
    row: &crate::models::assistant_conversation::AssistantConversation,
) -> crate::models::api_key::ApiKey {
    f.state
        .db
        .collection(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":&row.credential_api_key_id})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn collaboration_runtime_shared_threads_are_private_actor_keys_and_group_memory_only() {
    let (f, mut agent, group) = setup("collaboration_runtime_shared").await;
    agent
        .skills
        .push(crate::models::catalog_skill_revision::SkillReference {
            source: "ornn".into(),
            skill_id: "pinned-id".into(),
            name: "pinned-skill".into(),
            version: "1.0.0".into(),
            sha256: "a".repeat(64),
            dependencies: vec![],
            disabled: false,
        });
    f.state
        .db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .replace_one(doc! {"_id":&agent.id}, &agent)
        .await
        .unwrap();
    let group = join(&f, &group, &f.member).await;
    share(&f, &agent, &[&f.member]).await;
    svc::shared_agent_service::edit_private_note(
        &f.state.db,
        &f.member,
        &agent,
        Some("RECIPIENT PRIVATE SECRET"),
        None,
    )
    .await
    .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME).update_one(doc! {"_id":&agent.id},doc! {"$set":{"memory":[{"id":"private-owner","text":"OWNER SECRET","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}],"grants.account_read":true,"machine_node_ids":[uuid::Uuid::new_v4().to_string()],"saved_login_ids":[uuid::Uuid::new_v4().to_string()]}}).await.unwrap();
    f.state.db.collection::<bson::Document>(crate::models::assistant_sharing::COLLECTION_NAME).update_one(doc! {"_id":&agent.id},doc! {"$set":{"memory":[{"id":"published","text":"TEAM MEMORY","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}]}}).await.unwrap();
    let claim = runtime_start(&f, &f.member, &group, &agent).await;
    let row = &claim.conversation;
    assert_eq!(row.user_id, f.member);
    assert_eq!(row.agent_owner_id.as_deref(), Some(f.admin.as_str()));
    assert!(row.shared_agent_thread);
    assert!(!row.guest_turn);
    assert!(row.group_id.is_none());
    assert!(!svc::assistant_title_service::eligible(row));
    assert_eq!(
        row.collaboration,
        row.active_turn.as_ref().unwrap().collaboration
    );
    assert!(!row.active_turn.as_ref().unwrap().with_nyxbot);
    assert!(row.active_turn.as_ref().unwrap().report_to.is_none());
    let projected = svc::assistant_team_service::agent_for_conversation(&f.state.db, row)
        .await
        .unwrap();
    assert_eq!(
        projected
            .memory
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        vec!["TEAM MEMORY"]
    );
    assert_eq!(projected.skills, agent.skills);
    assert!(projected.machine_node_ids.is_empty());
    assert!(projected.saved_login_ids.is_empty());
    assert!(!projected.grants.account_read);
    for actor in [&f.admin, &f.member, &f.outsider] {
        assert!(
            svc::assistant_nyxagent::get(&f.state.db, actor, &row.id)
                .await
                .is_err()
        );
    }
    let key = runtime_key(&f, row).await;
    assert_eq!(key.user_id, f.member);
    assert!(!key.allow_all_nodes);
    assert_eq!(key.scopes, "proxy");
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    let chat =
        svc::assistant_acknowledgement_service::for_key(&f.state.db, &f.member, Some(&key.id))
            .await
            .unwrap()
            .unwrap();
    assert!(!chat.guest);
    assert_eq!(auth.user_id.to_string(), f.member);
    assert!(chat.org_agent_access.as_ref().unwrap().shared().is_some());
    let payer = svc::billing::owner_resolver::BillingOwnerResolver::new(f.state.db.clone())
        .resolve_for_assistant_execution(
            &f.member,
            &f.admin,
            crate::models::usage_meter::CredentialClass::UserOwned,
            chat.org_agent_access.as_deref(),
        )
        .await
        .unwrap();
    assert_eq!(payer.owner_id, f.member);
    let notes =
        svc::assistant_nyxagent_runtime::turn_notes(&f.state, row, Some(&projected), None).await;
    assert!(!notes.contains("OWNER SECRET"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_runtime_guests_have_owner_keys_but_distinct_requester_slots() {
    let (f, agent, group) = setup("collaboration_runtime_guests").await;
    let group = join(&f, &group, &f.member).await;
    let group = join(&f, &group, &f.viewer).await;
    change(
        &f,
        &f.admin,
        &group,
        Change::GuestAccess {
            agent_id: agent.id.clone(),
            enabled: true,
            expected_revision: 1,
        },
    )
    .await;
    let (first, second) = tokio::join!(
        runtime_start(&f, &f.member, &group, &agent),
        runtime_start(&f, &f.viewer, &group, &agent)
    );
    assert_ne!(first.conversation.id, second.conversation.id);
    assert_ne!(
        first.conversation.credential_api_key_id,
        second.conversation.credential_api_key_id
    );
    for claim in [&first, &second] {
        let row = &claim.conversation;
        assert!(row.guest_turn);
        assert_eq!(row.user_id, f.admin);
        assert!(!row.shared_agent_thread);
        let key = runtime_key(&f, row).await;
        crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .unwrap();
        let chat =
            svc::assistant_acknowledgement_service::for_key(&f.state.db, &f.admin, Some(&key.id))
                .await
                .unwrap()
                .unwrap();
        assert!(chat.guest);
        assert!(!chat.involved());
        assert!(chat.machine_node_ids.is_empty());
        assert!(chat.saved_login_ids.is_empty());
        assert!(
            svc::assistant_team_service::agent_for_conversation(&f.state.db, row)
                .await
                .unwrap()
                .memory
                .is_empty()
        );
    }
    assert_eq!(
        first
            .conversation
            .collaboration
            .as_ref()
            .unwrap()
            .original_requester_id,
        f.member
    );
    assert_eq!(
        second
            .conversation
            .collaboration
            .as_ref()
            .unwrap()
            .original_requester_id,
        f.viewer
    );
    let original = svc::assistant_team_service::agent(&f.state.db, &f.admin, &agent.id)
        .await
        .unwrap();
    assert_ne!(
        original.home_conversation_id.as_deref(),
        Some(first.conversation.id.as_str())
    );
    requests::cancel(
        &f.state.db,
        &f.member,
        &group.id,
        &first
            .conversation
            .collaboration
            .as_ref()
            .unwrap()
            .request_id,
    )
    .await
    .unwrap();
    assert!(
        runtime::validate_key(&f.state.db, &runtime_key(&f, &first.conversation).await)
            .await
            .is_err()
    );
    runtime::validate_key(&f.state.db, &runtime_key(&f, &second.conversation).await)
        .await
        .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_guest_enable_requires_delegation_approval_in_the_write() {
    let (f, agent, group) = setup("collaboration_guest_delegation").await;
    let active = runtime_start(&f, &f.admin, &group, &agent).await;
    let guest_change = |enabled, expected_revision| Change::GuestAccess {
        agent_id: agent.id.clone(),
        enabled,
        expected_revision,
    };
    let result = membership::apply_ready(
        &f.state.db,
        &f.admin,
        &group.id,
        marker(&group).unwrap().membership_revision,
        guest_change(true, 1),
        false,
    )
    .await;
    assert!(matches!(result, Err(AppError::Forbidden(_))));
    let policy = f
        .state
        .db
        .collection::<model::AgentAccess>(model::AGENT_ACCESS)
        .find_one(doc! {"group_id":&group.id,"agent_id":&agent.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!policy.guest_enabled);
    assert_eq!(policy.revision, 1);
    runtime::validate_key(&f.state.db, &runtime_key(&f, &active.conversation).await)
        .await
        .unwrap();

    change(&f, &f.admin, &group, guest_change(true, 1)).await;
    // No-op and disabling need no widening approval. A stale approved write
    // still cannot replace a newer owner decision.
    membership::apply_ready(
        &f.state.db,
        &f.admin,
        &group.id,
        marker(&group).unwrap().membership_revision,
        guest_change(true, 2),
        false,
    )
    .await
    .unwrap();
    membership::apply_ready(
        &f.state.db,
        &f.admin,
        &group.id,
        marker(&group).unwrap().membership_revision,
        guest_change(false, 2),
        false,
    )
    .await
    .unwrap();
    assert!(matches!(
        membership::apply_ready(
            &f.state.db,
            &f.admin,
            &group.id,
            marker(&group).unwrap().membership_revision,
            guest_change(true, 2),
            true
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let policy = f
        .state
        .db
        .collection::<model::AgentAccess>(model::AGENT_ACCESS)
        .find_one(doc! {"group_id":&group.id,"agent_id":&agent.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!policy.guest_enabled);
    assert_eq!(policy.revision, 3);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_runtime_handoff_keeps_requester_and_reads_published_reply() {
    let (f, target, group) = setup("collaboration_runtime_handoff").await;
    let group = join(&f, &group, &f.member).await;
    let source = specialist(&f, &f.member, "requester-specialist").await;
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::AddAgent {
            agent_id: source.id.clone(),
            proposed_by: f.member.clone(),
        },
    )
    .await
    .group
    .unwrap();
    share(&f, &target, &[&f.member]).await;
    let request = requests::enqueue_ready(
        &f.state.db,
        &f.member,
        &group.id,
        "Compare the planets",
        Some(std::slice::from_ref(&source.id)),
    )
    .await
    .unwrap();
    let started = runtime::begin_ready(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &group.id,
        &request.id,
        &source.id,
    )
    .await
    .unwrap();
    let source_thread = &started.claim.conversation;
    // A busy group may move the original trigger outside the recent context
    // window. Other people's messages must not become this chain's request.
    for i in 0..64 {
        let unrelated = requests::enqueue_ready(
            &f.state.db,
            &f.admin,
            &group.id,
            &format!(
                "Another participant's unrelated request {i}: {}",
                "x".repeat(1600)
            ),
            Some(std::slice::from_ref(&target.id)),
        )
        .await
        .unwrap();
        // Keep the published context while respecting the bounded work queue.
        requests::cancel(&f.state.db, &f.admin, &group.id, &unrelated.id)
            .await
            .unwrap();
    }
    svc::assistant_nyxagent::finish_turn(
        &f.state.db,
        source_thread,
        &source_thread.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &svc::assistant_nyxagent::TurnResult {
            text: format!(
                "@{} Continue from this group-visible finding: Neptune.",
                target.id
            ),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    assert!(
        requests::handoff_ready(&f.state.db, &started.admission, &target.id)
            .await
            .unwrap()
    );
    let next = runtime::begin_ready(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &group.id,
        &request.id,
        &target.id,
    )
    .await
    .unwrap();
    let target_thread = &next.claim.conversation;
    assert_eq!(target_thread.user_id, f.member);
    assert!(target_thread.shared_agent_thread);
    assert!(!target_thread.guest_turn);
    assert_ne!(
        target_thread.credential_api_key_id,
        source_thread.credential_api_key_id
    );
    let binding = target_thread.collaboration.as_ref().unwrap();
    assert_eq!(binding.original_requester_id, f.member);
    assert_eq!(binding.request_id, request.id);
    assert_eq!(binding.authority.lane, model::Lane::Shared);
    let prompt = f
        .state
        .db
        .collection::<crate::models::assistant_message::AssistantMessage>(
            crate::models::assistant_message::COLLECTION_NAME,
        )
        .find_one(doc! {"conversation_id":&target_thread.id,"role":"group"})
        .await
        .unwrap()
        .unwrap();
    assert!(prompt.text.contains("Neptune."));
    assert!(
        prompt
            .text
            .contains("Original request:\nCompare the planets")
    );
    runtime::validate_key(&f.state.db, &runtime_key(&f, target_thread).await)
        .await
        .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_runtime_settlement_publishes_only_final_text_once() {
    let (f, agent, group) = setup("collaboration_runtime_settle").await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&agent.id},
            doc! {"$set":{"home_conversation_id":null}},
        )
        .await
        .unwrap();
    let claim = runtime_start(&f, &f.admin, &group, &agent).await;
    let row = &claim.conversation;
    let result = svc::assistant_nyxagent::TurnResult {
        text: "Group answer".into(),
        session_id: None,
        response_id: None,
        error: None,
    };
    svc::assistant_nyxagent::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &result,
    )
    .await
    .unwrap();
    let history = messages(&f.state.db, &f.admin, &group.id, None)
        .await
        .unwrap();
    assert!(
        svc::assistant_team_service::agent(&f.state.db, &f.admin, &agent.id)
            .await
            .unwrap()
            .home_conversation_id
            .is_none()
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_checkpoint::COLLECTION_NAME)
            .count_documents(doc! {"_id":&row.id})
            .await
            .unwrap(),
        0
    );
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].text, "Group answer");
    assert!(history[1].activities.is_empty());
    assert!(history[1].attachments.is_empty());
    assert_eq!(
        history[1]
            .collaboration
            .as_ref()
            .unwrap()
            .execution
            .as_ref()
            .unwrap()
            .lane,
        model::Lane::Owner
    );
    assert!(
        svc::assistant_nyxagent::finish_turn(
            &f.state.db,
            row,
            &row.credential_api_key_id,
            &uuid::Uuid::new_v4().to_string(),
            &result
        )
        .await
        .is_err()
    );
    assert_eq!(
        messages(&f.state.db, &f.admin, &group.id, None)
            .await
            .unwrap()
            .len(),
        2
    );
    let again = runtime_start(&f, &f.admin, &group, &agent).await;
    assert_eq!(again.conversation.id, row.id);
    assert_ne!(again.conversation.collaboration, row.collaboration);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_runtime_revocation_fences_keys_and_resharing_gets_a_new_session() {
    let (f, agent, group) = setup("collaboration_runtime_revoke").await;
    let group = join(&f, &group, &f.member).await;
    share(&f, &agent, &[&f.member]).await;
    let claim = runtime_start(&f, &f.member, &group, &agent).await;
    let old = &claim.conversation;
    let key = runtime_key(&f, old).await;
    share(&f, &agent, &[]).await;
    assert!(
        crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .is_err()
    );
    let result = svc::assistant_nyxagent::TurnResult {
        text: "Do not publish after revocation".into(),
        session_id: None,
        response_id: None,
        error: None,
    };
    svc::assistant_nyxagent::finish_turn(
        &f.state.db,
        old,
        &old.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &result,
    )
    .await
    .unwrap();
    assert_eq!(
        messages(&f.state.db, &f.admin, &group.id, None)
            .await
            .unwrap()
            .len(),
        1
    );
    share(&f, &agent, &[&f.member]).await;
    let fresh = runtime_start(&f, &f.member, &group, &agent).await;
    assert_ne!(fresh.conversation.id, old.id);
    assert_ne!(
        fresh.conversation.credential_api_key_id,
        old.credential_api_key_id
    );
    assert!(fresh.conversation.nyxagent_session_id.is_none());
    assert!(
        crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_runtime_org_billing_keeps_org_byok_and_person_platform() {
    let (f, _, group) = setup("collaboration_runtime_org").await;
    let (agent, _) = f.create().await;
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::AddAgent {
            agent_id: agent.id.clone(),
            proposed_by: f.admin.clone(),
        },
    )
    .await
    .group
    .unwrap();
    let group = join(&f, &group, &f.member).await;
    let claim = runtime_start(&f, &f.member, &group, &agent).await;
    let row = &claim.conversation;
    assert_eq!(row.user_id, f.member);
    assert!(!row.shared_agent_thread);
    assert!(!row.guest_turn);
    let key = runtime_key(&f, row).await;
    crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    let chat =
        svc::assistant_acknowledgement_service::for_key(&f.state.db, &f.member, Some(&key.id))
            .await
            .unwrap()
            .unwrap();
    let resolver = svc::billing::owner_resolver::BillingOwnerResolver::new(f.state.db.clone());
    for (class, expected) in [
        (
            crate::models::usage_meter::CredentialClass::UserOwned,
            &f.org,
        ),
        (
            crate::models::usage_meter::CredentialClass::NyxidManagedMaster,
            &f.member,
        ),
    ] {
        let payer = resolver
            .resolve_for_assistant_execution(
                &f.member,
                &f.org,
                class,
                chat.org_agent_access.as_deref(),
            )
            .await
            .unwrap();
        assert_eq!(&payer.owner_id, expected);
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_runtime_native_tools_cannot_write_private_memory_but_keep_pins() {
    let (f, agent, group) = setup("collaboration_native_scope").await;
    let group = join(&f, &group, &f.member).await;
    share(&f, &agent, &[&f.member]).await;
    for actor in [&f.admin, &f.member] {
        let claim = runtime_start(&f, actor, &group, &agent).await;
        let row = &claim.conversation;
        let chat = svc::assistant_acknowledgement_service::for_key(
            &f.state.db,
            actor,
            Some(&row.credential_api_key_id),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(chat.collaboration_thread);
        let native = svc::assistant_account_tools::virtual_service_for(&f.state.db, &chat).await;
        assert_eq!(
            native
                .endpoints
                .iter()
                .map(|e| e.name.as_str())
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from(["get_agent_skills", "skill_read"])
        );
        let account = svc::assistant_acknowledgement_service::account_gate(&f.state.db, &chat)
            .await
            .unwrap()
            .unwrap();
        assert!(account.1.is_none());
        for name in [
            "remember",
            "forget",
            "draft_agent_skill",
            "set_agent_skills",
            "request_agent_access",
            "list_channel_chats",
            "post_to_chat",
        ] {
            let (result, error) = crate::handlers::assistant_team::execute_tool(
                &f.state,
                &chat,
                &format!("nyxid__{name}"),
                &serde_json::json!({"agent":&agent.id,"text":"GROUP SECRET","note_id":"private"}),
            )
            .await;
            assert!(error, "{name}: {result}");
            assert_eq!(result["error"], "collaboration_scope");
        }
        let (skills, error) = crate::handlers::assistant_team::execute_tool(
            &f.state,
            &chat,
            "nyxid__get_agent_skills",
            &serde_json::json!({"agent":&agent.id}),
        )
        .await;
        assert!(!error, "{skills}");
    }
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_sharing::NOTES_COLLECTION_NAME)
            .count_documents(doc! {"agent_id":&agent.id})
            .await
            .unwrap(),
        0
    );
    assert!(
        svc::assistant_team_service::agent(&f.state.db, &f.admin, &agent.id)
            .await
            .unwrap()
            .memory
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_private_transport_binding_cannot_escape_the_request_chain() {
    let (f, agent, group) = setup("collaboration_transport_boundary").await;
    share(&f, &agent, &[&f.member]).await;
    let group = join(&f, &group, &f.member).await;
    let claim = runtime_start(&f, &f.member, &group, &agent).await;
    let row = &claim.conversation;
    let key = runtime_key(&f, row).await;
    let access = svc::org_agent_service::resolve_credential_access(&f.state.db, &key)
        .await
        .unwrap()
        .unwrap();
    let shared = access.shared().unwrap();
    // The private transport receipt has no original-requester/audience binding.
    // Even a valid shared key cannot borrow it for a collaboration slot.
    let mut receipt = crate::models::assistant_sharing::SharedExecution {
        voice: false,
        model: false,
        actor: f.member.clone(),
        owner: agent.user_id.clone(),
        agent: agent.id.clone(),
        conversation: row.id.clone(),
        key: key.id,
        turn: row.active_turn.as_ref().unwrap().turn_id.clone(),
        connection: "must-not-resolve-a-connection".into(),
        revision: shared.revision(),
        behavior: svc::shared_agent_consent::behavior(shared),
        binding: "unused".into(),
        consent_id: None,
        consent_claim: None,
    };
    for voice in [false, true] {
        receipt.voice = voice;
        assert!(matches!(
            svc::shared_agent_transport::revalidate(&f.state.db, &receipt).await,
            Err(AppError::Forbidden(message)) if message.contains("collaboration request binding")
        ));
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_runtime_cancel_is_per_requester_and_departure_stops_delivery() {
    let (f, agent, group) = setup("collaboration_runtime_cancel").await;
    let group = join(&f, &group, &f.member).await;
    share(&f, &agent, &[&f.member]).await;
    let owner = runtime_start(&f, &f.admin, &group, &agent).await;
    let recipient = runtime_start(&f, &f.member, &group, &agent).await;
    let own_request = &owner
        .conversation
        .collaboration
        .as_ref()
        .unwrap()
        .request_id;
    assert!(
        requests::cancel(&f.state.db, &f.member, &group.id, own_request)
            .await
            .is_err()
    );
    requests::cancel(&f.state.db, &f.admin, &group.id, own_request)
        .await
        .unwrap();
    runtime::validate_key(&f.state.db, &runtime_key(&f, &owner.conversation).await)
        .await
        .unwrap_err();
    runtime::validate_key(&f.state.db, &runtime_key(&f, &recipient.conversation).await)
        .await
        .unwrap();
    let group = change(&f, &f.member, &group, Change::Leave)
        .await
        .group
        .unwrap();
    assert!(!group.participant_user_ids.contains(&f.member));
    runtime::validate_key(&f.state.db, &runtime_key(&f, &recipient.conversation).await)
        .await
        .unwrap_err();
    let result = svc::assistant_nyxagent::TurnResult {
        text: "Late private result".into(),
        session_id: None,
        response_id: None,
        error: None,
    };
    svc::assistant_nyxagent::finish_turn(
        &f.state.db,
        &recipient.conversation,
        &recipient.conversation.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &result,
    )
    .await
    .unwrap();
    assert!(
        messages(&f.state.db, &f.admin, &group.id, None)
            .await
            .unwrap()
            .iter()
            .all(|m| m.text != "Late private result")
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_invite_cancellation_races_acceptance_without_double_outcome() {
    let (f, _, group) = setup("collaboration_invite_cancel_race").await;
    let invite = invite(&f, &group).await;
    let (cancel, accept) = tokio::join!(
        membership::apply_ready(
            &f.state.db,
            &f.admin,
            &group.id,
            marker(&group).unwrap().membership_revision,
            Change::CancelInvite {
                invite_id: invite.id.clone()
            },
            false
        ),
        membership::accept_ready(&f.state.db, &f.member, &invite.token, true)
    );
    let stored = f
        .state
        .db
        .collection::<model::Invite>(model::INVITES)
        .find_one(doc! {"_id":&invite.id})
        .await
        .unwrap()
        .unwrap();
    let current = get(&f.state.db, &f.admin, &group.id).await.unwrap();
    if accept.is_ok() {
        assert!(cancel.is_err());
        assert!(stored.cancelled_at.is_none());
        assert!(stored.accepted_at.is_some());
        assert!(current.participant_user_ids.contains(&f.member));
    } else {
        cancel.unwrap();
        assert!(stored.cancelled_at.is_some());
        assert!(stored.accepted_at.is_none());
        assert!(!current.participant_user_ids.contains(&f.member));
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_runtime_busy_slot_keeps_pending_request_and_original_key_binding() {
    let (f, agent, group) = setup("collaboration_runtime_busy").await;
    let first = runtime_start(&f, &f.admin, &group, &agent).await;
    let next = requests::enqueue_ready(
        &f.state.db,
        &f.admin,
        &group.id,
        "Next request",
        Some(std::slice::from_ref(&agent.id)),
    )
    .await
    .unwrap();
    assert!(matches!(
        runtime::begin_ready(
            &f.state.db,
            &f.state.encryption_keys,
            &f.admin,
            &group.id,
            &next.id,
            &agent.id
        )
        .await,
        Err(AppError::AssistantTurnActive)
    ));
    let saved = f
        .state
        .db
        .collection::<crate::models::assistant_group::GroupRequest>(
            crate::models::assistant_group::REQUESTS_COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&next.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.pending_agent_ids, vec![agent.id.clone()]);
    assert!(saved.collaboration.unwrap().admitted_agent_ids.is_empty());
    let thread = f
        .state
        .db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&first.conversation.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(thread.collaboration, first.conversation.collaboration);
    assert_eq!(
        thread.credential_api_key_id,
        first.conversation.credential_api_key_id
    );
    runtime::validate_key(&f.state.db, &runtime_key(&f, &thread).await)
        .await
        .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_requests_are_invisible_to_legacy_queue_mutations() {
    let (f, agent, group) = setup("collaboration_legacy_queue").await;
    let request = requests::enqueue_ready(
        &f.state.db,
        &f.admin,
        &group.id,
        "A new group request",
        None,
    )
    .await
    .unwrap();
    assert!(
        svc::org_group_service::pending_requests(&f.state.db, &group.id)
            .await
            .unwrap()
            .is_empty()
    );
    svc::org_group_service::remove_pending_agent(&f.state.db, &request.id, &agent.id)
        .await
        .unwrap();
    svc::org_group_service::queue_handoff(
        &f.state.db,
        &request.id,
        &uuid::Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    assert!(
        svc::org_group_service::drop_request(&f.state.db, &group, &request.id, &f.admin)
            .await
            .is_err()
    );
    let current = f
        .state
        .db
        .collection::<crate::models::assistant_group::GroupRequest>(
            crate::models::assistant_group::REQUESTS_COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&request.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.pending_agent_ids, request.pending_agent_ids);
    assert_eq!(current.hops_remaining, request.hops_remaining);
    assert_eq!(
        messages(&f.state.db, &f.admin, &group.id, None)
            .await
            .unwrap()
            .len(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_http_projection_hides_threads_cards_and_other_request_ids() {
    let (f, agent, group) = setup("collaboration_http_privacy").await;
    share(&f, &agent, &[&f.member]).await;
    let group = join(&f, &group, &f.member).await;
    let claim = runtime_start(&f, &f.member, &group, &agent).await;
    let thread = &claim.conversation;
    let public = crate::handlers::collaboration_group::response(&f.state, &f.admin, group.clone())
        .await
        .unwrap();
    let public = serde_json::to_value(public).unwrap();
    assert!(!public.to_string().contains(&thread.id));
    assert!(!public.to_string().contains(&thread.credential_api_key_id));
    assert_eq!(
        public["collaboration"]["your_request_ids"],
        serde_json::json!([])
    );
    assert_eq!(public["participants"].as_array().unwrap().len(), 2);
    assert!(
        crate::handlers::collaboration_group::response(&f.state, &f.outsider, group.clone())
            .await
            .is_err()
    );
    assert!(
        runtime::actor_thread(&f.state.db, &f.admin, &group.id, &thread.id)
            .await
            .is_err()
    );
    let binding = thread.collaboration.as_ref().unwrap();
    assert!(
        requests::cancel(&f.state.db, &f.admin, &group.id, &binding.request_id)
            .await
            .is_err()
    );
    assert!(
        svc::assistant_nyxagent::get(&f.state.db, &f.member, &thread.id)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_participant_proposals_bind_proposer_and_creator_acceptance() {
    let (f, _agent, group) = setup("collaboration_proposals").await;
    let group = join(&f, &group, &f.member).await;
    let specialist = specialist(&f, &f.member, "member-specialist").await;
    change(
        &f,
        &f.member,
        &group,
        Change::ProposeAgent {
            agent_id: specialist.id.clone(),
        },
    )
    .await;
    assert_eq!(
        get(&f.state.db, &f.admin, &group.id)
            .await
            .unwrap()
            .member_agent_ids
            .len(),
        1
    );
    let proposal = f
        .state
        .db
        .collection::<model::AgentProposal>(model::PROPOSALS)
        .find_one(doc! {"group_id":&group.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(proposal.proposed_by, f.member);
    assert!(
        membership::apply_ready(
            &f.state.db,
            &f.member,
            &group.id,
            2,
            Change::AcceptAgent {
                proposal_id: proposal.id.clone()
            },
            true
        )
        .await
        .is_err()
    );
    let result = change(
        &f,
        &f.admin,
        &group,
        Change::AcceptAgent {
            proposal_id: proposal.id,
        },
    )
    .await
    .group
    .unwrap();
    assert!(result.member_agent_ids.contains(&specialist.id));
    let policy = f
        .state
        .db
        .collection::<model::AgentAccess>(model::AGENT_ACCESS)
        .find_one(doc! {"group_id":&group.id,"agent_id":&specialist.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(policy.added_by_user_id, f.member);
    assert!(!policy.guest_enabled);
    assert!(
        load(&f.state.db, &f.admin, &group.id, None)
            .await
            .unwrap()
            .agent(&specialist.id)
            .unwrap()
            .authority()
            .is_none()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_http_writes_reject_non_human_before_reading_groups() {
    use crate::handlers::collaboration_group as http;
    use axum::{
        Json,
        extract::{Path, State},
    };
    let state = crate::test_utils::test_app_state_no_db().await;
    for kind in ["key", "oauth", "delegated"] {
        let mut auth = crate::test_utils::test_auth_user("12345678-1234-4123-8123-123456789012");
        match kind {
            "key" => auth.api_key_id = Some("key".into()),
            "oauth" => auth.oauth_client_id = Some("client".into()),
            _ => auth.acting_client_id = Some("client".into()),
        }
        assert!(
            http::membership(
                State(state.clone()),
                auth.clone(),
                Path("unknown".into()),
                Json(http::MembershipInput {
                    expected_revision: 1,
                    operation: http::Operation::Invite {
                        intended_user_id: None
                    }
                })
            )
            .await
            .is_err()
        );
        assert!(
            http::accept_invite(
                State(state.clone()),
                auth.clone(),
                Json(http::InviteInput {
                    token: "secret".into(),
                    history_disclosed: true
                })
            )
            .await
            .is_err()
        );
        assert!(
            http::cancel(
                State(state.clone()),
                auth.clone(),
                Path(("unknown".into(), "request".into()))
            )
            .await
            .is_err()
        );
        assert!(
            crate::handlers::assistant_feedback::put_group(
                State(state.clone()),
                auth.clone(),
                Path(("unknown".into(), "message".into())),
                Json(crate::handlers::assistant_feedback::FeedbackInput {
                    vote: "up".into(),
                    comment: None
                })
            )
            .await
            .is_err()
        );
        assert!(
            crate::handlers::assistant_feedback::delete_group(
                State(state.clone()),
                auth,
                Path(("unknown".into(), "message".into()))
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn collaboration_cleanup_waits_for_settlement_then_removes_requester_artifacts() {
    let (f, agent, group) = setup("collaboration_cleanup").await;
    share(&f, &agent, &[&f.member]).await;
    let group = join(&f, &group, &f.member).await;
    let claim = runtime_start(&f, &f.member, &group, &agent).await;
    change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await;
    cleanup::sweep(&f.state.db).await.unwrap();
    let row = f
        .state
        .db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&claim.conversation.id})
        .await
        .unwrap()
        .unwrap();
    assert!(row.active_turn.as_ref().unwrap().stop_requested);
    svc::assistant_nyxagent::finish_turn(
        &f.state.db,
        &row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &svc::assistant_nyxagent::TurnResult {
            text: "Late private reply".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    cleanup::sweep(&f.state.db).await.unwrap();
    for (collection, filter) in [
        (
            crate::models::assistant_conversation::COLLECTION_NAME,
            doc! {"_id":&row.id},
        ),
        (
            crate::models::assistant_agent_credential::COLLECTION_NAME,
            doc! {"conversation_id":&row.id},
        ),
        (
            crate::models::assistant_message::COLLECTION_NAME,
            doc! {"conversation_id":&row.id},
        ),
    ] {
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(filter)
                .await
                .unwrap(),
            0
        );
    }
    let history = messages(&f.state.db, &f.admin, &group.id, None)
        .await
        .unwrap();
    assert!(!history.iter().any(|m| m.text == "Late private reply"));
    assert!(history.iter().any(|m| m.role == "user"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_live_notifications_are_identifiers_for_current_people_only() {
    let (f, _agent, group) = setup("collaboration_live_delivery").await;
    let group = join(&f, &group, &f.member).await;
    let hub = svc::assistant_live::AssistantLive::new();
    let mut owner = hub.subscribe_owner(&f.admin).unwrap();
    let mut member = hub.subscribe_owner(&f.member).unwrap();
    let mut stranger = hub.subscribe_owner(&f.outsider).unwrap();
    hub.publish_resolved(
        &f.state.db,
        svc::assistant_live::LiveEvent::OrgGroup {
            id: group.id.clone(),
            user_id: String::new(),
        },
    )
    .await;
    assert!(
        matches!(owner.events.try_recv().unwrap(),svc::assistant_live::LiveEvent::OrgGroup{id,..} if id==group.id)
    );
    assert!(
        matches!(member.events.try_recv().unwrap(),svc::assistant_live::LiveEvent::OrgGroup{id,..} if id==group.id)
    );
    assert!(stranger.events.try_recv().is_err());
    change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await;
    hub.publish_resolved(
        &f.state.db,
        svc::assistant_live::LiveEvent::OrgGroup {
            id: group.id.clone(),
            user_id: String::new(),
        },
    )
    .await;
    assert!(member.events.try_recv().is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_actor_cards_are_private_live_fenced_and_resume_exact_request() {
    use svc::assistant_acknowledgement_service as acks;
    let (f, agent, group) = setup("collaboration_cards").await;
    share(&f, &agent, &[&f.member]).await;
    let group = join(&f, &group, &f.member).await;
    let claim = runtime_start(&f, &f.member, &group, &agent).await;
    let row = &claim.conversation;
    let chat = acks::for_key(&f.state.db, &f.member, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let args = serde_json::json!({"method":"POST","path":"/private-account-operation"});
    let ack = acks::request(
        &f.state.db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("nyx__call_tool"),
            arguments: Some(&args),
            summary: "Private exact account action",
            platform: false,
        },
    )
    .await
    .unwrap();
    let owner = crate::handlers::collaboration_group::messages(&f.state, &f.admin, &group.id, None)
        .await
        .unwrap();
    assert!(!owner.to_string().contains("Private exact account action"));
    assert!(!owner.to_string().contains(&ack.id));
    assert!(!owner.to_string().contains(&row.id));
    assert_eq!(owner["collaboration_decisions"][0]["can_decide"], false);
    let own = crate::handlers::collaboration_group::messages(&f.state, &f.member, &group.id, None)
        .await
        .unwrap();
    assert_eq!(own["collaboration_decisions"][0]["id"], ack.id);
    assert!(!own.to_string().contains(&row.id));
    assert!(
        acks::decide(&f.state.db, &f.admin, &row.id, &ack.id, true)
            .await
            .is_err()
    );
    acks::decide(&f.state.db, &f.member, &row.id, &ack.id, true)
        .await
        .unwrap();
    let queued = requests::pending(&f.state.db, Some(&group.id))
        .await
        .unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id, row.collaboration.as_ref().unwrap().request_id);
    assert_eq!(queued[0].actor_user_id, f.member);
    assert_eq!(queued[0].pending_agent_ids, vec![agent.id.clone()]);
    svc::assistant_nyxagent::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &svc::assistant_nyxagent::TurnResult {
            text: "Waiting for your decision".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let resumed = runtime::begin_ready(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &group.id,
        &queued[0].id,
        &agent.id,
    )
    .await
    .unwrap();
    assert_eq!(resumed.claim.conversation.id, row.id);
    let chat = acks::for_key(
        &f.state.db,
        &f.member,
        Some(&resumed.claim.conversation.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        acks::consume_action(&f.state.db, &chat, &ack.id, "nyx__call_tool", &args)
            .await
            .unwrap()
    );
    let args = serde_json::json!({"method":"POST","path":"/another-private-operation"});
    let ack = acks::request(
        &f.state.db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("nyx__call_tool"),
            arguments: Some(&args),
            summary: "Another private action",
            platform: false,
        },
    )
    .await
    .unwrap();
    acks::decide(&f.state.db, &f.member, &row.id, &ack.id, true)
        .await
        .unwrap();
    change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await;
    assert!(
        !acks::consume_action(&f.state.db, &chat, &ack.id, "nyx__call_tool", &args)
            .await
            .unwrap_or(false)
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_uploads_are_published_to_participants_but_model_reads_bind_request() {
    let (f, agent, group) = setup("collaboration_upload_scope").await;
    share(&f, &agent, &[&f.member]).await;
    let group = join(&f, &group, &f.member).await;
    let upload = svc::assistant_upload_service::upload(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &group.id,
        "report.txt",
        b"Group report".to_vec(),
    )
    .await
    .unwrap();
    assert!(
        svc::assistant_upload_service::owner_read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.admin,
            &group.id,
            &upload.id
        )
        .await
        .is_err()
    );
    let request = requests::enqueue_with_uploads_ready(
        &f.state.db,
        &f.member,
        &group.id,
        "Read report",
        Some(std::slice::from_ref(&agent.id)),
        std::slice::from_ref(&upload.id),
    )
    .await
    .unwrap();
    assert!(
        svc::assistant_upload_service::owner_read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.admin,
            &group.id,
            &upload.id
        )
        .await
        .is_ok()
    );
    assert!(
        svc::assistant_upload_service::owner_read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.outsider,
            &group.id,
            &upload.id
        )
        .await
        .is_err()
    );
    let started = runtime::begin_ready(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &group.id,
        &request.id,
        &agent.id,
    )
    .await
    .unwrap();
    let row = &started.claim.conversation;
    let chat = svc::assistant_acknowledgement_service::for_key(
        &f.state.db,
        &f.member,
        Some(&row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        svc::assistant_upload_service::for_chat(&f.state.db, &chat, &upload.id)
            .await
            .is_ok()
    );
    let listing = svc::assistant_upload_service::list_for_chat(&f.state.db, &chat, None, 50)
        .await
        .unwrap();
    assert_eq!(listing["attachments"][0]["id"], upload.id);
    let other = runtime_start(&f, &f.admin, &group, &agent).await;
    let other_chat = svc::assistant_acknowledgement_service::for_key(
        &f.state.db,
        &f.admin,
        Some(&other.conversation.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        svc::assistant_upload_service::for_chat(&f.state.db, &other_chat, &upload.id)
            .await
            .is_err()
    );
    let listing = svc::assistant_upload_service::list_for_chat(&f.state.db, &other_chat, None, 50)
        .await
        .unwrap();
    assert_eq!(listing["attachments"], serde_json::json!([]));
    change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await;
    assert!(
        svc::assistant_upload_service::for_chat(&f.state.db, &chat, &upload.id)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_public_rest_gates_writes_and_hides_nonparticipant_groups() {
    use crate::handlers::{assistant_group as http, collaboration_group as api};
    use axum::{
        Json,
        extract::{Path, State},
    };
    let (f, agent, group) = setup("collaboration_rest_guards").await;
    let auth = crate::test_utils::test_auth_user(&f.admin);
    let body=serde_json::from_value(serde_json::json!({"collaboration":true,"name":"Public route","member_agent_ids":[agent.id]})).unwrap();
    let (_, Json(created)) = http::create_group(State(f.state.clone()), auth.clone(), Json(body))
        .await
        .unwrap();
    assert!(created.collaboration.is_some());
    let Json(issued) = api::invite(
        State(f.state.clone()),
        auth.clone(),
        Path(group.id.clone()),
        Json(api::IssueInviteInput {
            expected_revision: marker(&group).unwrap().membership_revision,
            intended_user_id: Some(f.member.clone()),
        }),
    )
    .await
    .unwrap();
    let token = issued["invite"]["token"].as_str().unwrap();
    assert!(
        membership::preview_invite(&f.state.db, &f.member, token)
            .await
            .is_ok()
    );
    assert!(
        membership::preview_invite(&f.state.db, &f.outsider, token)
            .await
            .is_err()
    );
    let stranger = crate::test_utils::test_auth_user(&f.outsider);
    assert!(matches!(
        http::get_group(
            State(f.state.clone()),
            stranger.clone(),
            Path(group.id.clone())
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    let update = serde_json::from_value(serde_json::json!({"name":"No access"})).unwrap();
    assert!(matches!(
        http::update_group(
            State(f.state.clone()),
            stranger.clone(),
            Path(group.id.clone()),
            Json(update)
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    let guest = api::GuestInput {
        expected_revision: 999,
        guest_revision: 999,
        enabled: true,
    };
    assert!(matches!(
        api::guest_access(
            State(f.state.clone()),
            stranger,
            Path((group.id.clone(), agent.id.clone())),
            Json(guest)
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    svc::feature_flag_service::set_platform_override(
        &f.state.db,
        svc::shared_agent_service::FLAG,
        &svc::feature_flag_service::FlagTarget::User(f.admin.clone()),
        false,
        &f.admin,
    )
    .await
    .unwrap();
    let body = serde_json::from_value(
        serde_json::json!({"collaboration":true,"name":"Disabled","member_agent_ids":[agent.id]}),
    )
    .unwrap();
    assert!(
        http::create_group(State(f.state.clone()), auth.clone(), Json(body))
            .await
            .is_err()
    );
    // Rollback keeps retained history and deletion reachable.
    assert!(
        http::get_group(State(f.state.clone()), auth.clone(), Path(group.id.clone()))
            .await
            .is_ok()
    );
    http::delete_group(State(f.state.clone()), auth, Path(group.id))
        .await
        .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_cancel_expires_cards_and_approvals_only_for_the_requester() {
    let (f, agent, group) = setup("collaboration_cancel_decisions").await;
    share(&f, &agent, &[&f.member]).await;
    let group = join(&f, &group, &f.member).await;
    let own = runtime_start(&f, &f.admin, &group, &agent).await;
    let other = runtime_start(&f, &f.member, &group, &agent).await;
    let approvals = f
        .state
        .db
        .collection::<bson::Document>(crate::models::approval_request::COLLECTION_NAME);
    for row in [&own.conversation, &other.conversation] {
        approvals.insert_one(doc! {"_id":&row.id,"status":"approved","assistant_group":{"group_id":&group.id,"conversation_id":&row.id}}).await.unwrap();
    }
    runtime::refresh(&f.state.db, &own.conversation)
        .await
        .unwrap();
    requests::cancel(
        &f.state.db,
        &f.admin,
        &group.id,
        &own.conversation.collaboration.as_ref().unwrap().request_id,
    )
    .await
    .unwrap();
    assert!(
        runtime::refresh(&f.state.db, &own.conversation)
            .await
            .is_err()
    );
    runtime::refresh(&f.state.db, &other.conversation)
        .await
        .unwrap();
    assert_eq!(
        approvals
            .find_one(doc! {"_id":&own.conversation.id})
            .await
            .unwrap()
            .unwrap()
            .get_str("status")
            .unwrap(),
        "expired"
    );
    assert_eq!(
        approvals
            .find_one(doc! {"_id":&other.conversation.id})
            .await
            .unwrap()
            .unwrap()
            .get_str("status")
            .unwrap(),
        "approved"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_pending_action_blocks_a_different_request_from_reusing_the_thread() {
    use svc::assistant_acknowledgement_service as acks;
    let (f, agent, group) = setup("collaboration_card_chain").await;
    let first = runtime_start(&f, &f.admin, &group, &agent).await;
    let row = &first.conversation;
    let chat = acks::for_key(&f.state.db, &f.admin, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let args = serde_json::json!({"value":"exact"});
    let card = acks::request(
        &f.state.db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("operation"),
            arguments: Some(&args),
            summary: "Choose",
            platform: false,
        },
    )
    .await
    .unwrap();
    svc::assistant_nyxagent::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &svc::assistant_nyxagent::TurnResult {
            text: "Waiting".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let next = requests::enqueue_ready(&f.state.db, &f.admin, &group.id, "Different request", None)
        .await
        .unwrap();
    assert!(matches!(
        runtime::begin_ready(
            &f.state.db,
            &f.state.encryption_keys,
            &f.admin,
            &group.id,
            &next.id,
            &agent.id
        )
        .await,
        Err(AppError::AssistantTurnActive)
    ));
    acks::decide(&f.state.db, &f.admin, &row.id, &card.id, true)
        .await
        .unwrap();
    let resumed = runtime::begin_ready(
        &f.state.db,
        &f.state.encryption_keys,
        &f.admin,
        &group.id,
        &row.collaboration.as_ref().unwrap().request_id,
        &agent.id,
    )
    .await
    .unwrap();
    assert_eq!(resumed.claim.conversation.id, row.id);
    assert!(resumed.input.contains(&card.id));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_only_explicit_images_survive_private_thread_cleanup() {
    use crate::models::{assistant_attachment as image, assistant_conversation::TurnAttachment};
    let (f, agent, group) = setup("collaboration_published_images").await;
    share(&f, &agent, &[&f.member]).await;
    let group = join(&f, &group, &f.member).await;
    let claim = runtime_start(&f, &f.member, &group, &agent).await;
    let row = &claim.conversation;
    let turn = row.active_turn.as_ref().unwrap();
    let mut items = Vec::new();
    for delivery in [true, false] {
        let mut image = image::AssistantAttachment {
            id: uuid::Uuid::new_v4().to_string(),
            origin: "tool".into(),
            user_id: f.member.clone(),
            conversation_id: row.id.clone(),
            turn_id: turn.turn_id.clone(),
            content_type: "image/png".into(),
            size: 4,
            secret_context_versions: BTreeMap::new(),
            data_encrypted: vec![],
            created_at: Utc::now(),
        };
        image.data_encrypted = f.state.encryption_keys.encrypt(b"png!").await.unwrap();
        f.state
            .db
            .collection::<image::AssistantAttachment>(image::COLLECTION_NAME)
            .insert_one(&image)
            .await
            .unwrap();
        items.push(TurnAttachment {
            id: image.id,
            origin: "tool".into(),
            content_type: "image/png".into(),
            size: 4,
            label: "Generated image".into(),
            chat_delivery: Some(delivery),
            image_input: None,
            image_inspection: Some(false),
            pages: None,
        });
    }
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"active_turn.attachments":bson::to_bson(&items).unwrap()}},
        )
        .await
        .unwrap();
    svc::assistant_nyxagent::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &svc::assistant_nyxagent::TurnResult {
            text: "Published image".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let history = messages(&f.state.db, &f.admin, &group.id, None)
        .await
        .unwrap();
    assert_eq!(
        history
            .last()
            .unwrap()
            .attachments
            .iter()
            .map(|a| &a.id)
            .collect::<Vec<_>>(),
        vec![&items[0].id]
    );
    change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await;
    cleanup::sweep(&f.state.db).await.unwrap();
    assert!(
        svc::assistant_nyxagent::read_group_tool_attachment(
            &f.state.db,
            &f.state.encryption_keys,
            &f.admin,
            &group.id,
            &items[0].id
        )
        .await
        .is_ok()
    );
    assert!(
        svc::assistant_nyxagent::read_group_tool_attachment(
            &f.state.db,
            &f.state.encryption_keys,
            &f.member,
            &group.id,
            &items[0].id
        )
        .await
        .is_err()
    );
    assert!(
        svc::assistant_nyxagent::read_group_tool_attachment(
            &f.state.db,
            &f.state.encryption_keys,
            &f.admin,
            &group.id,
            &items[1].id
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(image::COLLECTION_NAME)
            .count_documents(doc! {"_id":&items[1].id})
            .await
            .unwrap(),
        0
    );
    let group = get(&f.state.db, &f.admin, &group.id).await.unwrap();
    change(&f, &f.admin, &group, Change::Delete).await;
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(image::COLLECTION_NAME)
            .count_documents(doc! {"_id":&items[0].id})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_service_approval_resumes_its_request_and_cannot_outlive_departure() {
    use crate::models::approval_request::{ApprovalRequest, COLLECTION_NAME};
    let (f, agent, group) = setup("collaboration_service_approvals").await;
    share(&f, &agent, &[&f.member]).await;
    let group = join(&f, &group, &f.member).await;
    let claim = runtime_start(&f, &f.member, &group, &agent).await;
    let row = &claim.conversation;
    let binding = svc::org_group_service::approval_binding(
        &f.state.db,
        Some(&group.id),
        &f.member,
        Some(&row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    f.state.db.collection::<bson::Document>(COLLECTION_NAME).insert_one(doc! {
        "_id":&id,"user_id":&f.member,"service_id":"test-service","service_name":"Private account","service_slug":"private-account",
        "requester_type":"user","requester_id":&f.member,"operation_summary":"Exact operation","status":"pending","approval_mode":"per_request",
        "idempotency_key":&id,"created_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::minutes(5)),
        "assistant_group":bson::to_bson(&binding).unwrap()
    }).await.unwrap();
    let approval = f
        .state
        .db
        .collection::<ApprovalRequest>(COLLECTION_NAME)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        svc::org_group_service::authorize_approval(&f.state.db, &approval, &f.admin, None)
            .await
            .is_err()
    );
    svc::assistant_nyxagent::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &svc::assistant_nyxagent::TurnResult {
            text: "Waiting for approval".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let response =
        crate::handlers::collaboration_group::response(&f.state, &f.member, group.clone())
            .await
            .unwrap();
    assert_eq!(
        response.collaboration.unwrap()["your_request_ids"],
        serde_json::json!([binding.request_id])
    );
    let updated = svc::approval_service::process_decision(
        &f.state.db,
        &f.state.config,
        &f.state.http_client,
        None,
        None,
        &id,
        true,
        None,
        None,
        "web",
    )
    .await
    .unwrap();
    assert_eq!(updated.status, "approved");
    let pending = requests::pending(&f.state.db, Some(&group.id))
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, binding.request_id);
    let resumed = runtime::begin_ready(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &group.id,
        &binding.request_id,
        &agent.id,
    )
    .await
    .unwrap();
    assert_eq!(resumed.claim.conversation.id, row.id);
    assert_eq!(resumed.admission.requester(), f.member);
    change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await;
    assert!(
        svc::approval_service::process_decision(
            &f.state.db,
            &f.state.config,
            &f.state.http_client,
            None,
            None,
            &id,
            true,
            None,
            None,
            "web"
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[test]
fn collaboration_native_tools_and_mcp_share_the_same_schemas() {
    use svc::assistant_team_tools as tools;
    for (name, args) in [
        (
            "create_collaboration_group",
            serde_json::json!({"name":"Research","member_agent_ids":["agent"]}),
        ),
        ("list_collaboration_groups", serde_json::json!({})),
        (
            "manage_collaboration_group",
            serde_json::json!({"group":"group","expected_revision":2,"change":{"action":"invite","intended_user_id":"person"}}),
        ),
        (
            "post_to_collaboration_group",
            serde_json::json!({"group":"group","text":"Hello","recipient_agent_ids":["agent"]}),
        ),
        (
            "manage_collaboration_group",
            serde_json::json!({"group":"group","expected_revision":2,"change":{"action":"guest_access","agent_id":"agent","enabled":true,"guest_revision":1}}),
        ),
    ] {
        assert!(tools::is_team_tool(&format!("nyxid__{name}")));
        tools::validate(name, &args).unwrap();
        assert!(tools::endpoints().iter().any(|e| e.name == name));
    }
    assert!(tools::validate("manage_collaboration_group",&serde_json::json!({"group":"group","expected_revision":2,"change":{"action":"accept_transfer"}})).is_err());
}

struct GroupEgress {
    actor: String,
    key: String,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl svc::platform_manager_contract::NyxAgentEgress for GroupEgress {
    async fn request(
        &self,
        context: &svc::platform_manager_contract::NyxAgentExecutionContext,
        key: &svc::platform_manager_contract::RuntimeCredential,
        request: svc::platform_manager_contract::NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        use axum::response::IntoResponse;
        assert!(
            matches!(context,svc::platform_manager_contract::NyxAgentExecutionContext::Owner {user_id,platform_origin:None,..} if user_id==&self.actor)
        );
        assert_eq!(key.api_key_id, self.key);
        if request.relative_path == "v1/models" {
            return Ok(axum::Json(serde_json::json!({"data":[]})).into_response());
        }
        assert_eq!(request.relative_path, "v1/responses");
        self.calls.fetch_add(1, Ordering::SeqCst);
        let frame = serde_json::json!({"type":"response.completed","sequence_number":0,"response":{"id":"resp_1234567890abcdef1234567890abcdef_1234567890abcdef1234567890abcdef","conversation":{"id":"conv_1234567890abcdef1234567890abcdef"},"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Group answer"}]}]}});
        Ok((
            [("content-type", "text/event-stream")],
            format!("data: {frame}\n\n"),
        )
            .into_response())
    }
}
#[tokio::test]
async fn collaboration_model_runtime_uses_requester_key_and_refuses_after_revocation() {
    use svc::assistant_nyxagent_runtime as worker;
    let (f, agent, group) = setup("collaboration_real_runtime").await;
    share(&f, &agent, &[&f.member]).await;
    let group = join(&f, &group, &f.member).await;
    let request = requests::enqueue_ready(
        &f.state.db,
        &f.member,
        &group.id,
        "Reply to the group",
        None,
    )
    .await
    .unwrap();
    let mut started = runtime::begin_ready(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &group.id,
        &request.id,
        &agent.id,
    )
    .await
    .unwrap();
    let row = &started.claim.conversation;
    let egress = GroupEgress {
        actor: f.member.clone(),
        key: row.credential_api_key_id.clone(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    };
    assert!(!row.guest_model_setup);
    let mut setup_row = row.clone();
    setup_row.guest_model_setup = true;
    assert!(runtime::validate(&f.state.db, &setup_row).await.is_err());
    let context = worker::owner_context(row).unwrap();
    let (sender, _) = tokio::sync::broadcast::channel(256);
    let mut events = worker::Events {
        checkpoint: None,
        sender,
        cursor: 0,
        reset_notified: false,
    };
    let result = worker::execute_owner_turn(
        &f.state,
        &egress,
        &context,
        row,
        &started.input,
        &mut started.claim.credential,
        None,
        &mut events,
        "block",
        &mut String::new(),
        &mut None,
    )
    .await
    .unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.text, "Group answer");
    assert_eq!(egress.calls.load(Ordering::SeqCst), 1);
    // The private reader remains closed despite internal runtime access.
    assert!(
        svc::assistant_nyxagent::messages(&f.state.db, &f.member, &row.id, 20, None)
            .await
            .is_err()
    );
    change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await;
    assert!(
        worker::execute_owner_turn(
            &f.state,
            &egress,
            &context,
            row,
            &started.input,
            &mut started.claim.credential,
            None,
            &mut events,
            "block",
            &mut String::new(),
            &mut None
        )
        .await
        .is_err()
    );
    assert_eq!(egress.calls.load(Ordering::SeqCst), 1);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_card_snapshot_batches_threads_without_per_agent_reads() {
    let (f, _, reads) = Fixture::observed("collaboration_card_queries").await;
    enable(&f).await;
    let mut agents = Vec::new();
    for i in 0..8 {
        agents.push(specialist(&f, &f.admin, &format!("card-{i}")).await);
    }
    let one = membership::create_ready(
        &f.state.db,
        &f.admin,
        "One",
        std::slice::from_ref(&agents[0].id),
    )
    .await
    .unwrap();
    let eight = membership::create_ready(
        &f.state.db,
        &f.admin,
        "Eight",
        &agents.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
    )
    .await
    .unwrap();
    runtime_start(&f, &f.admin, &one, &agents[0]).await;
    for agent in &agents {
        runtime_start(&f, &f.admin, &eight, agent).await;
    }
    reads.store(0, Ordering::SeqCst);
    assert_eq!(
        runtime::decision_threads(&f.state.db, &f.admin, &one.id)
            .await
            .unwrap()
            .len(),
        1
    );
    let single = reads.load(Ordering::SeqCst);
    reads.store(0, Ordering::SeqCst);
    assert_eq!(
        runtime::decision_threads(&f.state.db, &f.admin, &eight.id)
            .await
            .unwrap()
            .len(),
        8
    );
    assert_eq!(single, reads.load(Ordering::SeqCst));
    f.state.db.drop().await.unwrap();
}
