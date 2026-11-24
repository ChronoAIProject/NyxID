use super::*;
use crate::models::assistant_agent::{AgentGrants, GuestAccess};
use crate::services::assistant_team_service::{self as team, GrantChange};
use serde_json::json;

async fn ready(f: &mut Fixture) {
    f.org.readiness.abort();
    f.org.readiness =
        crate::services::security_readiness_service::controlled_ready_test_replica(&f.org.state.db)
            .await;
}
async fn reload(f: &mut Fixture) {
    f.channel = management::load(&f.org.state.db, &f.org.admin, &f.channel.id, true)
        .await
        .unwrap();
}
async fn grant(f: &Fixture, name: &str) -> UserService {
    let db = &f.org.state.db;
    let id = f.org.service(&f.org.org, name).await;
    team::set_grants(
        db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::Add(AgentGrants {
            service_ids: vec![id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    db.collection::<UserService>("user_services")
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}
async fn adopt(f: &Fixture) -> adoption::Adoption {
    let v = adoption::preview(&f.org.state.db, &f.org.admin, &f.agent.id)
        .await
        .unwrap();
    let input = adoption::Adoption {
        review: v["review"].as_str().unwrap().into(),
    };
    adoption::adopt(&f.org.state.db, &f.org.admin, &f.agent.id, input.clone())
        .await
        .unwrap();
    input
}

#[tokio::test]
async fn activated_org_channel_link_enable_and_adopt_serve_overlap_and_reject_unsupported() {
    use crate::services::security_readiness_service as readiness;
    let mut f = Fixture::new("org_channel_roster_overlap").await;
    f.wallet().await;
    ready(&mut f).await;
    let db = f.org.state.db.clone();
    let sibling = readiness::overlapping_test_replica(&db).await;
    let binding = link_target(&db, &f.org.admin, &f.bot, &f.agent)
        .await
        .unwrap()
        .unwrap();
    assert!(binding.guests.follows_agent_grants);
    f.guests(vec![]).await;
    assert!(f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert_eq!(
        adoption::preview(&db, &f.org.admin, &f.agent.id)
            .await
            .unwrap()["ready"],
        true
    );
    let input = adopt(&f).await;
    reload(&mut f).await;
    assert!(
        f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .follows_agent_grants
    );
    readiness::remove_test_replica_capability(&db, &sibling, GUEST_GRANTS_READINESS).await;
    assert!(!guest_grants_ready(&db).await.unwrap());
    assert!(matches!(
        adoption::adopt(&db, &f.org.admin, &f.agent.id, input).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    readiness::remove_test_replica_capability(&db, &sibling, FLAG).await;
    assert!(matches!(
        link_target(&db, &f.org.admin, &f.bot, &f.agent).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(matches!(
        management::update(
            &db,
            &f.org.admin,
            &f.channel,
            management::GuestSettings {
                expected_revision: None,
                enabled: true,
                service_ids: None,
                daily_credits: None,
            },
            None
        )
        .await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn cutover_keeps_partial_and_empty_lists_until_explicit_adoption_and_fences_old_work() {
    let mut f = Fixture::new("org_guest_cutover_lists").await;
    f.wallet().await;
    let one = grant(&f, "Calendar").await;
    let two = grant(&f, "Mail").await;
    f.guests(vec![one.id.clone()]).await;
    let old = f.guest("visitor").await;
    let old_turn = channel_turn(&f, &old, "hello").await;
    let member = f.link(&f.org.member, "member").await;
    let member_turn = channel_turn(&f, &member, "member hello").await;
    let db = f.org.state.db.clone();
    // A second legacy bot on the same specialist has model-only access.
    let mut empty = f.channel.clone();
    empty.id = uuid::Uuid::new_v4().to_string();
    empty.bot_label = "Model only".into();
    let p = &mut empty.org_specialist.as_mut().unwrap().guests;
    p.service_ids.clear();
    p.service_authority.clear();
    db.collection::<NyxbotChannel>("nyxbot_channels")
        .insert_one(&empty)
        .await
        .unwrap();
    ready(&mut f).await;
    let access = validate(&db, &old, &f.org.org, &f.agent.id).await.unwrap();
    assert!(access.allows_service(&one));
    assert!(!access.allows_service(&two));
    let partial = management::view(&db, &f.org.admin, &f.channel)
        .await
        .unwrap();
    assert_eq!(partial["binding_limits_guest_services"], true);
    let mut covers_grants = f.channel.clone();
    let policy = &mut covers_grants.org_specialist.as_mut().unwrap().guests;
    policy.service_ids.push(two.id.clone());
    policy.service_authority.insert(
        two.id.clone(),
        management::service_digest(access.channel().unwrap().agent(), &two.id),
    );
    let covered = management::view(&db, &f.org.admin, &covers_grants)
        .await
        .unwrap();
    assert_eq!(covered["binding_limits_guest_services"], false);
    let view = management::view(&db, &f.org.admin, &empty).await.unwrap();
    assert_eq!(view["guest_policy_mode"], "legacy");
    assert!(
        view["effective_guest_services"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(view["binding_limits_guest_services"], true);
    let input = adopt(&f).await;
    adoption::adopt(&db, &f.org.admin, &f.agent.id, input)
        .await
        .unwrap(); // idempotent retry
    assert!(validate(&db, &old, &f.org.org, &f.agent.id).await.is_err());
    assert!(!f.org.key(&old_turn).await.is_active);
    assert!(f.org.key(&member_turn).await.is_active);
    assert!(
        validate(&db, &member, &f.org.member, &f.agent.id)
            .await
            .is_ok()
    );
    reload(&mut f).await;
    let origin = f.guest("visitor").await;
    let access = validate(&db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    assert!(access.allows_service(&one));
    assert!(access.allows_service(&two));
    let saved = &f.channel.org_specialist.as_ref().unwrap().guests;
    let adopted = management::view(&db, &f.org.admin, &f.channel)
        .await
        .unwrap();
    assert_eq!(adopted["binding_limits_guest_services"], false);
    assert_eq!(saved.service_ids, vec![one.id.clone()]); // retained, not erased
    assert!(saved.follows_agent_grants);
    team::set_grants(
        &db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::Guests([(two.id.clone(), GuestAccess::Read)].into()),
    )
    .await
    .unwrap();
    let access = validate(&db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    assert_eq!(
        crate::services::agent_operation_scope_service::guest_level(
            access.channel().unwrap().agent(),
            &two.id,
            None
        ),
        GuestAccess::Read
    );
    assert!(
        !adoption::preview(&db, &f.org.admin, &f.agent.id)
            .await
            .unwrap()["pending"]
            .as_bool()
            .unwrap()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn cutover_adoption_is_admin_only_and_review_binds_grants_and_bot_set() {
    let mut f = Fixture::new("org_guest_cutover_review").await;
    ready(&mut f).await;
    let db = f.org.state.db.clone();
    for actor in [&f.org.member, &f.org.viewer, &f.org.outsider] {
        assert!(adoption::preview(&db, actor, &f.agent.id).await.is_err());
    }
    let review = adoption::preview(&db, &f.org.admin, &f.agent.id)
        .await
        .unwrap();
    let input = adoption::Adoption {
        review: review["review"].as_str().unwrap().into(),
    };
    assert!(
        adoption::adopt(&db, &f.org.member, &f.agent.id, input.clone())
            .await
            .is_err()
    );
    grant(&f, "Late addition").await;
    assert!(matches!(
        adoption::adopt(&db, &f.org.admin, &f.agent.id, input).await,
        Err(AppError::Conflict(_))
    ));
    let review = adoption::preview(&db, &f.org.admin, &f.agent.id)
        .await
        .unwrap();
    let mut extra = f.channel.clone();
    extra.id = uuid::Uuid::new_v4().to_string();
    db.collection::<NyxbotChannel>("nyxbot_channels")
        .insert_one(&extra)
        .await
        .unwrap();
    assert!(matches!(
        adoption::adopt(
            &db,
            &f.org.admin,
            &f.agent.id,
            adoption::Adoption {
                review: review["review"].as_str().unwrap().into()
            }
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    assert!(
        !f.channel
            .org_specialist
            .unwrap()
            .guests
            .follows_agent_grants
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn cutover_removes_cap_for_legacy_preserves_holds_and_wallet_and_rate_floors() {
    let mut f = Fixture::new("org_guest_cutover_spend").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let origin = f.guest("guest").await;
    let db = f.org.state.db.clone();
    let payer = validate(&db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap()
        .channel()
        .unwrap()
        .guest_payer()
        .unwrap();
    reserve(&db, &payer, "before").await.unwrap();
    assert!(reserve(&db, &payer, "old-cap").await.is_err());
    ready(&mut f).await;
    for i in 0..4 {
        reserve(&db, &payer, &format!("after-{i}")).await.unwrap();
    }
    budget::settle(&db, "before", Credits::from_whole(100))
        .await
        .unwrap();
    budget::settle(&db, "before", Credits::from_whole(100))
        .await
        .unwrap();
    assert_eq!(
        budget::today(&db, &f.channel.id).await.unwrap(),
        (Credits::from_whole(100), "2.4".parse().unwrap())
    );
    reserve(&db, &payer, "high-spend").await.unwrap();
    // No cap value is required after readiness, including legacy bindings.
    db.collection::<Document>("nyxbot_channels")
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$unset":{"org_specialist.guests.daily_credits":""}},
        )
        .await
        .unwrap();
    reserve(&db, &payer, "no-cap").await.unwrap();
    for _ in 0..6 {
        assert!(
            guest_rate(&db, origin.org_execution.as_ref().unwrap(), &f.channel.id)
                .await
                .unwrap()
        );
    }
    assert!(
        !guest_rate(&db, origin.org_execution.as_ref().unwrap(), &f.channel.id)
            .await
            .unwrap()
    );
    db.collection::<Document>("billing_wallet")
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(reserve(&db, &payer, "missing-wallet").await.is_err());
    // Financial settlement stays available after policy/wallet loss.
    budget::settle(&db, "after-0", Credits::ZERO).await.unwrap();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn cutover_new_binding_is_off_and_uses_grants_on_enable_old_put_is_inert() {
    let mut f = Fixture::new("org_guest_cutover_new").await;
    ready(&mut f).await;
    f.wallet().await;
    let service = grant(&f, "Calendar").await;
    let binding = link_target(&f.org.state.db, &f.org.admin, &f.bot, &f.agent)
        .await
        .unwrap()
        .unwrap();
    assert!(binding.guests.enabled);
    assert!(!binding.guests.revision.is_empty());
    assert!(binding.guests.follows_agent_grants);
    let db = f.org.state.db.clone();
    db.collection::<Document>("nyxbot_channels")
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"org_specialist":bson::to_bson(&binding).unwrap()}},
        )
        .await
        .unwrap();
    reload(&mut f).await;
    management::update(
        &db,
        &f.org.admin,
        &f.channel,
        serde_json::from_value(json!({"enabled":true,"service_ids":[],"daily_credits":"0"}))
            .unwrap(),
        None,
    )
    .await
    .unwrap();
    reload(&mut f).await;
    let origin = f.guest("guest").await;
    let access = validate(&db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    assert!(access.allows_service(&service));
    let row = channel_turn(&f, &origin, "hello").await;
    let key = f.org.key(&row).await;
    assert_eq!(key.allowed_service_ids, vec![service.id.clone()]);
    assert!(!key.allow_all_nodes && key.allowed_node_ids.is_empty());
    assert!(!key.scopes.split_whitespace().any(|s| s == "account:read"));
    let before = f.channel.org_specialist.as_ref().unwrap().guests.clone();
    management::update(
        &db,
        &f.org.admin,
        &f.channel,
        serde_json::from_value(
            json!({"enabled":true,"service_ids":["ungranted"],"daily_credits":"999"}),
        )
        .unwrap(),
        None,
    )
    .await
    .unwrap();
    reload(&mut f).await;
    assert_eq!(before, f.channel.org_specialist.as_ref().unwrap().guests);
    // A fleet that loses readiness reverts to the retained conservative list/cap.
    f.org.readiness.abort();
    f.org.readiness =
        crate::services::security_readiness_service::legacy_org_guest_test_replica(&db).await;
    let access = validate(&db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    assert!(!access.allows_service(&service));
    assert!(
        adoption::adopt(
            &db,
            &f.org.admin,
            &f.agent.id,
            adoption::Adoption {
                review: "a".repeat(64)
            }
        )
        .await
        .is_err()
    );
    db.drop().await.unwrap();
}

#[test]
fn cutover_old_reader_ignores_adoption_fields_and_retains_legacy_authority() {
    #[derive(serde::Deserialize)]
    struct OldPolicy {
        enabled: bool,
        service_ids: Vec<String>,
        service_authority: std::collections::BTreeMap<String, String>,
        #[serde(with = "crate::models::credits::whole::optional")]
        daily_credits: Option<Credits>,
    }
    let p = GuestPolicy {
        follows_agent_grants: true,
        grants_adoption_id: Some("review".into()),
        enabled: true,
        daily_credits: Some(Credits::from_whole(1)),
        ..Default::default()
    };
    let old: OldPolicy = bson::from_document(bson::to_document(&p).unwrap()).unwrap();
    assert!(old.enabled);
    assert!(old.service_ids.is_empty());
    assert!(old.service_authority.is_empty());
    assert_eq!(old.daily_credits, Some(Credits::from_whole(1)));
    let absent: GuestPolicy = bson::from_document(doc! {}).unwrap();
    assert!(!absent.follows_agent_grants);
}

#[tokio::test]
async fn cutover_org_payer_still_covers_byok_and_platform_keys_and_refuses_suspension() {
    use crate::models::usage_meter::CredentialClass as C;
    use crate::services::billing::owner_resolver::{ExecutionLane, PaysFrom};
    let mut f = Fixture::new("org_guest_cutover_billing").await;
    f.wallet().await;
    f.guests(vec![]).await;
    ready(&mut f).await;
    adopt(&f).await;
    reload(&mut f).await;
    let db = &f.org.state.db;
    let origin = f.guest("visitor").await;
    let access = validate(db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    let payer = access.channel().unwrap().guest_payer().unwrap();
    for class in [C::NyxidManagedMaster, C::UserOwned] {
        let resolved = f
            .org
            .state
            .billing
            .owner_resolver()
            .resolve_for_execution(
                &f.org.org,
                &f.org.org,
                class,
                ExecutionLane::OrgChannelGuest(&payer),
            )
            .await
            .unwrap();
        assert_eq!(resolved.owner_id, f.org.org);
        assert_eq!(
            resolved.pays,
            PaysFrom::OrgWallet {
                org_id: f.org.org.clone()
            }
        );
    }
    db.collection::<Document>("billing_wallet")
        .update_one(
            doc! {"owner_id":&f.org.org},
            doc! {"$set":{"suspended":true}},
        )
        .await
        .unwrap();
    assert!(matches!(
        budget::require_payer(db, &payer).await,
        Err(AppError::WalletSuspended)
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn cutover_guest_grants_use_existing_http_evaluator_and_withheld_scope_on_live_key() {
    use crate::services::{
        agent_operation_scope_service as ops, proxy_authorization::CanonicalPath,
    };
    let mut f = Fixture::new("org_guest_cutover_operations").await;
    f.wallet().await;
    let service = grant(&f, "Calendar").await;
    f.guests(vec![]).await;
    ready(&mut f).await;
    adopt(&f).await;
    reload(&mut f).await;
    let origin = f.guest("visitor").await;
    let row = channel_turn(&f, &origin, "use Calendar").await;
    let key = f.org.key(&row).await;
    let db = &f.org.state.db;
    let auth = Box::pin(crate::mw::auth::api_key_auth_user(
        db, &key, None, None, None,
    ))
    .await
    .unwrap();
    let path = CanonicalPath::from_mcp_literal("/items").unwrap();
    assert!(
        ops::check_non_mcp_context(
            db,
            &auth,
            true,
            &service.id,
            None,
            "GET",
            Some(&path),
            Default::default()
        )
        .await
        .is_ok()
    );
    team::set_grants(
        db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::Guests([(service.id.clone(), GuestAccess::Read)].into()),
    )
    .await
    .unwrap();
    assert!(
        ops::check_non_mcp_context(
            db,
            &auth,
            true,
            &service.id,
            None,
            "POST",
            Some(&path),
            Default::default()
        )
        .await
        .is_err()
    );
    // Reuse the existing empty selection; no GuestAccess enum expansion.
    db.collection::<Document>("assistant_agents").update_one(doc!{"_id":&f.agent.id},doc!{"$set":{
        format!("guest_operation_scopes.{}",service.id):{"revision":1,"catalog_service_id":null,"operations":[]}
    }}).await.unwrap();
    assert!(
        ops::check_non_mcp_context(
            db,
            &auth,
            true,
            &service.id,
            None,
            "GET",
            Some(&path),
            Default::default()
        )
        .await
        .is_err()
    );
    let view = management::view(db, &f.org.admin, &f.channel)
        .await
        .unwrap();
    assert!(
        view["effective_guest_services"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(validate(db, &origin, &f.org.org, &f.agent.id).await.is_ok());
    let live_agent = team::agent_for_conversation(db, &row).await.unwrap();
    assert!(
        live_agent.memory.is_empty()
            && live_agent.machine_node_ids.is_empty()
            && live_agent.saved_login_ids.is_empty()
    );
    assert!(
        !auth
            .org_agent_access
            .as_ref()
            .unwrap()
            .approval_admin_permits(&[])
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn cutover_rebuild_preserves_legacy_or_adopted_mode_and_accounting_continuity() {
    for adopted in [false, true] {
        let mut f = Fixture::new("org_guest_cutover_rebuild").await;
        f.wallet().await;
        f.guests(vec![]).await;
        ready(&mut f).await;
        if adopted {
            adopt(&f).await;
            reload(&mut f).await;
        }
        let db = &f.org.state.db;
        db.collection::<Document>("nyxbot_channels")
            .update_one(
                doc! {"_id":&f.channel.id},
                doc! {"$set":{"status":"failed"}},
            )
            .await
            .unwrap();
        let mut rebuilt = f.channel.clone();
        rebuilt.id = uuid::Uuid::new_v4().to_string();
        rebuilt.status = "pending".into();
        rebuilt.created_at = chrono::Utc::now();
        rebuilt.org_specialist = link_target(db, &f.org.admin, &f.bot, &f.agent)
            .await
            .unwrap();
        assert!(
            rebuilt
                .org_specialist
                .as_ref()
                .unwrap()
                .guests
                .follows_agent_grants
        );
        let context = (db.clone(), rebuilt.clone());
        crate::services::transaction_service::run_db_only(db, &context, |tx, (db, row)| {
            Box::pin(async move {
                db.collection::<NyxbotChannel>("nyxbot_channels")
                    .insert_one(row)
                    .session(&mut *tx)
                    .await?;
                management::restore_in_session(db, tx, row).await
            })
        })
        .await
        .unwrap();
        let restored = management::load(db, &f.org.admin, &rebuilt.id, true)
            .await
            .unwrap();
        let policy = &restored.org_specialist.as_ref().unwrap().guests;
        assert_eq!(policy.follows_agent_grants, adopted);
        assert!(policy.enabled);
        assert!(policy.service_ids.is_empty());
        assert_eq!(
            policy.continuity_channel_id.as_deref(),
            Some(f.channel.id.as_str())
        );
        assert_ne!(
            policy.revision,
            f.channel.org_specialist.as_ref().unwrap().guests.revision
        );
        db.drop().await.unwrap();
    }
}

async fn sheets_tools(
    f: &Fixture,
    row: &crate::models::assistant_conversation::AssistantConversation,
) -> Vec<String> {
    use crate::services::{
        assistant_agent_credential_service,
        billing::{BillingIngress, route_inventory::BillingRoutePolicy},
    };
    use axum::{Extension, body::Body, extract::State, http::Request};
    let state = &f.org.state;
    let key = assistant_agent_credential_service::load_for_conversation(
        &state.db,
        &state.encryption_keys,
        &row.user_id,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let request = Request::post("/mcp")
        .header("x-api-key", key.raw_key.as_str())
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        ))
        .unwrap();
    let response = Box::pin(crate::handlers::mcp_transport::mcp_post(
        State(state.clone()),
        Extension(BillingRoutePolicy::Metered(BillingIngress::Mcp)),
        request,
    ))
    .await;
    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let mut names: Vec<_> = reply["result"]["tools"]
        .as_array()
        .expect("MCP tool list")
        .iter()
        .filter_map(|tool| {
            tool["name"]
                .as_str()?
                .strip_prefix("event-sheets__")
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn org_guest_sheets_discovery_and_operations_follow_the_reviewed_binding() {
    use crate::services::{
        agent_operation_scope_service as ops, feature_flag_service as flags,
        proxy_authorization::CanonicalPath,
    };
    let mut f = Fixture::new("org_guest_sheets_tools").await;
    f.wallet().await;
    let service = grant(&f, "event-sheets").await;
    let db = f.org.state.db.clone();
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "api-google-sheets".into();
    db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    )
    .insert_one(&catalog)
    .await
    .unwrap();
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&service.id},
            doc! {"$set":{"catalog_service_id":&catalog.id}},
        )
        .await
        .unwrap();
    let spec = crate::services::catalog_spec_registry::spec_for_slug(&catalog.slug).unwrap();
    let mut selected = Vec::new();
    for (method, path) in [
        ("GET", "/v4/spreadsheets/{spreadsheetId}/values/{range}"),
        (
            "POST",
            "/v4/spreadsheets/{spreadsheetId}/values/{range}:append",
        ),
        ("GET", "/v4/spreadsheets/{spreadsheetId}"),
    ] {
        let op = &spec["paths"][path][method.to_ascii_lowercase()];
        let mut endpoint = crate::test_utils::test_service_endpoint(
            &catalog.id,
            op["operationId"].as_str().unwrap(),
            method,
            path,
        );
        endpoint.parameters = op.get("parameters").cloned();
        if path.contains("/values/") {
            selected.push(endpoint.id.clone());
        }
        db.collection::<crate::models::service_endpoint::ServiceEndpoint>(
            crate::models::service_endpoint::COLLECTION_NAME,
        )
        .insert_one(&endpoint)
        .await
        .unwrap();
    }
    f.guests(vec![]).await;
    ready(&mut f).await;
    flags::set_platform_override(
        &db,
        flags::GUEST_OPERATION_SCOPES_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.org.admin,
    )
    .await
    .unwrap();
    team::set_grants(
        &db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::GuestOperations {
            base: Box::new(GrantChange::Guests(Default::default())),
            selections: [(
                service.id.clone(),
                serde_json::from_value(json!({"expected_revision":0,"endpoint_ids":selected}))
                    .unwrap(),
            )]
            .into(),
        },
    )
    .await
    .unwrap();
    let frozen = channel_turn(&f, &f.guest("visitor").await, "Event time?").await;
    let key = f.org.key(&frozen).await;
    let auth = Box::pin(crate::mw::auth::api_key_auth_user(
        &db, &key, None, None, None,
    ))
    .await
    .unwrap();
    assert!(auth.allowed_service_ids.is_empty());
    let scope = crate::services::mcp_service::caller_service_scope(
        &f.org.org,
        auth.assistant_agent_owner_id.as_deref(),
        auth.org_agent_access.as_deref(),
        auth.assistant_chat.as_deref(),
        auth.allow_all_services,
        &auth.allowed_service_ids,
    );
    assert!(
        matches!(
            scope,
            crate::services::mcp_service::ServiceScope::Allowed([])
        ),
        "model-only discovery must use the empty service allowlist"
    );
    assert!(sheets_tools(&f, &frozen).await.is_empty());
    adopt(&f).await;
    reload(&mut f).await;
    let row = channel_turn(&f, &f.guest("second visitor").await, "Event time?").await;
    assert_eq!(
        sheets_tools(&f, &row).await,
        ["sheets_append_values", "sheets_get_values"]
    );
    let key = f.org.key(&row).await;
    let auth = Box::pin(crate::mw::auth::api_key_auth_user(
        &db, &key, None, None, None,
    ))
    .await
    .unwrap();
    assert_eq!(
        auth.allowed_service_ids.as_slice(),
        std::slice::from_ref(&service.id)
    );
    for (method, path, query, allowed) in [
        (
            "GET",
            "/v4/spreadsheets/event/values/Event_Config",
            None,
            true,
        ),
        (
            "POST",
            "/v4/spreadsheets/event/values/Questions:append",
            Some("insertDataOption=INSERT_ROWS"),
            true,
        ),
        (
            "POST",
            "/v4/spreadsheets/event/values/Questions:append",
            Some("insertDataOption=OVERWRITE"),
            false,
        ),
        ("GET", "/v4/spreadsheets/event", None, false),
    ] {
        let path = CanonicalPath::from_mcp_literal(path).unwrap();
        let res = ops::check_non_mcp_context(
            &db,
            &auth,
            true,
            &service.id,
            Some(&catalog.id),
            method,
            Some(&path),
            ops::GuestHttpContext {
                query,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(res.is_ok(), allowed, "{method} {query:?}: {res:?}");
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn applied_guest_grants_execute_workspace_sheets_read_and_append() {
    use crate::billing_integration_tests::{FakeLago, billing_service, create_usage_index};
    use crate::models::service_approval_config::{
        ApprovalEffect, ApprovalMode, ApprovalRule, ApprovalVerb,
    };
    use crate::services::{
        approval_policy::GlobalApprovalMode, approval_service,
        assistant_agent_credential_service as credentials, destination_routing::tests as google,
        proxy_service::TARGET_HTTP_CLIENT_BUILDER,
    };
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use std::sync::Arc;
    use tower::ServiceExt;

    let mut f = Fixture::new("applied_guest_workspace_sheets").await;
    let db = f.org.state.db.clone();
    google::seed(&db).await;
    let workspace = google::connect(&db, &f.org.org, "api-google-workspace").await;
    let catalog_id = workspace.catalog_service_id.as_deref().unwrap();
    google::connect(&db, &f.org.org, "api-google-drive").await;
    // Exercise the real chat search limit with a tool whose full schema is too large.
    db.collection::<Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .update_one(
            doc! {"service_id":catalog_id,"name":"sheets_append_values"},
            doc! {"$set":{"description":"Sheet append help. ".repeat(40),
                "request_body_schema.properties.values.description":"Detailed row values help. ".repeat(600)}},
        )
        .await
        .unwrap();
    team::set_grants(
        &db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::Add(AgentGrants {
            service_ids: vec![workspace.id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    team::set_grants(
        &db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::Guests([(workspace.id, GuestAccess::Use)].into()),
    )
    .await
    .unwrap();
    create_usage_index(&db).await;
    f.org.state.billing = Arc::new(billing_service(&db, Arc::new(FakeLago::default()), 1000));
    f.org.state.billing.ensure_wallet(&f.org.org).await.unwrap();
    f.guests(vec![]).await;
    ready(&mut f).await;
    adopt(&f).await;
    reload(&mut f).await;
    assert!(
        f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .follows_agent_grants
    );
    assert_eq!(
        approval_service::user_approval_mode(&db, &f.org.org)
            .await
            .unwrap(),
        GlobalApprovalMode::WritesOnly
    );

    let origin = f.guest("visitor").await;
    let row = channel_turn(&f, &origin, "Read the event and record a question").await;
    let turn_id = &row.active_turn.as_ref().unwrap().turn_id;
    let traceparent = crate::turn_trace::TurnContext::for_turn(turn_id)
        .unwrap()
        .traceparent();
    let key = credentials::load_for_conversation(
        &db,
        &f.org.state.encryption_keys,
        &row.user_id,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(row.guest_turn);
    assert_eq!(row.user_id, f.org.org);
    assert_eq!(f.channel.user_id, f.org.admin);
    let echo = google::Echo::start_with_denied_workbook(Some("unshared-workbook")).await;
    f.org.state.http_client = echo.client.clone();
    let (_, router) = crate::routes::build_router_with_state(f.org.state.clone());
    let router = router.with_state(f.org.state.clone());
    let request = |method: &'static str, path: String, body: Option<Value>| {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("traceparent", &traceparent)
            .header("x-api-key", key.raw_key.as_str());
        match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap()
    };
    let rpc = |method: &'static str, params: Value| {
        request(
            "POST",
            "/mcp".into(),
            Some(json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})),
        )
    };
    let exercise_guest_routes = async {
        let list = Box::pin(router.clone().oneshot(rpc("tools/list", json!({}))))
            .await
            .unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        let body = to_bytes(list.into_body(), 1024 * 1024).await.unwrap();
        let list: Value = serde_json::from_slice(&body).unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        let append = tools
            .iter()
            .find(|tool| tool["name"] == "api-google-workspace__sheets_append_values")
            .unwrap();
        assert!(
            append["description"]
                .as_str()
                .unwrap()
                .contains("insertDataOption=INSERT_ROWS")
        );
        assert!(
            append["description"]
                .as_str()
                .unwrap()
                .contains("The effective approval policy must also allow the write")
        );
        assert!(append["inputSchema"]["properties"]["insertDataOption"].is_object());
        assert!(append["inputSchema"].to_string().len() > 10_000);
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "api-google-workspace__sheets_get_values")
        );
        assert!(!tools.iter().any(|tool| {
            tool["name"]
                .as_str()
                .unwrap()
                .starts_with("api-google-drive__")
        }));

        let search = rpc(
            "tools/call",
            json!({"name":"nyx__search_tools","arguments":{"query":"api-google-workspace__sheets_append_values"}}),
        );
        let response = Box::pin(router.clone().oneshot(search)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let response: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(response["result"]["isError"], false, "{response}");
        let search: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let found = &search["matches"][0];
        assert_eq!(found["name"], append["name"]);
        assert_eq!(found["input_schema_omitted"], true);
        assert_eq!(found["chat_access"], "granted");
        assert!(
            found["description"]
                .as_str()
                .unwrap()
                .contains("insertDataOption=INSERT_ROWS")
        );
        assert!(
            found["description"]
                .as_str()
                .unwrap()
                .contains("The effective approval policy must also allow the write")
        );
        assert!(
            search["chat_access_hint"]
                .as_str()
                .unwrap()
                .contains("within the specialist's live guest limits")
        );
        assert!(
            search["chat_access_hint"]
                .as_str()
                .unwrap()
                .contains("Each call must also pass the effective approval policy")
        );

        let sheet = "/api/v1/proxy/s/api-google-workspace/v4/spreadsheets/event/values";
        let written = json!({"values":[["Guest question"]]});
        let append_path = format!(
            "{sheet}/Questions!A:H:append?valueInputOption=RAW&insertDataOption=INSERT_ROWS"
        );
        let args = json!({"spreadsheetId":"event","range":"Questions!A:H","valueInputOption":"RAW","insertDataOption":"INSERT_ROWS","body":{"values":[["MCP question"]]}});
        let mcp_appends = |arguments: &Value| {
            [
                rpc(
                    "tools/call",
                    json!({"name":"nyx__call_tool","arguments":{"tool_name":found["name"],"arguments_json":arguments.to_string()}}),
                ),
                rpc(
                    "tools/call",
                    json!({"name":found["name"],"arguments":arguments}),
                ),
            ]
        };
        let mcp_result = async |call: Request<Body>| {
            let reply = Box::pin(router.clone().oneshot(call)).await.unwrap();
            assert_eq!(reply.status(), StatusCode::OK);
            serde_json::from_slice::<Value>(
                &to_bytes(reply.into_body(), 1024 * 1024).await.unwrap(),
            )
            .unwrap()
        };
        let read = Box::pin(router.clone().oneshot(request(
            "GET",
            format!("{sheet}/Event_Config!A1:B2"),
            None,
        )))
        .await
        .unwrap();
        assert_eq!(read.status(), StatusCode::OK);
        let refusal = Box::pin(router.clone().oneshot(request(
            "POST",
            append_path.clone(),
            Some(written.clone()),
        )))
        .await
        .unwrap();
        assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
        let refusal: Value =
            serde_json::from_slice(&to_bytes(refusal.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(refusal["error"], "api_key_scope_forbidden");
        assert!(
            refusal["message"]
                .as_str()
                .unwrap()
                .contains("Guest calls cannot request or use owner approvals")
        );
        for call in mcp_appends(&args) {
            let reply = mcp_result(call).await;
            assert_eq!(reply["result"]["isError"], true, "{reply}");
            let refusal: Value =
                serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(refusal["error"], "owner_only", "{refusal}");
            assert!(
                refusal["instructions"]
                    .as_str()
                    .unwrap()
                    .contains("anything that needs their approval")
            );
        }
        let mut uncorrelated = request("POST", append_path.clone(), Some(written.clone()));
        uncorrelated.headers_mut().remove("traceparent");
        let refusal = Box::pin(router.clone().oneshot(uncorrelated))
            .await
            .unwrap();
        assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
        let mut overwrite = args.clone();
        overwrite["insertDataOption"] = json!("OVERWRITE");
        for call in mcp_appends(&overwrite) {
            let reply = mcp_result(call).await;
            assert_eq!(reply["result"]["isError"], true, "{reply}");
            let refusal: Value =
                serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(refusal["error"], "owner_only");
        }
        let mut missing_input = args.clone();
        missing_input.as_object_mut().unwrap().remove("range");
        for call in mcp_appends(&missing_input) {
            let reply = mcp_result(call).await;
            assert_eq!(reply["result"]["isError"], true, "{reply}");
            assert!(
                reply["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .starts_with("Invalid tool arguments")
            );
        }
        assert_eq!(
            echo.calls.lock().unwrap().len(),
            1,
            "only the read reached the provider"
        );
        let approval_requests =
            db.collection::<Document>(crate::models::approval_request::COLLECTION_NAME);
        assert_eq!(approval_requests.count_documents(doc! {}).await.unwrap(), 0);

        // The approval descriptor uses the canonical forwarding path on both transports.
        let append_resource = "/v4/spreadsheets/event/values/Questions%21A%3AH:append";
        let rules = [
            ApprovalRule {
                mandatory_review: false,
                methods: vec!["GET".into()],
                resource_pattern: "*".into(),
                verbs: vec![ApprovalVerb::Read],
                effect: ApprovalEffect::AutoAllow,
                mode: ApprovalMode::PerRequest,
            },
            ApprovalRule {
                mandatory_review: false,
                methods: vec!["POST".into()],
                resource_pattern: append_resource.into(),
                verbs: vec![ApprovalVerb::Write],
                effect: ApprovalEffect::AutoAllow,
                mode: ApprovalMode::PerRequest,
            },
        ];
        approval_service::set_service_approval_config(
            &db,
            &f.org.org,
            catalog_id,
            "Google Workspace",
            Some(true),
            Some(&ApprovalMode::PerRequest),
            Some(&rules),
            Some(&ApprovalEffect::RequireApproval),
        )
        .await
        .unwrap();
        assert_eq!(
            approval_service::user_approval_mode(&db, &f.org.org)
                .await
                .unwrap(),
            GlobalApprovalMode::WritesOnly
        );
        for (method, path, body, status) in [
            (
                "POST",
                append_path.clone(),
                Some(written.clone()),
                StatusCode::OK,
            ),
            (
                "POST",
                format!(
                    "{sheet}/QuestionsArchive!A:H:append?valueInputOption=RAW&insertDataOption=INSERT_ROWS"
                ),
                Some(written.clone()),
                StatusCode::FORBIDDEN,
            ),
            (
                "POST",
                format!(
                    "{sheet}/Questions!A:H:append?valueInputOption=RAW&insertDataOption=OVERWRITE"
                ),
                Some(written.clone()),
                StatusCode::FORBIDDEN,
            ),
            (
                "POST",
                format!("{sheet}/Questions!A:H:append?valueInputOption=RAW"),
                Some(written.clone()),
                StatusCode::FORBIDDEN,
            ),
            (
                "GET",
                "/api/v1/proxy/s/api-google-drive/v4/spreadsheets/ungranted/values/A1".into(),
                None,
                StatusCode::FORBIDDEN,
            ),
        ] {
            let response = Box::pin(router.clone().oneshot(request(method, path.clone(), body)))
                .await
                .unwrap();
            let actual = response.status();
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            assert_eq!(
                actual,
                status,
                "{method} {path}: {}",
                String::from_utf8_lossy(&bytes)
            );
        }

        for call in mcp_appends(&args) {
            let reply = mcp_result(call).await;
            assert_eq!(reply["result"]["isError"], false, "{reply}");
        }
        let mut archived_tab = args.clone();
        archived_tab["range"] = json!("QuestionsArchive!A:H");
        for call in mcp_appends(&archived_tab) {
            let reply = mcp_result(call).await;
            assert_eq!(reply["result"]["isError"], true, "{reply}");
            let refusal: Value =
                serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(refusal["error"], "owner_only", "{refusal}");
        }
        assert_eq!(approval_requests.count_documents(doc! {}).await.unwrap(), 0);

        let unrelated =
            "/api/v1/proxy/s/api-google-workspace/v4/spreadsheets/unshared-workbook/values/A1";
        let refusal = Box::pin(
            router
                .clone()
                .oneshot(request("GET", unrelated.into(), None)),
        )
        .await
        .unwrap();
        assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(refusal.into_body(), 1024 * 1024).await.unwrap();
        let refusal: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(refusal["error"]["status"], "PERMISSION_DENIED", "{refusal}");
        assert_eq!(
            refusal["error"]["message"],
            "Fixture account has no access to this workbook"
        );
        let reply = mcp_result(rpc(
            "tools/call",
            json!({
                "name":"api-google-workspace__sheets_get_values",
                "arguments":{"spreadsheetId":"unshared-workbook","range":"A1"}
            }),
        ))
        .await;
        assert_eq!(reply["result"]["isError"], true, "{reply}");

        echo.fail_appends
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let failed = Box::pin(router.clone().oneshot(request(
            "POST",
            append_path,
            Some(written.clone()),
        )))
        .await
        .unwrap();
        assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
        for call in mcp_appends(&args) {
            let reply = mcp_result(call).await;
            assert_eq!(reply["result"]["isError"], true, "{reply}");
            assert!(
                reply["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("Error (500)")
            );
        }

        let calls = echo.calls.lock().unwrap();
        assert_eq!(calls.len(), 9);
        assert!(calls[6..].iter().all(|call| {
            call["request"]
                .as_str()
                .unwrap()
                .starts_with(&format!("POST {append_resource}?"))
        }));
        assert!(
            calls
                .iter()
                .all(|call| call["host"] == "sheets.googleapis.com")
        );
        assert!(calls[0]["request"].as_str().unwrap().starts_with("GET "));
        assert!(
            calls[4]["request"]
                .as_str()
                .unwrap()
                .starts_with("GET /v4/spreadsheets/unshared-workbook/")
        );
        assert!(calls.iter().all(|call| {
            call["headers"]
                .as_str()
                .unwrap()
                .lines()
                .any(|line| line.eq_ignore_ascii_case("authorization: Bearer test-google-token"))
        }));
        for (call, body) in
            calls[1..4]
                .iter()
                .zip([written, args["body"].clone(), args["body"].clone()])
        {
            let request = call["request"].as_str().unwrap();
            assert!(
                request.starts_with(&format!("POST {append_resource}?")),
                "{request}"
            );
            assert!(
                request.contains("valueInputOption=RAW")
                    && request.contains("insertDataOption=INSERT_ROWS"),
                "{request}"
            );
            let received: Value = serde_json::from_str(call["body"].as_str().unwrap()).unwrap();
            assert_eq!(received, body);
        }
    };
    let (_, traces) = crate::turn_trace::capture(
        TARGET_HTTP_CLIENT_BUILDER.scope(echo.client_builder.clone(), exercise_guest_routes),
    )
    .await;
    for (stage, reason, status, count) in [
        ("proxy", "approval_required", Some(403), 2),
        ("proxy", "guest_operation_denied", Some(403), 2),
        ("proxy", "downstream_http_error", Some(403), 1),
        ("proxy", "downstream_http_error", Some(500), 1),
        ("mcp", "approval_required", None, 4),
        ("mcp", "guest_operation_denied", None, 2),
        ("mcp", "invalid_tool_arguments", None, 2),
        ("mcp", "downstream_http_error", Some(403), 1),
        ("mcp", "downstream_http_error", Some(500), 2),
    ] {
        let outcome = if reason == "downstream_http_error" {
            "failed"
        } else {
            "rejected"
        };
        let observed = traces.iter().filter(|record| {
            record["turn_id"] == *turn_id
                && record["stage"] == stage
                && record["guest_service_failure"] == reason
                && record["outcome"] == outcome
                && record["http_status"] == json!(status)
        });
        assert_eq!(observed.count(), count, "{stage} {reason}");
    }
    let gap = traces
        .iter()
        .find(|record| {
            record["gap_reason"] == "missing_parent"
                && record["guest_service_failure"] == "approval_required"
        })
        .unwrap();
    assert!(gap["turn_id"].is_null() && gap["trace_id"].is_null());
    assert!(
        traces
            .iter()
            .any(|record| record["gap_reason"] == "missing_parent"
                && record["guest_service_failure"].is_null()
                && record["request_id"] == gap["request_id"])
    );
    let trace_text = json!(traces).to_string();
    for private in [
        "api-google-workspace",
        "sheets_append_values",
        "unshared-workbook",
        "Guest question",
        "INSERT_ROWS",
        "test-google-token",
    ] {
        assert!(!trace_text.contains(private));
    }
    let saved = crate::services::assistant_nyxagent::get(&db, &row.user_id, &row.id)
        .await
        .unwrap();
    let active = saved.active_turn.unwrap();
    assert_eq!(active.turn_id, *turn_id);
    for (reason, kind, status, count) in [
        ("approval_required", "tool_error", None, 4),
        ("guest_operation_denied", "tool_error", None, 2),
        ("invalid_tool_arguments", "tool_error", None, 2),
        ("downstream_http_error", "service_status", Some(403), 1),
        ("downstream_http_error", "service_status", Some(500), 2),
    ] {
        let observed = active
            .activities
            .iter()
            .filter_map(|activity| activity.failure.as_ref())
            .filter(|failure| {
                failure.reason.as_deref() == Some(reason)
                    && failure.kind == kind
                    && failure.status == status
            });
        assert_eq!(observed.count(), count, "activity {reason}");
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn separate_sheets_catalog_allows_one_guest_append_and_preserves_workspace_owner_put() {
    use crate::billing_integration_tests::{FakeLago, billing_service, create_usage_index};
    use crate::models::service_approval_config::{
        ApprovalEffect, ApprovalMode, ApprovalRule, ApprovalVerb,
    };
    use crate::services::{
        approval_policy::GlobalApprovalMode, approval_service,
        assistant_agent_credential_service as credentials, destination_routing::tests as google,
        feature_flag_service as flags, proxy_service::TARGET_HTTP_CLIENT_BUILDER,
    };
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use std::sync::Arc;
    use tower::ServiceExt;

    let mut f = Fixture::new("guest_sheets_catalog_policy").await;
    let db = f.org.state.db.clone();
    google::seed(&db).await;
    let workspace = google::connect(&db, &f.org.org, "api-google-workspace").await;
    let sheets = google::connect(&db, &f.org.org, "api-google-sheets").await;
    let workspace_id = workspace.catalog_service_id.as_deref().unwrap();
    let sheets_id = sheets.catalog_service_id.as_deref().unwrap();
    assert_ne!(workspace_id, sheets_id);
    assert_ne!(workspace.api_key_id, sheets.api_key_id);
    let guest_token = "test-guest-sheets-token";
    let encrypted = f
        .org
        .state
        .encryption_keys
        .encrypt(guest_token.as_bytes())
        .await
        .unwrap();
    db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id": sheets.api_key_id.as_deref().unwrap()},
            doc! {"$set": {"access_token_encrypted": bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted,
            }}},
        )
        .await
        .unwrap();
    team::set_grants(
        &db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::Add(AgentGrants {
            service_ids: vec![workspace.id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    create_usage_index(&db).await;
    f.org.state.billing = Arc::new(billing_service(&db, Arc::new(FakeLago::default()), 1000));
    f.org.state.billing.ensure_wallet(&f.org.org).await.unwrap();
    f.guests(vec![]).await;
    ready(&mut f).await;
    adopt(&f).await;
    reload(&mut f).await;
    let before = management::view(&db, &f.org.admin, &f.channel)
        .await
        .unwrap();
    assert_eq!(before["guest_policy_mode"], "grants");
    let before_services = before["effective_guest_services"].as_array().unwrap();
    assert_eq!(before_services.len(), 1);
    assert_eq!(before_services[0]["id"], workspace.id);
    assert_eq!(before_services[0]["access"], "use");

    flags::set_platform_override(
        &db,
        flags::GUEST_OPERATION_SCOPES_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.org.admin,
    )
    .await
    .unwrap();
    let agent = team::set_grants(
        &db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::GuestOperations {
            base: Box::new(GrantChange::Add(AgentGrants {
                service_ids: vec![sheets.id.clone()],
                ..Default::default()
            })),
            selections: [(
                workspace.id.clone(),
                serde_json::from_value(
                    json!({"expected_revision":0,"all_operations":false,"endpoint_ids":[]}),
                )
                .unwrap(),
            )]
            .into(),
        },
    )
    .await
    .unwrap();
    assert!(agent.grants.service_ids.contains(&workspace.id));
    reload(&mut f).await;
    let after = management::view(&db, &f.org.admin, &f.channel)
        .await
        .unwrap();
    assert_eq!(after["guest_policy_mode"], "grants");
    let after_services = after["effective_guest_services"].as_array().unwrap();
    assert_eq!(after_services.len(), 1);
    assert_eq!(after_services[0]["id"], sheets.id);
    assert_eq!(after_services[0]["access"], "use");

    // Model an already-allowed owner write with an exact rule on the Workspace catalog.
    let owner_resource = "/v4/spreadsheets/event/values/Event_Config%21B24";
    approval_service::set_service_approval_config(
        &db,
        &f.org.org,
        workspace_id,
        "Google Workspace",
        Some(true),
        Some(&ApprovalMode::PerRequest),
        Some(&[
            ApprovalRule {
                mandatory_review: false,
                methods: vec!["GET".into()],
                resource_pattern: "*".into(),
                verbs: vec![ApprovalVerb::Read],
                effect: ApprovalEffect::AutoAllow,
                mode: ApprovalMode::PerRequest,
            },
            ApprovalRule {
                mandatory_review: false,
                methods: vec!["PUT".into()],
                resource_pattern: owner_resource.into(),
                verbs: vec![ApprovalVerb::Write, ApprovalVerb::Destructive],
                effect: ApprovalEffect::AutoAllow,
                mode: ApprovalMode::PerRequest,
            },
        ]),
        Some(&ApprovalEffect::RequireApproval),
    )
    .await
    .unwrap();
    let owner_token = crate::crypto::jwt::generate_access_token(
        &f.org.state.jwt_keys,
        &f.org.state.config,
        &uuid::Uuid::parse_str(&f.org.admin).unwrap(),
        "proxy:*",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let origin = f.guest("visitor").await;
    let row = channel_turn(&f, &origin, "Record a question").await;
    assert!(row.guest_turn);
    let key = credentials::load_for_conversation(
        &db,
        &f.org.state.encryption_keys,
        &row.user_id,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap();

    let echo = google::Echo::start_with_denied_workbook(Some("unshared-workbook")).await;
    f.org.state.http_client = echo.client.clone();
    let (_, router) = crate::routes::build_router_with_state(f.org.state.clone());
    let router = router.with_state(f.org.state.clone());
    let guest = async |method: &str, path: &str, body: Value| {
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        Box::pin(
            router.clone().oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("x-api-key", key.raw_key.as_str())
                    .header("content-type", "application/json")
                    .body(body)
                    .unwrap(),
            ),
        )
        .await
        .unwrap()
    };
    let owner_call = async |method: &str, resource: &str, body: Body| {
        let response = Box::pin(
            router.clone().oneshot(
                Request::builder()
                    .method(method)
                    .uri(format!(
                        "/api/v1/proxy/s/api-google-workspace{resource}&_nyxid_via={}",
                        workspace.id
                    ))
                    .header("authorization", format!("Bearer {owner_token}"))
                    .header("content-type", "application/json")
                    .body(body)
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    };
    let owner_calls = async || {
        owner_call(
            "GET",
            "/v4/spreadsheets/event/values/Event_Config%21A1%3AB2?majorDimension=ROWS",
            Body::empty(),
        )
        .await;
        owner_call(
            "PUT",
            &format!("{owner_resource}?valueInputOption=RAW"),
            Body::from(json!({"values":[["owner update"]]}).to_string()),
        )
        .await;
    };
    TARGET_HTTP_CLIENT_BUILDER.scope(echo.client_builder.clone(), async {
        assert_eq!(approval_service::user_approval_mode(&db, &f.org.org).await.unwrap(), GlobalApprovalMode::WritesOnly);
        owner_calls().await;
        let values = json!({"values":[["guest question"]]});
        let append = "/v4/spreadsheets/event/values/Questions%21A%3AH:append";
        let raw = format!("/api/v1/proxy/s/api-google-sheets{append}?valueInputOption=RAW&insertDataOption=INSERT_ROWS");
        let refusal = guest("POST", &raw, values.clone()).await;
        assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(refusal.into_body(), 1024 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Guest calls cannot request or use owner approvals"));
        assert_eq!(echo.calls.lock().unwrap().len(), 2);

        approval_service::set_service_approval_config(
            &db,
            &f.org.org,
            sheets_id,
            "Google Sheets",
            Some(true),
            Some(&ApprovalMode::PerRequest),
            Some(&[
                ApprovalRule {
                    mandatory_review: false,
                    methods: vec!["GET".into()],
                    resource_pattern: "*".into(),
                    verbs: vec![ApprovalVerb::Read],
                    effect: ApprovalEffect::AutoAllow,
                    mode: ApprovalMode::PerRequest,
                },
                ApprovalRule {
                    mandatory_review: false,
                    methods: vec!["POST".into()],
                    resource_pattern: append.into(),
                    verbs: vec![ApprovalVerb::Write],
                    effect: ApprovalEffect::AutoAllow,
                    mode: ApprovalMode::PerRequest,
                },
            ]),
            Some(&ApprovalEffect::RequireApproval),
        )
        .await
        .unwrap();
        owner_calls().await;

        let tools = guest("POST", "/mcp", json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})).await;
        assert_eq!(tools.status(), StatusCode::OK);
        let body = to_bytes(tools.into_body(), 1024 * 1024).await.unwrap();
        let list: Value = serde_json::from_slice(&body).unwrap();
        let names: Vec<_> = list["result"]["tools"].as_array().unwrap().iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert!(names.contains(&"api-google-sheets__sheets_get_values"));
        assert!(!names.iter().any(|name| name.starts_with("api-google-workspace__")));

        let values_path = "/v4/spreadsheets/event/values/Event_Config%21A1%3AB2";
        let workspace_read = format!("/api/v1/proxy/s/api-google-workspace{values_path}");
        let response = guest("GET", &workspace_read, Value::Null).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let refusal: Value = serde_json::from_slice(&body).unwrap();
        assert!(refusal["message"].as_str().unwrap().contains("Allowed operations: none"), "{refusal}");
        assert_eq!(echo.calls.lock().unwrap().len(), 4);

        let sheets_read = format!("/api/v1/proxy/s/api-google-sheets{values_path}");
        let response = guest("GET", &sheets_read, Value::Null).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let event: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(event["request"], format!("GET {values_path} HTTP/1.1"));

        let unrelated_read = "/api/v1/proxy/s/api-google-sheets/v4/spreadsheets/unshared-workbook/values/A1";
        let response = guest("GET", unrelated_read, Value::Null).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let denial: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(denial["error"]["status"], "PERMISSION_DENIED");
        let response = guest("POST", &raw, values.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);

        let arguments = json!({"spreadsheetId":"event","range":"Questions!A:H","valueInputOption":"RAW","insertDataOption":"INSERT_ROWS","body":values});
        let tool_name = "api-google-sheets__sheets_append_values";
        for (name, arguments) in [
            (tool_name, arguments.clone()),
            ("nyx__call_tool", json!({"tool_name":tool_name,"arguments_json":arguments.to_string()})),
        ] {
            let response = guest("POST", "/mcp", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}})).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            let result: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(result["result"]["isError"], false, "{result}");
        }

        let other_tab = raw.replace("Questions%21", "QuestionsArchive%21");
        let other_write = "/api/v1/proxy/s/api-google-sheets/v4/spreadsheets";
        for (path, body) in [
            (other_tab.as_str(), values.clone()),
            (other_write, json!({"properties":{"title":"guest workbook"}})),
        ] {
            let refusal = guest("POST", path, body).await;
            assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
            let body = to_bytes(refusal.into_body(), 1024 * 1024).await.unwrap();
            assert!(String::from_utf8_lossy(&body).contains("Guest calls cannot request or use owner approvals"));
        }
        let calls = echo.calls.lock().unwrap();
        assert_eq!(calls.len(), 9);
        for (before, after) in calls[..2].iter().zip(&calls[2..4]) {
            assert_eq!(before["request"], after["request"]);
            assert_eq!(before["body"], after["body"]);
        }
        for (call, expected) in calls.iter().zip(["test-google-token"; 4].into_iter().chain([guest_token; 5])) {
            assert_eq!(call["host"], "sheets.googleapis.com");
            let headers = call["headers"].as_str().unwrap();
            assert!(headers.lines().any(|line| line.eq_ignore_ascii_case(&format!("authorization: Bearer {expected}"))));
        }
        assert_eq!(calls[4]["request"], format!("GET {values_path} HTTP/1.1"));
        assert_eq!(calls[5]["request"], "GET /v4/spreadsheets/unshared-workbook/values/A1 HTTP/1.1");
        for call in &calls[6..] {
            let request = call["request"].as_str().unwrap();
            assert!(request.starts_with(&format!("POST {append}?")), "{request}");
            assert!(request.contains("insertDataOption=INSERT_ROWS"), "{request}");
        }
    }).await;
    let approval_requests =
        db.collection::<Document>(crate::models::approval_request::COLLECTION_NAME);
    assert_eq!(approval_requests.count_documents(doc! {}).await.unwrap(), 0);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn workspace_booking_append_checks_the_connection_owners_policy_for_both_guest_turns() {
    use crate::billing_integration_tests::{FakeLago, billing_service, create_usage_index};
    use crate::models::assistant_conversation::TurnOrigin;
    use crate::models::service_approval_config::{
        ApprovalEffect, ApprovalMode, ApprovalRule, ApprovalVerb,
    };
    use crate::services::{
        approval_service, assistant_agent_credential_service as credentials,
        assistant_nyxagent as engine, destination_routing::tests as google,
        feature_flag_service as flags, proxy_service::TARGET_HTTP_CLIENT_BUILDER,
    };
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use std::sync::Arc;
    use tower::ServiceExt;

    let mut f = Fixture::new("guest_booking_append").await;
    let db = f.org.state.db.clone();
    google::seed(&db).await;
    create_usage_index(&db).await;
    f.org.state.billing = Arc::new(billing_service(&db, Arc::new(FakeLago::default()), 1000));
    f.org.state.billing.ensure_wallet(&f.org.org).await.unwrap();
    f.guests(vec![]).await;
    ready(&mut f).await;
    adopt(&f).await;
    reload(&mut f).await;
    flags::set_platform_override(
        &db,
        flags::GUEST_OPERATION_SCOPES_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.org.admin,
    )
    .await
    .unwrap();

    let person = &f.org.outsider;
    let (personal_agent, _) = team::create_specialist(
        &db,
        &f.org.state.encryption_keys,
        person,
        f.org.request("booking"),
    )
    .await
    .unwrap()
    .unwrap();
    let echo = google::Echo::start().await;
    f.org.state.http_client = echo.client.clone();
    let (_, router) = crate::routes::build_router_with_state(f.org.state.clone());
    let router = router.with_state(f.org.state.clone());
    let resource = "/v4/spreadsheets/event/values/%E7%99%BB%E8%AE%B0%21A%3AH:append";
    let values = json!({"values":[["reservation"]]});
    for (owner, admin, agent) in [
        (person, person, &personal_agent.id),
        (&f.org.org, &f.org.admin, &f.agent.id),
    ] {
        let workspace = google::connect(&db, owner, "api-google-workspace").await;
        let catalog_id = workspace.catalog_service_id.as_deref().unwrap();
        let ids = db
            .collection::<Document>(crate::models::service_endpoint::COLLECTION_NAME)
            .distinct(
                "_id",
                doc! {"service_id":catalog_id,"name":{"$in":[
                "sheets_get_spreadsheet","sheets_get_values","sheets_append_values"]}},
            )
            .await
            .unwrap();
        assert_eq!(ids.len(), 3);
        team::set_grants(
            &db,
            admin,
            agent,
            GrantChange::GuestOperations {
                base: Box::new(GrantChange::Add(AgentGrants {
                    service_ids: vec![workspace.id.clone()],
                    ..Default::default()
                })),
                selections: [(
                    workspace.id,
                    serde_json::from_value(json!({
                        "expected_revision":0,"endpoint_ids":ids
                    }))
                    .unwrap(),
                )]
                .into(),
            },
        )
        .await
        .unwrap();
        let row = if owner == person {
            let request: engine::TurnRequest =
                serde_json::from_value(json!({"agent_id":agent,"text":"Book a seat"})).unwrap();
            let mut start = engine::TurnStart::from(&request);
            start.origin = TurnOrigin::Channel;
            start.guest = true;
            Box::pin(engine::begin_turn(
                &db,
                owner,
                start,
                &f.org.state.encryption_keys,
            ))
            .await
            .unwrap()
        } else {
            Box::pin(channel_turn(&f, &f.guest("visitor").await, "Book a seat")).await
        };
        assert!(row.guest_turn);
        assert_eq!(&row.user_id, owner);
        let key =
            credentials::load_for_conversation(&db, &f.org.state.encryption_keys, owner, &row.id)
                .await
                .unwrap()
                .unwrap();
        let append = async || {
            let request = Request::post("/mcp")
                .header("x-api-key", key.raw_key.as_str())
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
                    "name":"api-google-workspace__sheets_append_values","arguments":{
                        "spreadsheetId":"event","range":"登记!A:H","valueInputOption":"RAW",
                        "insertDataOption":"INSERT_ROWS","body":values
                    }}})
                    .to_string(),
                ))
                .unwrap();
            let response = TARGET_HTTP_CLIENT_BUILDER
                .scope(
                    echo.client_builder.clone(),
                    Box::pin(router.clone().oneshot(request)),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            serde_json::from_slice::<Value>(&bytes).unwrap()["result"].clone()
        };
        let before = echo.calls.lock().unwrap().len();
        let result = append().await;
        assert_eq!(result["isError"], true);
        let refusal: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            refusal,
            crate::services::assistant_acknowledgement_service::guest_refusal()
        );
        assert_eq!(echo.calls.lock().unwrap().len(), before);

        approval_service::set_service_approval_config(
            &db,
            owner,
            catalog_id,
            "Google Workspace",
            Some(true),
            Some(&ApprovalMode::PerRequest),
            Some(&[ApprovalRule {
                mandatory_review: false,
                methods: vec!["POST".into()],
                resource_pattern: resource.into(),
                verbs: vec![ApprovalVerb::Write],
                effect: ApprovalEffect::AutoAllow,
                mode: ApprovalMode::PerRequest,
            }]),
            Some(&ApprovalEffect::RequireApproval),
        )
        .await
        .unwrap();
        let result = append().await;
        assert_eq!(result["isError"], false, "{}", result["content"][0]["text"]);
        let calls = echo.calls.lock().unwrap();
        assert_eq!(calls.len(), before + 1);
        let call = calls.last().unwrap();
        assert_eq!(call["host"], "sheets.googleapis.com");
        let request = call["request"].as_str().unwrap();
        assert!(request.starts_with(&format!("POST {resource}?")));
        assert!(
            request.contains("valueInputOption=RAW")
                && request.contains("insertDataOption=INSERT_ROWS")
        );
        assert_eq!(
            serde_json::from_str::<Value>(call["body"].as_str().unwrap()).unwrap(),
            values
        );
    }
    assert_eq!(
        db.collection::<Document>(crate::models::approval_request::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}
