use super::*;
use crate::{
    models::{
        downstream_service::{PlatformKeyConfig, PlatformKeyPurpose, test_helpers::dummy_service},
        org_membership::{MemberScopeSource, OrgMembership, OrgRole},
        user::UserType,
    },
    test_utils,
};

async fn setup() -> (Database, String, AuditActor) {
    let db = test_utils::connect_transaction_test_database("platform_models").await;
    ensure_indexes(&db).await.unwrap();
    audit_service::init_audit_chain_hmac_key(zeroize::Zeroizing::new([71; 32]));
    let person = Uuid::new_v4().to_string();
    super::super::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let roles = super::super::role_service::get_platform_role_ids(&db)
        .await
        .unwrap();
    let mut user = test_utils::test_user(&person, UserType::Person);
    user.role_ids.push(roles.admin);
    db.collection::<User>(USERS).insert_one(user).await.unwrap();
    let actor = AuditActor::from_auth_user(&test_utils::test_auth_user(&person));
    (db, person, actor)
}
async fn model(db: &Database, actor: &AuditActor) -> PlatformModel {
    create(
        db,
        actor,
        NewModel {
            public_id: "nyxid/claude-opus-5-5".into(),
            display_name: "Claude Opus 5.5".into(),
            description: "A platform model".into(),
            family: ModelFamily::Claude,
            capabilities: Default::default(),
        },
    )
    .await
    .unwrap()
}
async fn enabled_fixture(db: &Database, actor: &AuditActor) -> PlatformModel {
    let mut row = model(db, actor).await;
    row.lifecycle = Lifecycle::Active;
    row.enabled = true;
    row.entitlements.all_people = true;
    let mut offering = dummy_service();
    offering.id = Uuid::new_v4().to_string();
    offering.is_active = true;
    offering.platform_key = Some(PlatformKeyConfig {
        purpose: PlatformKeyPurpose::ModelOffering,
        enabled: true,
        ..Default::default()
    });
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&offering)
        .await
        .unwrap();
    row.billing_service_id = Some(offering.id);
    db.collection::<PlatformModel>(COLLECTION_NAME)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    // Deliberately bypass admin activation only in tests to exercise latent ACLs.
    db.collection::<Document>(crate::models::feature_flag_override::COLLECTION_NAME).insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,"flag_key":FLAG,"target_kind":"global","target_key":bson::Bson::Null,
        "enabled":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"updated_by":&actor.user_id
    }).await.unwrap();
    row
}
async fn credential(db: &Database) -> String {
    let mut row = dummy_service();
    row.id = Uuid::new_v4().to_string();
    row.is_active = true;
    row.provider_config_id = None;
    row.visibility = "private".into();
    row.auth_method = "bearer".into();
    row.credential_encrypted = vec![1, 2, 3];
    row.secret_context_versions.clear();
    row.platform_key = Some(PlatformKeyConfig {
        purpose: PlatformKeyPurpose::ModelRoute,
        enabled: true,
        ..Default::default()
    });
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    row.id
}
fn route(credential: &str) -> ModelRoute {
    ModelRoute {
        id: Uuid::new_v4().to_string(),
        credential_service_id: credential.into(),
        enabled: true,
        priority: 0,
        weight: 1,
        target: RouteTarget::VertexPublisher {
            publisher: VertexPublisher::Anthropic,
            project: "test-project".into(),
            location: "global".into(),
            native_model: "claude-opus-5-5".into(),
        },
    }
}
fn rule(kind: SubjectKind, id: &str, effect: Effect) -> EntitlementRule {
    EntitlementRule {
        subject_kind: kind,
        subject_id: id.into(),
        effect,
    }
}

#[tokio::test]
async fn registry_cas_history_retirement_and_metadata_audit() {
    let (db, person, actor) = setup().await;
    let row = model(&db, &actor).await;
    assert!(!row.enabled);
    assert_eq!(row.lifecycle, Lifecycle::Draft);
    let raw = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(raw.get_datetime("created_at").is_ok());
    let policy = Entitlements {
        all_people: false,
        rules: vec![rule(SubjectKind::Person, &person, Effect::Allow)],
    };
    let (a, b) = tokio::join!(
        set_entitlements(&db, &actor, &row.id, 1, policy.clone()),
        set_entitlements(&db, &actor, &row.id, 1, policy)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(get(&db, &row.id).await.unwrap().revision, 2);
    let c = credential(&db).await;
    let row = set_routes(&db, &actor, &row.id, 2, vec![route(&c)])
        .await
        .unwrap();
    assert_eq!(routing(&db, &row).await.unwrap().unwrap().model_revision, 3);
    let old_revision = row.routing_revision_id.clone();
    let row = set_routes(&db, &actor, &row.id, 3, vec![]).await.unwrap();
    assert_ne!(row.routing_revision_id, old_revision);
    assert_eq!(
        db.collection::<Document>(ROUTING_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    let retired = update(
        &db,
        &actor,
        &row.id,
        ModelUpdate {
            expected_revision: 4,
            display_name: row.display_name.clone(),
            description: row.description.clone(),
            capabilities: row.capabilities.clone(),
            lifecycle: Lifecycle::Retired,
            enabled: false,
            billing_service_id: None,
        },
    )
    .await
    .unwrap();
    assert!(
        update(
            &db,
            &actor,
            &row.id,
            ModelUpdate {
                expected_revision: 5,
                display_name: row.display_name,
                description: row.description,
                capabilities: row.capabilities,
                lifecycle: Lifecycle::Draft,
                enabled: false,
                billing_service_id: None
            }
        )
        .await
        .is_err()
    );
    assert_eq!(retired.public_id, "nyxid/claude-opus-5-5");
    let logs: Vec<Document> = db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":{"$regex":"^platform_model_"}})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(logs.len(), 5);
    for log in logs {
        assert!(log.contains_key("entry_hash"));
        let data = log.get_document("event_data").unwrap();
        assert_eq!(data.len(), 3);
        assert!(!data.contains_key("routes"));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn person_default_deny_kill_switch_and_unknown_are_uniform() {
    let (db, person, actor) = setup().await;
    let mut row = enabled_fixture(&db, &actor).await;
    row.entitlements = Default::default();
    db.collection::<PlatformModel>(COLLECTION_NAME)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    assert!(matches!(
        authorized_model(&db, &person, &row.public_id).await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        authorized_model(&db, &person, "nyxid/unknown").await,
        Err(AppError::NotFound(_))
    ));
    let row = set_entitlements(
        &db,
        &actor,
        &row.id,
        1,
        Entitlements {
            all_people: false,
            rules: vec![rule(SubjectKind::Person, &person, Effect::Allow)],
        },
    )
    .await
    .unwrap();
    assert!(authorized_model(&db, &person, &row.public_id).await.is_ok());
    let row = update(
        &db,
        &actor,
        &row.id,
        ModelUpdate {
            expected_revision: 2,
            display_name: row.display_name,
            description: row.description,
            capabilities: row.capabilities,
            lifecycle: Lifecycle::Active,
            enabled: false,
            billing_service_id: row.billing_service_id,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        authorized_model(&db, &person, &row.public_id).await,
        Err(AppError::NotFound(_))
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_membership_requires_active_proxy_role_and_live_owner() {
    let (db, person, actor) = setup().await;
    let row = enabled_fixture(&db, &actor).await;
    let org = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_utils::test_user(&org, UserType::Org))
        .await
        .unwrap();
    let member = OrgMembership {
        id: Uuid::new_v4().to_string(),
        org_user_id: org.clone(),
        member_user_id: person.clone(),
        role: OrgRole::Member,
        scope_source: MemberScopeSource::Override,
        allowed_service_ids: None,
        created_at: Utc::now(),
        revoked_at: None,
    };
    db.collection::<OrgMembership>(crate::models::org_membership::COLLECTION_NAME)
        .insert_one(&member)
        .await
        .unwrap();
    let row = set_entitlements(
        &db,
        &actor,
        &row.id,
        1,
        Entitlements {
            all_people: false,
            rules: vec![rule(SubjectKind::Org, &org, Effect::Allow)],
        },
    )
    .await
    .unwrap();
    assert!(authorized_model(&db, &person, &row.public_id).await.is_ok());
    for change in [
        doc! {"role":"viewer"},
        doc! {"role":"member","revoked_at":bson::DateTime::now()},
    ] {
        db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
            .update_one(doc! {"_id":&member.id}, doc! {"$set":change})
            .await
            .unwrap();
        assert!(
            authorized_model(&db, &person, &row.public_id)
                .await
                .is_err()
        );
    }
    db.collection::<OrgMembership>(crate::models::org_membership::COLLECTION_NAME)
        .replace_one(doc! {"_id":&member.id}, &member)
        .await
        .unwrap();
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":&org}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(
        authorized_model(&db, &person, &row.public_id)
            .await
            .is_err()
    );
    assert!(EntitlementSnapshot::load(&db, &org).await.is_err());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn groups_are_direct_live_and_deny_wins() {
    let (db, person, actor) = setup().await;
    let row = enabled_fixture(&db, &actor).await;
    let group = Group {
        id: Uuid::new_v4().to_string(),
        name: "Pilot".into(),
        slug: "pilot".into(),
        description: None,
        role_ids: vec![],
        parent_group_id: Some(Uuid::new_v4().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.collection::<Group>(GROUPS)
        .insert_one(&group)
        .await
        .unwrap();
    db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&person},
            doc! {"$set":{"group_ids":[&group.id]}},
        )
        .await
        .unwrap();
    let row = set_entitlements(
        &db,
        &actor,
        &row.id,
        1,
        Entitlements {
            all_people: false,
            rules: vec![rule(SubjectKind::Group, &group.id, Effect::Allow)],
        },
    )
    .await
    .unwrap();
    assert!(authorized_model(&db, &person, &row.public_id).await.is_ok());
    let snapshot = EntitlementSnapshot::load(&db, &person).await.unwrap();
    assert_eq!(
        snapshot.policy_reason(&Entitlements {
            all_people: false,
            rules: vec![rule(
                SubjectKind::Group,
                group.parent_group_id.as_ref().unwrap(),
                Effect::Allow
            )]
        }),
        "no_entitlement"
    );
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":&person}, doc! {"$set":{"group_ids":[]}})
        .await
        .unwrap();
    assert!(
        authorized_model(&db, &person, &row.public_id)
            .await
            .is_err()
    );
    db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&person},
            doc! {"$set":{"group_ids":[&group.id]}},
        )
        .await
        .unwrap();
    db.collection::<Group>(GROUPS)
        .delete_one(doc! {"_id":&group.id})
        .await
        .unwrap();
    assert!(
        authorized_model(&db, &person, &row.public_id)
            .await
            .is_err()
    );
    db.collection::<Group>(GROUPS)
        .insert_one(&group)
        .await
        .unwrap();
    let row = set_entitlements(
        &db,
        &actor,
        &row.id,
        2,
        Entitlements {
            all_people: true,
            rules: vec![
                rule(SubjectKind::Person, &person, Effect::Allow),
                rule(SubjectKind::Group, &group.id, Effect::Deny),
            ],
        },
    )
    .await
    .unwrap();
    assert!(
        authorized_model(&db, &person, &row.public_id)
            .await
            .is_err()
    );
    db.collection::<Group>(GROUPS)
        .delete_one(doc! {"_id":&group.id})
        .await
        .unwrap();
    assert!(authorized_model(&db, &person, &row.public_id).await.is_ok());
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":&person}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(
        authorized_model(&db, &person, &row.public_id)
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn flag_is_default_off_activation_is_blocked_and_disabling_works() {
    let (db, person, actor) = setup().await;
    assert!(
        !feature_flag_service::personal_flag_enabled(&db, &person, FLAG)
            .await
            .unwrap()
    );
    assert!(
        !feature_flag_service::find_flag(FLAG)
            .unwrap()
            .default_enabled
    );
    security_readiness_service::ready_test_replica(&db).await;
    assert!(
        security_readiness_service::require(&db, REGISTRY_READINESS)
            .await
            .is_ok()
    );
    for target in [
        feature_flag_service::FlagTarget::Global,
        feature_flag_service::FlagTarget::User(person.clone()),
    ] {
        assert!(matches!(
            feature_flag_service::set_platform_override(&db, FLAG, &target, true, &person).await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        feature_flag_service::set_platform_override(&db, FLAG, &target, false, &person)
            .await
            .unwrap();
    }
    let row = model(&db, &actor).await;
    assert!(matches!(
        update(
            &db,
            &actor,
            &row.id,
            ModelUpdate {
                expected_revision: 1,
                display_name: row.display_name,
                description: row.description,
                capabilities: row.capabilities,
                lifecycle: Lifecycle::Active,
                enabled: true,
                billing_service_id: None
            }
        )
        .await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(!security_readiness_service::CAPABILITIES.contains(&EXECUTION_READINESS));
    assert!(security_readiness_service::CAPABILITIES.contains(&BILLING_READINESS));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn route_only_rejects_all_ordinary_master_gates_even_with_public_grants() {
    let (db, person, _) = setup().await;
    let id = credential(&db).await;
    let mut row: DownstreamService = db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    row.visibility = "public".into();
    row.service_category = "internal".into();
    row.requires_user_credential = false;
    row.platform_key.as_mut().unwrap().audience =
        crate::models::downstream_service::PlatformKeyAudience::Public;
    let actor = super::super::proxy_service::EffectiveActor::from_user_id(&person);
    assert!(!platform_key_service::has_platform_key(&row));
    assert!(!platform_key_service::legacy_public_master(&row));
    assert!(!platform_key_service::legacy_master_credential(&row));
    assert!(matches!(
        super::super::proxy_service::authorize_master_credential(&db, &row, &actor).await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        super::super::proxy_service::authorize_master_credential_server_chosen(&db, &row).await,
        Err(AppError::NotFound(_))
    ));
    row.platform_key.as_mut().unwrap().purpose = PlatformKeyPurpose::Ordinary;
    assert!(platform_key_service::has_platform_key(&row));
    assert!(
        super::super::proxy_service::authorize_master_credential_server_chosen(&db, &row)
            .await
            .is_ok()
    );
    let raw = serde_json::json!({"enabled":true,"audience":"public"});
    assert_eq!(
        serde_json::from_value::<PlatformKeyConfig>(raw)
            .unwrap()
            .purpose,
        PlatformKeyPurpose::Ordinary
    );
    let legacy = serde_json::json!({"enabled":true,"audience":"public","allowed_owner_ids":[]});
    let decoded: PlatformKeyConfig = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), legacy);
    assert!(!bson::to_document(&decoded).unwrap().contains_key("purpose"));
    assert_eq!(
        bson::to_document(&PlatformKeyConfig {
            purpose: PlatformKeyPurpose::ModelRoute,
            ..decoded
        })
        .unwrap()
        .get_str("purpose")
        .unwrap(),
        "model_route"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn route_plan_rechecks_entitlement_and_credential_revocation() {
    let (db, person, actor) = setup().await;
    let mut row = enabled_fixture(&db, &actor).await;
    let credential = credential(&db).await;
    let routes = RoutingRevision {
        id: Uuid::new_v4().to_string(),
        model_id: row.id.clone(),
        model_revision: 1,
        routes: vec![route(&credential)],
        created_at: Utc::now(),
    };
    db.collection::<RoutingRevision>(ROUTING_COLLECTION)
        .insert_one(&routes)
        .await
        .unwrap();
    row.routing_revision_id = Some(routes.id);
    db.collection::<PlatformModel>(COLLECTION_NAME)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    assert_eq!(
        resolve_route_metadata(&db, &person, &row.public_id, None)
            .await
            .unwrap()
            .route_count(),
        1
    );
    assert!(
        resolve_route_metadata(&db, &person, &row.public_id, Some(&HashSet::new()))
            .await
            .is_err()
    );
    db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(doc! {"_id":credential}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(
        resolve_route_metadata(&db, &person, &row.public_id, None)
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn restricted_key_grants_keep_org_resource_and_admin_boundaries() {
    let (db, person, _) = setup().await;
    let org = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_utils::test_user(&org, UserType::Org))
        .await
        .unwrap();
    let member = OrgMembership {
        id: Uuid::new_v4().to_string(),
        org_user_id: org.clone(),
        member_user_id: person.clone(),
        role: OrgRole::Member,
        scope_source: MemberScopeSource::Override,
        allowed_service_ids: Some(vec![]),
        created_at: Utc::now(),
        revoked_at: None,
    };
    let coll = db.collection::<OrgMembership>(crate::models::org_membership::COLLECTION_NAME);
    coll.insert_one(&member).await.unwrap();
    let id = Uuid::new_v4().to_string();
    let mut connection = test_utils::test_user_service(
        &id,
        &org,
        "platform-model",
        &Uuid::new_v4().to_string(),
        None,
        None,
    );
    connection.catalog_service_id = Some(Uuid::new_v4().to_string());
    connection.is_active = true;
    db.collection::<crate::models::user_service::UserService>("user_services")
        .insert_one(&connection)
        .await
        .unwrap();
    let ids = vec![id.clone()];
    assert!(
        authorized_catalog_scope(
            &db,
            &EntitlementSnapshot::load(&db, &person).await.unwrap(),
            false,
            &ids,
            &Default::default()
        )
        .await
        .unwrap()
        .unwrap()
        .is_empty()
    );
    coll.update_one(
        doc! {"_id":&member.id},
        doc! {"$set":{"allowed_service_ids":[&id]}},
    )
    .await
    .unwrap();
    assert_eq!(
        authorized_catalog_scope(
            &db,
            &EntitlementSnapshot::load(&db, &person).await.unwrap(),
            false,
            &ids,
            &Default::default()
        )
        .await
        .unwrap()
        .unwrap()
        .len(),
        1
    );
    db.collection::<Document>("user_services")
        .update_one(doc! {"_id":&id}, doc! {"$set":{"admin_only":true}})
        .await
        .unwrap();
    assert!(
        authorized_catalog_scope(
            &db,
            &EntitlementSnapshot::load(&db, &person).await.unwrap(),
            false,
            &ids,
            &Default::default()
        )
        .await
        .unwrap()
        .unwrap()
        .is_empty()
    );
    assert!(
        authorized_catalog_scope(
            &db,
            &EntitlementSnapshot::load(&db, &person).await.unwrap(),
            false,
            &[],
            &Default::default()
        )
        .await
        .unwrap()
        .unwrap()
        .is_empty()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn all_route_families_bind_exact_model_and_reject_injected_destinations() {
    let (db, _, actor) = setup().await;
    let mut row = model(&db, &actor).await;
    let mut r = route(&Uuid::new_v4().to_string());
    for target in [
        RouteTarget::VertexPublisher {
            publisher: VertexPublisher::Anthropic,
            project: "project".into(),
            location: "global".into(),
            native_model: "claude-opus-5-5@20261101".into(),
        },
        RouteTarget::DirectProvider {
            provider: DirectProvider::Anthropic,
            native_model: "claude-opus-5-5-20261101".into(),
        },
    ] {
        r.target = target;
        validate_route(&row, &r).unwrap();
    }
    r.target = RouteTarget::DirectProvider {
        provider: DirectProvider::Anthropic,
        native_model: "claude-sonnet-5".into(),
    };
    assert!(validate_route(&row, &r).is_err());
    row.family = ModelFamily::Gemini;
    row.public_id = "nyxid/gemini-3-pro".into();
    r.target = RouteTarget::VertexPublisher {
        publisher: VertexPublisher::Google,
        project: "project".into(),
        location: "us-central1".into(),
        native_model: "gemini-3-pro".into(),
    };
    validate_route(&row, &r).unwrap();
    row.family = ModelFamily::OpenModel;
    row.public_id = "nyxid/gpt-oss-120b".into();
    r.target = RouteTarget::VertexOpenapiMaas {
        project: "project".into(),
        location: "global".into(),
        publisher: "openai".into(),
        native_model: "gpt-oss-120b".into(),
    };
    validate_route(&row, &r).unwrap();
    row.family = ModelFamily::Openai;
    row.public_id = "nyxid/gpt-5".into();
    r.target = RouteTarget::DirectProvider {
        provider: DirectProvider::Openai,
        native_model: "gpt-5".into(),
    };
    validate_route(&row, &r).unwrap();
    let mut value = serde_json::to_value(&r).unwrap();
    value["target"]["url"] = "https://attacker.example".into();
    assert!(serde_json::from_value::<ModelRoute>(value).is_err());
    assert!(validate_public_id("nyxid/../secret", ModelFamily::OpenModel).is_err());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn listing_and_single_authorization_agree_without_hidden_cursors() {
    let (db, person, actor) = setup().await;
    let row = enabled_fixture(&db, &actor).await;
    let snapshot = EntitlementSnapshot::load(&db, &person).await.unwrap();
    assert_eq!(
        list_authorized(&db, &snapshot, None, None, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    // Denied rows sort before the allowed row: they cannot consume the page or
    // appear as next-page cursors even with a one-model page size.
    for index in 0..3 {
        let mut hidden = row.clone();
        hidden.id = Uuid::new_v4().to_string();
        hidden.public_id = format!("nyxid/claude-hidden-{index}");
        hidden.entitlements = Default::default();
        db.collection::<PlatformModel>(COLLECTION_NAME)
            .insert_one(hidden)
            .await
            .unwrap();
    }
    let page = list_authorized(&db, &snapshot, None, None, 1)
        .await
        .unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].public_id, row.public_id);
    assert!(
        list_authorized(&db, &snapshot, Some(&HashSet::new()), None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let row = set_entitlements(
        &db,
        &actor,
        &row.id,
        1,
        Entitlements {
            all_people: true,
            rules: vec![rule(SubjectKind::Person, &person, Effect::Deny)],
        },
    )
    .await
    .unwrap();
    assert!(
        list_authorized(&db, &snapshot, None, None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        authorized_model(&db, &person, &row.public_id)
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn invalid_policy_and_revoked_admin_cannot_change_registry() {
    let (db, person, actor) = setup().await;
    let row = model(&db, &actor).await;
    let unknown = Uuid::new_v4().to_string();
    assert!(
        set_entitlements(
            &db,
            &actor,
            &row.id,
            1,
            Entitlements {
                all_people: false,
                rules: vec![rule(SubjectKind::Person, &unknown, Effect::Allow)]
            }
        )
        .await
        .is_err()
    );
    assert!(
        set_entitlements(
            &db,
            &actor,
            &row.id,
            1,
            Entitlements {
                all_people: false,
                rules: vec![
                    rule(SubjectKind::Person, &person, Effect::Allow),
                    rule(SubjectKind::Person, &person, Effect::Deny)
                ]
            }
        )
        .await
        .is_err()
    );
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":person}, doc! {"$set":{"role_ids":[]}})
        .await
        .unwrap();
    assert!(matches!(
        set_entitlements(
            &db,
            &actor,
            &row.id,
            1,
            Entitlements {
                all_people: true,
                rules: vec![]
            }
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(get(&db, &row.id).await.unwrap().revision, 1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn route_only_service_is_never_callable_through_proxy_or_mcp() {
    use crate::services::{mcp_service, node_ws_manager::NodeWsManager, proxy_service};
    let (db, person, _) = setup().await;
    let id = credential(&db).await;
    let endpoint = Uuid::new_v4().to_string();
    let connection = Uuid::new_v4().to_string();
    db.collection::<crate::models::service_endpoint::ServiceEndpoint>(
        crate::models::service_endpoint::COLLECTION_NAME,
    )
    .insert_one(test_utils::test_service_endpoint(
        &id,
        "chat",
        "POST",
        "/v1/messages",
    ))
    .await
    .unwrap();
    let row = test_utils::test_user_endpoint(
        &endpoint,
        &person,
        "Hidden route",
        "https://example.test",
        None,
        Some(&id),
    );
    db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
        .insert_one(row)
        .await
        .unwrap();
    let mut row = test_utils::test_user_service(
        &connection,
        &person,
        "hidden-route",
        &endpoint,
        Some(&id),
        None,
    );
    row.credential_binding = Some("platform".into());
    row.auth_method = "bearer".into();
    row.is_active = true;
    db.collection::<crate::models::user_service::UserService>("user_services")
        .insert_one(row)
        .await
        .unwrap();
    let keys = test_utils::test_encryption_keys();
    assert!(matches!(
        super::super::tool_twin_service::prepare_create_for_actor(
            &db,
            Some(&keys),
            crate::crypto::secret_json::SecretJson(
                serde_json::json!({"twin_of_service_id":id,"slug":"leaked-route"})
            ),
            None
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        proxy_service::resolve_proxy_target(
            &db,
            &keys,
            &person,
            &id,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled()
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        proxy_service::resolve_admin_proxy_target(&db, &keys, &id).await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        proxy_service::read_default_model_authority(&db, &person, "hidden-route").await,
        Err(AppError::NotFound(_))
    ));
    let nodes = NodeWsManager::new(30, 10);
    let catalog = mcp_service::load_operation_catalog(
        &db,
        &nodes,
        &person,
        mcp_service::NodeScope::Unrestricted,
        mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    assert!(catalog.services.iter().all(|s| s.service_id != connection));
    assert!(
        catalog
            .unpublished_services
            .iter()
            .all(|s| s.service_id != connection)
    );
    assert!(
        catalog
            .unavailable_connections
            .iter()
            .all(|s| s.service_id != connection)
    );
    assert!(catalog.diagnostics.no_visible_connections);
    // Control: an ordinary platform service with the same published operation
    // is discoverable. The test is not passing because no operation exists.
    db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"platform_key.purpose":"ordinary","platform_key.audience":"public"}},
        )
        .await
        .unwrap();
    let catalog = mcp_service::load_operation_catalog(
        &db,
        &nodes,
        &person,
        mcp_service::NodeScope::Unrestricted,
        mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    assert!(catalog.services.iter().any(|s| s.service_id == connection));
    db.drop().await.unwrap();
}
