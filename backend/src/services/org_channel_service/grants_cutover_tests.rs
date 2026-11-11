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
    let view = management::view(&db, &f.org.admin, &empty).await.unwrap();
    assert_eq!(view["guest_policy_mode"], "legacy");
    assert!(
        view["effective_guest_services"]
            .as_array()
            .unwrap()
            .is_empty()
    );
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
    assert!(!binding.guests.enabled);
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
