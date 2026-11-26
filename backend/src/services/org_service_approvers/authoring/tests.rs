use super::*;
use crate::{
    handlers::approvals::{OrgApproversQuery, get_org_approvers, put_org_approvers},
    models::{downstream_service, org_membership, security_effect_intent, user, user_service},
    mw::auth::{AuthMethod, AuthUser},
    services::{org_agent_tests::Fixture, security_readiness_service as readiness},
    test_utils::{test_auth_user, test_auto_connected_catalog_service, test_membership, test_user},
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use bson::Document;
use serde_json::Value;

struct Settings {
    f: Fixture,
    catalog: String,
    left: String,
    right: String,
    config: String,
}

impl Settings {
    async fn new(label: &str) -> Self {
        let f = Fixture::new(label).await;
        let db = &f.state.db;
        let catalog = test_auto_connected_catalog_service();
        db.collection::<downstream_service::DownstreamService>(downstream_service::COLLECTION_NAME)
            .insert_one(&catalog)
            .await
            .unwrap();
        let left = f.service(&f.org, "left").await;
        let right = f.service(&f.org, "right").await;
        db.collection::<Document>(user_service::COLLECTION_NAME)
            .update_many(
                doc! {"user_id":&f.org},
                doc! {"$set":{"catalog_service_id":&catalog.id}},
            )
            .await
            .unwrap();
        db.collection::<Document>(org_membership::COLLECTION_NAME).update_one(doc! {"member_user_id":&f.member},doc! {"$set":{"role":"admin","scope_source":"override","allowed_service_ids":[&right]}}).await.unwrap();
        let config = Uuid::new_v4().to_string();
        let now = bson::DateTime::now();
        db.collection::<Document>(CONFIGS).insert_one(doc!{"_id":&config,"user_id":&f.org,"service_id":&catalog.id,"service_name":"Catalog","approval_required":true,"created_at":now,"updated_at":now}).await.unwrap();
        Self {
            f,
            catalog: catalog.id,
            left,
            right,
            config,
        }
    }

    async fn available(&self, enabled: bool) -> AppResult<()> {
        feature_flag_service::set_platform_org_override(
            &self.f.state.db,
            &self.f.org,
            FLAG,
            enabled,
            &self.f.admin,
        )
        .await
        .map(|_| ())
    }
    fn query(&self) -> Query<OrgApproversQuery> {
        Query(OrgApproversQuery {
            org_id: self.f.org.parse().unwrap(),
        })
    }
    async fn get(&self, actor: AuthUser, service: &str) -> AppResult<Value> {
        let Json(policy) = get_org_approvers(
            State(self.f.state.clone()),
            actor,
            Path(service.into()),
            self.query(),
        )
        .await?;
        Ok(serde_json::to_value(policy).unwrap())
    }
    async fn put(
        &self,
        actor: AuthUser,
        service: &str,
        revision: i64,
        ids: Value,
    ) -> AppResult<Value> {
        let policy = serde_json::from_value(
            json!({"config_id":self.config,"expected_revision":revision,"approver_user_ids":ids}),
        )
        .unwrap();
        let Json(result) = put_org_approvers(
            State(self.f.state.clone()),
            actor,
            Path(service.into()),
            self.query(),
            Json(policy),
        )
        .await?;
        Ok(serde_json::to_value(result).unwrap())
    }
    async fn stored(&self) -> Document {
        self.f
            .state
            .db
            .collection::<Document>(CONFIGS)
            .find_one(doc! {"_id":&self.config})
            .await
            .unwrap()
            .unwrap()
    }
}

#[tokio::test]
async fn authoring_governance_candidates_and_readiness_follow_live_authority() {
    let s = Settings::new("approver_authoring_entitlement").await;
    let admin = || test_auth_user(&s.f.admin);
    let scoped = || test_auth_user(&s.f.member);
    let get = || s.get(scoped(), &s.right);
    let put = || s.put(scoped(), &s.right, 0, json!([s.f.member]));
    let initial = get().await.unwrap();
    assert_eq!(initial["approver_user_ids"], Value::Null);
    assert_eq!(initial["config_id"], s.config);
    assert_eq!(initial["revision"], 0);
    assert_eq!(initial["authoring_available"], false);
    assert_eq!(initial["authoring_unavailable_reason"], "flag_off");
    assert_eq!(initial["unavailable"], json!([]));
    let ids: std::collections::BTreeSet<_> = initial["eligible_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["user_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [s.f.admin.as_str(), s.f.member.as_str()]
            .into_iter()
            .collect()
    );
    assert!(matches!(put().await, Err(AppError::Forbidden(_))));
    s.available(true).await.unwrap();
    assert_eq!(
        s.get(scoped(), &s.catalog).await.unwrap()["authoring_available"],
        true
    );
    for actor in [&s.f.viewer, &s.f.outsider] {
        assert!(s.get(test_auth_user(actor), &s.catalog).await.is_err());
        assert!(
            s.put(test_auth_user(actor), &s.catalog, 0, Value::Null)
                .await
                .is_err()
        );
    }
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::ServiceAccount,
        AuthMethod::Delegated,
        AuthMethod::AccessToken,
    ] {
        let auth = || {
            let mut a = admin();
            a.auth_method = method.clone();
            a.oauth_client_id = Some("external".into());
            a
        };
        assert!(s.get(auth(), &s.catalog).await.is_err());
        assert!(s.put(auth(), &s.catalog, 0, Value::Null).await.is_err());
    }
    assert!(s.get(scoped(), &s.left).await.is_err());
    assert!(s.put(scoped(), &s.left, 0, Value::Null).await.is_err());
    let mut first_party = scoped();
    first_party.auth_method = AuthMethod::AccessToken;
    assert_eq!(
        s.get(first_party, &s.right).await.unwrap()["authoring_available"],
        true
    );
    readiness::stop_test_replica(&s.f.state.db).await;
    let activations =
        s.f.state
            .db
            .collection::<Document>(crate::models::security_capability_activation::COLLECTION_NAME);
    let activation = activations.find_one_and_delete(doc! {"record_kind":crate::models::security_capability_activation::RECORD_KIND,"capability":FLAG}).await.unwrap().unwrap();
    let view = get().await.unwrap();
    assert_eq!(view["authoring_available"], false);
    assert_eq!(view["authoring_unavailable_reason"], "readiness_missing");
    assert!(matches!(put().await, Err(AppError::Forbidden(_))));
    activations.insert_one(activation).await.unwrap();
    let view = get().await.unwrap();
    assert_eq!(view["authoring_available"], true);
    assert_eq!(view.get("authoring_unavailable_reason"), Some(&Value::Null));
    let peer = readiness::overlapping_test_replica(&s.f.state.db).await;
    readiness::remove_test_replica_capability(&s.f.state.db, &peer, FLAG).await;
    let view = get().await.unwrap();
    assert_eq!(view["authoring_available"], false);
    assert_eq!(view["authoring_unavailable_reason"], "readiness_missing");
    assert!(matches!(put().await, Err(AppError::Forbidden(_))));
    s.available(false).await.unwrap();
    assert!(matches!(
        s.available(true).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    s.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn edits_apply_cas_grant_cutoff_and_atomic_self_selection_audit() {
    let s = Settings::new("approver_authoring_cas").await;
    let db = &s.f.state.db;
    s.available(true).await.unwrap();
    let put = |rev, ids| s.put(test_auth_user(&s.f.admin), &s.left, rev, ids);
    let first = json!([s.f.member]);
    let (a, b) = tokio::join!(put(0, first.clone()), put(0, first.clone()));
    assert!(matches!(
        (&a, &b),
        (Ok(_), Err(AppError::Conflict(_))) | (Err(AppError::Conflict(_)), Ok(_))
    ));
    assert_eq!(s.stored().await.get_i64("min_grant_revision"), Ok(1));
    assert!(
        s.stored()
            .await
            .get_bool("approver_grants_pending")
            .unwrap()
    );
    let both = json!([s.f.member, s.f.admin]);
    let widened = put(1, both.clone()).await.unwrap();
    assert_eq!(widened["revision"], 2);
    assert_eq!(s.stored().await.get_i64("min_grant_revision"), Ok(1));
    let intents = db.collection::<Document>(security_effect_intent::COLLECTION_NAME);
    let filter = doc! {"event.event_type":"org_service_approvers_changed"};
    let stored = intents
        .find_one(
            doc! {"event.event_type":"org_service_approvers_changed","event.event_data.revision":2},
        )
        .await
        .unwrap()
        .unwrap();
    let event = stored.get_document("event").unwrap();
    assert_eq!(stored.get_str("_id"), event.get_str("_id"));
    assert_eq!(event.get_str("user_id"), Ok(s.f.admin.as_str()));
    let data = event.get_document("event_data").unwrap();
    assert_eq!(data.get_str("config_id"), Ok(s.config.as_str()));
    assert_eq!(data.get_str("org_user_id"), Ok(s.f.org.as_str()));
    assert_eq!(data.get_str("service_id"), Ok(s.catalog.as_str()));
    assert_eq!(
        serde_json::to_value(data.get_document("approvers").unwrap()).unwrap(),
        json!({"added":1,"removed":0,"inherited_before":false,"inherited_after":false,"self_added":true})
    );
    assert!(!format!("{data:?}").contains(&s.f.member));
    let before = s.stored().await;
    assert_eq!(
        put(2, json!([s.f.admin, s.f.member])).await.unwrap(),
        widened
    );
    assert!(matches!(put(1, both).await, Err(AppError::Conflict(_))));
    assert_eq!(s.stored().await, before);
    db.run_command(doc!{"collMod":security_effect_intent::COLLECTION_NAME,"validator":{"force_test_failure":{"$exists":true}}}).await.unwrap();
    assert!(put(2, first.clone()).await.is_err());
    assert_eq!(s.stored().await, before);
    assert_eq!(intents.count_documents(filter).await.unwrap(), 2);
    db.run_command(doc! {"collMod":security_effect_intent::COLLECTION_NAME,"validator":{}})
        .await
        .unwrap();
    put(2, first.clone()).await.unwrap();
    assert_eq!(s.stored().await.get_i64("min_grant_revision"), Ok(3));
    s.available(false).await.unwrap();
    assert_eq!(
        s.get(test_auth_user(&s.f.admin), &s.catalog).await.unwrap()["approver_user_ids"],
        first
    );
    assert!(put(3, Value::Null).await.is_err());
    s.available(true).await.unwrap();
    assert_eq!(put(3, Value::Null).await.unwrap()["revision"], 4);
    assert_eq!(s.stored().await.get_i64("min_grant_revision"), Ok(3));
    let configs = db.collection::<Document>(CONFIGS);
    let mut recreated = configs
        .find_one_and_delete(doc! {"_id":&s.config})
        .await
        .unwrap()
        .unwrap();
    recreated.insert("_id", Uuid::new_v4().to_string());
    configs.insert_one(recreated).await.unwrap();
    assert!(matches!(
        put(4, Value::Null).await,
        Err(AppError::Conflict(_))
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn authoring_accepts_128_distinct_eligible_admins_and_rejects_invalid_selections() {
    let s = Settings::new("approver_authoring_bounds").await;
    s.available(true).await.unwrap();
    let db = &s.f.state.db;
    let put = |ids| s.put(test_auth_user(&s.f.admin), &s.catalog, 0, ids);
    let ids: Vec<_> = (0..128).map(|_| Uuid::new_v4().to_string()).collect();
    for invalid in [
        json!([]),
        json!([s.f.admin, s.f.admin]),
        json!([s.f.viewer]),
        json!([s.f.outsider]),
        json!([s.f.org]),
        json!(vec![&s.f.admin; 129]),
    ] {
        assert!(matches!(
            put(invalid).await,
            Err(AppError::ValidationError(_))
        ));
    }
    let members = db.collection::<Document>(org_membership::COLLECTION_NAME);
    members
        .update_one(
            doc! {"member_user_id":&s.f.member},
            doc! {"$set":{"allowed_service_ids":[]}},
        )
        .await
        .unwrap();
    assert!(matches!(
        put(json!([s.f.member])).await,
        Err(AppError::ValidationError(_))
    ));
    let view = s.get(test_auth_user(&s.f.admin), &s.catalog).await.unwrap();
    assert_eq!(view["eligible_candidates"].as_array().unwrap().len(), 1);
    members
        .update_one(
            doc! {"member_user_id":&s.f.member},
            doc! {"$set":{"allowed_service_ids":null}},
        )
        .await
        .unwrap();
    for field in [
        doc! {"is_active":false},
        doc! {"is_active":true,"platform_personal_deleting":true},
    ] {
        db.collection::<Document>(user::COLLECTION_NAME)
            .update_one(doc! {"_id":&s.f.member}, doc! {"$set":field})
            .await
            .unwrap();
        assert!(matches!(
            put(json!([s.f.member])).await,
            Err(AppError::ValidationError(_))
        ));
    }
    db.collection::<user::User>(user::COLLECTION_NAME)
        .insert_many(ids.iter().map(|id| test_user(id, user::UserType::Person)))
        .await
        .unwrap();
    db.collection::<org_membership::OrgMembership>(org_membership::COLLECTION_NAME)
        .insert_many(
            ids.iter()
                .map(|id| test_membership(&s.f.org, id, org_membership::OrgRole::Admin, None)),
        )
        .await
        .unwrap();
    assert_eq!(
        put(json!(ids)).await.unwrap()["approver_user_ids"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn a_candidate_demotion_before_or_after_save_removes_their_authority() {
    use crate::services::org_service;
    let s = Settings::new("approver_authoring_demotion").await;
    let db = &s.f.state.db;
    s.available(true).await.unwrap();
    let membership = org_service::get_active_membership(db, &s.f.org, &s.f.member)
        .await
        .unwrap()
        .unwrap();
    let edit = |revision| {
        s.put(
            test_auth_user(&s.f.admin),
            &s.catalog,
            revision,
            json!([s.f.member]),
        )
    };
    edit(0).await.unwrap();
    org_service::update_membership(
        db,
        &membership.id,
        Some(org_membership::OrgRole::Member),
        None,
        None,
    )
    .await
    .unwrap();
    let pruned = s.get(test_auth_user(&s.f.admin), &s.catalog).await.unwrap();
    assert_eq!(pruned["approver_user_ids"], json!([]));
    assert_eq!(pruned["revision"], 2);
    assert_eq!(pruned["eligible_candidates"].as_array().unwrap().len(), 1);
    assert!(matches!(edit(1).await, Err(AppError::Conflict(_))));
    assert!(matches!(edit(2).await, Err(AppError::ValidationError(_))));
    let reset = s
        .put(test_auth_user(&s.f.admin), &s.catalog, 2, Value::Null)
        .await
        .unwrap();
    assert_eq!(reset["approver_user_ids"], Value::Null);
    assert_eq!(s.stored().await.get_i64("min_grant_revision"), Ok(2));
    db.drop().await.unwrap();
}

#[test]
fn replacement_requires_explicit_list_and_rejects_unknown_keys() {
    let id = Uuid::new_v4();
    for body in [
        json!({"config_id":id,"expected_revision":0}),
        json!({"config_id":id,"expected_revision":0,"approver_user_ids":null,"extra":false}),
        json!({"expected_revision":0,"approver_user_ids":null}),
        json!({"config_id":id,"approver_user_ids":null}),
        json!({"config_id":id,"expected_revision":0,"approver_user_ids":["not a uuid"]}),
    ] {
        assert!(serde_json::from_value::<Replacement>(body).is_err());
    }
}

#[tokio::test]
async fn unavailable_approvers_explain_saved_ids_with_org_scoped_names() {
    use org_membership::OrgRole;
    let s = Settings::new("approver_unavailable_evidence").await;
    let db = &s.f.state.db;
    let users = db.collection::<Document>(user::COLLECTION_NAME);
    let memberships =
        db.collection::<org_membership::OrgMembership>(org_membership::COLLECTION_NAME);
    let configs = db.collection::<Document>(CONFIGS);
    let mut expected = serde_json::Map::new();
    let mut saved = vec![s.f.admin.clone()];
    for (name, role, active, revoked, reason) in [
        ("Former Admin", OrgRole::Admin, true, true, "not_member"),
        ("Current Member", OrgRole::Member, true, false, "not_admin"),
        ("Disabled Admin", OrgRole::Admin, false, false, "inactive"),
        ("Scoped Admin", OrgRole::Admin, true, false, "out_of_scope"),
    ] {
        let id = Uuid::new_v4().to_string();
        let mut person = test_user(&id, user::UserType::Person);
        person.display_name = Some(name.into());
        person.is_active = active;
        db.collection::<user::User>(user::COLLECTION_NAME)
            .insert_one(person)
            .await
            .unwrap();
        let mut membership = test_membership(&s.f.org, &id, role, Some(vec![]));
        membership.revoked_at = revoked.then(chrono::Utc::now);
        memberships.insert_one(membership).await.unwrap();
        expected.insert(id.clone(), json!([name, reason]));
        saved.push(id);
    }
    // Even a named member of a different org has no display-name evidence here.
    memberships
        .insert_one(test_membership(
            "other-org",
            &s.f.outsider,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    users
        .update_one(
            doc! {"_id":&s.f.outsider},
            doc! {"$set":{"display_name":"Other org only"}},
        )
        .await
        .unwrap();
    saved.push(s.f.outsider.clone());
    expected.insert(s.f.outsider.clone(), json!([null, "not_member"]));

    let missing_person = Uuid::new_v4().to_string();
    memberships
        .insert_one(test_membership(
            &s.f.org,
            &missing_person,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    expected.insert(missing_person.clone(), json!([null, "unknown"]));
    saved.push(missing_person);
    configs
        .update_one(
            doc! {"_id":&s.config},
            doc! {"$set":{"approver_user_ids":&saved}},
        )
        .await
        .unwrap();
    let get = || s.get(test_auth_user(&s.f.admin), &s.catalog);
    let result = get().await.unwrap();
    let rows = result["unavailable"].as_array().unwrap();
    assert_eq!(rows.len(), expected.len());
    for row in rows {
        assert_eq!(
            expected.remove(row["user_id"].as_str().unwrap()),
            Some(json!([row["display_name"], row["reason"]]))
        );
    }
    assert!(expected.is_empty());
    assert!(
        s.get(test_auth_user(&s.f.outsider), &s.catalog)
            .await
            .is_err()
    );

    // Cap metadata even if an older or malformed stored document exceeds the write bound.
    let oversized: Vec<_> = (0..130).map(|_| Uuid::new_v4().to_string()).collect();
    configs
        .update_one(
            doc! {"_id":&s.config},
            doc! {"$set":{"approver_user_ids":&oversized}},
        )
        .await
        .unwrap();
    let result = get().await.unwrap();
    assert_eq!(result["unavailable"].as_array().unwrap().len(), 128);
    assert_eq!(result["approver_user_ids"].as_array().unwrap().len(), 130);
    db.drop().await.unwrap();
}
