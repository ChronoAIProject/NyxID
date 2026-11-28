// Account-export access paths are separate from destructive ownership. Keep the
// exceptions next to executable evidence so a new selector cannot silently scan
// an unclassified collection.
mod export_index_coverage {
    use super::*;
    use crate::services::account_export_service::{additional_export_filter, owner_filter};
    use bson::{Bson, Document, doc};

    const PERSON: &str = "export-index-person";

    #[derive(Debug)]
    struct AccessPath {
        collection: &'static str,
        // Each OR branch needs one selective anchor, not every residual field.
        anchors: BTreeSet<String>,
    }

    fn branches(filter: &Document) -> Vec<BTreeSet<String>> {
        let mut result = vec![BTreeSet::new()];
        for (key, value) in filter {
            let alternatives = match (key.as_str(), value) {
                ("$or", Bson::Array(terms)) => terms
                    .iter()
                    .flat_map(|term| branches(term.as_document().unwrap()))
                    .collect(),
                ("$and", Bson::Array(terms)) => {
                    let mut conjunction = vec![BTreeSet::new()];
                    for term in terms {
                        conjunction = cross(conjunction, branches(term.as_document().unwrap()));
                    }
                    conjunction
                }
                (_, Bson::String(value)) if value == PERSON => vec![BTreeSet::from([key.clone()])],
                _ => vec![BTreeSet::new()],
            };
            result = cross(result, alternatives);
        }
        result
    }

    fn cross(left: Vec<BTreeSet<String>>, right: Vec<BTreeSet<String>>) -> Vec<BTreeSet<String>> {
        left.into_iter()
            .flat_map(|left| {
                right
                    .iter()
                    .map(move |right| left.union(right).cloned().collect())
            })
            .collect()
    }

    fn export_paths() -> Vec<AccessPath> {
        let mut paths = Vec::new();
        let mut add = |collection, filter: Document| {
            for anchors in branches(&filter) {
                assert!(
                    !anchors.is_empty(),
                    "export query has no person/parent anchor: {collection}: {filter:?}"
                );
                paths.push(AccessPath {
                    collection,
                    anchors,
                });
            }
        };
        for entry in ENTRIES
            .iter()
            .filter(|entry| matches!(entry.export, Export::Section(_)))
        {
            match entry.export_selector {
                ExportSelector::JoinedPerson {
                    parent,
                    field,
                    owner_field,
                } => {
                    add(parent, doc! {owner_field: PERSON});
                    // The query uses an $in of resolved ids. A single equality
                    // exercises exactly the same child index prefix.
                    add(entry.collection, doc! {field: PERSON});
                }
                _ => add(
                    entry.collection,
                    owner_filter(entry, PERSON).expect("export selector"),
                ),
            }
            if entry.collection == "assistant_messages" {
                // message_scope resolves private conversation ids once, then
                // queries messages with the mandatory user_id anchor.
                add("assistant_conversations", doc! {"user_id":PERSON});
            }
        }
        for view in ADDITIONAL_EXPORT_VIEWS {
            add(view.collection, additional_export_filter(view, PERSON));
        }
        paths
    }

    pub(super) fn collections() -> BTreeSet<&'static str> {
        export_paths()
            .into_iter()
            .map(|path| path.collection)
            .collect()
    }

    #[derive(Clone, Copy, Debug)]
    enum Bound {
        // Fixed writer lifetime AND a matching MongoDB expiry index.
        ShortTtl {
            field: &'static str,
            index_seconds: i64,
            lifetime_seconds: i64,
        },
        // Metadata at resource/relationship/configuration cadence. This is not
        // a claimed hard global row limit or a per-message/event stream.
        SmallOrResourceBounded,
    }
    struct Exception {
        collection: &'static str,
        field: &'static str,
        bound: Bound,
        why: &'static str,
        source: &'static str,
        pins: &'static [&'static str],
    }
    macro_rules! resource {
        ($collection:literal, $field:literal, $why:literal, $source:literal, [$($pin:literal),+ $(,)?]) => {
            Exception { collection:$collection, field:$field, bound:Bound::SmallOrResourceBounded,
                why:$why, source:include_str!($source), pins:&[$($pin),+] }
        };
    }
    macro_rules! ttl {
        ($collection:literal, $field:literal, $expiry:literal, $index_seconds:expr, $lifetime:expr, $why:literal, $source:literal, [$($pin:literal),+ $(,)?]) => {
            Exception { collection:$collection, field:$field,
                bound:Bound::ShortTtl { field:$expiry, index_seconds:$index_seconds, lifetime_seconds:$lifetime },
                why:$why, source:include_str!($source), pins:&[$($pin),+] }
        };
    }

    const BOUNDED: &[Exception] = &[
        resource!(
            "agent_key_login_requests",
            "approved_user_id",
            "auth_agent_key_login_service::request writes REQUEST_TTL_SECS=600 login requests; sweep_expired revokes undelivered credentials before terminal-only TTL can purge them. Approved/incomplete cleanup is deliberately retained, so no unconditional short TTL is claimed.",
            "auth_agent_key_login_service.rs",
            [
                "pub const REQUEST_TTL_SECS: i64 = 600;",
                "expires_at: now + Duration::seconds(REQUEST_TTL_SECS)",
                "pub async fn sweep_expired("
            ]
        ),
        resource!(
            "auth_device_codes_v2",
            "approved_user_id",
            "auth_device_service shares the ten-minute login writer with the legacy protocol, initially leaving purge_at unset. Cleanup must revoke undelivered grants before purge, so this is login lifecycle inventory rather than an unconditional TTL bound.",
            "auth_device_service.rs",
            [
                "const AUTH_DEVICE_EXPIRES_IN_SECS: i64 = 10 * 60;",
                "purge_at: None,",
                "expires_at: now + Duration::seconds(AUTH_DEVICE_EXPIRES_IN_SECS)"
            ]
        ),
        resource!(
            "assistant_cma_agents",
            "user_id",
            "assistant_cma_agent_service::register_live upserts one owner/service/remote-agent profile, not each task. Its existing owner prefix is partial on cma_agent_id being a string; the owner-only export does not guarantee that predicate.",
            "assistant_cma_agent_service.rs",
            [
                "let identity=doc! {\"user_id\":owner,\"cma_service_id\":service,\"cma_agent_id\":&remote.id};",
                ".upsert(true).return_document(ReturnDocument::After)",
                ".partial_filter_expression(doc! {\"cma_agent_id\":{\"$type\":\"string\"}})"
            ]
        ),
        resource!(
            "assistant_cma_sandboxes",
            "user_id",
            "assistant_cma_sandbox_service::indexes permits one occupied account slot per owner; creation stores numbered resource generations, not per-task events. Historical generations persist for receipt reconciliation. Existing owner prefixes are partial on occupied slots, idempotency keys or remote ids, none guaranteed by the export.",
            "assistant_cma_sandbox_service.rs",
            [
                ".keys(doc! {\"user_id\": 1})",
                ".unique(true)",
                ".partial_filter_expression(doc! {\"occupies_account_slot\": true})",
                "let generation = previous",
                "occupies_account_slot: true,"
            ]
        ),
        resource!(
            "assistant_agent_learning_members",
            "member_user_id",
            "assistant_agent_learning::set_member_opt_in replaces one agent/member relationship by its deterministic id.",
            "assistant_agent_learning.rs",
            [
                "let id = format!(\"{agent_id}:{actor}\");",
                ".replace_one(doc! {\"_id\": &row.id}, &row)"
            ]
        ),
        resource!(
            "assistant_agent_learning_runs",
            "owner_id",
            "assistant_agent_learning::create_run/reserve_run stores analysis-run metadata, not each source turn; at most MAX_PENDING=16 queued/analyzing runs per agent. Historical runs have no TTL or hard total cap.",
            "assistant_agent_learning.rs",
            [
                "const MAX_PENDING: u64 = 16;",
                "async fn create_run(",
                "if mode == \"threshold\" && candidates.len() < config.threshold as usize",
                "async fn reserve_run("
            ]
        ),
        resource!(
            "assistant_agent_learning_skill_roots",
            "owner_id",
            "assistant_agent_learning::ensure_indexes makes (agent_id,skill_id,version) unique; assistant_agent_learning_review writes one root per published skill version, not a turn log.",
            "assistant_agent_learning.rs",
            [
                ".keys(doc! {\"agent_id\": 1, \"skill_id\": 1, \"version\": 1})",
                ".options(IndexOptions::builder().unique(true).build())"
            ]
        ),
        resource!(
            "assistant_learning_publication_targets",
            "owner_id",
            "assistant_agent_learning_review reserves a single publication target per skill/version and removes released reservations; ids are target:version, not event UUIDs.",
            "assistant_agent_learning_review.rs",
            [
                "let id = format!(\"{}:{}\", target.as_deref().unwrap_or_default(), p.version);",
                "PUBLICATION_TARGETS_COLLECTION_NAME).delete_one("
            ]
        ),
        resource!(
            "assistant_agent_private_notes",
            "actor_user_id",
            "shared_agent_service::update_notes replaces the agent/actor notes document; ensure_indexes uniquely fences that pair.",
            "shared_agent_service.rs",
            [
                "let filter = doc! {\"agent_id\": &agent.id, \"actor_user_id\": actor};",
                ".replace_one(filter, &row)",
                "doc! {\"agent_id\": 1, \"actor_user_id\": 1}"
            ]
        ),
        resource!(
            "assistant_agent_shares",
            "owner_id",
            "shared_agent_service replaces each agent's audience; MAX_SHARES=500 and unique agent/subject pairs bound each current audience.",
            "shared_agent_service.rs",
            [
                "const MAX_SHARES: usize = 500;",
                ".delete_many(doc! {\"agent_id\": id})",
                "doc! {\"agent_id\": 1, \"subject_kind\": 1, \"subject_id\": 1}"
            ]
        ),
        resource!(
            "assistant_agent_sharing",
            "owner_id",
            "shared_agent_service stores one current sharing policy with _id equal to the agent id.",
            "shared_agent_service.rs",
            [
                "\"$setOnInsert\": {\"_id\": id}",
                ".upsert(previous.is_none())"
            ]
        ),
        resource!(
            "assistant_async_quotas",
            "user_id",
            "async_service_operation quota admission upserts _id=user_id; one counter per person.",
            "async_service_operation.rs",
            ["QUOTAS_COLLECTION_NAME).update_one(doc!{\"_id\":&chat.user_id}"]
        ),
        resource!(
            "assistant_group_agent_access",
            "owner_id",
            "collaboration_group_service has one current access relationship per group/agent, enforced by a unique index.",
            "collaboration_group_service/mod.rs",
            ["(model::AGENT_ACCESS, doc! {\"group_id\":1,\"agent_id\":1}, true)"]
        ),
        resource!(
            "assistant_group_agent_proposals",
            "proposed_by",
            "collaboration_group_service::membership upserts deterministic group/proposer/agent proposals with a seven-day expiry; this is membership metadata.",
            "collaboration_group_service/membership.rs",
            [
                "id:format!(\"{id}:{actor}:{agent_id}\")",
                "expires_at:Utc::now()+Duration::days(7)",
                "model::PROPOSALS).replace_one"
            ]
        ),
        resource!(
            "assistant_group_invites",
            "issuer_user_id",
            "collaboration_group_service::membership stores seven-day, single-use group invitations with at most 32 pending per group, not group messages.",
            "collaboration_group_service/membership.rs",
            [
                "model::INVITES",
                "expires_at:now+Duration::days(7)",
                "if count >= 32"
            ]
        ),
        resource!(
            "assistant_group_invites",
            "accepted_by_user_id",
            "collaboration_group_service::membership records the sole accepting member on the same expiring invitation.",
            "collaboration_group_service/membership.rs",
            [
                "\"accepted_by_user_id\":actor",
                "expires_at:now+Duration::days(7)"
            ]
        ),
        resource!(
            "assistant_group_invites",
            "intended_user_id",
            "collaboration_group_service::membership records an optional intended member on the same expiring invitation.",
            "collaboration_group_service/membership.rs",
            [
                "\"intended_user_id\":actor",
                "expires_at:now+Duration::days(7)"
            ]
        ),
        resource!(
            "assistant_group_read_states",
            "user_id",
            "collaboration_group_service::notifications updates one read position per group/member; ensure_indexes uniquely fences that pair.",
            "collaboration_group_service/mod.rs",
            ["(model::READ_STATES, doc! {\"group_id\":1,\"user_id\":1}, true)"]
        ),
        ttl!(
            "assistant_skill_draft_attempts",
            "user_id",
            "created_at",
            86_400,
            86_400,
            "db::ensure_indexes fixes the created_at TTL at 86400 seconds; assistant_skill_authoring::record_format_rejection updates one conversation/turn counter.",
            "../db.rs",
            [
                ".name(\"assistant_skill_draft_attempts_ttl\".to_owned())",
                ".expire_after(Duration::from_secs(86400))"
            ]
        ),
        resource!(
            "assistant_voice_windows",
            "user_id",
            "assistant_voice::ensure_indexes uniquely fences billing_request_id; billing::voice reserves deterministic call/window funding records, not audio frames.",
            "assistant_voice.rs",
            [
                "crate::models::assistant_voice::WINDOWS",
                ".keys(doc! {\"billing_request_id\":1})",
                ".options(IndexOptions::builder().unique(true).build())"
            ]
        ),
        ttl!(
            "assistant_wire_logs",
            "user_id",
            "expires_at",
            0,
            900,
            "assistant_wire_log_service::WIRE_LOG_TTL_SECS is 15 minutes and record sets expires_at from it.",
            "assistant_wire_log_service.rs",
            [
                "pub const WIRE_LOG_TTL_SECS: i64 = 15 * 60;",
                "expires_at: created_at + Duration::seconds(WIRE_LOG_TTL_SECS)"
            ]
        ),
        resource!(
            "auth_device_codes",
            "approved_user_id",
            "auth_device_service writes 10-minute login requests but purge_at is set only after grant cleanup. Pending/approved cleanup can be retried, so AUTH_DEVICE_EXPIRES_IN_SECS is not a fixed retention bound; these are login lifecycle records.",
            "auth_device_service.rs",
            [
                "const AUTH_DEVICE_EXPIRES_IN_SECS: i64 = 10 * 60;",
                "purge_at: None,",
                "expires_at: now + Duration::seconds(AUTH_DEVICE_EXPIRES_IN_SECS)"
            ]
        ),
        ttl!(
            "authorization_codes",
            "user_id",
            "expires_at",
            0,
            300,
            "oauth_service::authorize writes expires_at=now+Duration::minutes(5).",
            "oauth_service.rs",
            ["expires_at: now + Duration::minutes(5)"]
        ),
        resource!(
            "channel_adapter_allocations",
            "user_id",
            "call_channels::session uniquely fences catalog/provider/account/resource allocations; management inserts on resource allocation, never for a call event.",
            "call_channels/session.rs",
            [
                "ALLOCATIONS",
                "doc! {\"provider\":1,\"account_id\":1,\"resource_id\":1}"
            ]
        ),
        resource!(
            "channel_email_subscriptions",
            "user_id",
            "channel_adapters::aurinko_lifecycle keys the one callback subscription binding by bot.id.",
            "channel_adapters/aurinko_lifecycle.rs",
            [
                "collection.find_one(doc! { \"_id\": &bot.id })",
                "let binding = EmailSubscription {",
                "id: bot.id.clone()"
            ]
        ),
        ttl!(
            "device_codes",
            "approved_by_user_id",
            "expires_at",
            0,
            86_400,
            "device_code_service uses DEVICE_CODE_EXPIRES_IN_SECS=900 while pending and DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS=86400 after approval.",
            "device_code_service/mod.rs",
            [
                "pub const DEVICE_CODE_EXPIRES_IN_SECS: i64 = 15 * 60;",
                "pub const DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS: i64 = 24 * 60 * 60;"
            ]
        ),
        resource!(
            "feature_flag_overrides",
            "target_key",
            "feature_flag_service updates the current flag/target configuration instead of appending evaluations.",
            "feature_flag_service.rs",
            [
                "\"target_key\": target_key.clone()",
                "\"flag_key\": flag_key"
            ]
        ),
        resource!(
            "invite_codes",
            "usages.user_id",
            "invite_code_service stores signup invite configuration with embedded usage entries, not a separate per-request stream; max_uses controls redeemable uses.",
            "invite_code_service.rs",
            ["\"usages\"", "max_uses", "pub async fn create"]
        ),
        ttl!(
            "invocation_diagnostics",
            "actor_user_id",
            "expires_at",
            0,
            60,
            "invocation_diagnostic_service::HANDLE_TTL_SECS=60; diagnostic handles expire from creation.",
            "invocation_diagnostic_service.rs",
            [
                "pub(crate) const HANDLE_TTL_SECS: i64 = 60;",
                "expires_at: now + chrono::Duration::seconds(HANDLE_TTL_SECS)"
            ]
        ),
        ttl!(
            "login_approvals",
            "user_id",
            "expires_at",
            0,
            600,
            "login_approval_service::begin caps request-bound approval expiry at ten minutes.",
            "login_approval_service.rs",
            ["expires_at: expires_at.min(Utc::now() + Duration::minutes(10))"]
        ),
        resource!(
            "machine_orphan_inventory",
            "owner_id",
            "remote_machine_events::record_investigation upserts one finding per controlled-resource identity; recurring sweeps refresh last_seen_at rather than append events. No TTL is claimed for unresolved findings.",
            "remote_machine_events.rs",
            [
                "doc! {\"namespace\":&resource.namespace,\"kind\":resource.kind,\"identity_hash\":&resource.identity_hash}",
                "\"last_seen_at\":now",
                ").upsert(true).await?"
            ]
        ),
        ttl!(
            "mfa_challenges",
            "user_id",
            "expires_at",
            0,
            300,
            "token_service::insert_mfa_pending_session_with_method writes a five-minute pending session.",
            "token_service.rs",
            [
                "let expires = now + Duration::minutes(5);",
                "let pending_session = Session {",
                "expires_at: expires"
            ]
        ),
        ttl!(
            "oauth_consent_requests",
            "user_id",
            "expires_at",
            0,
            86_400,
            "oauth_consent_request_service::create fixes expires_at at 24 hours.",
            "oauth_consent_request_service.rs",
            ["expires_at: Utc::now() + Duration::hours(24)"]
        ),
        resource!(
            "openclaw_channel_mappings",
            "nyxid_user_id",
            "openclaw_channel_service::upsert_mapping updates the current channel/external-user binding instead of creating a row per message.",
            "openclaw_channel_service.rs",
            [
                "pub async fn upsert_mapping(",
                "\"channel\": channel,",
                "\"channel_user_id\": channel_user_id,",
                "\"prepareUnique\": true",
                "\"nyxid_user_id\": nyxid_user_id",
                ".upsert(true)"
            ]
        ),
        resource!(
            "oracle_workers",
            "enrollment.owner_user_id",
            "oracle_worker_enrollment_service serializes enrollment on the pool and caps enrolled workers at MAX_ENROLLED_WORKERS=256; one enrollment embedded per worker.",
            "oracle_worker_enrollment_service.rs",
            [
                "pub const MAX_ENROLLED_WORKERS: u64 = 256;",
                "if count >= MAX_ENROLLED_WORKERS",
                "worker.enrollment = Some(enrollment)"
            ]
        ),
        resource!(
            "org_channel_lanes",
            "actor_id",
            "org_channel_service::transcript::lane upserts one current channel/partition/actor/agent/binding-generation/sender lane, not each message.",
            "org_channel_service/transcript.rs",
            [
                "pub(crate) async fn lane(",
                "\"actor_id\":actor",
                "let mut writable = filter.clone();",
                "rows.update_one(writable,",
                ").upsert(true).await"
            ]
        ),
        ttl!(
            "org_channel_link_challenges",
            "user_id",
            "expires_at",
            0,
            600,
            "org_channel_service::links::issue writes a ten-minute expires_at; the lifetime applies to issued, observed and confirmed rows.",
            "org_channel_service/links.rs",
            [
                "let expires = Utc::now() + Duration::minutes(10);",
                "\"expires_at\":bson::DateTime::from_chrono(expires)"
            ]
        ),
        resource!(
            "org_channel_sender_links",
            "user_id",
            "org_channel_service::links::confirm reuses the channel/sender binding; a unique channel_id/sender_hash key prevents per-link-attempt history.",
            "org_channel_service/links.rs",
            [
                "find_one(doc!{\"channel_id\":&channel.id,\"sender_hash\":hash})",
                "if let Some(existing)=existing",
                "insert_one(SenderLink"
            ]
        ),
        resource!(
            "org_invites",
            "redeemed_by",
            "org_invite_service::redeem_invite records one redeemer on a single-use organization invitation. DEFAULT_INVITE_TTL=24h is caller-overridable, so it is not treated as a fixed short-TTL guarantee.",
            "org_invite_service.rs",
            [
                "pub const DEFAULT_INVITE_TTL: Duration = Duration::hours(24);",
                "ttl.unwrap_or(DEFAULT_INVITE_TTL)",
                "pub async fn redeem_invite("
            ]
        ),
        resource!(
            "platform_personal_setup_selections",
            "user_id",
            "platform_manager_management replaces the selected setup under _id=personal binding.id; the current context also expires after 30 days.",
            "platform_manager_management.rs",
            [
                "doc! {\"_id\":&binding.id}",
                "chrono::Utc::now()+chrono::Duration::days(30)"
            ]
        ),
        resource!(
            "platform_public_probe_consents",
            "actor_user_id",
            "platform_public_contract_service stores human readiness consent for five minutes; sweep_probes deletes expired consent rows. This uses an application purge, not a MongoDB TTL guarantee.",
            "platform_public_contract_service.rs",
            [
                "let consent = ProbeConsent {",
                "expires_at: Utc::now() + Duration::minutes(5)",
                "db.collection::<ProbeConsent>(PROBE_CONSENTS)",
                "pub(crate) async fn sweep_probes(",
                ".delete_many(doc! {\"_id\": {\"$in\": ids}, \"expires_at\": {\"$lte\": bson::DateTime::now()}})"
            ]
        ),
        resource!(
            "service_account_key_read_grants",
            "owner_id",
            "service_account_key_read_service replaces the single current key-read grant with _id=service account id.",
            "service_account_key_read_service.rs",
            [
                ".replace_one(doc! {\"_id\": &grant.service_account_id}, grant)",
                ".upsert(true)"
            ]
        ),
        ttl!(
            "service_pool_member_health",
            "owner_id",
            "expires_at",
            0,
            3600,
            "service_pool_health_service::HEALTH_RETENTION_MS=3600000; current health expires one hour after an observation.",
            "service_pool_health_service.rs",
            [
                "pub const HEALTH_RETENTION_MS: u64 = 60 * 60 * 1_000;",
                "const HEALTH_RETENTION: Duration = Duration::milliseconds(HEALTH_RETENTION_MS as i64);",
                "\"expires_at\": bson::DateTime::from_chrono(now + HEALTH_RETENTION)"
            ]
        ),
        resource!(
            "telegram_bot_claims",
            "owner_user_id",
            "telegram_new_claims uniquely fences manager/bot claims. Redeemed claims intentionally unset purge_after, so their bound is bot provisioning metadata, not the initial 15-minute expiry.",
            "telegram_new_claims.rs",
            [
                "doc! {\"manager_bot_id\": 1, \"telegram_bot_id\": 1}",
                "\"$unset\": {\"purge_after\": \"\"}"
            ]
        ),
        resource!(
            "telegram_bot_requests",
            "owner_user_id",
            "telegram_new_service keeps one active provisioning request per actor. Terminal requests purge after one day; provisioning/connected are exempt from expiry and are resource lifecycle records, not short-TTL event history.",
            "telegram_new_service.rs",
            [
                "doc! {\"actor_user_id\": 1}",
                ".partial_filter_expression(doc! {\"active\": true})",
                "\"status\": {\"$nin\": [\"provisioning\", \"connected\"]}"
            ]
        ),
    ];

    fn has_prefix(snapshot: &[Document], field: &str) -> bool {
        field == "_id"
            || snapshot.iter().any(|index| {
                !index.contains_key("partialFilterExpression")
                    && index
                        .get_document("key")
                        .is_ok_and(|keys| keys.keys().next().is_some_and(|key| key == field))
            })
    }

    fn assert_classified(snapshot: &BTreeMap<String, Vec<Document>>, additions: bool) {
        let paths = export_paths();
        let mut used = BTreeSet::new();
        let mut missing = Vec::new();
        for path in paths {
            if path
                .anchors
                .iter()
                .any(|field| has_prefix(&snapshot[path.collection], field))
            {
                continue;
            }
            if additions
                && path.anchors.iter().any(|field| {
                    EXPORT_SELECTOR_INDEX_GAPS
                        .iter()
                        .chain(EXPORT_JOIN_INDEX_GAPS)
                        .chain(OWNER_INDEX_GAPS)
                        .any(|&(collection, key)| collection == path.collection && key == field)
                })
            {
                continue;
            }
            if let Some(entry) = BOUNDED.iter().find(|entry| {
                entry.collection == path.collection && path.anchors.contains(entry.field)
            }) {
                used.insert((entry.collection, entry.field));
            } else {
                missing.push(format!("{} {:?}", path.collection, path.anchors));
            }
        }
        assert!(
            missing.is_empty(),
            "unclassified export selector paths: {}",
            missing.join(", ")
        );
        let declared: BTreeSet<_> = BOUNDED
            .iter()
            .map(|entry| (entry.collection, entry.field))
            .collect();
        assert_eq!(declared.len(), BOUNDED.len(), "duplicate export exception");
        assert_eq!(used, declared, "stale/redundant export exception");
    }

    pub(super) fn assert_export_origin_classification(snapshot: &BTreeMap<String, Vec<Document>>) {
        assert_classified(snapshot, true);
    }

    pub(super) async fn assert_export_prefix_coverage(db: &mongodb::Database) {
        let snapshot = index_snapshot(db).await;
        assert_classified(&snapshot, false);
        for entry in BOUNDED {
            if let Bound::ShortTtl {
                field,
                index_seconds,
                lifetime_seconds,
            } = entry.bound
            {
                assert!(lifetime_seconds > 0 && lifetime_seconds <= 86_400);
                assert!(
                    snapshot[entry.collection].iter().any(|index| {
                        let seconds =
                            index
                                .get("expireAfterSeconds")
                                .and_then(|value| match value {
                                    Bson::Int32(value) => Some(i64::from(*value)),
                                    Bson::Int64(value) => Some(*value),
                                    Bson::Double(value) => Some(*value as i64),
                                    _ => None,
                                });
                        has_prefix(std::slice::from_ref(index), field)
                            && seconds == Some(index_seconds)
                    }),
                    "{} has no matching lifetime index: {field} + {index_seconds}s",
                    entry.collection
                );
            }
        }
        let messages = &snapshot["assistant_messages"];
        assert!(
            messages
                .iter()
                .any(|index| index.get_document("key").is_ok_and(|keys| keys
                    .iter()
                    .take(3)
                    .eq(doc! {"user_id":1,"created_at":-1,"_id":-1}.iter()))),
            "message-history selector and newest-first sort must share an index"
        );
    }

    #[test]
    fn export_exception_evidence_and_fixed_lifetimes_match_writers() {
        fn normalize(source: &str) -> String {
            let compact: String = source.split_whitespace().collect();
            compact
                .replace(",)", ")")
                .replace(",}", "}")
                .replace(",]", "]")
                .trim_end_matches(',')
                .to_owned()
        }
        let docs = include_str!("../../../docs/security/PERSONAL_ERASURE_INVENTORY.md");
        let test_module =
            regex::Regex::new(r"(?m)^#\[cfg\(test\)\]\s*(?:#\[path[^\n]*\]\s*)?mod\s+\w+\s*\{")
                .unwrap();
        for entry in BOUNDED {
            assert!(
                docs.contains(entry.why),
                "missing export-bound evidence in inventory: {}.{}",
                entry.collection,
                entry.field
            );
            assert!(!entry.why.is_empty());
            assert!(!entry.pins.is_empty());
            // Only truncate top-level test modules. cfg(test) parameters and
            // hooks inside production writers must not hide those writers.
            let end = test_module
                .find(entry.source)
                .map_or(entry.source.len(), |found| found.start());
            let source = normalize(&entry.source[..end]);
            for pin in entry.pins {
                assert!(
                    source.contains(&normalize(pin)),
                    "export exception evidence changed for {}.{}: {pin}; verify its bound again ({})",
                    entry.collection,
                    entry.field,
                    entry.why
                );
            }
        }
        let initiation = include_str!("device_code_service/initiate.rs");
        let approval = include_str!("device_code_service/approve.rs");
        assert!(
            initiation.contains("expires_at: now + Duration::seconds(DEVICE_CODE_EXPIRES_IN_SECS)")
        );
        assert!(approval.contains("let delivery_expires_at = now + Duration::seconds(DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS);"));
        assert!(
            approval.contains("\"expires_at\": bson::DateTime::from_chrono(delivery_expires_at)")
        );
        assert_eq!(
            crate::services::assistant_wire_log_service::WIRE_LOG_TTL_SECS,
            900
        );
        assert_eq!(
            crate::services::device_code_service::DEVICE_CODE_EXPIRES_IN_SECS,
            900
        );
        assert_eq!(
            crate::services::device_code_service::DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS,
            86_400
        );
        assert_eq!(
            crate::services::invocation_diagnostic_service::HANDLE_TTL_SECS,
            60
        );
        assert_eq!(
            crate::services::service_pool_health_service::HEALTH_RETENTION_MS,
            3_600_000
        );
    }

    #[test]
    fn export_paths_cover_or_branches_joins_and_additional_views() {
        assert_eq!(
            branches(
                &doc! {"$and":[{"user_id":PERSON},{"$or":[{"author_user_id":PERSON},{"role":"user"}]}]}
            ),
            vec![
                BTreeSet::from(["user_id".into(), "author_user_id".into()]),
                BTreeSet::from(["user_id".into()])
            ]
        );
        let paths = export_paths();
        assert!(
            paths
                .iter()
                .any(|path| path.collection == "assistant_group_messages"
                    && path.anchors == BTreeSet::from(["user_id".into()]))
        );
        assert!(
            paths
                .iter()
                .any(|path| path.collection == "assistant_group_messages"
                    && path.anchors == BTreeSet::from(["author_user_id".into()]))
        );
        assert!(
            !paths
                .iter()
                .any(|path| path.collection == "assistant_group_messages"
                    && path.anchors.contains("approved_user_id"))
        );
        for (collection, field) in EXPORT_SELECTOR_INDEX_GAPS
            .iter()
            .chain(EXPORT_JOIN_INDEX_GAPS)
        {
            assert!(
                paths
                    .iter()
                    .any(|path| path.collection == *collection && path.anchors.contains(*field)),
                "stale export index: {collection}.{field}"
            );
        }
    }
}
