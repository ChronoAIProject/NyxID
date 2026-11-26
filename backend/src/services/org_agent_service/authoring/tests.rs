use super::*;
use crate::{
    handlers::assistant_team,
    mw::auth::{AuthMethod, AuthUser},
    services::{org_agent_tests::Fixture, security_readiness_service as readiness},
    test_utils::*,
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde_json::Value;

async fn setup() -> (Fixture, AssistantAgent) {
    let f = Fixture::new("org_access_authoring").await;
    let (agent, _) = f.create().await;
    flags::set_platform_override(
        &f.state.db,
        flags::NYXAGENT_ENGINE_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.admin,
    )
    .await
    .unwrap();
    (f, agent)
}
async fn enable(f: &Fixture) {
    flags::set_platform_org_override(
        &f.state.db,
        &f.org,
        flags::ORG_AGENT_ACCESS_FLAG_KEY,
        true,
        &f.admin,
    )
    .await
    .unwrap();
}
async fn put(f: &Fixture, id: &str, actor: AuthUser, body: Value) -> AppResult<Value> {
    let body = Json::from_bytes(&serde_json::to_vec(&body).unwrap());
    let Json(data) = Box::pin(assistant_team::put_org_access(
        State(f.state.clone()),
        actor,
        Path(id.into()),
        body,
    ))
    .await?;
    Ok(serde_json::to_value(data).unwrap())
}
fn policy(revision: i64, maintainers: Value, users: Value) -> Value {
    json!({"maintainers":maintainers,"users":users,"expected_revision":revision})
}
async fn get(f: &Fixture, id: &str) -> Value {
    let Json(data) = assistant_team::get_org_access(
        State(f.state.clone()),
        test_auth_user(&f.admin),
        Path(id.into()),
    )
    .await
    .unwrap();
    serde_json::to_value(data).unwrap()
}
async fn revisions(f: &Fixture, id: &str) -> (i64, i64, i64) {
    let a = super::super::governed_agent(&f.state.db, &f.admin, id)
        .await
        .unwrap();
    (
        a.org_access_revision,
        a.org_maintain_revision,
        a.org_use_revision,
    )
}

#[tokio::test]
async fn org_access_authoring_gate_and_human_admin() {
    let (f, agent) = setup().await;
    let body = policy(0, json!([]), json!([]));
    let admin = || test_auth_user(&f.admin);
    assert_eq!(get(&f, &agent.id).await["authoring_available"], false);
    assert!(matches!(
        put(&f, &agent.id, admin(), body.clone()).await,
        Err(AppError::OrgAgentAccessAuthoringUnavailable)
    ));
    enable(&f).await;
    assert_eq!(get(&f, &agent.id).await["authoring_available"], true);
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::Delegated,
        AuthMethod::ServiceAccount,
    ] {
        let mut auth = admin();
        auth.auth_method = method;
        assert!(put(&f, &agent.id, auth, body.clone()).await.is_err());
    }
    for actor in [&f.member, &f.viewer, &f.outsider] {
        assert!(
            put(&f, &agent.id, test_auth_user(actor), body.clone())
                .await
                .is_err()
        );
    }
    let sibling = readiness::overlapping_test_replica(&f.state.db).await;
    readiness::remove_test_replica_capability(
        &f.state.db,
        &sibling,
        flags::ORG_AGENT_ACCESS_FLAG_KEY,
    )
    .await;
    assert_eq!(get(&f, &agent.id).await["authoring_available"], false);
    assert!(matches!(
        put(&f, &agent.id, admin(), body).await,
        Err(AppError::OrgAgentAccessAuthoringUnavailable)
    ));
    assert_eq!(revisions(&f, &agent.id).await, (0, 0, 0));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_access_authoring_set_cas_narrowing_noop_and_atomic_self_add_audit() {
    use crate::models::security_effect_intent::COLLECTION_NAME as INTENTS;
    let (f, agent) = setup().await;
    enable(&f).await;
    let db = &f.state.db;
    let submit = |body| put(&f, &agent.id, test_auth_user(&f.admin), body);
    let remove_all = policy(0, json!([]), json!([]));
    let (a, b) = tokio::join!(submit(remove_all.clone()), submit(remove_all));
    assert!(matches!(
        (&a, &b),
        (Ok(_), Err(AppError::Conflict(_))) | (Err(AppError::Conflict(_)), Ok(_))
    ));
    assert_eq!(revisions(&f, &agent.id).await, (1, 1, 1));
    let allow = policy(1, json!([f.admin, f.member]), json!(null));
    let saved = submit(allow).await.unwrap();
    assert_eq!(saved, get(&f, &agent.id).await);
    assert_eq!(revisions(&f, &agent.id).await, (2, 1, 1));
    let intents = db.collection::<Document>(INTENTS);
    let filter = doc! {"event.event_type":"org_agent_access_changed"};
    let event = intents
        .find_one(
            doc! {"event.event_type":"org_agent_access_changed","event.event_data.revision":2},
        )
        .await
        .unwrap()
        .unwrap();
    let intent_id = event.get_str("_id").unwrap();
    let event = event.get_document("event").unwrap();
    assert_eq!(event.get_str("_id").unwrap(), intent_id);
    assert_eq!(event.get_str("user_id").unwrap(), &f.admin);
    let data = event.get_document("event_data").unwrap();
    assert_eq!(data.get_str("org_user_id").unwrap(), &f.org);
    assert_eq!(data.get_str("agent_id").unwrap(), &agent.id);
    let facts = data.get_document("maintainers").unwrap();
    assert_eq!(
        facts
            .get("added")
            .and_then(bson::Bson::as_i64)
            .or_else(|| facts.get_i32("added").ok().map(i64::from)),
        Some(2)
    );
    assert!(facts.get_bool("self_added").unwrap());
    assert!(!format!("{data:?}").contains(&f.member));
    assert_eq!(
        submit(policy(2, json!([f.member, f.admin]), json!(null)))
            .await
            .unwrap(),
        saved
    );
    assert_eq!(intents.count_documents(filter.clone()).await.unwrap(), 2);
    assert_eq!(revisions(&f, &agent.id).await, (2, 1, 1));
    assert!(matches!(
        submit(policy(1, json!([f.member, f.admin]), json!(null))).await,
        Err(AppError::Conflict(_))
    ));
    let narrow = policy(2, json!([f.member]), json!([]));
    db.run_command(doc! {"collMod":INTENTS,"validator":{"refuse_test_insert":{"$exists":true}}})
        .await
        .unwrap();
    assert!(submit(narrow.clone()).await.is_err());
    assert_eq!(get(&f, &agent.id).await, saved);
    assert_eq!(revisions(&f, &agent.id).await, (2, 1, 1));
    assert_eq!(intents.count_documents(filter).await.unwrap(), 2);
    db.run_command(doc! {"collMod":INTENTS,"validator":{}})
        .await
        .unwrap();
    submit(narrow).await.unwrap();
    assert_eq!(revisions(&f, &agent.id).await, (3, 2, 2));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_access_authoring_validates_shape_bounds_and_live_subjects() {
    use crate::models::{org_membership::OrgRole, user::UserType};
    let (f, agent) = setup().await;
    enable(&f).await;
    let submit = |body| put(&f, &agent.id, test_auth_user(&f.admin), body);
    let all: Vec<_> = (0..128).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let mut too_many = all.clone();
    too_many.push(f.admin.clone());
    for invalid in [
        json!({"maintainers":null,"expected_revision":0}),
        json!({"users":null,"expected_revision":0}),
        json!({"maintainers":null,"users":null}),
        json!({"maintainers":null,"users":null,"expected_revision":0,"extra":true}),
        policy(-1, json!(null), json!(null)),
        policy(0, json!([f.member, f.member]), json!(null)),
        policy(0, json!(null), json!(["invalid"])),
        policy(0, json!(too_many), json!(null)),
        policy(0, json!(null), json!(too_many)),
        policy(0, json!([f.viewer]), json!([])),
        policy(0, json!([]), json!([f.outsider])),
        policy(0, json!([f.org]), json!([])),
    ] {
        assert!(matches!(
            submit(invalid).await,
            Err(AppError::ValidationError(_))
        ));
    }
    let people: Vec<_> = all
        .iter()
        .map(|id| test_user(id, UserType::Person))
        .collect();
    let memberships: Vec<_> = all
        .iter()
        .map(|id| test_membership(&f.org, id, OrgRole::Member, None))
        .collect();
    let db = &f.state.db;
    db.collection(USERS).insert_many(people).await.unwrap();
    db.collection(MEMBERS)
        .insert_many(memberships)
        .await
        .unwrap();
    submit(policy(0, json!(all), json!(all))).await.unwrap();
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":&all[0]}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(matches!(
        submit(policy(1, json!(all), json!([]))).await,
        Err(AppError::ValidationError(_))
    ));
    assert_eq!(revisions(&f, &agent.id).await, (1, 1, 1));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_access_authoring_legacy_policy_key_invalidation_and_membership_pruning() {
    use crate::services::{
        assistant_team_service as team, org_agent_service as access, org_service,
    };
    let (f, agent) = setup().await;
    let db = &f.state.db;
    db.collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&agent.id},
            doc! {"$unset":{
                "org_access_revision":"","org_maintain_revision":"","org_use_revision":""
            }},
        )
        .await
        .unwrap();
    enable(&f).await;
    let thread = team::home_thread_for(db, &f.state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    let key = f.key(&thread).await;
    assert!(
        access::resolve_credential_access(db, &key)
            .await
            .unwrap()
            .is_some()
    );
    let submit = |body| put(&f, &agent.id, test_auth_user(&f.admin), body);
    submit(policy(0, json!(null), json!([]))).await.unwrap();
    submit(policy(1, json!([f.member]), json!([f.member])))
        .await
        .unwrap();
    assert_eq!(revisions(&f, &agent.id).await, (2, 1, 1));
    assert!(access::resolve_credential_access(db, &key).await.is_err());
    flags::set_platform_org_override(
        db,
        &f.org,
        flags::ORG_AGENT_ACCESS_FLAG_KEY,
        false,
        &f.admin,
    )
    .await
    .unwrap();
    let current = access::governed_agent(db, &f.admin, &agent.id)
        .await
        .unwrap();
    assert!(access::require_use(db, &f.admin, &current).await.is_err());
    assert!(access::require_use(db, &f.member, &current).await.is_ok());
    let membership = org_service::get_active_membership(db, &f.org, &f.member)
        .await
        .unwrap()
        .unwrap();
    org_service::update_membership(db, &membership.id, Some(OrgRole::Viewer), None, None)
        .await
        .unwrap();
    let pruned = get(&f, &agent.id).await;
    assert_eq!(
        pruned,
        json!({"maintainers":[],"users":[],"revision":3,"authoring_available":false})
    );
    assert_eq!(revisions(&f, &agent.id).await, (3, 2, 2));
    db.drop().await.unwrap();
}
