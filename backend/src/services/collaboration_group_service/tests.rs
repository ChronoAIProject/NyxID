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
        requests::enqueue_ready(
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
