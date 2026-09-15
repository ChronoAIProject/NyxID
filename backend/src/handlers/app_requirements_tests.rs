use super::*;
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, Document};
use uuid::Uuid;

use crate::models::app_requirement_manifest::{COLLECTION_NAME as MANIFESTS, ServiceRequirement};
use crate::models::app_requirement_result::{AppRequirementResult, COLLECTION_NAME as RESULTS};
use crate::models::org_membership::OrgRole;
use crate::models::service_validation_record::{
    CallerContext, ServiceValidationRecord, ValidationOutcome,
};
use crate::models::user_api_key::UserApiKey;
use crate::models::user_service::UserService;
use crate::services::{execution_authority, proxy_service, service_validation_service};
use crate::test_utils::*;

pub(crate) struct Fixture {
    pub(crate) state: AppState,
    pub(crate) app: OauthClient,
    pub(crate) auth: AuthUser,
    pub(crate) owner: String,
}

pub(crate) async fn fixture(name: &str) -> Option<Fixture> {
    let db = connect_test_database(name).await?;
    crate::db::ensure_app_requirement_indexes(&db)
        .await
        .unwrap();
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let owner = Uuid::new_v4().to_string();
    let person = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_many([
            test_user(&owner, UserType::Org),
            test_user(&person, UserType::Person),
        ])
        .await
        .unwrap();
    db.collection::<crate::models::org_membership::OrgMembership>("org_memberships")
        .insert_one(test_membership(&owner, &person, OrgRole::Admin, None))
        .await
        .unwrap();
    let mut config = test_app_config();
    config.app_connect_rollout = AppConnectRollout::Allowlist;
    config.app_connect_allowed_org_ids = vec![owner.clone()];
    let state = test_app_state_with_config(db.clone(), config);
    static AUDIT_INIT: std::sync::Once = std::sync::Once::new();
    AUDIT_INIT.call_once(|| {
        crate::services::audit_service::init_audit_chain_hmac_key(
            state.audit_chain_hmac_key.as_ref().clone(),
        )
    });
    let (mut app, _) = crate::services::oauth_client_service::create_client(
        &db,
        "Test app",
        &["https://app.example/callback".into()],
        "public",
        &owner,
        "",
        "openid proxy",
        Default::default(),
        false,
        None,
        None,
        &[],
    )
    .await
    .unwrap();
    assert!(!app.app_connect_capability_enabled);
    app_connect_rollout::set_client_capability(&db, &app.id, true)
        .await
        .unwrap();
    app.app_connect_capability_enabled = true;
    let mut auth = test_auth_user(&person);
    auth.auth_method = AuthMethod::AccessToken;
    auth.oauth_client_id = Some(app.id.clone());
    auth.allow_all_services = false;
    Some(Fixture {
        state,
        app,
        auth,
        owner,
    })
}

pub(crate) async fn catalog(f: &Fixture, slug: &str, auth: &str) -> String {
    let id = Uuid::new_v4().to_string();
    f.state.db.collection::<Document>("downstream_services").insert_one(doc! {
        "_id": &id, "name": slug, "slug": slug, "base_url": "https://api.github.com",
        "auth_method": auth, "auth_key_name": "Authorization", "credential_encrypted": bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic, bytes: vec![],
        }, "created_by": "system", "is_active": true, "service_category": "connection",
        "service_type": "http", "requires_user_credential": auth != "none", "visibility": "public",
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
    id
}

pub(crate) fn requirement(slug: &str) -> ServiceRequirement {
    ServiceRequirement {
        id: "required".into(),
        label: "Required service".into(),
        any_of_catalog_slugs: vec![slug.into()],
        any_of_catalog_prefix: None,
        owner_policy: OwnerPolicy::PersonalOnly,
        accepted_credential_types: vec![],
        allow_master_credential: false,
        allow_no_credential: false,
        required_downstream_scopes: vec![],
        validator: ValidatorSelection::StoredOnly,
        optional: false,
    }
}

pub(crate) async fn publish(f: &Fixture, requirement: ServiceRequirement) -> ManifestResponse {
    publish_manifest(
        State(f.state.clone()),
        f.auth.clone(),
        Path(f.app.id.clone()),
        Json(manifests::PublishManifest {
            enforcement: Enforcement::Advise,
            requirements: vec![requirement],
        }),
    )
    .await
    .unwrap()
    .0
}

pub(crate) async fn service(
    f: &Fixture,
    owner: &str,
    catalog_id: &str,
    slug: &str,
) -> (UserService, UserApiKey) {
    let endpoint = test_user_endpoint(
        &Uuid::new_v4().to_string(),
        owner,
        slug,
        "https://api.github.com",
        None,
        Some(catalog_id),
    );
    f.state
        .db
        .collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
        .insert_one(&endpoint)
        .await
        .unwrap();
    // Invalid ciphertext proves the evaluator never decrypts or refreshes it.
    let key: UserApiKey = bson::from_document(doc! {
        "_id": Uuid::new_v4().to_string(), "user_id": owner, "label": "Test connection",
        "credential_type": "oauth2", "status": "active", "token_scopes": "repo",
        "access_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1,2,3] },
        "credential_source": "platform", "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).unwrap();
    f.state
        .db
        .collection::<UserApiKey>("user_api_keys")
        .insert_one(&key)
        .await
        .unwrap();
    let mut service = test_user_service(
        &Uuid::new_v4().to_string(),
        owner,
        slug,
        &endpoint.id,
        Some(catalog_id),
        None,
    );
    service.api_key_id = Some(key.id.clone());
    service.auth_method = "bearer".into();
    service.auth_key_name = "Authorization".into();
    f.state
        .db
        .collection::<UserService>("user_services")
        .insert_one(&service)
        .await
        .unwrap();
    (service, key)
}

async fn report(f: &Fixture) -> StatusResponse {
    status(State(f.state.clone()), f.auth.clone())
        .await
        .unwrap()
        .0
}

#[tokio::test]
async fn app_requirements_db_rollout_is_dark_and_allowlist_requires_org_and_capability() {
    let Some(mut f) = fixture("requirements_rollout").await else {
        return;
    };
    assert!(
        app_connect_rollout::is_enabled_for(&f.state, &f.app)
            .await
            .unwrap()
    );
    f.app.app_connect_capability_enabled = false;
    assert!(
        !app_connect_rollout::is_enabled_for(&f.state, &f.app)
            .await
            .unwrap()
    );
    f.app.app_connect_capability_enabled = true;
    f.app.created_by = Some(f.auth.user_id.to_string());
    f.state
        .config
        .app_connect_allowed_org_ids
        .push(f.auth.user_id.to_string());
    assert!(
        !app_connect_rollout::is_enabled_for(&f.state, &f.app)
            .await
            .unwrap()
    );
    f.app.created_by = Some(f.owner.clone());
    f.state.config.app_connect_allowed_org_ids.clear();
    assert!(
        !app_connect_rollout::is_enabled_for(&f.state, &f.app)
            .await
            .unwrap()
    );
    let policy = app_connect_rollout::update_policy(
        &f.state.db,
        &f.state.config,
        Some(AppConnectRollout::Disabled),
    )
    .await
    .unwrap();
    f.state.set_app_connect_policy_if_fresh(policy);
    assert!(matches!(
        list_manifests(
            State(f.state.clone()),
            f.auth.clone(),
            Path(f.app.id.clone())
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(matches!(
        status(State(f.state.clone()), f.auth.clone()).await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(matches!(
        publish_manifest(
            State(f.state.clone()),
            f.auth.clone(),
            Path(f.app.id.clone()),
            Json(manifests::PublishManifest {
                enforcement: Enforcement::Advise,
                requirements: vec![]
            })
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert_eq!(
        f.state
            .db
            .collection::<Document>(RESULTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn app_requirements_db_admin_policy_refresh_and_capability_cannot_be_self_granted() {
    let Some(f) = fixture("requirements_admin").await else {
        return;
    };
    assert!(matches!(
        update_capability(
            State(f.state.clone()),
            f.auth.clone(),
            Path(f.app.id.clone()),
            Json(UpdateCapabilityRequest { enabled: true })
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        update_rollout(
            State(f.state.clone()),
            f.auth.clone(),
            Json(UpdateRolloutRequest {
                rollout: Some(Some(AppConnectRollout::Public))
            })
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    f.state
        .db
        .collection::<Document>(USERS)
        .update_one(
            doc! { "_id": f.auth.user_id.to_string() },
            doc! { "$set": { "role_ids": [crate::services::role_service::get_platform_role_ids(&f.state.db).await.unwrap().admin] } },
        )
        .await
        .unwrap();
    let old = f.state.app_connect_policy();
    let _ = update_rollout(
        State(f.state.clone()),
        f.auth.clone(),
        Json(UpdateRolloutRequest {
            rollout: Some(Some(AppConnectRollout::Disabled)),
        }),
    )
    .await
    .unwrap();
    assert!(!f.state.set_app_connect_policy_if_fresh(old));
    assert_eq!(
        app_connect_rollout::load_policy(&f.state.db, &f.state.config)
            .await
            .unwrap(),
        f.state.app_connect_policy()
    );
    let _ = update_rollout(
        State(f.state.clone()),
        f.auth.clone(),
        Json(UpdateRolloutRequest {
            rollout: Some(None),
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        f.state.app_connect_policy().rollout,
        AppConnectRollout::Allowlist
    );
    let _ = update_capability(
        State(f.state.clone()),
        f.auth.clone(),
        Path(f.app.id.clone()),
        Json(UpdateCapabilityRequest { enabled: false }),
    )
    .await
    .unwrap();
    assert!(matches!(
        enabled_client(&f.state, &f.app.id).await,
        Err(AppError::AppConnectLinkNotFound)
    ));
}

#[tokio::test]
async fn app_requirements_db_publish_freezes_prefix_and_versions_are_atomic() {
    let Some(f) = fixture("requirements_publish").await else {
        return;
    };
    let id = catalog(&f, "llm-first", "bearer").await;
    for (slug, patch) in [
        ("llm-provider", doc! { "service_category": "provider" }),
        ("llm-inactive", doc! { "is_active": false }),
    ] {
        let excluded = catalog(&f, slug, "bearer").await;
        f.state
            .db
            .collection::<Document>("downstream_services")
            .update_one(doc! { "_id": excluded }, doc! { "$set": patch })
            .await
            .unwrap();
    }
    let mut req = requirement("llm-first");
    req.any_of_catalog_slugs.clear();
    req.any_of_catalog_prefix = Some("llm-".into());
    let first = publish(&f, req.clone()).await;
    assert_eq!(
        first.compiled.catalog_service_ids.get("llm-first"),
        Some(&id)
    );
    catalog(&f, "llm-later", "bearer").await;
    let (a, b) = tokio::join!(publish(&f, req.clone()), publish(&f, req));
    let mut versions = vec![a.version, b.version];
    versions.sort();
    assert_eq!(versions, vec![2, 3]);
    let rows = manifests::list(&f.state.db, &f.app.id).await.unwrap();
    assert_eq!(
        rows[2].requirements[0].any_of_catalog_slugs,
        vec!["llm-first"]
    );
    assert_eq!(rows[0].requirements[0].any_of_catalog_slugs.len(), 2);
    let client = enabled_client(&f.state, &f.app.id).await.unwrap();
    assert_eq!(client.current_manifest_version, Some(3));
    let doc = f
        .state
        .db
        .collection::<Document>(MANIFESTS)
        .find_one(doc! { "_id": first.id })
        .await
        .unwrap()
        .unwrap();
    assert!(doc.get_datetime("published_at").is_ok());
    assert!(Uuid::parse_str(doc.get_str("_id").unwrap()).is_ok());
}

#[tokio::test]
async fn app_requirements_db_publish_rejects_invalid_catalog_profiles() {
    let Some(f) = fixture("requirements_invalid").await else {
        return;
    };
    let id = catalog(&f, "api-github", "bearer").await;
    let mut inputs = vec![];
    for (slug, profile) in [
        ("missing", "github_user_v1"),
        ("api-github", "missing"),
        ("api-github", "slack_auth_test_v1"),
    ] {
        let mut r = requirement(slug);
        r.validator = ValidatorSelection::Profile { id: profile.into() };
        inputs.push(manifests::PublishManifest {
            enforcement: Enforcement::Advise,
            requirements: vec![r],
        });
    }
    inputs.push(manifests::PublishManifest {
        enforcement: Enforcement::Advise,
        requirements: vec![requirement("api-github"); 2],
    });
    for mut input in inputs {
        assert!(matches!(
            manifests::compile(&f.state.db, &f.app.id, &mut input).await,
            Err(AppError::AppRequirementsInvalid(_))
        ));
    }
    for update in [
        doc! { "is_active": false },
        doc! { "is_active": true, "service_category": "provider" },
    ] {
        f.state
            .db
            .collection::<Document>("downstream_services")
            .update_one(doc! { "_id": &id }, doc! { "$set": update })
            .await
            .unwrap();
        let mut input = manifests::PublishManifest {
            enforcement: Enforcement::Advise,
            requirements: vec![requirement("api-github")],
        };
        assert!(matches!(
            manifests::compile(&f.state.db, &f.app.id, &mut input).await,
            Err(AppError::AppRequirementsInvalid(_))
        ));
    }
    assert_eq!(
        f.state
            .db
            .collection::<Document>(MANIFESTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn app_requirements_db_status_rejects_non_app_user_tokens_and_discloses_manifest_only() {
    let Some(f) = fixture("requirements_status_auth").await else {
        return;
    };
    catalog(&f, "api-github", "bearer").await;
    publish(&f, requirement("api-github")).await;
    for method in [
        AuthMethod::Session,
        AuthMethod::ApiKey,
        AuthMethod::Delegated,
        AuthMethod::Relay,
        AuthMethod::ServiceAccount,
    ] {
        let mut auth = f.auth.clone();
        auth.auth_method = method;
        assert!(matches!(
            status(State(f.state.clone()), auth).await,
            Err(AppError::Forbidden(_))
        ));
    }
    let mut auth = f.auth.clone();
    auth.oauth_client_id = None;
    assert!(matches!(
        status(State(f.state.clone()), auth).await,
        Err(AppError::Forbidden(_))
    ));
    let other = catalog(&f, "not-in-manifest", "bearer").await;
    service(&f, &f.auth.user_id.to_string(), &other, "not-in-manifest").await;
    let response = report(&f).await;
    assert_eq!(response.requirements.len(), 1);
    assert_eq!(response.requirements[0].state, "unmet");
    assert!(response.requirements[0].slug.is_none());
    let result = f
        .state
        .db
        .collection::<AppRequirementResult>(RESULTS)
        .find_one(doc! { "_id": &response.result_id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.oauth_client_id, f.app.id);
    assert_eq!(result.user_id, f.auth.user_id.to_string());
    assert_eq!(
        result.expires_at - result.created_at,
        chrono::Duration::hours(1)
    );
}

#[tokio::test]
async fn app_requirements_db_owner_acl_and_org_proxy_permission() {
    let Some(f) = fixture("requirements_org").await else {
        return;
    };
    let id = catalog(&f, "api-github", "bearer").await;
    service(&f, &f.owner, &id, "api-github").await;
    publish(&f, requirement("api-github")).await;
    assert_eq!(report(&f).await.requirements[0].state, "unmet");
    let mut r = requirement("api-github");
    r.owner_policy = OwnerPolicy::PersonalOrOrgAllowed;
    publish(&f, r).await;
    assert_eq!(report(&f).await.requirements[0].state, "met");
    f.state
        .db
        .collection::<Document>("org_memberships")
        .update_one(
            doc! { "member_user_id": f.auth.user_id.to_string() },
            doc! { "$set": { "role": "member", "allowed_service_ids": [] } },
        )
        .await
        .unwrap();
    assert_eq!(report(&f).await.requirements[0].state, "unmet");
    f.state
        .db
        .collection::<Document>("org_memberships")
        .update_one(
            doc! { "member_user_id": f.auth.user_id.to_string() },
            doc! { "$set": { "allowed_service_ids": bson::Bson::Null } },
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! { "catalog_service_id": &id },
            doc! { "$set": { "admin_only": true } },
        )
        .await
        .unwrap();
    assert_eq!(report(&f).await.requirements[0].state, "unmet");
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! { "catalog_service_id": &id },
            doc! { "$set": { "admin_only": false } },
        )
        .await
        .unwrap();
    for role in ["member", "viewer"] {
        f.state
            .db
            .collection::<Document>("org_memberships")
            .update_one(
                doc! { "member_user_id": f.auth.user_id.to_string() },
                doc! { "$set": { "role": role } },
            )
            .await
            .unwrap();
        assert_eq!(
            report(&f).await.requirements[0].state,
            if role == "member" { "met" } else { "unmet" }
        );
        assert!(
            list_manifests(
                State(f.state.clone()),
                f.auth.clone(),
                Path(f.app.id.clone())
            )
            .await
            .is_ok()
        );
        assert!(matches!(
            publish_manifest(
                State(f.state.clone()),
                f.auth.clone(),
                Path(f.app.id.clone()),
                Json(manifests::PublishManifest {
                    enforcement: Enforcement::Advise,
                    requirements: vec![],
                })
            )
            .await,
            Err(AppError::OrgRoleInsufficient(_))
        ));
    }
    let stranger = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_user(&stranger, UserType::Person))
        .await
        .unwrap();
    assert!(matches!(
        list_manifests(
            State(f.state.clone()),
            test_auth_user(&stranger),
            Path(f.app.id.clone())
        )
        .await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn app_requirements_db_scopes_and_token_grants_are_independent() {
    let Some(f) = fixture("requirements_scopes").await else {
        return;
    };
    let id = catalog(&f, "api-github", "bearer").await;
    let (service, key) = service(&f, &f.auth.user_id.to_string(), &id, "api-github").await;
    let mut r = requirement("api-github");
    r.accepted_credential_types = vec!["oauth2".into()];
    r.required_downstream_scopes = vec!["repo".into(), "user".into()];
    publish(&f, r).await;
    assert_eq!(report(&f).await.requirements[0].state, "needs_reauth");
    f.state
        .db
        .collection::<Document>("user_api_keys")
        .update_one(
            doc! { "_id": key.id },
            doc! { "$set": { "token_scopes": "repo user" } },
        )
        .await
        .unwrap();
    let response = report(&f).await;
    assert_eq!(response.requirements[0].state, "met");
    assert!(!response.requirements[0].granted_to_caller);
    let mut auth = f.auth.clone();
    auth.allowed_service_ids.push(service.id);
    assert!(
        status(State(f.state.clone()), auth)
            .await
            .unwrap()
            .0
            .requirements[0]
            .granted_to_caller
    );
}

#[tokio::test]
async fn app_requirements_db_no_key_included_and_disabled_never_enabled() {
    let Some(f) = fixture("requirements_no_key").await else {
        return;
    };
    let id = catalog(&f, "identity-test", "none").await;
    let mut r = requirement("identity-test");
    r.allow_no_credential = true;
    publish(&f, r).await;
    let response = report(&f).await;
    assert_eq!(response.requirements[0].state, "included");
    let service = response.requirements[0].user_service_id.as_ref().unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! { "_id": service },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    assert_eq!(report(&f).await.requirements[0].state, "disabled");
    assert_eq!(
        f.state
            .db
            .collection::<Document>("user_services")
            .count_documents(doc! { "catalog_service_id": id })
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>("user_services")
            .count_documents(doc! { "_id": service, "is_active": true })
            .await
            .unwrap(),
        0
    );
}

pub(crate) async fn evidence(
    f: &Fixture,
    service: &UserService,
    key: &UserApiKey,
    outcome: ValidationOutcome,
) -> ServiceValidationRecord {
    let resolution = proxy_service::read_proxy_authority_snapshot_by_user_service_id(
        &f.state.db,
        &f.state.encryption_keys,
        &f.auth.user_id.to_string(),
        &service.id,
        Some(&service.slug),
    )
    .await
    .unwrap()
    .unwrap();
    let now = Utc::now();
    let record = ServiceValidationRecord {
        node_credential: None,
        id: Uuid::new_v4().to_string(),
        user_service_id: service.id.clone(),
        owner_id: service.user_id.clone(),
        validator_id: "github_user_v1".into(),
        validator_version: 1,
        execution_authority_digest: execution_authority::digest(
            &execution_authority::build_projection(&resolution, None, vec![]),
        ),
        api_key_id: Some(key.id.clone()),
        credential_epoch: Some(key.credential_epoch),
        attempt_id: Uuid::new_v4().to_string(),
        completed: true,
        credential_revision: Some(service_validation_service::credential_revision(key)),
        reason_code: "test_fixture".into(),
        outcome,
        checked_at: now,
        valid_until: now + chrono::Duration::minutes(5),
        caller_context: CallerContext::Human {
            session: Uuid::new_v4().to_string(),
        },
    };
    f.state
        .db
        .collection::<ServiceValidationRecord>("service_validation_records")
        .insert_one(&record)
        .await
        .unwrap();
    record
}

#[tokio::test]
async fn app_requirements_db_evidence_freshness_and_authority_binding() {
    let Some(f) = fixture("requirements_fresh").await else {
        return;
    };
    let id = catalog(&f, "api-github", "bearer").await;
    let (service, key) = service(&f, &f.auth.user_id.to_string(), &id, "api-github").await;
    let mut r = requirement("api-github");
    r.validator = ValidatorSelection::Profile {
        id: "github_user_v1".into(),
    };
    publish(&f, r).await;
    assert_eq!(report(&f).await.requirements[0].state, "unknown");
    let record = evidence(&f, &service, &key, ValidationOutcome::Authenticated).await;
    assert_eq!(report(&f).await.requirements[0].state, "met");
    for patch in [
        doc! { "valid_until": bson::DateTime::from_chrono(record.checked_at) },
        doc! { "execution_authority_digest": "changed" },
        doc! { "credential_revision": "changed" },
        doc! { "validator_version": 2 },
        doc! { "completed": false },
    ] {
        f.state
            .db
            .collection::<ServiceValidationRecord>("service_validation_records")
            .replace_one(doc! { "_id": &record.id }, &record)
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("service_validation_records")
            .update_one(doc! { "_id": &record.id }, doc! { "$set": patch })
            .await
            .unwrap();
        assert_eq!(report(&f).await.requirements[0].state, "unknown");
    }
}

#[tokio::test]
async fn app_requirements_db_explicit_broken_selection_stays_selected() {
    let Some(f) = fixture("requirements_selection").await else {
        return;
    };
    let id = catalog(&f, "api-github", "bearer").await;
    let (a, ka) = service(&f, &f.auth.user_id.to_string(), &id, "github-a").await;
    let (b, kb) = service(&f, &f.auth.user_id.to_string(), &id, "github-b").await;
    let mut r = requirement("api-github");
    r.validator = ValidatorSelection::Profile {
        id: "github_user_v1".into(),
    };
    publish(&f, r).await;
    evidence(&f, &a, &ka, ValidationOutcome::Authenticated).await;
    evidence(&f, &b, &kb, ValidationOutcome::Authenticated).await;
    let first = report(&f).await;
    assert_eq!(
        first.requirements[0].user_service_id.as_deref(),
        Some(b.id.as_str())
    );
    f.state
        .db
        .collection::<Document>(RESULTS)
        .update_one(
            doc! { "_id": first.result_id },
            doc! { "$set": {
                "selections.0.user_service_id": &a.id, "selections.0.explicit": true,
            } },
        )
        .await
        .unwrap();
    assert_eq!(
        report(&f).await.requirements[0].user_service_id.as_deref(),
        Some(a.id.as_str())
    );
    f.state
        .db
        .collection::<Document>("service_validation_records")
        .update_one(
            doc! { "user_service_id": &a.id },
            doc! { "$set": { "outcome": { "kind": "credential_rejected" } } },
        )
        .await
        .unwrap();
    let broken = report(&f).await;
    assert_eq!(broken.requirements[0].state, "broken");
    assert_eq!(
        broken.requirements[0].user_service_id.as_deref(),
        Some(a.id.as_str())
    );
}

#[tokio::test]
async fn app_requirements_db_evaluator_never_calls_provider_transport_or_touches_usage() {
    let Some(f) = fixture("requirements_no_io").await else {
        return;
    };
    let id = catalog(&f, "api-github", "bearer").await;
    let (service, key) = service(&f, &f.auth.user_id.to_string(), &id, "api-github").await;
    // An actual listening origin detects HTTP effects, including unexpected refresh.
    let server = wiremock::MockServer::start().await;
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! { "_id": &service.endpoint_id },
            doc! { "$set": { "url": server.uri() } },
        )
        .await
        .unwrap();
    let mut r = requirement("api-github");
    r.validator = ValidatorSelection::Profile {
        id: "github_user_v1".into(),
    };
    publish(&f, r).await;
    assert_eq!(report(&f).await.requirements[0].state, "unknown");
    assert!(server.received_requests().await.unwrap().is_empty());
    let after = f
        .state
        .db
        .collection::<UserApiKey>("user_api_keys")
        .find_one(doc! { "_id": &key.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bson::to_document(&key).unwrap(),
        bson::to_document(&after).unwrap()
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>("service_validation_records")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn app_requirements_db_result_ttl_index_and_legacy_client_defaults() {
    let Some(f) = fixture("requirements_indexes").await else {
        return;
    };
    let indexes: Vec<_> = f
        .state
        .db
        .collection::<Document>(RESULTS)
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(indexes.iter().any(|i| i.keys == doc! { "expires_at": 1 }
        && i.options.as_ref().and_then(|o| o.expire_after) == Some(std::time::Duration::ZERO)));
    let mut legacy = bson::to_document(&f.app).unwrap();
    legacy.remove("app_connect_capability_enabled");
    legacy.remove("current_manifest_version");
    let decoded: OauthClient = bson::from_document(legacy).unwrap();
    assert!(!decoded.app_connect_capability_enabled);
    assert!(decoded.current_manifest_version.is_none());
}

#[test]
fn app_requirements_unknown_tags_and_fields_are_rejected() {
    let input = serde_json::json!({ "enforcement": "advise", "requirements": [], "tags": ["llm"] });
    assert!(serde_json::from_value::<manifests::PublishManifest>(input).is_err());
    let mut value = serde_json::to_value(requirement("api-github")).unwrap();
    value["any_of_catalog_tags"] = serde_json::json!(["llm"]);
    assert!(serde_json::from_value::<ServiceRequirement>(value).is_err());
}

#[tokio::test]
async fn app_requirements_db_developer_and_dcr_cannot_request_capability() {
    let Some(f) = fixture("requirements_no_self_grant").await else {
        return;
    };
    let request = serde_json::from_value(serde_json::json!({
        "name": "Developer self grant", "client_type": "public", "redirect_uris": ["https://app.example/callback"],
        "app_connect_capability_enabled": true,
    })).unwrap();
    let response = crate::handlers::developer_apps::create_my_oauth_client(
        State(f.state.clone()),
        f.auth.clone(),
        crate::telemetry::TelemetryContext::default(),
        Json(request),
    )
    .await
    .unwrap();
    assert!(!response.0.app_connect_capability_enabled);
    let request = serde_json::from_value(serde_json::json!({
        "client_name": "DCR self grant", "redirect_uris": ["https://app.example/callback"],
        "scope": "openid", "app_connect_capability_enabled": true,
    }))
    .unwrap();
    let (_, response) =
        crate::handlers::oauth::register_client(State(f.state.clone()), Json(request))
            .await
            .unwrap();
    let client = f
        .state
        .db
        .collection::<OauthClient>(CLIENTS)
        .find_one(doc! { "_id": &response.client_id })
        .await
        .unwrap()
        .unwrap();
    assert!(!client.app_connect_capability_enabled);
    let request = serde_json::from_value(serde_json::json!({
        "redirect_uris": ["https://app.example/callback"], "scope": "openid urn:nyxid:scope:app_connect",
    })).unwrap();
    assert!(
        crate::handlers::oauth::register_client(State(f.state.clone()), Json(request))
            .await
            .is_err()
    );
}

#[test]
fn app_requirements_rollout_patch_distinguishes_omitted_and_null() {
    let omitted: UpdateRolloutRequest = serde_json::from_str("{}").unwrap();
    assert_eq!(omitted.rollout, None);
    let cleared: UpdateRolloutRequest = serde_json::from_str(r#"{"rollout":null}"#).unwrap();
    assert_eq!(cleared.rollout, Some(None));
}

#[tokio::test]
async fn app_requirements_db_admin_changes_emit_metadata_only_audit_events() {
    let Some(f) = fixture("requirements_audit").await else {
        return;
    };
    let roles = crate::services::role_service::get_platform_role_ids(&f.state.db)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(USERS)
        .update_one(
            doc! { "_id": f.auth.user_id.to_string() },
            doc! { "$set": { "role_ids": [roles.admin] } },
        )
        .await
        .unwrap();
    for enabled in [false, true] {
        let _ = update_capability(
            State(f.state.clone()),
            f.auth.clone(),
            Path(f.app.id.clone()),
            Json(UpdateCapabilityRequest { enabled }),
        )
        .await
        .unwrap();
    }
    let _ = update_rollout(
        State(f.state.clone()),
        f.auth.clone(),
        Json(UpdateRolloutRequest {
            rollout: Some(Some(AppConnectRollout::Disabled)),
        }),
    )
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let rows: Vec<Document> = f.state.db.collection::<Document>("audit_log")
                .find(doc! { "event_type": { "$in": ["app_connect_capability_granted", "app_connect_capability_revoked", "app_connect_rollout_changed"] } })
                .await.unwrap().try_collect().await.unwrap();
            if rows.len() == 3 {
                for row in rows {
                    assert_eq!(row.get_str("user_id").unwrap(), f.auth.user_id.to_string());
                    let data = row.get_document("event_data").unwrap();
                    assert!(data.keys().all(|k| ["client_id", "rollout", "override", "revision"].contains(&k.as_str())));
                }
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap();
}

#[tokio::test]
async fn app_requirements_db_master_credential_requires_opt_in_and_inactive_catalog_is_unsatisfiable()
 {
    let Some(f) = fixture("requirements_master").await else {
        return;
    };
    let id = catalog(&f, "internal-test", "bearer").await;
    f.state.db.collection::<Document>("downstream_services").update_one(doc! { "_id": &id }, doc! { "$set": {
        "service_category": "internal", "requires_user_credential": false,
        "credential_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1,2,3] },
    } }).await.unwrap();
    let mut r = requirement("internal-test");
    r.allow_master_credential = true;
    publish(&f, r.clone()).await;
    assert_eq!(report(&f).await.requirements[0].state, "included");
    r.allow_master_credential = false;
    publish(&f, r).await;
    assert_eq!(report(&f).await.requirements[0].state, "unmet");
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(doc! { "_id": &id }, doc! { "$set": { "is_active": false } })
        .await
        .unwrap();
    assert_eq!(report(&f).await.requirements[0].state, "unsatisfiable");
}

#[tokio::test]
async fn app_requirements_db_platform_binding_uses_live_grants_and_ignores_retained_key() {
    let Some(f) = fixture("requirements_platform_merge").await else {
        return;
    };
    let id = catalog(&f, "api-github", "bearer").await;
    let (connection, key) = service(&f, &f.auth.user_id.to_string(), &id, "github").await;
    f.state.db.collection::<Document>("downstream_services").update_one(
        doc! { "_id": &id }, doc! { "$set": {
            "credential_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1,2,3] },
            "platform_key": { "enabled": true, "audience": "restricted", "allowed_owner_ids": [f.auth.user_id.to_string()] },
        } },
    ).await.unwrap();
    f.state
        .db
        .collection::<UserService>("user_services")
        .update_one(
            doc! { "_id": &connection.id },
            doc! { "$set": { "credential_binding": "platform" } },
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<UserApiKey>("user_api_keys")
        .update_one(
            doc! { "_id": &key.id },
            doc! { "$set": { "status": "revoked" } },
        )
        .await
        .unwrap();
    let mut req = requirement("api-github");
    publish(&f, req.clone()).await;
    assert_eq!(report(&f).await.requirements[0].state, "unmet");
    req.allow_master_credential = true;
    publish(&f, req).await;
    let included = report(&f).await;
    assert_eq!(included.requirements[0].state, "included");
    assert_eq!(
        included.requirements[0].user_service_id.as_deref(),
        Some(connection.id.as_str())
    );
    assert!(!included.requirements[0].granted_to_caller);
    f.state
        .db
        .collection::<UserService>("user_services")
        .update_one(
            doc! { "_id": &connection.id },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    assert_eq!(report(&f).await.requirements[0].state, "disabled");
    f.state
        .db
        .collection::<UserService>("user_services")
        .update_one(
            doc! { "_id": &connection.id },
            doc! { "$set": { "is_active": true } },
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! { "_id": &id },
            doc! { "$set": { "platform_key.allowed_owner_ids": [] } },
        )
        .await
        .unwrap();
    assert_ne!(report(&f).await.requirements[0].state, "included");
}

#[tokio::test]
async fn app_requirements_db_seeded_prefixes_preserve_reviewed_validator_boundary() {
    let Some(f) = fixture("requirements_new_seeds_merge").await else {
        return;
    };
    use crate::services::{provider_service, validator_profiles};
    provider_service::seed_default_providers(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    provider_service::seed_default_services(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let mut req = requirement("api-google-workspace");
    req.any_of_catalog_slugs.clear();
    req.any_of_catalog_prefix = Some("api-google-".into());
    let manifest = publish(&f, req).await;
    for slug in [
        "api-google-workspace",
        "api-google-calendar",
        "api-google-drive",
        "api-google-gmail",
    ] {
        assert!(manifest.compiled.catalog_service_ids.contains_key(slug));
        assert!(validator_profiles::for_slug(slug).is_none());
    }
    publish(&f, requirement("api-notion")).await;
    assert!(validator_profiles::for_slug("api-notion").is_none());
    for profile in validator_profiles::PROFILES {
        for slug in profile.catalog_slugs {
            assert!(
                f.state
                    .db
                    .collection::<Document>("downstream_services")
                    .find_one(doc! { "slug": *slug, "is_active": true })
                    .await
                    .unwrap()
                    .is_some(),
                "{slug}"
            );
        }
    }
    let mut req = requirement("llm-xai");
    publish(&f, req.clone()).await;
    req.any_of_catalog_slugs.push("llm-openai".into());
    req.validator = ValidatorSelection::Profile {
        id: "llm_models_v1".into(),
    };
    let error = manifests::compile(
        &f.state.db,
        &f.app.id,
        &mut manifests::PublishManifest {
            enforcement: Enforcement::Advise,
            requirements: vec![req],
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, AppError::AppRequirementsInvalid(message) if message.contains("Every catalog alternative"))
    );
}

#[tokio::test]
async fn app_requirements_db_node_stored_only_reads_dispatchability_without_sending() {
    let Some(f) = fixture("requirements_node").await else {
        return;
    };
    let id = catalog(&f, "api-github", "bearer").await;
    let (service, key) = service(&f, &f.auth.user_id.to_string(), &id, "api-github").await;
    let node_id = Uuid::new_v4().to_string();
    f.state.db.collection::<Document>("nodes").insert_one(doc! {
        "_id": &node_id, "user_id": f.auth.user_id.to_string(), "name": "Local credential node",
        "status": "online", "auth_token_hash": "test-hash", "signing_secret_hash": "test-hash",
        "is_active": true, "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! { "_id": &service.id },
            doc! { "$set": { "node_id": &node_id } },
        )
        .await
        .unwrap();
    f.state.db.collection::<Document>("user_api_keys").update_one(doc! { "_id": key.id },
        doc! { "$set": { "credential_type": "node_managed" }, "$unset": { "access_token_encrypted": "" } }).await.unwrap();
    let (tx, mut outbound) = tokio::sync::mpsc::channel(8);
    register_test_node_connection(&f.state, &node_id, tx).await;
    publish(&f, requirement("api-github")).await;
    assert_eq!(report(&f).await.requirements[0].state, "met");
    f.state
        .db
        .collection::<Document>("nodes")
        .update_one(
            doc! { "_id": &node_id },
            doc! { "$set": { "status": "offline" } },
        )
        .await
        .unwrap();
    assert_eq!(report(&f).await.requirements[0].state, "unknown");
    assert!(matches!(
        outbound.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn app_requirements_db_unknown_candidate_outranks_broken_without_explicit_choice() {
    let Some(f) = fixture("requirements_unknown_choice").await else {
        return;
    };
    let id = catalog(&f, "api-github", "bearer").await;
    let (dead, key) = service(&f, &f.auth.user_id.to_string(), &id, "github-dead").await;
    let (untested, _) = service(&f, &f.auth.user_id.to_string(), &id, "github-untested").await;
    f.state
        .db
        .collection::<Document>("user_api_keys")
        .update_one(
            doc! { "_id": &key.id },
            doc! { "$set": { "last_used_at": bson::DateTime::now() } },
        )
        .await
        .unwrap();
    let mut r = requirement("api-github");
    r.validator = ValidatorSelection::Profile {
        id: "github_user_v1".into(),
    };
    publish(&f, r).await;
    evidence(&f, &dead, &key, ValidationOutcome::CredentialRejected).await;
    let result = report(&f).await;
    assert_eq!(result.requirements[0].state, "unknown");
    assert_eq!(
        result.requirements[0].user_service_id.as_deref(),
        Some(untested.id.as_str())
    );
}

#[tokio::test]
async fn app_requirements_db_status_without_manifest_is_not_found_with_relevant_message() {
    let Some(f) = fixture("requirements_no_manifest").await else {
        return;
    };
    let error = status(State(f.state.clone()), f.auth.clone())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AppError::NotFound(message) if message == "No published requirements for this app")
    );
    assert_eq!(
        axum::response::IntoResponse::into_response(error).status(),
        axum::http::StatusCode::NOT_FOUND,
    );
}

#[tokio::test]
async fn app_requirements_db_status_limits_per_user_before_evaluation() {
    let Some(f) = fixture("requirements_status_limit").await else {
        return;
    };
    catalog(&f, "api-github", "bearer").await;
    publish(&f, requirement("api-github")).await;
    for _ in 0..30 {
        report(&f).await;
    }
    assert!(matches!(
        status(State(f.state.clone()), f.auth.clone()).await,
        Err(AppError::RateLimited)
    ));
    assert_eq!(
        f.state
            .db
            .collection::<Document>(RESULTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let another = Uuid::new_v4();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_user(&another.to_string(), UserType::Person))
        .await
        .unwrap();
    let mut auth = f.auth.clone();
    auth.user_id = another;
    assert!(status(State(f.state.clone()), auth).await.is_ok());
    // A spent user bucket must not reveal an app behind a disabled rollout.
    app_connect_rollout::set_client_capability(&f.state.db, &f.app.id, false)
        .await
        .unwrap();
    assert!(matches!(
        status(State(f.state.clone()), f.auth.clone()).await,
        Err(AppError::AppConnectLinkNotFound)
    ));
}

#[tokio::test]
async fn app_requirements_db_reuses_identical_selections_with_fresh_status_and_fixed_expiry() {
    let Some(f) = fixture("requirements_result_reuse").await else {
        return;
    };
    let id = catalog(&f, "api-github", "bearer").await;
    let (service, key) = service(&f, &f.auth.user_id.to_string(), &id, "api-github").await;
    let mut r = requirement("api-github");
    r.validator = ValidatorSelection::Profile {
        id: "github_user_v1".into(),
    };
    publish(&f, r.clone()).await;
    let first = report(&f).await;
    assert_eq!(first.requirements[0].state, "unknown");
    let results = f.state.db.collection::<AppRequirementResult>(RESULTS);
    let stored = results
        .find_one(doc! { "_id": &first.result_id })
        .await
        .unwrap()
        .unwrap();
    evidence(&f, &service, &key, ValidationOutcome::Authenticated).await;
    let second = report(&f).await;
    assert_eq!(second.requirements[0].state, "met");
    assert_eq!(second.result_id, first.result_id);
    assert_eq!(results.count_documents(doc! {}).await.unwrap(), 1);
    assert_eq!(
        results
            .find_one(doc! { "_id": &first.result_id })
            .await
            .unwrap()
            .unwrap()
            .expires_at,
        stored.expires_at
    );
    let expired = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1));
    results
        .update_one(
            doc! { "_id": &first.result_id },
            doc! { "$set": { "expires_at": expired } },
        )
        .await
        .unwrap();
    let renewed = report(&f).await;
    assert_ne!(renewed.result_id, first.result_id);
    // Even identical selections must not cross an immutable manifest version.
    publish(&f, r).await;
    let versioned = report(&f).await;
    assert_ne!(versioned.result_id, renewed.result_id);
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! { "_id": &service.id },
            doc! { "$set": { "catalog_service_id": bson::Bson::Null } },
        )
        .await
        .unwrap();
    let changed = report(&f).await;
    assert_eq!(changed.requirements[0].state, "unmet");
    assert_ne!(changed.result_id, versioned.result_id);
}

#[tokio::test]
async fn app_requirements_db_private_no_credential_requires_independent_prerequisite() {
    let Some(f) = fixture("requirements_private_cycle").await else {
        return;
    };
    let id = catalog(&f, "private-no-key", "none").await;
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! { "_id": &id },
            doc! { "$set": { "visibility": "private", "developer_app_ids": [&f.app.id] } },
        )
        .await
        .unwrap();
    let mut r = requirement("private-no-key");
    r.allow_no_credential = true;
    let mut input = manifests::PublishManifest {
        enforcement: Enforcement::Gate,
        requirements: vec![r],
    };
    assert!(
        matches!(manifests::compile(&f.state.db, &f.app.id, &mut input).await,
        Err(AppError::AppRequirementsInvalid(message)) if message.contains("independent prerequisite"))
    );
    assert!(
        manifests::compile(&f.state.db, &Uuid::new_v4().to_string(), &mut input)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn app_requirements_db_compiles_validator_for_every_alternative() {
    let Some(f) = fixture("requirements_alternative_profiles").await else {
        return;
    };
    catalog(&f, "llm-openai", "bearer").await;
    catalog(&f, "llm-openrouter", "bearer").await;
    let mut r = requirement("llm-openai");
    r.any_of_catalog_slugs.push("llm-openrouter".into());
    r.validator = ValidatorSelection::Profile {
        id: "llm_models_v1".into(),
    };
    let published = publish(&f, r.clone()).await;
    let compiled = &published.compiled.validators_by_requirement["required"];
    assert_eq!(compiled["llm-openai"], "llm_models_v1");
    assert_eq!(compiled["llm-openrouter"], "openrouter_key_v1");
    let catalog_id = published.compiled.catalog_service_ids["llm-openrouter"].clone();
    let (selected, key) = service(
        &f,
        &f.auth.user_id.to_string(),
        &catalog_id,
        "router-account",
    )
    .await;
    let record = evidence(&f, &selected, &key, ValidationOutcome::Authenticated).await;
    f.state
        .db
        .collection::<Document>("service_validation_records")
        .update_one(
            doc! { "_id": &record.id },
            doc! { "$set": { "validator_id": "openrouter_key_v1" } },
        )
        .await
        .unwrap();
    let status = status(State(f.state.clone()), f.auth.clone())
        .await
        .unwrap()
        .0;
    assert_eq!(status.requirements[0].state, "met");
    assert_eq!(
        status.requirements[0].user_service_id.as_deref(),
        Some(selected.id.as_str())
    );
    catalog(&f, "unknown-validator", "bearer").await;
    r.any_of_catalog_slugs.push("unknown-validator".into());
    let mut input = manifests::PublishManifest {
        enforcement: Enforcement::Gate,
        requirements: vec![r],
    };
    assert!(
        matches!(manifests::compile(&f.state.db, &f.app.id, &mut input).await,
        Err(AppError::AppRequirementsInvalid(message)) if message.contains("Every catalog alternative"))
    );
}
