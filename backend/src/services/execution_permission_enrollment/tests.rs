use super::*;
use crate::{
    models::{
        execution_enrollment_job::{COLLECTION_NAME as JOBS, ExecutionEnrollmentJob as Job},
        org_membership::MemberScopeSource,
        user::UserType,
    },
    services::{
        audit_service::AuditActor,
        feature_flag_admin_service::{self as admin, Mutation},
        org_service, security_readiness_service as readiness,
    },
    test_utils::{
        connect_transaction_test_database, init_test_audit_chain_hmac_key, test_membership,
        test_user,
    },
};
use chrono::Utc;
use uuid::Uuid;

fn row(
    kind: FlagTargetKind,
    target: Option<&str>,
    org: Option<&str>,
    enabled: bool,
) -> FeatureFlagOverride {
    FeatureFlagOverride {
        id: Uuid::new_v4().to_string(),
        flag_key: FLAG.into(),
        definition_id: None,
        org_user_id: org.map(str::to_owned),
        target_kind: kind,
        target_key: target.map(str::to_owned),
        enabled,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        updated_by: "test".into(),
    }
}

struct Fixture {
    db: Database,
    person: String,
    org: String,
    actor: AuditActor,
}
impl Fixture {
    async fn new(name: &str) -> Self {
        init_test_audit_chain_hmac_key();
        let db = connect_transaction_test_database(name).await;
        jobs::ensure_indexes(&db).await.unwrap();
        let person = Uuid::new_v4().to_string();
        let org = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&person, UserType::Person),
                test_user(&org, UserType::Org),
            ])
            .await
            .unwrap();
        readiness::ready_test_replica(&db).await;
        let actor = AuditActor {
            user_id: person.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        Self {
            db,
            person,
            org,
            actor,
        }
    }
    async fn change(
        &self,
        kind: FlagTargetKind,
        target: Option<&str>,
        enabled: Option<bool>,
    ) -> AppResult<()> {
        admin::mutate(
            &self.db,
            FLAG,
            Mutation::Override {
                kind,
                target_id: target.map(str::to_owned),
                enabled,
            },
            &self.actor,
        )
        .await
        .map(|_| ())
    }
    async fn marker(&self, person: &str) -> Option<u32> {
        self.db
            .collection::<User>(USERS)
            .find_one(doc! {"_id":person})
            .await
            .unwrap()
            .unwrap()
            .execution_permission_enrollment
    }
    async fn membership(&self, role: OrgRole) -> OrgMembership {
        org_service::create_membership(
            &self.db,
            &self.org,
            &self.person,
            role,
            MemberScopeSource::Inherit,
            None,
        )
        .await
        .unwrap()
    }
}

#[test]
fn single_flag_precedence_including_viewers_and_legacy_scopes() {
    let membership = vec![("a".into(), OrgRole::Viewer), ("b".into(), OrgRole::Member)];
    for global in [None, Some(false), Some(true)] {
        for org_a in [None, Some(false), Some(true)] {
            for org_b in [None, Some(false), Some(true)] {
                for user in [None, Some(false), Some(true)] {
                    let mut rows = Vec::new();
                    if let Some(on) = global {
                        rows.push(row(FlagTargetKind::Global, None, None, on));
                    }
                    if let Some(on) = org_a {
                        rows.push(row(FlagTargetKind::Org, None, Some("a"), on));
                    }
                    if let Some(on) = org_b {
                        rows.push(row(FlagTargetKind::Org, None, Some("b"), on));
                    }
                    if let Some(on) = user {
                        rows.push(row(FlagTargetKind::User, Some("p"), None, on));
                    }
                    let org = if org_a == Some(false) || org_b == Some(false) {
                        Some(false)
                    } else {
                        org_a.or(org_b)
                    };
                    assert_eq!(
                        resolve(&rows, &membership, "p", true),
                        user.or(org).or(global).unwrap_or(false)
                    );
                    assert_eq!(
                        resolve(&rows, &membership, "p", false),
                        user.or(org).unwrap_or(false)
                    );
                }
            }
        }
    }
    let mut rows = vec![
        row(FlagTargetKind::Org, None, Some("a"), true),
        row(FlagTargetKind::Role, Some("viewer"), Some("a"), false),
    ];
    assert!(!resolve(&rows, &membership, "p", true));
    rows.push(row(FlagTargetKind::User, Some("p"), Some("a"), true));
    assert!(resolve(&rows, &membership, "p", true));
    rows.push(row(FlagTargetKind::User, Some("p"), None, false));
    assert!(!resolve(&rows, &membership, "p", true));
}

#[test]
fn snapshot_candidates_are_db_free_bounded_and_never_infer_org_membership() {
    let cache = snapshot::Cache::default();
    let mut person = test_user("p", UserType::Person);
    assert!(!cache.candidate(&person, &[]));
    let global = row(FlagTargetKind::Global, None, None, true);
    let org = row(FlagTargetKind::Org, None, Some("org"), true);
    cache.replace(Some(vec![org.clone()]));
    assert!(!cache.candidate(&person, &[]));
    assert!(!cache.candidate(&person, &[("org".into(), OrgRole::Member)]));
    cache.replace(Some(vec![
        global.clone(),
        row(FlagTargetKind::Org, None, Some("org"), false),
    ]));
    assert!(cache.candidate(&person, &[])); // unknown membership is only a hint
    assert!(!cache.candidate(&person, &[("org".into(), OrgRole::Viewer)]));
    cache.replace(Some(vec![
        global.clone(),
        row(FlagTargetKind::User, Some("p"), None, false),
    ]));
    assert!(!cache.candidate(&person, &[]));
    cache.replace(Some(vec![global; 10_001]));
    assert!(!cache.candidate(&person, &[]));
    person.execution_permission_enrollment = Some(1);
    assert!(cache.candidate(&person, &[])); // existing marker survives cold/overflow
    person.is_active = false;
    assert!(!cache.candidate(&person, &[]));
    let mut organization = test_user("org", UserType::Org);
    organization.execution_permission_enrollment = Some(1);
    assert!(!cache.candidate(&organization, &[]));
}

#[tokio::test]
async fn global_changes_write_no_markers_or_jobs_and_live_resolution_catches_cached_denies() {
    let f = Fixture::new("enrollment_global").await;
    let cache = snapshot::Cache::default();
    f.change(FlagTargetKind::Global, None, Some(true))
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    assert_eq!(
        f.db.collection::<Job>(JOBS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    cache.refresh(&f.db).await.unwrap();
    let person = test_user(&f.person, UserType::Person);
    assert!(cache.candidate(&person, &[]));
    assert!(live_enabled(&f.db, &f.person, None).await.unwrap());
    f.change(FlagTargetKind::User, Some(&f.person), Some(false))
        .await
        .unwrap();
    assert!(cache.candidate(&person, &[])); // intentionally stale positive
    assert!(!live_enabled(&f.db, &f.person, None).await.unwrap());
    cache.refresh(&f.db).await.unwrap();
    assert!(!cache.candidate(&person, &[]));
    f.change(FlagTargetKind::User, Some(&f.person), None)
        .await
        .unwrap();
    assert!(live_enabled(&f.db, &f.person, None).await.unwrap());
    assert_eq!(f.marker(&f.person).await, None); // global does not need a marker
    f.change(FlagTargetKind::Global, None, Some(false))
        .await
        .unwrap();
    assert!(!live_enabled(&f.db, &f.person, None).await.unwrap());
    assert_eq!(
        f.db.collection::<Job>(JOBS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_jobs_are_scoped_coalesced_and_clear_restores_inheritance() {
    let f = Fixture::new("enrollment_org_jobs").await;
    f.membership(OrgRole::Viewer).await;
    let outsider = Uuid::new_v4().to_string();
    f.db.collection::<User>(USERS)
        .insert_one(test_user(&outsider, UserType::Person))
        .await
        .unwrap();
    f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    let job = jobs::claim(&f.db).await.unwrap().unwrap();
    assert!(jobs::claim(&f.db).await.unwrap().is_none());
    assert!(jobs::batch(&f.db, &job).await.unwrap());
    assert_eq!(f.marker(&f.person).await, Some(1));
    assert_eq!(f.marker(&outsider).await, None);
    let progress = jobs::progress(&f.db).await.unwrap();
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].state, "complete");
    assert_eq!(progress[0].processed, 1);
    f.change(FlagTargetKind::User, Some(&f.person), Some(false))
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    f.change(FlagTargetKind::User, Some(&f.person), None)
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, Some(1));
    f.change(FlagTargetKind::Org, Some(&f.org), Some(false))
        .await
        .unwrap();
    f.change(FlagTargetKind::Org, Some(&f.org), None)
        .await
        .unwrap();
    assert_eq!(jobs::progress(&f.db).await.unwrap().len(), 1);
    let job = jobs::claim(&f.db).await.unwrap().unwrap();
    jobs::batch(&f.db, &job).await.unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn membership_join_role_revoke_reactivate_and_disabled_person_recompute_atomically() {
    let f = Fixture::new("enrollment_lifecycle").await;
    f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
        .await
        .unwrap();
    let membership = f.membership(OrgRole::Member).await;
    assert_eq!(f.marker(&f.person).await, Some(1));
    // Legacy role rows remain part of the existing model.
    f.db.collection::<FeatureFlagOverride>(OVERRIDES)
        .insert_one(row(
            FlagTargetKind::Role,
            Some("viewer"),
            Some(&f.org),
            false,
        ))
        .await
        .unwrap();
    org_service::update_membership(&f.db, &membership.id, Some(OrgRole::Viewer), None, None)
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    org_service::update_membership(&f.db, &membership.id, Some(OrgRole::Member), None, None)
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, Some(1));
    org_service::revoke_membership(&f.db, &f.org, &f.person)
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    assert!(!live_enabled(&f.db, &f.person, None).await.unwrap());
    f.membership(OrgRole::Member).await;
    assert_eq!(f.marker(&f.person).await, Some(1));
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    crate::services::admin_user_service::apply_user_security_state_in_session(
        &f.db,
        &mut session,
        &f.person,
        false,
        false,
        None,
        Some(true),
    )
    .await
    .unwrap();
    session.abort_transaction().await.unwrap();
    assert_eq!(f.marker(&f.person).await, Some(1));
    session.start_transaction().await.unwrap();
    crate::services::admin_user_service::apply_user_security_state_in_session(
        &f.db,
        &mut session,
        &f.person,
        false,
        false,
        None,
        Some(true),
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    assert!(!live_enabled(&f.db, &f.person, None).await.unwrap());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn expired_lease_resumes_but_superseded_worker_cannot_write_hints_or_progress() {
    let f = Fixture::new("enrollment_lease").await;
    f.membership(OrgRole::Member).await;
    f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
        .await
        .unwrap();
    let first = jobs::claim(&f.db).await.unwrap().unwrap();
    f.db.collection::<Job>(JOBS)
        .update_one(
            doc! {"_id":&first.id},
            doc! {"$set":{"lease_until":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let resumed = jobs::claim(&f.db).await.unwrap().unwrap();
    assert_ne!(first.lease_token, resumed.lease_token);
    assert!(!jobs::batch(&f.db, &first).await.unwrap());
    f.change(FlagTargetKind::Org, Some(&f.org), Some(false))
        .await
        .unwrap();
    assert!(!jobs::batch(&f.db, &resumed).await.unwrap());
    assert_eq!(f.marker(&f.person).await, None);
    let fresh = jobs::claim(&f.db).await.unwrap().unwrap();
    assert_ne!(fresh.generation, resumed.generation);
    jobs::batch(&f.db, &fresh).await.unwrap();
    assert_eq!(jobs::progress(&f.db).await.unwrap()[0].processed, 1);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn keyset_batches_cover_org_members_only_and_preserve_cursor_after_expiry() {
    let f = Fixture::new("enrollment_batches").await;
    let mut people = Vec::new();
    let mut memberships = Vec::new();
    for i in 0..103 {
        let id = format!("member-{i:03}");
        people.push(test_user(&id, UserType::Person));
        memberships.push(test_membership(&f.org, &id, OrgRole::Viewer, None));
    }
    f.db.collection::<User>(USERS)
        .insert_many(people)
        .await
        .unwrap();
    f.db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_many(memberships)
        .await
        .unwrap();
    f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
        .await
        .unwrap();
    let first = jobs::claim(&f.db).await.unwrap().unwrap();
    jobs::batch(&f.db, &first).await.unwrap();
    let progress = jobs::progress(&f.db).await.unwrap();
    assert_eq!(progress[0].processed, 100);
    assert_eq!(progress[0].state, "pending");
    let next = jobs::claim(&f.db).await.unwrap().unwrap();
    assert_eq!(next.cursor.as_deref(), Some("member-099"));
    jobs::batch(&f.db, &next).await.unwrap();
    let progress = jobs::progress(&f.db).await.unwrap();
    assert_eq!(progress[0].processed, 103);
    assert_eq!(progress[0].state, "complete");
    assert_eq!(
        f.db.collection::<User>(USERS)
            .count_documents(doc! {"execution_permission_enrollment":1})
            .await
            .unwrap(),
        103
    );
    assert_eq!(f.marker(&f.person).await, None);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn readiness_gates_broad_enables_and_clear_that_widens_but_not_disable() {
    let f = Fixture::new("enrollment_readiness").await;
    f.change(FlagTargetKind::Global, None, Some(true))
        .await
        .unwrap();
    f.change(FlagTargetKind::User, Some(&f.person), Some(false))
        .await
        .unwrap();
    f.db.collection::<Document>("security_replicas")
        .update_many(doc! {}, doc! {"$pull":{"capabilities":READINESS}})
        .await
        .unwrap();
    assert!(matches!(
        f.change(FlagTargetKind::User, Some(&f.person), None).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(!live_enabled(&f.db, &f.person, None).await.unwrap());
    assert!(matches!(
        f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
            .await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    f.change(FlagTargetKind::Global, None, Some(false))
        .await
        .unwrap();
    f.change(FlagTargetKind::User, Some(&f.person), None)
        .await
        .unwrap(); // clear inherits false
    f.change(FlagTargetKind::Org, Some(&f.org), Some(false))
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn signup_keeps_plain_insert_without_enrollment_reads_or_transactions() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let f = Fixture::new("enrollment_signup").await;
    f.change(FlagTargetKind::Global, None, Some(true))
        .await
        .unwrap();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let observed = commands.clone();
    let name = f.db.name().to_owned();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && event.db == name
        {
            observed.lock().unwrap().push(event.command);
        }
    }));
    let db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.db.name());
    let created = crate::services::auth_service::register_user(
        &db,
        "new@example.test",
        // Runtime test credential; never a fixed password.
        &crate::crypto::token::generate_random_token(),
        None,
        None,
        true,
    )
    .await
    .unwrap();
    assert!(created.actually_created);
    let commands = commands.lock().unwrap().clone();
    assert!(
        commands
            .iter()
            .all(|command| !command.contains_key("autocommit"))
    );
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.get_str("insert").ok() == Some(USERS))
            .count(),
        1
    );
    for collection in [
        MEMBERSHIPS,
        OVERRIDES,
        JOBS,
        crate::models::execution_enrollment_repair::COLLECTION_NAME,
    ] {
        assert!(
            !commands
                .iter()
                .any(|command| command.get_str("find").ok() == Some(collection))
        );
    }
    assert_eq!(f.marker(&created.user_id).await, None);
    assert!(live_enabled(&f.db, &created.user_id, None).await.unwrap());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn incomplete_live_scope_never_becomes_allow_and_cannot_block_hint_cleanup() {
    let f = Fixture::new("enrollment_bounds").await;
    f.change(FlagTargetKind::Global, None, Some(true))
        .await
        .unwrap();
    let rows: Vec<_> = (0..257)
        .map(|i| test_membership(&format!("org-{i}"), &f.person, OrgRole::Member, None))
        .collect();
    f.db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_many(rows)
        .await
        .unwrap();
    assert!(live_enabled(&f.db, &f.person, None).await.is_err());
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert_eq!(
        recompute_in_session(&f.db, &f.person, &mut session)
            .await
            .unwrap(),
        None
    );
    session.commit_transaction().await.unwrap();
    f.change(FlagTargetKind::User, Some(&f.person), Some(false))
        .await
        .unwrap();
    f.change(FlagTargetKind::User, Some(&f.person), Some(false))
        .await
        .unwrap(); // idempotent disable
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn snapshot_refresh_includes_deletes_and_invalidates_on_database_failure() {
    let f = Fixture::new("enrollment_refresh").await;
    let cache = snapshot::Cache::default();
    f.change(FlagTargetKind::Global, None, Some(true))
        .await
        .unwrap();
    cache.refresh(&f.db).await.unwrap();
    let person = test_user(&f.person, UserType::Person);
    assert!(cache.candidate(&person, &[]));
    f.change(FlagTargetKind::Global, None, None).await.unwrap();
    cache.refresh(&f.db).await.unwrap();
    assert!(!cache.candidate(&person, &[]));
    let mut options =
        mongodb::options::ClientOptions::parse("mongodb://127.0.0.1:1/?directConnection=true")
            .await
            .unwrap();
    options.server_selection_timeout = Some(std::time::Duration::from_millis(50));
    let unavailable = mongodb::Client::with_options(options)
        .unwrap()
        .database("unavailable");
    cache.replace(Some(vec![row(FlagTargetKind::Global, None, None, true)]));
    assert!(cache.refresh(&unavailable).await.is_err());
    assert!(!cache.candidate(&person, &[]));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn live_read_budget_is_constant_and_ordinary_snapshot_checks_issue_zero_commands() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let f = Fixture::new("enrollment_query_budget").await;
    f.change(FlagTargetKind::Global, None, Some(true))
        .await
        .unwrap();
    let memberships: Vec<_> = (0..30)
        .map(|i| test_membership(&format!("o-{i}"), &f.person, OrgRole::Viewer, None))
        .collect();
    f.db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_many(memberships)
        .await
        .unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let events = captured.clone();
    let name = f.db.name().to_string();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && event.db == name
        {
            events.lock().unwrap().push(event.command_name);
        }
    }));
    let db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.db.name());
    let cache = snapshot::Cache::default();
    cache.refresh(&db).await.unwrap();
    captured.lock().unwrap().clear();
    let person = test_user(&f.person, UserType::Person);
    for _ in 0..100 {
        assert!(cache.candidate(&person, &[]));
    }
    assert!(captured.lock().unwrap().is_empty());
    assert!(live_enabled(&db, &f.person, None).await.unwrap());
    assert_eq!(*captured.lock().unwrap(), vec!["find", "find", "find"]);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_clear_widening_is_gated_and_job_enqueue_rolls_back_with_override() {
    let f = Fixture::new("enrollment_clear_org").await;
    f.membership(OrgRole::Viewer).await;
    f.change(FlagTargetKind::Global, None, Some(true))
        .await
        .unwrap();
    f.change(FlagTargetKind::Org, Some(&f.org), Some(false))
        .await
        .unwrap();
    let generation = jobs::progress(&f.db).await.unwrap()[0].generation.clone();
    f.db.collection::<Document>("security_replicas")
        .update_many(doc! {}, doc! {"$pull":{"capabilities":READINESS}})
        .await
        .unwrap();
    assert!(matches!(
        f.change(FlagTargetKind::Org, Some(&f.org), None).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(!live_enabled(&f.db, &f.person, None).await.unwrap());
    assert_eq!(
        jobs::progress(&f.db).await.unwrap()[0].generation,
        generation
    );
    readiness::ready_test_replica(&f.db).await;
    f.change(FlagTargetKind::Org, Some(&f.org), None)
        .await
        .unwrap();
    assert!(live_enabled(&f.db, &f.person, None).await.unwrap());
    assert_ne!(
        jobs::progress(&f.db).await.unwrap()[0].generation,
        generation
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn periodic_org_repair_covers_late_members_without_population_scan() {
    let f = Fixture::new("enrollment_org_repair").await;
    f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
        .await
        .unwrap();
    let job = jobs::claim(&f.db).await.unwrap().unwrap();
    jobs::batch(&f.db, &job).await.unwrap();
    // Simulate an old membership writer/race that committed behind the cursor.
    f.db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(&f.org, &f.person, OrgRole::Member, None))
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    f.db.collection::<Job>(JOBS)
        .update_one(
            doc! {"_id":&job.id},
            doc! {"$set":{"lease_until":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let repair = jobs::claim(&f.db).await.unwrap().unwrap();
    assert!(repair.cursor.is_none());
    jobs::batch(&f.db, &repair).await.unwrap();
    assert_eq!(f.marker(&f.person).await, Some(1));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn snapshot_worker_observes_changes_and_cleans_up_on_abort() {
    let f = Fixture::new("enrollment_stream").await;
    let cache = snapshot::Cache::default();
    let worker_cache = cache.clone();
    let db = f.db.clone();
    let worker = tokio::spawn(async move { worker_cache.run(db).await });
    let person = test_user(&f.person, UserType::Person);
    f.change(FlagTargetKind::Global, None, Some(true))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while !cache.candidate(&person, &[]) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    f.change(FlagTargetKind::Global, None, None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while cache.candidate(&person, &[]) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    worker.abort();
    let _ = worker.await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn production_heartbeat_advertises_complete_broad_enrollment() {
    let f = Fixture::new("enrollment_staged_readiness").await;
    readiness::observe(
        &f.db,
        &crate::services::node_owner_service::ReplicaIdentity {
            instance_name: "test-backend".into(),
            generation_id: "test-process".into(),
            internal_base_url: "http://127.0.0.1:3002".into(),
        },
        &readiness::RolloutConfig {
            expected_replicas: vec!["test-backend".into()],
            drained: true,
        },
        false,
    )
    .await
    .unwrap();
    assert!(readiness::report(&f.db, READINESS).await.unwrap().ready);
    assert!(readiness::report(&f.db, FLAG).await.unwrap().ready);
    f.change(FlagTargetKind::Global, None, Some(true))
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    f.change(FlagTargetKind::User, Some(&f.person), Some(true))
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, Some(1));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn personal_override_still_wins_when_org_scope_exceeds_live_limit() {
    let f = Fixture::new("enrollment_overflow_personal").await;
    let rows: Vec<_> = (0..257)
        .map(|i| test_membership(&format!("org-{i}"), &f.person, OrgRole::Member, None))
        .collect();
    f.db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_many(rows)
        .await
        .unwrap();
    for enabled in [true, false] {
        f.change(FlagTargetKind::User, Some(&f.person), Some(enabled))
            .await
            .unwrap();
        assert_eq!(live_enabled(&f.db, &f.person, None).await.unwrap(), enabled);
        assert_eq!(f.marker(&f.person).await, enabled.then_some(1));
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn unrelated_legacy_org_user_overrides_do_not_exhaust_the_live_read_budget() {
    let f = Fixture::new("enrollment_unrelated_overrides").await;
    f.membership(OrgRole::Viewer).await;
    let mut overrides: Vec<_> = (0..MAX_SCOPE_ROWS + 1)
        .map(|i| {
            row(
                FlagTargetKind::User,
                Some(&format!("other-{i}")),
                Some(&f.org),
                false,
            )
        })
        .collect();
    overrides.push(row(FlagTargetKind::Org, None, Some(&f.org), true));
    f.db.collection::<FeatureFlagOverride>(OVERRIDES)
        .insert_many(overrides)
        .await
        .unwrap();
    assert!(live_enabled(&f.db, &f.person, None).await.unwrap());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_deletion_retains_person_repair_after_membership_cascade() {
    let f = Fixture::new("enrollment_org_delete").await;
    f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
        .await
        .unwrap();
    f.membership(OrgRole::Member).await;
    assert_eq!(f.marker(&f.person).await, Some(1));
    org_service::delete_org_user(&f.db, &f.org).await.unwrap();
    // A durable actor receipt survives the membership cascade; deletion never
    // waits for 100 per-person hint reads inside its transaction.
    assert_eq!(f.marker(&f.person).await, Some(1));
    assert!(!live_enabled(&f.db, &f.person, None).await.unwrap());
    lifecycle::repair_due(&f.db).await.unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    let job = jobs::claim(&f.db).await.unwrap().unwrap();
    jobs::batch(&f.db, &job).await.unwrap();
    assert!(jobs::progress(&f.db).await.unwrap().is_empty());
    assert!(!live_enabled(&f.db, &f.person, None).await.unwrap());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn membership_overflow_commits_join_role_revoke_and_reactivation_with_coalesced_repair() {
    use crate::models::execution_enrollment_repair::COLLECTION_NAME as REPAIRS;
    let f = Fixture::new("hint_overflow_action").await;
    f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
        .await
        .unwrap();
    f.db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_many(
            (0..257)
                .map(|i| test_membership(&format!("extra-{i}"), &f.person, OrgRole::Member, None)),
        )
        .await
        .unwrap();
    let member = f.membership(OrgRole::Member).await;
    let changed =
        org_service::update_membership(&f.db, &member.id, Some(OrgRole::Viewer), None, None)
            .await
            .unwrap();
    assert_eq!(changed.role, OrgRole::Viewer);
    org_service::revoke_membership(&f.db, &f.org, &f.person)
        .await
        .unwrap();
    let revoked =
        f.db.collection::<OrgMembership>(MEMBERSHIPS)
            .find_one(doc! {"_id":&member.id})
            .await
            .unwrap()
            .unwrap();
    assert!(revoked.revoked_at.is_some());
    assert_eq!(f.membership(OrgRole::Member).await.id, member.id);
    assert_eq!(f.marker(&f.person).await, None);
    assert_eq!(jobs::progress(&f.db).await.unwrap().len(), 1);
    assert_eq!(
        f.db.collection::<Document>(REPAIRS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    f.db.collection::<OrgMembership>(MEMBERSHIPS)
        .delete_many(doc! {"org_user_id":{"$ne":&f.org}})
        .await
        .unwrap();
    lifecycle::repair_due(&f.db).await.unwrap();
    assert_eq!(f.marker(&f.person).await, Some(1));
    assert_eq!(
        f.db.collection::<Document>(REPAIRS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn membership_hint_deadline_does_not_abort_its_transaction_and_repairs_afterward() {
    use crate::models::execution_enrollment_repair::COLLECTION_NAME as REPAIRS;
    let f = Fixture::new("hint_deadline_action").await;
    f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
        .await
        .unwrap();
    lifecycle::DELAY_READ.insert(f.db.name().to_owned());
    let member = f.membership(OrgRole::Member).await;
    lifecycle::DELAY_READ.remove(f.db.name());
    assert!(
        f.db.collection::<OrgMembership>(MEMBERSHIPS)
            .find_one(doc! {"_id":&member.id,"revoked_at":null})
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(f.marker(&f.person).await, None);
    assert_eq!(jobs::progress(&f.db).await.unwrap()[0].state, "pending");
    assert_eq!(
        f.db.collection::<Document>(REPAIRS)
            .count_documents(doc! {"actor_id":&f.person})
            .await
            .unwrap(),
        1
    );
    lifecycle::repair_due(&f.db).await.unwrap();
    assert_eq!(f.marker(&f.person).await, Some(1));
    assert_eq!(
        f.db.collection::<Document>(REPAIRS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn account_disable_needs_no_hint_read_and_reactivation_deadline_defers_repair() {
    use crate::models::execution_enrollment_repair::COLLECTION_NAME as REPAIRS;
    let f = Fixture::new("hint_account_deadline").await;
    f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
        .await
        .unwrap();
    f.membership(OrgRole::Member).await;
    assert_eq!(f.marker(&f.person).await, Some(1));
    lifecycle::DELAY_READ.insert(f.db.name().to_owned());
    crate::services::admin_user_service::set_user_active(&f.db, "admin", &f.person, false)
        .await
        .unwrap();
    assert_eq!(f.marker(&f.person).await, None);
    assert_eq!(
        f.db.collection::<Document>(REPAIRS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    crate::services::admin_user_service::set_user_active(&f.db, "admin", &f.person, true)
        .await
        .unwrap();
    lifecycle::DELAY_READ.remove(f.db.name());
    assert!(
        f.db.collection::<User>(USERS)
            .find_one(doc! {"_id":&f.person})
            .await
            .unwrap()
            .unwrap()
            .is_active
    );
    assert_eq!(
        f.db.collection::<Document>(REPAIRS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    lifecycle::repair_due(&f.db).await.unwrap();
    assert_eq!(f.marker(&f.person).await, Some(1));
    assert_eq!(
        f.db.collection::<Document>(REPAIRS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn repair_write_error_still_aborts_membership_and_its_hint_job() {
    use crate::models::execution_enrollment_repair::COLLECTION_NAME as REPAIRS;
    let f = Fixture::new("hint_write_failure").await;
    f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
        .await
        .unwrap();
    let generation = jobs::progress(&f.db).await.unwrap()[0].generation.clone();
    f.db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_many(
            (0..257)
                .map(|i| test_membership(&format!("extra-{i}"), &f.person, OrgRole::Member, None)),
        )
        .await
        .unwrap();
    // Fixture-local validator, not a server-wide failpoint. Only the repair
    // write fails; the derived read correctly degrades to a repair request.
    f.db.run_command(
        doc! {"collMod":REPAIRS,"validator":{"actor_id":"unmatchable"},"validationLevel":"strict"},
    )
    .await
    .unwrap();
    let result = org_service::create_membership(
        &f.db,
        &f.org,
        &f.person,
        OrgRole::Member,
        MemberScopeSource::Inherit,
        None,
    )
    .await;
    assert!(result.is_err());
    assert!(
        f.db.collection::<OrgMembership>(MEMBERSHIPS)
            .find_one(doc! {"org_user_id":&f.org,"member_user_id":&f.person})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        jobs::progress(&f.db).await.unwrap()[0].generation,
        generation
    );
    assert_eq!(
        f.db.collection::<Document>(REPAIRS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn broad_auth_candidates_add_zero_reads_and_org_markers_remain_independent() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let f = Fixture::new("broad_auth_budget").await;
    let commands = Arc::new(Mutex::new(Vec::new()));
    let observed = commands.clone();
    let name = f.db.name().to_owned();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && event.db == name
        {
            observed.lock().unwrap().push((
                event.command_name,
                event.command.get_str("find").ok().map(str::to_owned),
            ));
        }
    }));
    let db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.db.name());
    let state = crate::test_utils::test_app_state(db.clone());
    let created = crate::services::key_service::create_api_key(
        &db,
        &f.person,
        "budget",
        "proxy",
        None,
        None,
        None,
        None,
        Some(true),
        Some(false),
        Some(true),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let key = crate::services::key_service::get_api_key(&db, &f.person, &created.id)
        .await
        .unwrap();
    let mut baseline = None;
    for scenario in ["cold", "global", "user_deny", "org_marker"] {
        match scenario {
            "global" => {
                f.change(FlagTargetKind::Global, None, Some(true))
                    .await
                    .unwrap();
                state.execution_enrollment.refresh(&db).await.unwrap();
            }
            "user_deny" => {
                f.change(FlagTargetKind::User, Some(&f.person), Some(false))
                    .await
                    .unwrap();
                state.execution_enrollment.refresh(&db).await.unwrap();
            }
            "org_marker" => {
                f.change(FlagTargetKind::User, Some(&f.person), None)
                    .await
                    .unwrap();
                f.change(FlagTargetKind::Global, None, Some(false))
                    .await
                    .unwrap();
                f.membership(OrgRole::Viewer).await;
                f.change(FlagTargetKind::Org, Some(&f.org), Some(true))
                    .await
                    .unwrap();
                let job = jobs::claim(&db).await.unwrap().unwrap();
                jobs::batch(&db, &job).await.unwrap();
                state.execution_enrollment.replace(None);
                assert_eq!(f.marker(&f.person).await, Some(1));
            }
            _ => {}
        }
        commands.lock().unwrap().clear();
        let auth = crate::mw::auth::api_key_auth_user(&db, &key, None, None, None)
            .await
            .unwrap();
        let actual = commands.lock().unwrap().clone();
        let expected = matches!(scenario, "global" | "org_marker");
        assert_eq!(auth.execution_enrolled, expected, "{scenario}");
        assert_eq!(auth.execution_permission.is_some(), expected, "{scenario}");
        if let Some(baseline) = &baseline {
            assert_eq!(
                &actual, baseline,
                "{scenario}: authentication adds no commands"
            );
        } else {
            assert_eq!(actual, vec![("find".into(), Some("users".into()))]);
            baseline = Some(actual);
        }
    }
    f.db.drop().await.unwrap();
}
