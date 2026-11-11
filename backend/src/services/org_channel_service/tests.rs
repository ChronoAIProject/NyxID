use super::*;
use crate::models::credits::Credits;

#[test]
fn absent_guest_policy_is_closed_and_exact_cap_round_trips() {
    let policy: GuestPolicy = bson::from_document(doc! {}).unwrap();
    assert!(!policy.enabled);
    assert!(policy.daily_credits.is_none());
    let policy = GuestPolicy {
        daily_credits: Some("0.000000000001".parse::<Credits>().unwrap()),
        ..Default::default()
    };
    let stored = bson::to_document(&policy).unwrap();
    assert!(matches!(
        stored.get("daily_credits"),
        Some(bson::Bson::Decimal128(_))
    ));
    assert_eq!(bson::from_document::<GuestPolicy>(stored).unwrap(), policy);
}

#[test]
fn guest_rate_identity_is_binding_scoped_and_does_not_store_sender_id() {
    assert_ne!(sender_hash("one", "123"), sender_hash("two", "123"));
    assert_eq!(sender_hash("one", "123"), sender_hash("one", "123"));
    assert_eq!(sender_hash("one", "123").len(), 64);
}

#[test]
fn budget_day_changes_at_utc_midnight() {
    let before = chrono::DateTime::parse_from_rfc3339("2026-11-04T23:59:59Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(budget::day_key("channel", before), "channel:2026-11-04");
    assert_eq!(
        budget::day_key("channel", before + chrono::Duration::seconds(1)),
        "channel:2026-11-05"
    );
}

pub(crate) struct Fixture {
    pub(crate) org: crate::services::org_agent_tests::Fixture,
    channel: NyxbotChannel,
    pub(crate) agent: AssistantAgent,
    bot: ChannelBot,
}
impl Fixture {
    pub(crate) async fn new(name: &str) -> Self {
        let mut org = crate::services::org_agent_tests::Fixture::new(name).await;
        // Existing #2215 coverage explicitly models the pre-cutover fleet.
        org.readiness.abort();
        org.readiness =
            super::super::security_readiness_service::legacy_org_guest_test_replica(&org.state.db)
                .await;
        let (agent, _) = org.create().await;
        let db = &org.state.db;
        ensure_indexes(db).await.unwrap();
        crate::services::feature_flag_service::set_platform_override(
            db,
            FLAG,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            &org.admin,
        )
        .await
        .unwrap();
        let bot:ChannelBot=bson::from_document(doc!{"_id":uuid::Uuid::new_v4().to_string(),"user_id":&org.org,"platform":"lark","label":"Organization bot",
            "platform_bot_id":"123","platform_bot_username":"helper","bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},
            "webhook_secret_hash":"hash","webhook_registered":true,"status":"active","is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
        db.collection::<ChannelBot>("channel_bots")
            .insert_one(&bot)
            .await
            .unwrap();
        let binding = link_target(db, &org.admin, &bot, &agent)
            .await
            .unwrap()
            .unwrap();
        let channel:NyxbotChannel=bson::from_document(doc!{"_id":uuid::Uuid::new_v4().to_string(),"user_id":&org.admin,"channel_bot_id":&bot.id,"bot_owner_id":&org.org,
            "platform":"lark","bot_label":"Organization bot","transport":"direct","status":"active","route_api_key_id":uuid::Uuid::new_v4().to_string(),
            "route_id":uuid::Uuid::new_v4().to_string(),"agent_id":&agent.id,"org_specialist":bson::to_bson(&binding).unwrap(),"private_chats":"everyone",
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
        db.collection::<NyxbotChannel>("nyxbot_channels")
            .insert_one(&channel)
            .await
            .unwrap();
        db.collection::<Document>("api_keys")
            .insert_one(
                doc! {"_id":&channel.route_api_key_id,"user_id":&org.org,"is_active":true,
                "name":"Org channel route","key_prefix":"test","key_hash":"test","scopes":"",
                "callback_url":"https://example.com/channel","created_at":bson::DateTime::now()},
            )
            .await
            .unwrap();
        db.collection::<Document>("channel_conversations").insert_one(doc!{"_id":&channel.route_id,"user_id":&org.org,"channel_bot_id":&bot.id,
            "agent_api_key_id":&channel.route_api_key_id,"is_active":true,"platform":"lark","platform_conversation_id":"chat",
            "platform_conversation_type":"private","default_agent":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
        db.collection::<Document>("nyxbot_threads").insert_one(doc!{"_id":uuid::Uuid::new_v4().to_string(),"channel_id":&channel.id,"user_id":&org.admin,
            "partition":"private","kind":"private","platform_chat_id":"chat","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
        Self {
            org,
            channel,
            agent,
            bot,
        }
    }
    async fn link(&self, actor: &str, sender_id: &str) -> ChannelOrigin {
        let db = &self.org.state.db;
        let issued = links::issue(db, actor, &self.channel).await.unwrap();
        assert!(
            links::receive(
                db,
                &self.channel,
                sender_id,
                issued["code"].as_str().unwrap(),
                Some("Account")
            )
            .await
            .unwrap()
        );
        links::confirm(db, actor, &self.channel, issued["id"].as_str().unwrap())
            .await
            .unwrap();
        let (execution, _) = sender(db, &self.channel, sender_id, true, &self.agent.id)
            .await
            .unwrap();
        ChannelOrigin {
            org_execution: Some(Box::new(execution)),
            native_event_id: None,
            thread: None,
            nyxbot_channel_id: self.channel.id.clone(),
            partition: "private".into(),
            platform: "lark".into(),
        }
    }
    pub(crate) async fn wallet(&self) {
        self.org.state.db.collection::<Document>("billing_wallet").insert_one(doc!{"_id":uuid::Uuid::new_v4().to_string(),"owner_id":&self.org.org,"lago_customer_id":"customer",
            "lago_wallet_id":"wallet","lago_subscription_id":"subscription","plan_kind":"prepaid","collection_state":"good",
            "balance_credits":Credits::from_whole(10),"balance_synced_at":bson::DateTime::now(),"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    }
    pub(crate) async fn guests(&mut self, ids: Vec<String>) {
        management::update(
            &self.org.state.db,
            &self.org.admin,
            &self.channel,
            management::GuestSettings {
                expected_revision: None,
                enabled: true,
                service_ids: Some(ids),
                daily_credits: Some(Credits::from_whole(1)),
            },
        )
        .await
        .unwrap();
        self.channel =
            management::load(&self.org.state.db, &self.org.admin, &self.channel.id, true)
                .await
                .unwrap();
    }
    pub(crate) async fn guest(&self, sender_id: &str) -> ChannelOrigin {
        let (execution, _) = sender(
            &self.org.state.db,
            &self.channel,
            sender_id,
            true,
            &self.agent.id,
        )
        .await
        .unwrap();
        ChannelOrigin {
            org_execution: Some(Box::new(execution)),
            native_event_id: None,
            thread: None,
            nyxbot_channel_id: self.channel.id.clone(),
            partition: "private".into(),
            platform: "lark".into(),
        }
    }
}

#[tokio::test]
async fn org_channel_members_cannot_clear_guest_restrictions_but_can_maintain_grants() {
    use crate::models::assistant_agent::{AgentGrants, GuestAccess};
    use crate::services::assistant_team_service::{self as team, GrantChange};
    use std::collections::BTreeMap;

    let f = Fixture::new("org_channel_guest_policy_admin").await;
    let db = &f.org.state.db;
    let service = f.org.service(&f.org.org, "guest-service").await;
    let grants = AgentGrants {
        service_ids: vec![service.clone()],
        ..Default::default()
    };
    team::set_grants(
        db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::Replace {
            grants: grants.clone(),
            guests: BTreeMap::from([(service.clone(), GuestAccess::Read)]),
        },
    )
    .await
    .unwrap();
    let selections = BTreeMap::from([(
        service.clone(),
        crate::models::agent_operation_scope::OperationSelection {
            expected_revision: 0,
            all_operations: true,
            endpoint_ids: vec![],
            rules: vec![],
            contract_digest: None,
            inputs: BTreeMap::new(),
        },
    )]);
    for change in [
        GrantChange::Guests(BTreeMap::from([(service.clone(), GuestAccess::Use)])),
        GrantChange::Replace {
            grants: grants.clone(),
            guests: BTreeMap::from([(service.clone(), GuestAccess::Use)]),
        },
        GrantChange::GuestOperations {
            base: Box::new(GrantChange::Add(AgentGrants::default())),
            selections,
        },
    ] {
        assert!(matches!(
            team::set_grants(db, &f.org.member, &f.agent.id, change).await,
            Err(crate::errors::AppError::Forbidden(_))
        ));
    }
    let maintained = team::set_grants(
        db,
        &f.org.member,
        &f.agent.id,
        GrantChange::Replace {
            grants,
            guests: BTreeMap::from([(service.clone(), GuestAccess::Read)]),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        maintained.guest_access.get(&service),
        Some(&GuestAccess::Read)
    );
    let cleared = team::set_grants(
        db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::Guests(BTreeMap::from([(service, GuestAccess::Use)])),
    )
    .await
    .unwrap();
    assert!(cleared.guest_access.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn same_org_admin_only_linking_and_default_off() {
    let f = Fixture::new("org_channel_link_authority").await;
    let db = &f.org.state.db;
    for actor in [&f.org.member, &f.org.viewer, &f.org.outsider] {
        assert!(link_target(db, actor, &f.bot, &f.agent).await.is_err());
    }
    assert!(
        require_target(db, &f.org.member, &f.agent)
            .await
            .unwrap_err()
            .to_string()
            .contains("organization admin")
    );
    let mut inactive = f.bot.clone();
    inactive.is_active = false;
    assert!(
        link_target(db, &f.org.admin, &inactive, &f.agent)
            .await
            .unwrap_err()
            .to_string()
            .contains("bot is inactive")
    );
    let mut foreign = f.bot.clone();
    foreign.user_id = f.org.admin.clone();
    assert!(
        link_target(db, &f.org.admin, &foreign, &f.agent)
            .await
            .unwrap_err()
            .to_string()
            .contains("same organization")
    );
    foreign.user_id = uuid::Uuid::new_v4().to_string();
    assert!(
        link_target(db, &f.org.admin, &foreign, &f.agent)
            .await
            .is_err()
    );
    assert!(
        sender(db, &f.channel, "visitor", true, &f.agent.id)
            .await
            .is_err()
    );
    crate::services::feature_flag_service::set_platform_override(
        db,
        FLAG,
        &crate::services::feature_flag_service::FlagTarget::Global,
        false,
        &f.org.admin,
    )
    .await
    .unwrap();
    assert!(
        link_target(db, &f.org.admin, &f.bot, &f.agent)
            .await
            .is_err()
    );
    assert!(
        require_target(db, &f.org.admin, &f.agent)
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled for this account")
    );
    assert_eq!(
        transport_access(db, &f.channel).await.unwrap(),
        OrgAccess::Unavailable
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn member_challenge_requires_same_session_confirmation_and_cannot_be_replayed_or_stolen() {
    let f = Fixture::new("org_channel_challenges").await;
    let db = &f.org.state.db;
    let issued = links::issue(db, &f.org.member, &f.channel).await.unwrap();
    let id = issued["id"].as_str().unwrap();
    let code = issued["code"].as_str().unwrap();
    assert!(
        links::confirm(db, &f.org.member, &f.channel, id)
            .await
            .is_err()
    );
    assert!(
        links::receive(db, &f.channel, "member-chat", code, Some("Member"))
            .await
            .unwrap()
    );
    assert!(
        !links::receive(db, &f.channel, "attacker", code, None)
            .await
            .unwrap()
    );
    assert!(
        links::status(db, &f.org.admin, &f.channel, id)
            .await
            .is_err()
    );
    assert!(
        links::confirm(db, &f.org.admin, &f.channel, id)
            .await
            .is_err()
    );
    links::confirm(db, &f.org.member, &f.channel, id)
        .await
        .unwrap();
    assert!(
        links::confirm(db, &f.org.member, &f.channel, id)
            .await
            .is_err()
    );
    let (identity, admin) = sender(db, &f.channel, "member-chat", true, &f.agent.id)
        .await
        .unwrap();
    assert_eq!(identity.actor_id, f.org.member);
    assert!(!identity.guest);
    assert!(!admin);
    assert!(
        sender(db, &f.channel, "member-chat", false, &f.agent.id)
            .await
            .is_err()
    );
    let hijack = links::issue(db, &f.org.admin, &f.channel).await.unwrap();
    links::receive(
        db,
        &f.channel,
        "member-chat",
        hijack["code"].as_str().unwrap(),
        None,
    )
    .await
    .unwrap();
    assert!(
        links::confirm(db, &f.org.admin, &f.channel, hijack["id"].as_str().unwrap())
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn member_revocation_never_downgrades_to_org_paid_guest() {
    let mut f = Fixture::new("org_channel_removed_member").await;
    let origin = f.link(&f.org.member, "member-chat").await;
    validate(&f.org.state.db, &origin, &f.org.member, &f.agent.id)
        .await
        .unwrap();
    f.wallet().await;
    f.guests(vec![]).await;
    f.org.revoke(&f.org.member).await;
    assert!(
        validate(&f.org.state.db, &origin, &f.org.member, &f.agent.id)
            .await
            .is_err()
    );
    assert!(
        sender(
            &f.org.state.db,
            &f.channel,
            "member-chat",
            true,
            &f.agent.id
        )
        .await
        .is_err()
    );
    let guest = f.guest("unverified").await;
    validate(&f.org.state.db, &guest, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    f.org.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn exact_guest_billing_lane_preserves_ordinary_platform_key_payers() {
    use crate::models::usage_meter::CredentialClass as C;
    use crate::services::billing::owner_resolver::{ExecutionLane, PaysFrom};
    let mut f = Fixture::new("org_channel_guest_billing").await;
    assert!(
        management::update(
            &f.org.state.db,
            &f.org.admin,
            &f.channel,
            management::GuestSettings {
                expected_revision: None,
                enabled: true,
                service_ids: Some(vec![]),
                daily_credits: Some(Credits::from_whole(1))
            }
        )
        .await
        .is_err()
    );
    f.wallet().await;
    f.guests(vec![]).await;
    let guest = f.guest("visitor").await;
    let access = validate(&f.org.state.db, &guest, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    let payer = access.channel().unwrap().guest_payer().unwrap();
    let resolver = f.org.state.billing.owner_resolver();
    for class in [C::NyxidManagedMaster, C::UserOwned, C::NoAuth] {
        let billed = resolver
            .resolve_for_execution(
                &f.org.org,
                &f.org.org,
                class,
                ExecutionLane::OrgChannelGuest(&payer),
            )
            .await
            .unwrap();
        assert_eq!(
            billed.pays,
            PaysFrom::OrgWallet {
                org_id: f.org.org.clone()
            }
        );
        assert!(
            resolver
                .resolve_for_execution(
                    &f.org.admin,
                    &f.org.org,
                    class,
                    ExecutionLane::OrgChannelGuest(&payer)
                )
                .await
                .is_err()
        );
    }
    let ordinary = resolver
        .resolve_for_ordinary_execution(&f.org.org, &f.org.org, C::NyxidManagedMaster)
        .await
        .unwrap();
    assert_eq!(ordinary.owner_id, f.org.org);
    assert_eq!(ordinary.pays, PaysFrom::Personal);
    let origin = f.link(&f.org.member, "member").await;
    let member = validate(&f.org.state.db, &origin, &f.org.member, &f.agent.id)
        .await
        .unwrap();
    assert!(member.channel().unwrap().guest_payer().is_none());
    let byok = resolver
        .resolve_for_assistant_execution(&f.org.member, &f.org.org, C::UserOwned, Some(&member))
        .await
        .unwrap();
    assert_eq!(byok.owner_id, f.org.org);
    let pk = resolver
        .resolve_for_assistant_execution(
            &f.org.member,
            &f.org.org,
            C::NyxidManagedMaster,
            Some(&member),
        )
        .await
        .unwrap();
    assert_eq!(pk.owner_id, f.org.member);
    assert_eq!(pk.pays, PaysFrom::Personal);
    f.org
        .state
        .db
        .collection::<Document>("billing_wallet")
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(
        resolver
            .resolve_for_execution(
                &f.org.org,
                &f.org.org,
                C::NyxidManagedMaster,
                ExecutionLane::OrgChannelGuest(&payer)
            )
            .await
            .is_err()
    );
    f.org.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_delegation_requires_admin_and_live_agent_policy_intersection() {
    let mut f = Fixture::new("org_channel_guest_services").await;
    f.wallet().await;
    let id = f.org.service(&f.org.org, "approved-service").await;
    f.org
        .state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id":&f.agent.id},
            doc! {"$set":{"grants.service_ids":[&id]}},
        )
        .await
        .unwrap();
    for actor in [&f.org.member, &f.org.viewer, &f.org.outsider] {
        assert!(
            management::update(
                &f.org.state.db,
                actor,
                &f.channel,
                management::GuestSettings {
                    expected_revision: None,
                    enabled: true,
                    service_ids: Some(vec![id.clone()]),
                    daily_credits: Some(Credits::from_whole(1))
                }
            )
            .await
            .is_err()
        );
    }
    f.guests(vec![id.clone()]).await;
    let origin = f.guest("visitor").await;
    let access = validate(&f.org.state.db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    let service: UserService = f
        .org
        .state
        .db
        .collection("user_services")
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert!(access.allows_service(&service));
    let mut moved = service.clone();
    moved.node_id = Some(uuid::Uuid::new_v4().to_string());
    assert!(
        !access.allows_service(&moved),
        "moving a delegated service onto a node cannot widen guest authority"
    );
    moved.node_id = None;
    moved.service_type = "ssh".into();
    assert!(!access.allows_service(&moved));
    let other = f.org.service(&f.org.org, "not-delegated").await;
    let other: UserService = f
        .org
        .state
        .db
        .collection("user_services")
        .find_one(doc! {"_id":other})
        .await
        .unwrap()
        .unwrap();
    assert!(!access.allows_service(&other));
    f.org
        .state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id":&f.agent.id},
            doc! {"$set":{format!("guest_access.{id}"):"read"}},
        )
        .await
        .unwrap();
    assert!(
        validate(&f.org.state.db, &origin, &f.org.org, &f.agent.id)
            .await
            .is_err()
    );
    f.org.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn per_sender_rate_limit_does_not_limit_members_or_other_senders() {
    let mut f = Fixture::new("org_channel_guest_rate").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let a = f.guest("a").await;
    let b = f.guest("b").await;
    let member = f.link(&f.org.member, "member").await;
    for _ in 0..6 {
        assert!(
            guest_rate(
                &f.org.state.db,
                a.org_execution.as_ref().unwrap(),
                &f.channel.id
            )
            .await
            .unwrap()
        );
    }
    assert!(
        !guest_rate(
            &f.org.state.db,
            a.org_execution.as_ref().unwrap(),
            &f.channel.id
        )
        .await
        .unwrap()
    );
    assert!(
        guest_rate(
            &f.org.state.db,
            b.org_execution.as_ref().unwrap(),
            &f.channel.id
        )
        .await
        .unwrap()
    );
    for _ in 0..8 {
        assert!(
            guest_rate(
                &f.org.state.db,
                member.org_execution.as_ref().unwrap(),
                &f.channel.id
            )
            .await
            .unwrap()
        );
    }
    f.org.state.db.drop().await.unwrap();
}

fn charge(
    org: &str,
    id: &str,
) -> (
    crate::models::usage_meter::UsageMeterRow,
    crate::services::billing::reservation::LayerReservation,
) {
    use crate::models::{
        service_billing::BillingMetric,
        usage_meter::{BillingLayer, UsageFunding},
    };
    let funding = UsageFunding {
        credits_per_unit_micros: 100_000,
        ..Default::default()
    };
    let row=bson::from_document(doc!{"_id":id,"transaction_id":id,"billing_request_id":id,"layer":"platform","billing_owner_id":org,"actor_user_id":org,
        "metric":"requests","lago_metric_code":"test","credential_class":"nyxid_managed_master","funding":bson::to_bson(&funding).unwrap(),
        "status":"reserved","forwarded":false,"released":false,"lago_acked":false,"attempt":0,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
    let layer = crate::services::billing::reservation::LayerReservation {
        layer: BillingLayer::Platform,
        metric: BillingMetric::Requests,
        lago_metric_code: "test".into(),
        estimated_quantity: 6,
        credits_per_unit_micros: 100_000,
        credits_per_unit_pico: None,
        // Exposure counts the whole 0.6 credit estimate, including benefits:
        // 0.3 wallet + 0.2 grant + one 0.1-credit allowance unit.
        reserved_credits: "0.3".parse().unwrap(),
        allowance_reservations: vec![crate::models::usage_meter::AllowanceReservationAllocation {
            allowance_id: "test-allowance".into(),
            period_id: "test-period".into(),
            quantity: 1,
        }],
        grant_reservations: vec![crate::models::usage_meter::GrantReservationAllocation {
            grant_id: "test-grant".into(),
            amount: "0.2".parse().unwrap(),
            service_limit_key: None,
        }],
        tax_cache_ttl_secs: 60,
        applied_tax: None,
    };
    (row, layer)
}
async fn reserve(db: &Database, payer: &GuestPayer, id: &str) -> AppResult<()> {
    let (row, layer) = charge(&payer.org_id, id);
    let context = (db.clone(), payer.clone(), row, layer);
    crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, payer, row, layer)| {
            Box::pin(async move {
                budget::reserve(
                    db,
                    tx,
                    payer,
                    std::slice::from_ref(row),
                    std::slice::from_ref(layer),
                )
                .await
            })
        },
    )
    .await
}

#[tokio::test]
async fn guest_budget_reserves_fractional_tax_upward_at_the_exact_cap() {
    let mut f = Fixture::new("org_channel_budget_tax").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let db = &f.org.state.db;
    let (mut row, mut layer) = charge(&f.org.org, "tax-hold");
    let funding = row.funding.as_mut().unwrap();
    funding.credits_per_unit_pico = Some(1);
    funding.applied_tax = Some(crate::models::billing_tax_profile::AppliedUsageTax {
        taxes: vec![],
        rate_percent: "1".parse().unwrap(),
        fingerprint: "test-tax".into(),
    });
    layer.estimated_quantity = 1;
    layer.credits_per_unit_pico = Some(1);
    layer.allowance_reservations.clear();
    layer.grant_reservations.clear();
    layer.reserved_credits = crate::services::billing::taxes::gross_ceiling(
        Credits::from_pico(1).unwrap(),
        "1".parse().unwrap(),
    )
    .unwrap();
    for (cap, allowed) in [(1, false), (2, true)] {
        management::update(
            db,
            &f.org.admin,
            &f.channel,
            management::GuestSettings {
                expected_revision: None,
                enabled: true,
                service_ids: Some(vec![]),
                daily_credits: Some(Credits::from_pico(cap).unwrap()),
            },
        )
        .await
        .unwrap();
        f.channel = management::load(db, &f.org.admin, &f.channel.id, true)
            .await
            .unwrap();
        let origin = f.guest("tax-visitor").await;
        let access = validate(db, &origin, &f.org.org, &f.agent.id)
            .await
            .unwrap();
        let payer = access.channel().unwrap().guest_payer().unwrap();
        let context = (db.clone(), payer, row.clone(), layer.clone());
        let result = crate::services::transaction_service::run_db_only(
            db,
            &context,
            |tx, (db, payer, row, layer)| {
                Box::pin(async move {
                    budget::reserve(
                        db,
                        tx,
                        payer,
                        std::slice::from_ref(row),
                        std::slice::from_ref(layer),
                    )
                    .await
                })
            },
        )
        .await;
        assert_eq!(
            result.is_ok(),
            allowed,
            "1 pico plus fractional tax needs a 2-pico reservation"
        );
        assert_eq!(
            budget::today(db, &f.channel.id).await.unwrap().1,
            Credits::from_pico(if allowed { 2 } else { 0 }).unwrap()
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn exact_daily_budget_fences_concurrent_admission_and_settles_once_after_restart() {
    let mut f = Fixture::new("org_channel_budget").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let origin = f.guest("visitor").await;
    let access = validate(&f.org.state.db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    let payer = access.channel().unwrap().guest_payer().unwrap();
    let db = &f.org.state.db;
    let (a, b) = tokio::join!(reserve(db, &payer, "one"), reserve(db, &payer, "two"));
    assert_ne!(
        a.is_ok(),
        b.is_ok(),
        "only one 0.6-credit hold fits a 1-credit cap"
    );
    let accepted = if a.is_ok() { "one" } else { "two" };
    reserve(db, &payer, accepted).await.unwrap();
    assert_eq!(
        budget::today(db, &f.channel.id).await.unwrap(),
        (Credits::ZERO, "0.6".parse().unwrap())
    );
    budget::settle(db, accepted, "0.5".parse().unwrap())
        .await
        .unwrap();
    budget::settle(db, accepted, "0.5".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(
        budget::today(db, &f.channel.id).await.unwrap(),
        ("0.5".parse().unwrap(), Credits::ZERO)
    );
    assert!(reserve(db, &payer, "third").await.is_err());
    assert!(
        db.collection::<Document>(crate::models::org_channel::HOLDS)
            .find_one(doc! {"_id":"third"})
            .await
            .unwrap()
            .is_none()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn failed_admission_releases_hold_and_policy_revision_cannot_reset_spend() {
    let mut f = Fixture::new("org_channel_budget_policy").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let origin = f.guest("visitor").await;
    let access = validate(&f.org.state.db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    let old = access.channel().unwrap().guest_payer().unwrap();
    reserve(&f.org.state.db, &old, "failed").await.unwrap();
    budget::settle(&f.org.state.db, "failed", Credits::ZERO)
        .await
        .unwrap();
    reserve(&f.org.state.db, &old, "paid").await.unwrap();
    budget::settle(&f.org.state.db, "paid", "0.6".parse().unwrap())
        .await
        .unwrap();
    f.guests(vec![]).await;
    assert!(reserve(&f.org.state.db, &old, "stale").await.is_err());
    let origin = f.guest("visitor").await;
    let access = validate(&f.org.state.db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    assert!(
        reserve(
            &f.org.state.db,
            &access.channel().unwrap().guest_payer().unwrap(),
            "new-policy"
        )
        .await
        .is_err()
    );
    assert_eq!(
        budget::today(&f.org.state.db, &f.channel.id).await.unwrap(),
        ("0.6".parse().unwrap(), Credits::ZERO)
    );
    f.org.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn group_execution_is_per_actor_and_admin_presence_does_not_make_members_owners() {
    let f = Fixture::new("org_channel_actor_partitions").await;
    let mut admin = f.link(&f.org.admin, "admin").await;
    let mut member = f.link(&f.org.member, "member").await;
    admin.partition = "group".into();
    member.partition = "group".into();
    let (first, _) = transcript::lane(&f.org.state.db, &admin, &f.org.admin, &f.agent.id)
        .await
        .unwrap();
    let (second, _) = transcript::lane(&f.org.state.db, &member, &f.org.member, &f.agent.id)
        .await
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        first,
        transcript::lane(&f.org.state.db, &admin, &f.org.admin, &f.agent.id)
            .await
            .unwrap()
            .0
    );
    let (_, admin) = project_sender(&f.org.state.db, &f.channel, "admin", true, &f.agent.id)
        .await
        .unwrap();
    let (_, member) = project_sender(&f.org.state.db, &f.channel, "member", true, &f.agent.id)
        .await
        .unwrap();
    assert!(admin);
    assert!(!member);
    f.org.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_follow_binds_member_lane_and_stop_invalidates_unclaimed_admission() {
    use crate::models::channel_thread::{
        ChannelThreadFacts, ThreadAddress, ThreadKind, ThreadSenderKind,
    };
    use crate::services::{
        channel_thread_follow_service as follow, channel_thread_service::ThreadReplyTarget,
    };
    let f = Fixture::new("org_channel_follow_actor").await;
    let db = &f.org.state.db;
    let mut origin = f.link(&f.org.member, "member").await;
    crate::services::feature_flag_service::set_platform_override(
        db,
        crate::services::feature_flag_service::NYXBOT_THREAD_FOLLOW_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::Global,
        true,
        &f.org.admin,
    )
    .await
    .unwrap();
    db.collection::<Document>("nyxbot_threads")
        .insert_one(doc! {
            "_id":"parent","user_id":&f.org.admin,"channel_id":&f.channel.id,"partition":"parent",
            "kind":"group","platform_chat_id":"group","members":"owner",
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        })
        .await
        .unwrap();
    let facts = ChannelThreadFacts {
        version: 1,
        kind: ThreadKind::ReplyChain,
        chat_id: "group".into(),
        message_id: "message".into(),
        root_id: Some("message".into()),
        sender_kind: ThreadSenderKind::Human,
        address: ThreadAddress::Mention,
        ..Default::default()
    };
    let inbound:crate::services::channel_platform::InboundMessage=serde_json::from_value(serde_json::json!({
        "platform_message_id":"message","conversation_id":"group","conversation_type":"group","sender_platform_id":"member",
        "content_type":"text","text":"Question","attachments":[],"raw_data":{}
    })).unwrap();
    let mut source = crate::services::channel_relay_service::inbound_metadata(
        &f.bot.id,
        f.channel.route_id.as_deref().unwrap(),
        &f.org.org,
        "lark",
        &inbound,
        &f.channel.route_api_key_id,
        "source",
    );
    source.thread_context = Some(facts.clone());
    db.collection::<crate::models::channel_message::ChannelMessage>("channel_messages")
        .insert_one(source)
        .await
        .unwrap();
    let follow::Selection::Child(child, binding) = follow::select(
        db,
        &f.org.admin,
        &f.channel.id,
        "parent",
        "parent",
        &ThreadReplyTarget::fixture("lark", facts),
        "source",
        "member",
        true,
        false,
        &f.agent.id,
        true,
    )
    .await
    .unwrap() else {
        panic!("member must open followed thread")
    };
    assert!(!binding.guest);
    origin.partition = child.partition.clone();
    origin.thread = Some(Box::new(binding));
    let (conversation, _) = transcript::lane(db, &origin, &f.org.member, &f.agent.id)
        .await
        .unwrap();
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    follow::admit_turn(db, &f.org.member, &origin, &conversation, &mut tx)
        .await
        .unwrap();
    tx.commit_transaction().await.unwrap();
    follow::validate_delivery(db, &f.org.member, &origin, &conversation)
        .await
        .unwrap();
    assert!(
        follow::validate_delivery(db, &f.org.admin, &origin, &conversation)
            .await
            .is_err()
    );
    follow::stop(db, &f.org.admin, &f.channel.id, "parent", &child.id)
        .await
        .unwrap();
    tx.start_transaction().await.unwrap();
    assert!(
        follow::admit_turn(db, &f.org.member, &origin, &conversation, &mut tx)
            .await
            .is_err()
    );
    tx.abort_transaction().await.unwrap();
    f.org.revoke(&f.org.member).await;
    assert!(
        follow::validate_delivery(db, &f.org.member, &origin, &conversation)
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn live_validator_refuses_foreign_actor_target_route_and_disabled_binding() {
    let f = Fixture::new("org_channel_live_rechecks").await;
    let origin = f.link(&f.org.member, "member").await;
    let db = &f.org.state.db;
    assert!(
        validate(db, &origin, &f.org.admin, &f.agent.id)
            .await
            .is_err()
    );
    assert!(
        validate(
            db,
            &origin,
            &f.org.member,
            &uuid::Uuid::new_v4().to_string()
        )
        .await
        .is_err()
    );
    let mut tampered = origin.clone();
    tampered.org_execution.as_mut().unwrap().sender_hash = sender_hash(&f.channel.id, "other");
    assert!(
        validate(db, &tampered, &f.org.member, &f.agent.id)
            .await
            .is_err()
    );
    db.collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.channel.route_api_key_id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    assert!(
        validate(db, &origin, &f.org.member, &f.agent.id)
            .await
            .is_err()
    );
    db.collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.channel.route_api_key_id},
            doc! {"$set":{"is_active":true}},
        )
        .await
        .unwrap();
    f.org.revoke(&f.org.admin).await;
    assert!(
        validate(db, &origin, &f.org.member, &f.agent.id)
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

async fn channel_turn(
    f: &Fixture,
    origin: &ChannelOrigin,
    text: &str,
) -> crate::models::assistant_conversation::AssistantConversation {
    use crate::services::assistant_nyxagent as engine;
    let e = origin.org_execution.as_ref().unwrap();
    let (id, exists) = transcript::lane(&f.org.state.db, origin, &e.actor_id, &e.agent_id)
        .await
        .unwrap();
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: exists.then(|| id.clone()),
        agent_id: Some(e.agent_id.clone()),
        text: text.into(),
        model: None,
        access_mode: None,
    });
    start.new_id = (!exists).then_some(id);
    start.channel = Some(origin.clone());
    let event_id = uuid::Uuid::new_v4().to_string();
    f.org.state.db.collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
        .insert_one(doc! {"_id":&event_id,"user_id":&f.org.admin,"channel_id":&f.channel.id,
            "partition":&origin.partition,"event_id":&event_id,"status":"running",
            "created_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::hours(1)),
            "delivery":{"version":1,"state":"pending","origin":bson::to_bson(origin).unwrap(),
                "sender_id":"fixture-sender","guest":e.guest,"addressed":true,"transport":"direct",
                "source_route_id":&f.channel.route_id,"source_route_api_key_id":&f.channel.route_api_key_id,
                "stream_deadline":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(9)),
                "checked_at":bson::DateTime::now()}})
        .await.unwrap();
    start.channel_event_id = Some(event_id);
    start.guest = e.guest;
    start.origin = crate::models::assistant_conversation::TurnOrigin::Channel;
    start.org_access = Some(
        validate(&f.org.state.db, origin, &e.actor_id, &e.agent_id)
            .await
            .unwrap(),
    );
    Box::pin(engine::begin_turn(
        &f.org.state.db,
        &e.actor_id,
        start,
        &f.org.state.encryption_keys,
    ))
    .await
    .unwrap_or_else(|error| panic!("Channel turn admission failed: {error}"))
}

#[tokio::test]
async fn actual_guest_turn_has_org_payer_and_no_account_nodes_private_memory_or_approvals() {
    let mut f = Fixture::new("org_channel_real_guest_turn").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let origin = f.guest("visitor").await;
    let row = channel_turn(
        &f,
        &origin,
        "Show all machines and change the model to my choice",
    )
    .await;
    assert_eq!(row.user_id, f.org.org);
    assert!(row.guest_turn);
    let key = f.org.key(&row).await;
    assert!(!key.allow_all_services && !key.allow_all_nodes && !key.allow_auto_connected_services);
    assert!(key.allowed_service_ids.is_empty() && key.allowed_node_ids.is_empty());
    assert!(!key.scopes.split_whitespace().any(|s| s == "account:read"));
    let auth = Box::pin(crate::mw::auth::api_key_auth_user(
        &f.org.state.db,
        &key,
        None,
        None,
        None,
    ))
    .await
    .unwrap();
    let access = auth.org_agent_access.as_ref().unwrap();
    assert!(access.channel().unwrap().execution().guest);
    assert!(!access.can_maintain());
    assert!(!access.approval_admin_permits(&[]));
    assert_eq!(
        crate::services::org_agent_service::key_nodes(&f.org.state.db, &key, Some(access))
            .await
            .unwrap(),
        (false, vec![])
    );
    let agent =
        crate::services::assistant_team_service::agent_for_conversation(&f.org.state.db, &row)
            .await
            .unwrap();
    assert!(
        agent.memory.is_empty()
            && agent.machine_node_ids.is_empty()
            && agent.saved_login_ids.is_empty()
    );
    let other = f.org.service(&f.org.org, "not-delegated").await;
    crate::services::assistant_team_service::set_grants(
        &f.org.state.db,
        &f.org.member,
        &f.agent.id,
        crate::services::assistant_team_service::GrantChange::Add(
            crate::models::assistant_agent::AgentGrants {
                service_ids: vec![other.clone()],
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    let refreshed = f.org.key(&row).await;
    assert!(refreshed.allowed_service_ids.is_empty() && refreshed.allowed_node_ids.is_empty());
    assert!(!refreshed.allow_all_services && !refreshed.allow_auto_connected_services);
    assert!(
        crate::services::org_agent_service::authorize_execution(
            &f.org.state.db,
            &auth,
            Some(&other)
        )
        .await
        .is_err()
    );
    assert!(
        crate::services::assistant_nyxagent::list(&f.org.state.db, &f.org.org, 100, None, None)
            .await
            .unwrap()
            .iter()
            .all(|r| r.id != row.id)
    );
    f.org.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_rotated_guest_key_keeps_typed_lane_and_is_revoked_with_exposure() {
    let mut f = Fixture::new("org_channel_rotate_guest").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let origin = f.guest("visitor").await;
    let row = channel_turn(&f, &origin, "A guest question").await;
    let db = &f.org.state.db;
    db.collection::<Document>("assistant_conversations")
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"active_turn":null}})
        .await
        .unwrap();
    let rotated = crate::services::key_service::rotate_api_key(
        db,
        &f.org.state.encryption_keys,
        &f.org.org,
        &row.credential_api_key_id,
    )
    .await
    .unwrap();
    assert!(rotated.full_key.is_empty());
    let key = crate::services::key_service::get_api_key(db, &f.org.org, &rotated.id)
        .await
        .unwrap();
    let auth = Box::pin(crate::mw::auth::api_key_auth_user(
        db, &key, None, None, None,
    ))
    .await
    .unwrap();
    assert!(
        auth.org_agent_access
            .as_ref()
            .unwrap()
            .channel()
            .unwrap()
            .guest_payer()
            .is_some()
    );
    let current = db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.credential_api_key_id, key.id);
    management::update(
        db,
        &f.org.admin,
        &f.channel,
        management::GuestSettings {
            expected_revision: None,
            enabled: false,
            service_ids: Some(vec![]),
            daily_credits: Some(Credits::from_whole(1)),
        },
    )
    .await
    .unwrap();
    assert!(
        db.collection::<crate::models::api_key::ApiKey>("api_keys")
            .find_one(doc! {"_id":&key.id,"is_active":true})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        Box::pin(crate::mw::auth::api_key_auth_user(
            db, &key, None, None, None
        ))
        .await
        .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn actual_member_threads_have_distinct_keys_and_revocation_cannot_reuse_a_snapshot() {
    let f = Fixture::new("org_channel_real_member_turns").await;
    let admin = f.link(&f.org.admin, "admin").await;
    let member = f.link(&f.org.member, "member").await;
    let a = channel_turn(&f, &admin, "Admin private request").await;
    let m = channel_turn(&f, &member, "Member private request").await;
    assert_ne!(a.id, m.id);
    assert_ne!(a.credential_api_key_id, m.credential_api_key_id);
    assert_eq!(m.user_id, f.org.member);
    assert_eq!(m.agent_owner_id.as_deref(), Some(f.org.org.as_str()));
    let key = f.org.key(&m).await;
    Box::pin(crate::mw::auth::api_key_auth_user(
        &f.org.state.db,
        &key,
        None,
        None,
        None,
    ))
    .await
    .unwrap();
    crate::services::org_service::revoke_membership(&f.org.state.db, &f.org.org, &f.org.member)
        .await
        .unwrap();
    assert!(!f.org.key(&m).await.is_active);
    assert!(
        Box::pin(crate::mw::auth::api_key_auth_user(
            &f.org.state.db,
            &key,
            None,
            None,
            None
        ))
        .await
        .is_err()
    );
    assert!(f.org.key(&a).await.is_active);
    assert!(
        f.org
            .state
            .db
            .collection::<Document>("assistant_agent_credentials")
            .find_one(doc! {"api_key_id":&m.credential_api_key_id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        sender(&f.org.state.db, &f.channel, "member", true, &f.agent.id)
            .await
            .is_err()
    );
    f.org.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn public_transcript_excludes_private_messages_and_erasure_purges_actor_content() {
    let f = Fixture::new("org_channel_public_transcript").await;
    let private = f.link(&f.org.member, "member").await;
    channel_turn(&f, &private, "Private member history").await;
    assert!(
        transcript::context(&f.org.state.db, &private)
            .await
            .unwrap()
            .contains("Everyone")
    );
    assert_eq!(
        f.org
            .state
            .db
            .collection::<Document>(crate::models::org_channel::MESSAGES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.org.state.db.collection::<Document>("nyxbot_threads").insert_one(doc!{"_id":uuid::Uuid::new_v4().to_string(),"channel_id":&f.channel.id,"user_id":&f.org.admin,
        "partition":"group","kind":"group","members":"everyone","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    let mut group = private.clone();
    group.partition = "group".into();
    let row = channel_turn(&f, &group, "Public member request").await;
    let context = transcript::context(&f.org.state.db, &group).await.unwrap();
    assert!(context.contains("Public member request"));
    assert!(!context.contains("Private member history"));
    // Busy actor lanes must publish the actual queued question exactly once,
    // atomically with acceptance, without exposing generated event instructions.
    let sources = f
        .org
        .state
        .db
        .collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME);
    let mut source = sources
        .find_one(doc! {"_id":row.active_turn.as_ref().unwrap().channel_event_id.as_ref().unwrap()})
        .await
        .unwrap()
        .unwrap();
    let source_id = uuid::Uuid::new_v4().to_string();
    source.insert("_id", &source_id);
    source.insert("event_id", &source_id);
    source.insert("turn_id", bson::Bson::Null);
    sources.insert_one(source).await.unwrap();
    let mut work = crate::services::assistant_team_service::bound_event(
        "message",
        "Private generated queue instructions".into(),
        None,
        None,
    );
    work.reply_to = vec![group.clone()];
    work.source_event_ids = vec![source_id.clone()];
    let db = &f.org.state.db;
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    assert!(
        crate::services::channel_turn_delivery::enqueue_in_session(
            db,
            &f.org.member,
            &row.id,
            &work,
            Some("Rolled back question"),
            &mut tx
        )
        .await
        .unwrap()
        .accepted
    );
    tx.abort_transaction().await.unwrap();
    assert!(
        !transcript::context(db, &group)
            .await
            .unwrap()
            .contains("Rolled back question")
    );
    assert!(
        crate::services::channel_turn_delivery::enqueue(
            db,
            &f.org.member,
            &row.id,
            work.clone(),
            Some("Queued public question")
        )
        .await
        .unwrap()
    );
    assert!(
        crate::services::channel_turn_delivery::enqueue(
            db,
            &f.org.member,
            &row.id,
            work,
            Some("Duplicate question")
        )
        .await
        .is_err()
    );
    let context = transcript::context(db, &group).await.unwrap();
    assert_eq!(context.matches("Queued public question").count(), 1);
    assert!(!context.contains("Private generated queue instructions"));
    assert!(!context.contains("Duplicate question"));
    crate::services::assistant_nyxagent::finish_turn(
        db,
        &row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &crate::services::assistant_nyxagent::TurnResult {
            text: "Unfinished private reply draft".into(),
            session_id: None,
            response_id: None,
            error: Some(crate::services::assistant_nyxagent::TurnError::new(
                "cancelled",
            )),
        },
    )
    .await
    .unwrap();
    assert!(
        !transcript::context(db, &group)
            .await
            .unwrap()
            .contains("Unfinished private reply draft")
    );
    lifecycle::erase_person(&f.org.state.db, &f.org.member)
        .await
        .unwrap();
    assert!(
        !transcript::context(&f.org.state.db, &group)
            .await
            .unwrap()
            .contains("Public member request")
    );
    assert!(
        sender(&f.org.state.db, &f.channel, "member", true, &f.agent.id)
            .await
            .is_err()
    );
    f.org.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn erasure_waits_for_hidden_execution_and_purges_transport_owned_recovery() {
    let f = Fixture::new("org_channel_erasure_settlement").await;
    let origin = f.link(&f.org.member, "member").await;
    let row = channel_turn(&f, &origin, "Private channel request").await;
    let db = &f.org.state.db;
    db.collection::<Document>(crate::models::nyxbot_gateway_response::COLLECTION_NAME).insert_one(doc!{
        "_id":"recovery","user_id":&f.org.admin,"conversation_id":&row.id,"payload_ciphertext":"encrypted fixture"}).await.unwrap();
    let filter =
        doc! {"channel.nyxbot_channel_id":&f.channel.id,"channel.org_execution":{"$type":"object"}};
    assert!(!lifecycle::erase_hidden(db, filter.clone()).await.unwrap());
    let stopped = crate::services::assistant_nyxagent::get(db, &f.org.member, &row.id)
        .await
        .unwrap();
    assert!(stopped.active_turn.unwrap().stop_requested);
    assert_eq!(
        db.collection::<Document>(crate::models::nyxbot_gateway_response::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    // Simulate the normal settlement boundary, not a deletion of live work.
    db.collection::<Document>("assistant_conversations")
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"active_turn":null}})
        .await
        .unwrap();
    assert!(!lifecycle::erase_hidden(db, filter.clone()).await.unwrap());
    assert!(lifecycle::erase_hidden(db, filter).await.unwrap());
    assert_eq!(
        db.collection::<Document>(crate::models::nyxbot_gateway_response::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(
        db.collection::<Document>("assistant_agent_credentials")
            .find_one(doc! {"conversation_id":&row.id})
            .await
            .unwrap()
            .is_none()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn member_private_home_remains_usable_from_own_session_after_unlink() {
    use crate::services::{assistant_nyxagent as engine, assistant_team_service as team};
    let f = Fixture::new("org_channel_member_home_unlink").await;
    let origin = f.link(&f.org.member, "member").await;
    let db = &f.org.state.db;
    let home = Box::pin(team::home_thread_for(
        db,
        &f.org.state.encryption_keys,
        &f.org.member,
        &f.agent,
    ))
    .await
    .unwrap();
    let request = engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: Some(home.id.clone()),
        agent_id: Some(f.agent.id.clone()),
        text: "My DM".into(),
        model: None,
        access_mode: None,
    };
    let mut start = engine::TurnStart::from(&request);
    start.origin = crate::models::assistant_conversation::TurnOrigin::Channel;
    start.reply_channel = Some(origin.clone());
    start.org_access = Some(
        validate(db, &origin, &f.org.member, &f.agent.id)
            .await
            .unwrap(),
    );
    let linked = Box::pin(engine::begin_turn(
        db,
        &f.org.member,
        start,
        &f.org.state.encryption_keys,
    ))
    .await
    .unwrap();
    db.collection::<Document>("assistant_conversations")
        .update_one(doc! {"_id":&home.id}, doc! {"$set":{"active_turn":null}})
        .await
        .unwrap();
    let context = (db.clone(), f.channel.id.clone(), f.org.member.clone());
    crate::services::transaction_service::run_db_only(db, &context, |tx, (db, channel, actor)| {
        Box::pin(async move { lifecycle::retire_in_session(db, tx, channel, Some(actor)).await })
    })
    .await
    .unwrap();
    let direct = Box::pin(engine::begin_turn(
        db,
        &f.org.member,
        engine::TurnStart::from(&request),
        &f.org.state.encryption_keys,
    ))
    .await
    .unwrap();
    assert!(direct.channel.is_none() && direct.reply_channel.is_none());
    assert_eq!(direct.agent_owner_id.as_deref(), Some(f.org.org.as_str()));
    assert_ne!(direct.credential_api_key_id, linked.credential_api_key_id);
    db.collection::<Document>(crate::models::nyxbot_gateway_response::COLLECTION_NAME)
        .insert_one(doc! {"_id":"former-channel-replay","user_id":&f.org.admin,"conversation_id":&home.id,"payload_ciphertext":"encrypted fixture"})
        .await.unwrap();
    db.collection::<Document>("assistant_conversations")
        .update_one(doc! {"_id":&home.id}, doc! {"$set":{"active_turn":null}})
        .await
        .unwrap();
    engine::delete(db, &f.org.member, &home.id).await.unwrap();
    assert_eq!(
        db.collection::<Document>(crate::models::nyxbot_gateway_response::COLLECTION_NAME)
            .count_documents(doc! {"conversation_id":&home.id})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_member_media_uses_live_actor_and_guests_cannot_fetch() {
    use crate::services::{channel_media_authority as media, channel_media_events};
    let f = Fixture::new("org_channel_member_media").await;
    let db = &f.org.state.db;
    let origin = f.link(&f.org.member, "member").await;
    let conversation = channel_turn(&f, &origin, "Review this image").await;
    crate::services::feature_flag_service::set_platform_override(
        db,
        crate::services::feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::Global,
        true,
        &f.org.admin,
    )
    .await
    .unwrap();
    let source = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>("channel_messages").insert_one(doc!{"_id":&source,"user_id":&f.org.org,"channel_bot_id":&f.bot.id,
        "conversation_id":&f.channel.route_id,"platform_conversation_id":"chat","direction":"inbound","platform":"lark",
        "platform_message_id":"native-image","sender_platform_id":"member","content_type":"image",
        "agent_api_key_id":&f.channel.route_api_key_id,"created_at":bson::DateTime::now(),
        "attachments":[{"content_type":"image","provider_ref":"image","image_key":"image"}]}).await.unwrap();
    let mut request = media::MediaRequest {
        channel: f.channel.clone(),
        source_message_id: source.clone(),
        chat_id: "chat".into(),
        sender_id: "member".into(),
        native_thread_id: None,
        origin: None,
        conversation_id: None,
        agent_id: Some(f.agent.id.clone()),
    };
    assert!(
        channel_media_events::admission(db, &request)
            .await
            .unwrap()
            .is_some(),
        "authenticated member can claim media without transport-admin identity"
    );
    request.origin = Some(origin);
    request.conversation_id = Some(conversation.id);
    assert!(
        media::resolve(db, &request).await.is_ok(),
        "fetch checks the bound member execution"
    );
    let mut wrong = request.clone();
    wrong
        .origin
        .as_mut()
        .unwrap()
        .org_execution
        .as_mut()
        .unwrap()
        .actor_id = f.org.admin.clone();
    assert!(media::resolve(db, &wrong).await.is_err());
    f.org.revoke(&f.org.member).await;
    assert!(
        media::resolve(db, &request).await.is_err(),
        "revocation immediately blocks a subsequent fetch"
    );
    db.collection::<Document>("channel_messages")
        .update_one(
            doc! {"_id":source},
            doc! {"$set":{"sender_platform_id":"visitor"}},
        )
        .await
        .unwrap();
    request.sender_id = "visitor".into();
    request.origin = None;
    request.conversation_id = None;
    assert!(
        channel_media_events::admission(db, &request)
            .await
            .unwrap()
            .is_none(),
        "unverified source has no download authority"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn narrowing_guest_exposure_stops_guest_and_setup_keys_but_preserves_members() {
    let mut f = Fixture::new("org_channel_policy_stop").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let origin = f.guest("visitor").await;
    let guest = channel_turn(&f, &origin, "Guest turn").await;
    let member_origin = f.link(&f.org.member, "member").await;
    let member = channel_turn(&f, &member_origin, "Member turn").await;
    management::update(
        &f.org.state.db,
        &f.org.admin,
        &f.channel,
        management::GuestSettings {
            expected_revision: None,
            enabled: false,
            service_ids: Some(vec![]),
            daily_credits: Some(Credits::from_whole(1)),
        },
    )
    .await
    .unwrap();
    let db = &f.org.state.db;
    let stopped = db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {"_id":&guest.id})
        .await
        .unwrap()
        .unwrap();
    assert!(stopped.active_turn.unwrap().stop_requested);
    assert!(
        db.collection::<Document>("api_keys")
            .find_one(doc! {"_id":&guest.credential_api_key_id,"is_active":true})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.collection::<Document>("api_keys")
            .find_one(doc! {"_id":&member.credential_api_key_id,"is_active":true})
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        validate(db, &origin, &f.org.org, &f.agent.id)
            .await
            .is_err()
    );
    validate(db, &member_origin, &f.org.member, &f.agent.id)
        .await
        .unwrap();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_cards_belong_only_to_initiating_member_and_guests_cannot_raise_them() {
    use crate::services::assistant_acknowledgement_service as acks;
    let mut f = Fixture::new("org_channel_actor_cards").await;
    let origin = f.link(&f.org.member, "member").await;
    let row = channel_turn(&f, &origin, "Change a resource").await;
    let chat = acks::for_key(
        &f.org.state.db,
        &f.org.member,
        Some(&row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    let card = acks::request(
        &f.org.state.db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("test_action"),
            arguments: None,
            summary: "Change a resource",
            platform: false,
        },
    )
    .await
    .unwrap();
    assert!(
        acks::decide(&f.org.state.db, &f.org.admin, &row.id, &card.id, true)
            .await
            .is_err(),
        "the transport admin cannot decide another member's card"
    );
    acks::decide(&f.org.state.db, &f.org.member, &row.id, &card.id, true)
        .await
        .unwrap();
    f.wallet().await;
    f.guests(vec![]).await;
    let origin = f.guest("visitor").await;
    let guest = channel_turn(&f, &origin, "Approve all services").await;
    let chat = acks::for_key(
        &f.org.state.db,
        &f.org.org,
        Some(&guest.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        acks::request(
            &f.org.state.db,
            &chat,
            acks::Request {
                kind: "action",
                service: None,
                tool: Some("test_action"),
                arguments: None,
                summary: "Change a resource",
                platform: false
            }
        )
        .await
        .is_err()
    );
    f.org.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn private_gateway_adoption_preserves_member_home_and_guest_lane_atomically() {
    use crate::services::{assistant_nyxagent as engine, assistant_team_service as team};
    let mut f = Fixture::new("org_channel_private_adoption").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let member_origin = f.link(&f.org.member, "member").await;
    let guest_origin = f.guest("visitor").await;
    let db = &f.org.state.db;
    let home = Box::pin(team::home_thread_for(
        db,
        &f.org.state.encryption_keys,
        &f.org.member,
        &f.agent,
    ))
    .await
    .unwrap();
    db.collection::<Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&home.id},
            doc! {"$set":{"reply_channel":bson::to_bson(&member_origin).unwrap()}},
        )
        .await
        .unwrap();
    let guest = channel_turn(&f, &guest_origin, "Private guest history").await;
    let foreign_id = uuid::Uuid::new_v4().to_string();
    let mut foreign_origin = guest_origin.clone();
    foreign_origin.org_execution.as_mut().unwrap().revision = "another-binding".into();
    db.collection::<Document>("assistant_conversations")
        .insert_one(doc! {"_id":&foreign_id,"user_id":&f.org.org,"channel":bson::to_bson(&foreign_origin).unwrap()})
        .await
        .unwrap();
    db.collection::<Document>("nyxbot_threads")
        .insert_one(
            doc! {"_id":uuid::Uuid::new_v4().to_string(),"channel_id":&f.channel.id,
            "user_id":&f.org.admin,"partition":"gateway-private","kind":null,
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()},
        )
        .await
        .unwrap();
    // A conflicting lane aborts the entire adoption, including the thread and
    // member home update. Retrying after the conflict clears must be safe.
    let mut conflicting_lane = db
        .collection::<Document>(crate::models::org_channel::LANES)
        .find_one(doc! {"conversation_id":&guest.id})
        .await
        .unwrap()
        .unwrap();
    let conflicting_id = uuid::Uuid::new_v4().to_string();
    conflicting_lane.insert("_id", &conflicting_id);
    conflicting_lane.insert("partition", "gateway-private");
    db.collection::<Document>(crate::models::org_channel::LANES)
        .insert_one(conflicting_lane)
        .await
        .unwrap();
    assert!(
        transcript::adopt_private_partition(db, &f.channel, "private", "gateway-private")
            .await
            .is_err()
    );
    let unchanged_home = engine::get(db, &f.org.member, &home.id).await.unwrap();
    assert_eq!(
        unchanged_home.reply_channel.as_ref().unwrap().partition,
        "private"
    );
    assert!(
        db.collection::<Document>("nyxbot_threads")
            .find_one(doc! {
                "channel_id":&f.channel.id,"partition":"private","kind":"private"
            })
            .await
            .unwrap()
            .is_some()
    );
    db.collection::<Document>(crate::models::org_channel::LANES)
        .delete_one(doc! {"_id":&conflicting_id})
        .await
        .unwrap();
    for _ in 0..2 {
        transcript::adopt_private_partition(db, &f.channel, "private", "gateway-private")
            .await
            .unwrap();
    }
    let moved_home = engine::get(db, &f.org.member, &home.id).await.unwrap();
    let moved_guest = engine::get(db, &f.org.org, &guest.id).await.unwrap();
    assert_eq!(
        moved_home.reply_channel.as_ref().unwrap().partition,
        "gateway-private"
    );
    assert_eq!(
        moved_guest.channel.as_ref().unwrap().partition,
        "gateway-private"
    );
    assert_eq!(
        moved_guest.channel.as_ref().unwrap().org_execution,
        guest_origin.org_execution
    );
    conversation_access(db, &moved_home).await.unwrap().unwrap();
    conversation_access(db, &moved_guest)
        .await
        .unwrap()
        .unwrap();
    let (id, exists) = transcript::lane(
        db,
        moved_guest.channel.as_ref().unwrap(),
        &f.org.org,
        &f.agent.id,
    )
    .await
    .unwrap();
    assert_eq!(id, guest.id);
    assert!(exists);
    let alias = db
        .collection::<NyxbotThread>("nyxbot_threads")
        .find_one(doc! {"channel_id":&f.channel.id,"relay_partition":"private"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        alias.partition, "gateway-private",
        "flag-off relay lookup keeps the same partition"
    );
    assert_eq!(
        db.collection::<Document>("nyxbot_threads")
            .count_documents(doc! {"channel_id":&f.channel.id})
            .await
            .unwrap(),
        1
    );
    let foreign = db
        .collection::<Document>("assistant_conversations")
        .find_one(doc! {"_id":&foreign_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        foreign
            .get_document("channel")
            .unwrap()
            .get_str("partition")
            .unwrap(),
        "private"
    );
    assert_eq!(
        db.collection::<Document>(crate::models::org_channel::MESSAGES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0,
        "private history is not published"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_credentials_do_not_verify_an_org_member() {
    use crate::services::{
        channel_adapters::lark::LarkFamilyAdapter, channel_verification_service as checks,
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let f = Fixture::new("org_lark_setup_independent").await;
    let state = &f.org.state;
    let db = &state.db;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"code":0,"tenant_access_token":"test-token","expire":7200}),
        ))
        .expect(2)
        .mount(&server)
        .await;
    for platform in ["lark", "feishu"] {
        let adapter = LarkFamilyAdapter::media_test_adapter(&server.uri(), platform);
        let mut bot = f.bot.clone();
        bot.platform = platform.into();
        bot.bot_token_encrypted = state
            .encryption_keys
            .encrypt(b"cli_app:app-secret")
            .await
            .unwrap();
        bot.lark_verification_token_encrypted = Some(
            state
                .encryption_keys
                .encrypt(b"verification-token")
                .await
                .unwrap(),
        );
        bot.status = "pending_webhook".into();
        bot.webhook_registered = false;
        db.collection::<ChannelBot>("channel_bots")
            .replace_one(doc! {"_id":&bot.id}, &bot)
            .await
            .unwrap();
        let result = checks::verify(
            db,
            &state.encryption_keys,
            &state.http_client,
            &adapter,
            &bot,
        )
        .await
        .unwrap();
        assert!(matches!(
            result.status,
            crate::models::channel_bot::VerificationStatus::Verified
        ));
        let bot = crate::services::channel_bot_service::get_bot(db, &bot.id)
            .await
            .unwrap();
        assert_eq!(bot.status, "pending_webhook");
        assert!(!bot.webhook_registered);
        assert!(bot.diagnostics.last_inbound_at.is_none());
        assert_eq!(
            management::view(db, &f.org.member, &f.channel)
                .await
                .unwrap()["member_verified"],
            false
        );
        assert!(
            sender(db, &f.channel, "member-chat", true, &f.agent.id)
                .await
                .is_err()
        );
    }
    db.collection::<ChannelBot>("channel_bots")
        .replace_one(doc! {"_id":&f.bot.id}, &f.bot)
        .await
        .unwrap();
    let issued = links::issue(db, &f.org.member, &f.channel).await.unwrap();
    let id = issued["id"].as_str().unwrap();
    links::receive(
        db,
        &f.channel,
        "member-chat",
        issued["code"].as_str().unwrap(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        management::view(db, &f.org.member, &f.channel)
            .await
            .unwrap()["member_verified"],
        false
    );
    links::confirm(db, &f.org.member, &f.channel, id)
        .await
        .unwrap();
    assert_eq!(
        management::view(db, &f.org.member, &f.channel)
            .await
            .unwrap()["member_verified"],
        true
    );
    assert_eq!(
        management::view(db, &f.org.admin, &f.channel)
            .await
            .unwrap()["member_verified"],
        false
    );
    let mut rebound = f.channel.clone();
    rebound.org_specialist.as_mut().unwrap().revision = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        management::view(db, &f.org.member, &rebound).await.unwrap()["member_verified"],
        false
    );
    f.org.revoke(&f.org.member).await;
    assert_eq!(
        management::view(db, &f.org.member, &f.channel)
            .await
            .unwrap()["member_verified"],
        false
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn setup_guidance_keeps_readiness_and_expiry_checks_closed() {
    let f = Fixture::new("org_channel_setup_guards").await;
    let db = &f.org.state.db;
    let issued = links::issue(db, &f.org.member, &f.channel).await.unwrap();
    let id = issued["id"].as_str().unwrap();
    let code = issued["code"].as_str().unwrap();
    links::receive(db, &f.channel, "member-chat", code, None)
        .await
        .unwrap();
    db.collection::<Document>(crate::models::org_channel::CHALLENGES).update_one(doc!{"_id":id},
        doc!{"$set":{"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
    assert!(
        links::status(db, &f.org.member, &f.channel, id)
            .await
            .is_err()
    );
    assert!(
        links::confirm(db, &f.org.member, &f.channel, id)
            .await
            .is_err()
    );
    assert_eq!(
        management::view(db, &f.org.member, &f.channel)
            .await
            .unwrap()["member_verified"],
        false
    );
    // A live incompatible replica produces the existing distinct readiness error.
    let replicas = db.collection::<Document>(crate::models::security_replica::COLLECTION_NAME);
    let mut replica = replicas.find_one(doc! {}).await.unwrap().unwrap();
    replica.insert("_id", "unready-extra-replica");
    replica.insert("instance_name", "unready-extra-replica");
    replicas.insert_one(replica).await.unwrap();
    assert!(matches!(
        require_target(db, &f.org.admin, &f.agent).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_access_2283_enabled_guest_does_not_revoke_transport_admin() {
    let mut f = Fixture::new("org_access_2283_guest").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let origin = f.guest("unverified-visitor").await;
    assert!(origin.org_execution.as_ref().unwrap().guest);
    assert_eq!(
        crate::handlers::nyxbot::org_access(&f.org.state, &f.channel)
            .await
            .unwrap(),
        OrgAccess::Holds
    );
    crate::handlers::nyxbot::route_key_auth(&f.org.state, &f.channel)
        .await
        .unwrap();
    let current = f
        .org
        .state
        .db
        .collection::<NyxbotChannel>("nyxbot_channels")
        .find_one(doc! {"_id":&f.channel.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.status, "active");
    assert_eq!(current.last_error, None);
    for (collection, id) in [
        ("api_keys", &f.channel.route_api_key_id),
        (
            "channel_conversations",
            f.channel.route_id.as_ref().unwrap(),
        ),
    ] {
        assert!(
            f.org
                .state
                .db
                .collection::<Document>(collection)
                .find_one(doc! {"_id":id,"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
    }
    f.org.state.db.drop().await.unwrap();
}

#[path = "guest_settings_tests.rs"]
mod guest_settings;

#[path = "grants_cutover_tests.rs"]
mod grants_cutover;
