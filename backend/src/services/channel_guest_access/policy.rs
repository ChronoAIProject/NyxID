use bson::doc;
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};
use serde::Serialize;

use super::{AnswerAudience, GuestAssurance, GuestTurn, google_sheets, personal_records};
use crate::{
    errors::{AppError, AppResult},
    models::channel_guest_policy::{
        COLLECTION_NAME, GuestAudience, GuestDataDelivery, GuestDataOperation, GuestDataPolicy,
        GuestDataRule, GuestDataScope, GuestFields,
    },
};

const MAX_RULES: usize = 64;
const MAX_RESOURCES_PER_RULE: usize = 64;

pub(crate) struct GuestPolicySubject<'a> {
    pub guest_id: Option<&'a str>,
    pub private: bool,
    pub shared_room_verified: bool,
    pub room_id: Option<&'a str>,
}

pub(crate) struct GuestDataRequest<'a> {
    pub source_id: &'a str,
    pub collection_id: &'a str,
    pub action: &'a str,
    pub record_id: Option<&'a str>,
    pub record_owner_id: Option<&'a str>,
    pub fields: &'a [&'a str],
}

#[derive(Default, Serialize)]
pub(crate) struct GuestDataDecision {
    pub allowed: bool,
    pub read_fields: Vec<String>,
    pub write_fields: Vec<String>,
}

/// Created only by the evaluator. The connector applies this owner as part of
/// the database predicate for reads and writes.
pub(crate) struct GuestDataPermit {
    bot_id: String,
    owner_id: Option<String>,
    source_id: String,
    action: String,
    read_fields: Vec<String>,
    write_fields: Vec<String>,
}

impl GuestDataPermit {
    pub(crate) fn bot_id(&self) -> &str {
        &self.bot_id
    }

    pub(crate) fn owner_id(&self) -> Option<&str> {
        self.owner_id.as_deref()
    }

    pub(crate) fn source_id(&self) -> &str {
        &self.source_id
    }

    pub(crate) fn action(&self) -> &str {
        &self.action
    }

    pub(crate) fn read_fields(&self) -> &[String] {
        &self.read_fields
    }

    pub(crate) fn write_fields(&self) -> &[String] {
        &self.write_fields
    }
}

pub(crate) fn denied() -> AppError {
    AppError::Forbidden("Guest data access is unavailable for this operation".into())
}

fn known_operation(action: &str) -> Option<GuestDataOperation> {
    match action {
        "create" => Some(GuestDataOperation::Create),
        "read" | "list" => Some(GuestDataOperation::Read),
        "update" => Some(GuestDataOperation::Update),
        "delete" => Some(GuestDataOperation::Delete),
        _ => None,
    }
}

fn valid_selector(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

pub(crate) fn validate_rules(rules: &[GuestDataRule]) -> AppResult<()> {
    if rules.len() > MAX_RULES || !rules.iter().all(valid_rule) {
        return Err(AppError::ValidationError(
            "Guest data rule has an unsupported source, scope, operation, field, or audience"
                .into(),
        ));
    }
    Ok(())
}

fn valid_rule(rule: &GuestDataRule) -> bool {
    let audience_ok = match &rule.audience {
        GuestAudience::AllGuests => true,
        GuestAudience::Guest { guest_id } => {
            valid_selector(guest_id) && rule.delivery == GuestDataDelivery::Private
        }
        GuestAudience::VerifiedAudience {
            audience_id,
            guest_ids,
            room_id,
        } => {
            valid_selector(audience_id)
                && !guest_ids.is_empty()
                && guest_ids.len() <= MAX_RESOURCES_PER_RULE
                && guest_ids.iter().all(|id| valid_selector(id))
                && match rule.delivery {
                    GuestDataDelivery::Private => room_id.is_none(),
                    GuestDataDelivery::Shared => room_id.as_deref().is_some_and(valid_selector),
                }
        }
    };
    if !audience_ok || rule.operations.is_empty() {
        return false;
    }
    match (
        rule.source_id.as_str(),
        rule.collection_id.as_str(),
        &rule.scope,
    ) {
        (personal_records::SOURCE_ID, personal_records::SOURCE_ID, GuestDataScope::OwnRecords) => {
            rule.delivery == GuestDataDelivery::Private
                && rule.operations.iter().all(|(operation, fields)| {
                    fields
                        .read
                        .iter()
                        .chain(fields.write.iter())
                        .all(|field| personal_records::FIELDS.contains(&field.as_str()))
                        && (fields.write.is_empty()
                            || matches!(
                                operation,
                                GuestDataOperation::Create | GuestDataOperation::Update
                            ))
                })
        }
        ("google_sheets", "values", GuestDataScope::Resources { record_ids }) => {
            !record_ids.is_empty()
                && record_ids.len() <= MAX_RESOURCES_PER_RULE
                && rule.operations.iter().all(|(operation, fields)| {
                    let action = match operation {
                        GuestDataOperation::Read => "read",
                        GuestDataOperation::Create
                            if rule.delivery == GuestDataDelivery::Shared =>
                        {
                            "create"
                        }
                        _ => return false,
                    };
                    fields == &GuestFields::default()
                        && record_ids
                            .iter()
                            .all(|id| google_sheets::validate_resource_id(id, action))
                })
        }
        _ => false,
    }
}

pub(crate) async fn load_policy(db: &Database, bot_id: &str) -> AppResult<Option<GuestDataPolicy>> {
    let mut policies = db
        .collection::<GuestDataPolicy>(COLLECTION_NAME)
        .find(doc! { "bot_id": bot_id })
        .limit(2)
        .await?;
    let policy = policies.try_next().await?;
    if policies.try_next().await?.is_some() {
        return Err(denied());
    }
    if let Some(row) = &policy {
        validate_loaded(row)?;
    }
    Ok(policy)
}

fn validate_loaded(policy: &GuestDataPolicy) -> AppResult<()> {
    if policy.revision <= 0 || validate_rules(&policy.rules).is_err() {
        return Err(denied());
    }
    Ok(())
}

pub(crate) async fn load_bound_policy(
    db: &Database,
    turn: &GuestTurn,
) -> AppResult<Option<GuestDataPolicy>> {
    super::revalidate(db, turn).await?;
    let policy = load_policy(db, &turn.bot_id).await?;
    if policy.as_ref().map_or(0, |row| row.revision) != turn.policy_revision {
        return Err(denied());
    }
    if let Some(row) = &policy {
        super::setup::validate_applied_declarations(db, row).await?;
    }
    Ok(policy)
}

pub(crate) async fn fence_policy_in_session(
    db: &Database,
    session: &mut ClientSession,
    turn: &GuestTurn,
) -> AppResult<GuestDataPolicy> {
    let row = db
        .collection::<GuestDataPolicy>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "bot_id": &turn.bot_id, "revision": turn.policy_revision },
            doc! { "$set": { "last_effect_fence": uuid::Uuid::new_v4().to_string() } },
        )
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    validate_loaded(&row)?;
    super::setup::fence_applied_declarations_in_session(db, session, &row).await?;
    Ok(row)
}

pub(crate) async fn revalidate_delivery(
    db: &Database,
    turn: &GuestTurn,
    bot_id: &str,
    binding_id: &str,
    platform: &str,
    conversation_id: &str,
    audience: AnswerAudience,
) -> AppResult<()> {
    if bot_id != turn.bot_id
        || binding_id != turn.binding_id
        || platform != turn.platform
        || conversation_id != turn.conversation_id
        || audience != turn.audience
        || (audience == AnswerAudience::Private
            && turn.actor.assurance != GuestAssurance::ProviderVerified)
    {
        return Err(denied());
    }
    load_bound_policy(db, turn).await?;
    Ok(())
}

pub(crate) fn preview(
    rules: &[GuestDataRule],
    subject: &GuestPolicySubject<'_>,
    request: &GuestDataRequest<'_>,
) -> GuestDataDecision {
    if validate_rules(rules).is_err()
        || (request.source_id == personal_records::SOURCE_ID
            && request.record_id.is_some()
            && (subject.guest_id.is_none() || subject.guest_id != request.record_owner_id))
    {
        return GuestDataDecision::default();
    }
    evaluate(rules, subject, request)
}

fn evaluate(
    rules: &[GuestDataRule],
    subject: &GuestPolicySubject<'_>,
    request: &GuestDataRequest<'_>,
) -> GuestDataDecision {
    let Some(operation) = known_operation(request.action) else {
        return GuestDataDecision::default();
    };
    // A private turn can contain personal records or restricted reads. Its
    // model-derived tool arguments cannot be published to a shared spreadsheet.
    if request.source_id == google_sheets::SOURCE_ID
        && operation == GuestDataOperation::Create
        && !subject.shared_room_verified
    {
        return GuestDataDecision::default();
    }
    let matching = rules.iter().filter_map(|rule| {
        if rule.source_id != request.source_id
            || rule.collection_id != request.collection_id
            || (rule.delivery == GuestDataDelivery::Private && !subject.private)
            || !audience_matches(&rule.audience, subject)
        {
            return None;
        }
        let scope_matches = match &rule.scope {
            GuestDataScope::OwnRecords => subject.private && subject.guest_id.is_some(),
            GuestDataScope::Resources { record_ids } => request
                .record_id
                .is_some_and(|record_id| record_ids.contains(record_id)),
            GuestDataScope::Collection => true,
        };
        let fields = rule.operations.get(&operation)?;
        let allowed = match operation {
            GuestDataOperation::Create | GuestDataOperation::Update => &fields.write,
            GuestDataOperation::Read | GuestDataOperation::Delete => &fields.read,
        };
        (scope_matches && request.fields.iter().all(|field| allowed.contains(*field)))
            .then_some(fields)
    });
    // Only combine field sets from rules that independently authorize the
    // requested fields. The supported sources have one effective row scope
    // per request: the current guest or one exact range of one connection.
    let mut allowed = false;
    let mut read_fields = std::collections::BTreeSet::new();
    let mut write_fields = std::collections::BTreeSet::new();
    for fields in matching {
        allowed = true;
        read_fields.extend(fields.read.iter().cloned());
        write_fields.extend(fields.write.iter().cloned());
    }
    GuestDataDecision {
        allowed,
        read_fields: read_fields.into_iter().collect(),
        write_fields: write_fields.into_iter().collect(),
    }
}

fn audience_matches(audience: &GuestAudience, subject: &GuestPolicySubject<'_>) -> bool {
    match audience {
        GuestAudience::AllGuests => true,
        GuestAudience::Guest { guest_id } => Some(guest_id.as_str()) == subject.guest_id,
        GuestAudience::VerifiedAudience {
            guest_ids, room_id, ..
        } => {
            subject
                .guest_id
                .is_some_and(|guest_id| guest_ids.contains(guest_id))
                && if subject.private {
                    room_id.is_none()
                } else {
                    room_id.is_some() && room_id.as_deref() == subject.room_id
                }
        }
    }
}

pub(crate) fn subject<'a>(turn: &'a GuestTurn, room_id: Option<&'a str>) -> GuestPolicySubject<'a> {
    let verified = turn.actor.assurance == GuestAssurance::ProviderVerified;
    GuestPolicySubject {
        guest_id: verified.then_some(turn.actor.id.as_str()),
        private: verified && turn.audience == AnswerAudience::Private,
        shared_room_verified: turn.allows_shared_sheets_write(),
        room_id: verified.then_some(room_id).flatten(),
    }
}

pub(crate) fn authorize(
    policy: &GuestDataPolicy,
    turn: &GuestTurn,
    request: &GuestDataRequest<'_>,
) -> AppResult<GuestDataPermit> {
    if turn.bot_id != policy.bot_id || turn.policy_revision != policy.revision {
        return Err(denied());
    }
    let room_id = turn.verified_room_id();
    let subject = subject(turn, room_id.as_deref());
    let decision = evaluate(&policy.rules, &subject, request);
    if !decision.allowed {
        return Err(denied());
    }
    Ok(GuestDataPermit {
        bot_id: turn.bot_id.clone(),
        owner_id: (request.source_id == personal_records::SOURCE_ID)
            .then_some(subject.guest_id)
            .flatten()
            .map(str::to_owned),
        source_id: request.source_id.into(),
        action: request.action.into(),
        read_fields: decision.read_fields,
        write_fields: decision.write_fields,
    })
}

pub(crate) fn has_sheets_grant(
    policy: &GuestDataPolicy,
    turn: &GuestTurn,
    connection_id: &str,
    action: &str,
) -> bool {
    let Some(operation) = known_operation(action) else {
        return false;
    };
    let room_id = turn.verified_room_id();
    let subject = subject(turn, room_id.as_deref());
    if operation == GuestDataOperation::Create && !subject.shared_room_verified {
        return false;
    }
    policy.rules.iter().any(|rule| {
        if rule.source_id != google_sheets::SOURCE_ID
            || rule.collection_id != google_sheets::COLLECTION_ID
            || (rule.delivery == GuestDataDelivery::Private && !subject.private)
            || !audience_matches(&rule.audience, &subject)
            || !rule.operations.contains_key(&operation)
        {
            return false;
        }
        matches!(&rule.scope, GuestDataScope::Resources { record_ids } if
            record_ids.iter().any(|id| id.split_once('/').is_some_and(|(id, _)| id == connection_id)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::channel_guest_access::GuestActor;

    const EVENT: &str = "connection-1/workbook-1/Event_Config!A1:H24";
    const QUESTIONS: &str = "connection-1/workbook-1/Questions";

    fn turn(actor: &str, audience: AnswerAudience) -> GuestTurn {
        GuestTurn {
            bot_id: "bot".into(),
            binding_id: "binding".into(),
            platform: "discord".into(),
            conversation_id: "room-1".into(),
            source_event_id: "event".into(),
            actor: GuestActor {
                id: actor.into(),
                assurance: GuestAssurance::ProviderVerified,
            },
            audience,
            shared_room_verified: audience == AnswerAudience::Shared,
            policy_revision: 1,
            activation_epoch: 1,
            bot_revision: 1,
            meta_grant_id: None,
            hosted_assignment: None,
        }
    }

    fn sheets_rule(
        audience: GuestAudience,
        operation: GuestDataOperation,
        resource_id: &str,
    ) -> GuestDataRule {
        GuestDataRule {
            source_id: google_sheets::SOURCE_ID.into(),
            collection_id: google_sheets::COLLECTION_ID.into(),
            audience,
            delivery: GuestDataDelivery::Shared,
            operations: [(operation, GuestFields::default())].into(),
            scope: GuestDataScope::Resources {
                record_ids: [resource_id.into()].into(),
            },
        }
    }

    fn sheets_request<'a>(action: &'a str, resource: &'a str) -> GuestDataRequest<'a> {
        GuestDataRequest {
            source_id: google_sheets::SOURCE_ID,
            collection_id: google_sheets::COLLECTION_ID,
            action,
            record_id: Some(resource),
            record_owner_id: None,
            fields: &[],
        }
    }

    fn policy(rules: Vec<GuestDataRule>) -> GuestDataPolicy {
        validate_rules(&rules).unwrap();
        GuestDataPolicy {
            id: uuid::Uuid::new_v4().to_string(),
            bot_id: "bot".into(),
            revision: 1,
            rules,
        }
    }

    #[test]
    fn channel_guest_policy_exact_sheet_scope_and_revocation() {
        let turn = turn("guest-a", AnswerAudience::Shared);
        let mut policy = policy(vec![
            sheets_rule(GuestAudience::AllGuests, GuestDataOperation::Read, EVENT),
            sheets_rule(
                GuestAudience::AllGuests,
                GuestDataOperation::Create,
                QUESTIONS,
            ),
        ]);
        assert!(authorize(&policy, &turn, &sheets_request("read", EVENT)).is_ok());
        assert!(authorize(&policy, &turn, &sheets_request("create", QUESTIONS)).is_ok());
        let private = GuestTurn {
            audience: AnswerAudience::Private,
            shared_room_verified: false,
            ..turn.clone()
        };
        assert!(authorize(&policy, &private, &sheets_request("read", EVENT)).is_ok());
        assert!(authorize(&policy, &private, &sheets_request("create", QUESTIONS)).is_err());
        assert!(!has_sheets_grant(
            &policy,
            &private,
            "connection-1",
            "create"
        ));
        for (action, resource) in [
            ("create", EVENT),
            ("read", QUESTIONS),
            ("update", QUESTIONS),
            ("read", "connection-2/workbook-1/Event_Config!A1:H24"),
            ("read", "connection-1/workbook-2/Event_Config!A1:H24"),
            ("read", "connection-1/workbook-1/Event_Config!A1:H25"),
        ] {
            assert!(authorize(&policy, &turn, &sheets_request(action, resource)).is_err());
        }
        policy.revision += 1;
        assert!(authorize(&policy, &turn, &sheets_request("read", EVENT)).is_err());
    }

    #[test]
    fn channel_guest_policy_verified_membership_is_bound_to_answer_room() {
        let a = turn("guest-a", AnswerAudience::Shared);
        let b = turn("guest-b", AnswerAudience::Shared);
        let room_id = a.verified_room_id().unwrap();
        let mut policy = policy(vec![sheets_rule(
            GuestAudience::VerifiedAudience {
                audience_id: "attendees".into(),
                guest_ids: ["guest-a".into()].into(),
                room_id: Some(room_id.clone()),
            },
            GuestDataOperation::Read,
            EVENT,
        )]);
        let request = sheets_request("read", EVENT);
        for candidate in [&a, &b] {
            assert_eq!(
                authorize(&policy, candidate, &request).is_ok(),
                preview(&policy.rules, &subject(candidate, Some(&room_id)), &request).allowed,
            );
        }
        assert!(authorize(&policy, &a, &request).is_ok());
        assert!(authorize(&policy, &b, &request).is_err());
        let other_room = GuestTurn {
            conversation_id: "room-2".into(),
            ..a.clone()
        };
        assert!(authorize(&policy, &other_room, &request).is_err());
        let unverified = GuestTurn {
            actor: GuestActor {
                id: a.actor.id.clone(),
                assurance: GuestAssurance::Unverified,
            },
            ..a.clone()
        };
        assert!(authorize(&policy, &unverified, &request).is_err());

        let GuestAudience::VerifiedAudience { guest_ids, .. } = &mut policy.rules[0].audience
        else {
            panic!("expected named audience");
        };
        *guest_ids = ["guest-b".into()].into();
        policy.revision = 2;
        assert!(authorize(&policy, &a, &request).is_err());
        assert!(
            authorize(
                &policy,
                &GuestTurn {
                    policy_revision: 2,
                    ..a
                },
                &request
            )
            .is_err()
        );
        assert!(
            authorize(
                &policy,
                &GuestTurn {
                    policy_revision: 2,
                    ..b
                },
                &request
            )
            .is_ok()
        );
    }

    #[test]
    fn sheets_write_requires_room_proof_and_legacy_reads_keep_their_scope() {
        let policy = policy(vec![
            sheets_rule(GuestAudience::AllGuests, GuestDataOperation::Read, EVENT),
            sheets_rule(
                GuestAudience::AllGuests,
                GuestDataOperation::Create,
                QUESTIONS,
            ),
        ]);
        let proven = turn("guest-a", AnswerAudience::Shared);
        assert!(authorize(&policy, &proven, &sheets_request("create", QUESTIONS)).is_ok());
        let mut stored = serde_json::to_value(&proven).unwrap();
        stored
            .as_object_mut()
            .unwrap()
            .remove("shared_room_verified");
        let legacy: GuestTurn = serde_json::from_value(stored).unwrap();
        assert!(!legacy.shared_room_verified);
        assert!(authorize(&policy, &legacy, &sheets_request("read", EVENT)).is_ok());
        assert!(authorize(&policy, &legacy, &sheets_request("create", QUESTIONS)).is_err());
        assert!(!has_sheets_grant(
            &policy,
            &legacy,
            "connection-1",
            "create"
        ));
        assert_ne!(
            legacy.hosted_conversation_id(),
            proven.hosted_conversation_id()
        );
        let mut refused = proven.clone();
        refused.audience = AnswerAudience::Private;
        assert!(authorize(&policy, &refused, &sheets_request("create", QUESTIONS)).is_err());
        refused.audience = AnswerAudience::Shared;
        refused.actor.assurance = GuestAssurance::Unverified;
        assert!(authorize(&policy, &refused, &sheets_request("create", QUESTIONS)).is_err());
    }

    #[test]
    fn channel_guest_policy_own_record_fields_and_verified_private_destination() {
        let a = turn("guest-a", AnswerAudience::Private);
        let b = turn("guest-b", AnswerAudience::Private);
        let rule = |write: &str, read: &str| GuestDataRule {
            source_id: personal_records::SOURCE_ID.into(),
            collection_id: personal_records::SOURCE_ID.into(),
            scope: GuestDataScope::OwnRecords,
            delivery: GuestDataDelivery::Private,
            audience: GuestAudience::AllGuests,
            operations: [(
                GuestDataOperation::Update,
                GuestFields {
                    write: [write.into()].into(),
                    read: [read.into()].into(),
                },
            )]
            .into(),
        };
        let mut policy = policy(vec![
            rule("title", "title"),
            rule("title", "body"),
            rule("body", "body"),
        ]);
        let mut request = GuestDataRequest {
            source_id: personal_records::SOURCE_ID,
            collection_id: personal_records::SOURCE_ID,
            action: "update",
            record_id: Some("record-a"),
            record_owner_id: Some("guest-a"),
            fields: &["title"],
        };
        assert!(preview(&policy.rules, &subject(&a, None), &request).allowed);
        assert!(!preview(&policy.rules, &subject(&b, None), &request).allowed);
        let permit = authorize(&policy, &a, &request).unwrap();
        assert_eq!(permit.owner_id(), Some("guest-a"));
        assert_eq!(permit.read_fields(), &["body", "title"]);
        policy.rules.reverse();
        assert_eq!(
            authorize(&policy, &a, &request).unwrap().read_fields(),
            permit.read_fields()
        );
        request.fields = &["title", "body"];
        assert!(authorize(&policy, &a, &request).is_err());
        request.fields = &["title"];
        let group = GuestTurn {
            audience: AnswerAudience::Shared,
            shared_room_verified: true,
            ..a.clone()
        };
        assert!(authorize(&policy, &group, &request).is_err());
        for rule in &mut policy.rules {
            rule.audience = GuestAudience::Guest {
                guest_id: a.actor.id.clone(),
            };
        }
        assert!(authorize(&policy, &a, &request).is_ok());
        let b_owned = GuestDataRequest {
            record_owner_id: Some(&b.actor.id),
            ..request
        };
        assert!(!preview(&policy.rules, &subject(&b, None), &b_owned).allowed);
        policy.rules.clear();
        assert!(authorize(&policy, &b, &b_owned).is_err());
    }

    #[tokio::test]
    async fn channel_guest_policy_delivery_rejects_retargeting_revocation_and_duplicate_rows() {
        use crate::models::{channel_bot, feature_flag_override::FlagTargetKind};
        use crate::services::{
            audit_service::AuditActor, channel_guest_access as runtime,
            feature_flag_admin_service as admin, security_readiness_service,
        };

        let db =
            crate::test_utils::connect_transaction_test_database("guest_policy_delivery").await;
        let actor = AuditActor {
            user_id: uuid::Uuid::new_v4().to_string(),
            api_key_id: None,
            api_key_name: None,
            ip_address: None,
            user_agent: None,
        };
        db.collection::<bson::Document>(channel_bot::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": "bot", "is_active": true, "guest_access_v1": true,
                "guest_access_v1_revision": 1_i64,
            })
            .await
            .unwrap();
        runtime::set_pilot(
            &db,
            runtime::PilotSelection {
                bot_ids: None,
                fleet: true,
            },
            &actor,
        )
        .await
        .unwrap();
        security_readiness_service::ready_test_replica(&db).await;
        admin::mutate(
            &db,
            runtime::FLAG,
            admin::Mutation::Override {
                kind: FlagTargetKind::Global,
                target_id: None,
                enabled: Some(true),
            },
            &actor,
        )
        .await
        .unwrap();

        let runtime::GuestMode::AddOn {
            activation_epoch,
            bot_revision,
        } = runtime::select_mode(&db, "bot").await.unwrap()
        else {
            panic!("opted-in fixture bot must use the add-on");
        };
        let turn = GuestTurn {
            activation_epoch,
            bot_revision,
            ..turn("guest-a", AnswerAudience::Private)
        };
        let policies = db.collection::<GuestDataPolicy>(COLLECTION_NAME);
        let mut current = policy(Vec::new());
        policies.insert_one(&current).await.unwrap();
        assert!(
            revalidate_delivery(
                &db,
                &turn,
                "bot",
                "binding",
                "discord",
                "room-1",
                AnswerAudience::Private
            )
            .await
            .is_ok()
        );
        assert!(
            revalidate_delivery(
                &db,
                &turn,
                "bot",
                "binding",
                "discord",
                "room-2",
                AnswerAudience::Private
            )
            .await
            .is_err()
        );
        assert!(
            revalidate_delivery(
                &db,
                &turn,
                "bot",
                "binding",
                "discord",
                "room-1",
                AnswerAudience::Shared
            )
            .await
            .is_err()
        );
        current.id = uuid::Uuid::new_v4().to_string();
        policies.insert_one(&current).await.unwrap();
        assert!(load_policy(&db, "bot").await.is_err());
        policies
            .delete_one(doc! {"_id": &current.id})
            .await
            .unwrap();
        policies
            .update_one(doc! {"bot_id": "bot"}, doc! {"$inc": {"revision": 1_i64}})
            .await
            .unwrap();
        assert!(
            revalidate_delivery(
                &db,
                &turn,
                "bot",
                "binding",
                "discord",
                "room-1",
                AnswerAudience::Private
            )
            .await
            .is_err()
        );
        db.drop().await.unwrap();
    }
}
