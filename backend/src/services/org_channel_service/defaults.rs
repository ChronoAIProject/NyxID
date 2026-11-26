//! Versioned background defaults. Saved authority is never inferred on a request path.
use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::{Database, IndexModel, options::Hint};

use crate::{
    errors::{AppError, AppResult},
    models::{channel_bot::ChannelBot, nyxbot_channel::COLLECTION_NAME as CHANNELS},
    services::assistant_access_policy::MIGRATIONS_COLLECTION_NAME as MIGRATIONS,
};

const MIGRATION: &str = "issue-2482";
const BATCH: i64 = 100;
const CHAIN_BOUND: i64 = 32;
pub(crate) const RESTORE_EVENT: &str = "org_channel_guest_policy_restored";
pub(crate) const SAVE_EVENT: &str = "org_channel_guest_policy_changed";
pub(super) const SAVE_INDEX: &str = "event_type_1_created_at_-1";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoricalRule {
    PreserveOff,
    NoAuditedSave,
}

// Changing this selection also changes the completion version, revisiting old rows.
const HISTORICAL_RULE: HistoricalRule = HistoricalRule::NoAuditedSave;

impl HistoricalRule {
    fn version(self) -> i32 {
        if self == Self::PreserveOff { 1 } else { 3 }
    }
}

pub(crate) fn switch_guest_access_on(
    rule: HistoricalRule,
    policy: &super::GuestPolicy,
    choice: ChoiceEvidence,
) -> bool {
    match rule {
        HistoricalRule::PreserveOff => false,
        HistoricalRule::NoAuditedSave => !policy.enabled && choice == ChoiceEvidence::Untouched,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChoiceEvidence {
    Chosen,
    Untouched,
    Unknown,
}

// All history reads use the row transaction. Absence of old asynchronous audit
// evidence is the owner's accepted limit; an unavailable lookup is never absence.
async fn binding_chain_choice(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    row: &super::NyxbotChannel,
) -> AppResult<ChoiceEvidence> {
    let binding = row.org_specialist.as_ref().ok_or_else(super::denied)?;
    let mut cursor = db
        .collection::<Document>(CHANNELS)
        .find(doc! {
            "channel_bot_id": &row.channel_bot_id, "org_specialist.org_id": &binding.org_id
        })
        .sort(doc! {"created_at":-1,"_id":-1})
        .projection(
            doc! {"_id":1,"org_specialist.version":1,"org_specialist.guests.revision":1,
            "org_specialist.guests.enabled":1,"org_specialist.guests.continuity_channel_id":1},
        )
        .limit(CHAIN_BOUND + 1)
        .max_time(std::time::Duration::from_secs(5))
        .session(&mut *tx)
        .await?;
    let mut rows = Vec::new();
    while let Some(raw) = cursor.next(&mut *tx).await.transpose()? {
        rows.push(raw);
    }
    if rows.len() > CHAIN_BOUND as usize {
        return Ok(ChoiceEvidence::Unknown);
    }
    let mut policies = Vec::new();
    for raw in &rows {
        let Ok(id) = raw.get_str("_id") else {
            return Ok(ChoiceEvidence::Unknown);
        };
        if raw
            .get_document("org_specialist")
            .ok()
            .and_then(|b| b.get_i32("version").ok())
            != Some(1)
        {
            return Ok(ChoiceEvidence::Unknown);
        }
        let Some(policy) = raw
            .get_document("org_specialist")
            .ok()
            .and_then(|b| b.get_document("guests").ok())
        else {
            return Ok(ChoiceEvidence::Unknown);
        };
        let Ok(revision) = policy.get_str("revision") else {
            return Ok(ChoiceEvidence::Unknown);
        };
        if !revision.is_empty() && uuid::Uuid::parse_str(revision).is_err() {
            return Ok(ChoiceEvidence::Unknown);
        }
        let continuity = match policy.get("continuity_channel_id") {
            None | Some(Bson::Null) => None,
            Some(Bson::String(id)) if !id.is_empty() => Some(id.as_str()),
            _ => return Ok(ChoiceEvidence::Unknown),
        };
        let Ok(enabled) = policy.get_bool("enabled") else {
            return Ok(ChoiceEvidence::Unknown);
        };
        policies.push((id, revision, continuity, enabled));
    }
    let ids: Vec<_> = policies.iter().map(|(id, _, _, _)| *id).collect();
    if !ids.contains(&row.id.as_str())
        || policies
            .iter()
            .any(|(_, _, continuity, _)| continuity.is_some_and(|id| !ids.contains(&id)))
    {
        return Ok(ChoiceEvidence::Unknown);
    }
    if policies
        .iter()
        .any(|(_, revision, continuity, _)| continuity.is_none() && !revision.is_empty())
    {
        return Ok(ChoiceEvidence::Chosen);
    }
    if db
        .collection::<Document>("audit_log")
        .find_one(doc! {
            "event_type":SAVE_EVENT,"event_data.channel_id":{"$in":&ids}
        })
        .hint(save_hint(db, &row.id))
        .projection(doc! {"_id":1})
        .max_time(std::time::Duration::from_secs(5))
        .session(&mut *tx)
        .await?
        .is_some()
    {
        return Ok(ChoiceEvidence::Chosen);
    }
    // Older restoration audits omit the generated revision. Timestamps and the
    // unchanged enabled bit cannot identify it, so those rows rely on save evidence alone.
    for (id, revision, continuity, enabled) in policies {
        if continuity.is_none() {
            continue;
        }
        let restored = db
            .collection::<Document>("audit_log")
            .find_one(doc! {
                "event_type":RESTORE_EVENT,"event_data.channel_id":id
            })
            .hint(save_hint(db, &row.id))
            .sort(doc! {"created_at":-1,"_id":-1})
            .projection(doc! {"event_data.guest_revision":1,"event_data.enabled":1})
            .max_time(std::time::Duration::from_secs(5))
            .session(&mut *tx)
            .await?;
        let data = restored
            .as_ref()
            .and_then(|a| a.get_document("event_data").ok());
        if data
            .and_then(|d| d.get_bool("enabled").ok())
            .is_some_and(|saved| saved != enabled)
        {
            return Ok(ChoiceEvidence::Chosen);
        }
        if let Some(restored_revision) = data.and_then(|data| data.get_str("guest_revision").ok())
            && uuid::Uuid::parse_str(restored_revision).is_ok()
            && restored_revision != revision
        {
            return Ok(ChoiceEvidence::Chosen);
        }
    }
    Ok(ChoiceEvidence::Untouched)
}

pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    for keys in [
        doc! {"org_specialist.version":1,"status":1,"org_guest_defaults_version":1,"_id":1},
        doc! {"org_guest_defaults_notice_pending":1,"_id":1},
    ] {
        db.collection::<Document>(CHANNELS)
            .create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    db.collection::<Document>(MIGRATIONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"migration":1,"org_id":1,"channel_bot_id":1})
                .build(),
        )
        .await?;
    db.collection::<Document>(MIGRATIONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"migration":1,"channel_id":1})
                .build(),
        )
        .await?;
    Ok(())
}

fn completion_id(id: &str, rule: HistoricalRule) -> String {
    format!("{MIGRATION}:{}:{id}", rule.version())
}

fn exact(filter: &mut Document, snapshot: &Document, field: &str) {
    filter.insert(
        field,
        match snapshot.get(field) {
            Some(value) => Bson::Document(doc! {"$eq":value,"$exists":true}),
            None => Bson::Document(doc! {"$exists":false}),
        },
    );
}

pub(super) fn pending_filter(rule: HistoricalRule) -> Document {
    doc! {"_id":{"$type":"string"},"$or":[
        {"org_specialist.version":1,"status":"active","org_guest_defaults_version":null},
        {"org_specialist.version":1,"status":"active","org_guest_defaults_version":{"$lt":rule.version()}},
        {"org_guest_defaults_notice_pending":true}
    ]}
}

#[derive(Default)]
pub(crate) struct Sweep {
    after: Option<String>,
    last_failure_log: Option<std::time::Instant>,
}

impl Sweep {
    pub(crate) async fn run(&mut self, db: &Database) -> AppResult<()> {
        self.run_with_rule(db, HISTORICAL_RULE).await
    }

    pub(super) async fn run_with_rule(
        &mut self,
        db: &Database,
        rule: HistoricalRule,
    ) -> AppResult<()> {
        // Progress is only a scan cursor; transactional completion remains authoritative.
        // Wrap on the next tick, including retries for failed rows behind the cursor.
        let mut filter = pending_filter(rule);
        if let Some(after) = &self.after {
            filter.insert("_id", doc! {"$gt":after});
        }
        let rows: Vec<Document> = db
            .collection::<Document>(CHANNELS)
            .find(filter)
            .sort(doc! {"_id":1})
            .projection(doc! {"_id":1})
            .limit(BATCH)
            .max_time(std::time::Duration::from_secs(5))
            .await?
            .try_collect()
            .await?;
        let mut failures = 0;
        let mut first_failure = None;
        for row in &rows {
            let Some(_work) = crate::services::shutdown::process().background() else {
                break;
            };
            let Ok(id) = row.get_str("_id") else { continue };
            self.after = Some(id.to_owned());
            let stage = if apply(db, id, rule).await.is_err() {
                Some("apply")
            } else if notify(db, id).await.is_err() {
                Some("notice")
            } else {
                None
            };
            if let Some(stage) = stage {
                failures += 1;
                first_failure.get_or_insert((id, stage));
            }
        }
        if rows.len() < BATCH as usize {
            self.after = None;
        }
        if let Some((id, stage)) = first_failure
            && self
                .last_failure_log
                .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(60))
        {
            tracing::warn!(
                channel_id = id,
                stage,
                failed_rows = failures,
                "Organization guest defaults will retry failed rows"
            );
            self.last_failure_log = Some(std::time::Instant::now());
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) async fn sweep(db: &Database) -> AppResult<()> {
    Sweep::default().run(db).await
}

#[cfg(test)]
pub(crate) async fn sweep_with_rule(db: &Database, rule: HistoricalRule) -> AppResult<()> {
    Sweep::default().run_with_rule(db, rule).await
}

#[cfg(test)]
pub(super) static FAIL_AUDIT_HINT: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashSet<(String, String)>>,
> = std::sync::LazyLock::new(Default::default);

fn save_hint(_db: &Database, _id: &str) -> Hint {
    #[cfg(test)]
    if FAIL_AUDIT_HINT
        .lock()
        .unwrap()
        .contains(&(_db.name().to_owned(), _id.to_owned()))
    {
        return Hint::Name("issue_2482_missing_test_index".into());
    }
    Hint::Name(SAVE_INDEX.into())
}

pub(crate) async fn apply(db: &Database, id: &str, rule: HistoricalRule) -> AppResult<()> {
    let context = (db.clone(), id.to_owned(), rule);
    crate::services::transaction_service::run_db_only(db, &context, |tx, (db, id, rule)| Box::pin(async move {
        let channels = db.collection::<Document>(CHANNELS);
        let Some(raw) = channels.find_one(doc! {"_id":id,"org_specialist.version":1})
            .session(&mut *tx).await? else { return Ok(()) };
        if raw.get_str("status").ok() != Some("active") {
            return Ok(());
        }
        if raw.get_i32("org_guest_defaults_version").is_ok_and(|version| version >= rule.version()) {
            return Ok(());
        }
        let marker = completion_id(id, *rule);
        if let Some(completed) = db.collection::<Document>(MIGRATIONS).find_one(doc! {"_id":&marker})
            .session(&mut *tx).await? {
            let mut fields = doc! {"org_guest_defaults_version":rule.version()};
            if completed.get_bool("guest_access_switched_on").unwrap_or(false)
                && !completed.get_bool("notice_claimed").unwrap_or(false) {
                fields.insert("org_guest_defaults_notice_pending", true);
            }
            channels.update_one(doc! {"_id":id},doc! {"$set":fields})
                .session(&mut *tx).await?;
            return Ok(());
        }
        let row = bson::from_document::<super::NyxbotChannel>(raw.clone()).ok();
        let safe = row.as_ref().filter(|row| stored_choices_are_known(&raw, row));
        let Some(row) = safe else {
            channels.update_one(doc! {"_id":id},doc! {"$set":{"org_guest_defaults_version":rule.version()}}).session(&mut *tx).await?;
            return Ok(());
        };
        // Authority can recover without retiring this binding (reactivation or
        // renewed admin membership). Leave it pending until that happens.
        if !live_authority(db, tx, row).await? { return Ok(()); }
        let binding = row.org_specialist.as_ref().ok_or_else(super::denied)?;
        let policy = &binding.guests;
        let choice = binding_chain_choice(db, tx, row).await?;
        let enable = switch_guest_access_on(*rule, policy, choice);
        let private = choice != ChoiceEvidence::Unknown && row.private_chats.is_none();
        let mut fields = doc! {"org_guest_defaults_version":rule.version()};
        let guest_revision = enable.then(|| uuid::Uuid::new_v4().to_string());
        if private { fields.insert("private_chats", "everyone"); }
        if enable {
            fields.insert("org_specialist.guests.enabled", true);
            fields.insert("org_specialist.guests.revision", guest_revision.as_deref());
            fields.insert("org_guest_defaults_notice_pending", true);
        }
        let mut filter = doc! {"_id":id};
        for field in ["org_specialist","private_chats","execution_policy_revision","org_guest_defaults_version","status","user_id","bot_owner_id","agent_id","channel_bot_id"] {
            exact(&mut filter, &raw, field);
        }
        let changed = private || enable;
        let mut update = doc! {"$set":fields};
        if changed {
            update.insert("$inc", doc! {"execution_policy_revision":1_i64});
            update.get_document_mut("$set").expect("set").insert("updated_at", bson::DateTime::now());
        }
        if channels.update_one(filter,update).session(&mut *tx).await?.matched_count != 1 {
            return Err(AppError::Conflict("Channel settings changed during default migration".into()));
        }
        db.collection::<Document>(MIGRATIONS).insert_one(doc! {
            "_id":marker,"migration":MIGRATION,"version":rule.version(),"channel_id":id,
            "org_id":&binding.org_id,"channel_bot_id":&row.channel_bot_id,
            "binding_revision":&binding.revision,"previous_guest_revision":&policy.revision,"guest_revision":guest_revision,"guest_access_switched_on":enable,"private_chats_opened":private,
            "completed_at":bson::DateTime::now()
        }).session(&mut *tx).await?;
        if changed {
            crate::services::nyxbot_gateway_release_service::audit(db,tx,"system",
                "org_channel_guest_defaults_applied",serde_json::json!({
                    "version":rule.version(),"channel_id":id,"org_id":binding.org_id,
                    "guest_enabled":policy.enabled || enable,"guest_access_switched_on":enable,"private_chats_opened":private
                })).await?;
        }
        Ok(())
    })).await
}

fn stored_choices_are_known(raw: &Document, row: &super::NyxbotChannel) -> bool {
    let Some(binding) = &row.org_specialist else {
        return false;
    };
    row.status == "active"
        && row.bot_owner_id.as_deref() == Some(binding.org_id.as_str())
        && !binding.revision.is_empty()
        && (binding.guests.revision.is_empty()
            || uuid::Uuid::parse_str(&binding.guests.revision).is_ok())
        && matches!(
            row.private_chats.as_deref(),
            None | Some("owner" | "everyone")
        )
        && raw
            .get_document("org_specialist")
            .ok()
            .and_then(|binding| binding.get_document("guests").ok())
            .is_some_and(|policy| {
                policy.get_str("revision").is_ok() && policy.get_bool("enabled").is_ok()
            })
}

async fn live_authority(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    row: &super::NyxbotChannel,
) -> AppResult<bool> {
    let binding = row.org_specialist.as_ref().ok_or_else(super::denied)?;
    for (collection, filter) in [
        (
            "users",
            doc! {"_id":&binding.org_id,"user_type":"org","is_active":true},
        ),
        ("users", doc! {"_id":&row.user_id,"is_active":true}),
        (
            "org_memberships",
            doc! {"org_user_id":&binding.org_id,"member_user_id":&row.user_id,"role":"admin","revoked_at":null},
        ),
        (
            "channel_bots",
            doc! {"_id":&row.channel_bot_id,"user_id":&binding.org_id,"is_active":true},
        ),
        (
            "assistant_agents",
            doc! {"_id":&row.agent_id,"user_id":&binding.org_id,"kind":"specialist","destroyed_at":null},
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one_and_update(filter, doc! {"$inc":{"org_channel_fence":1_i64}})
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?
            .is_none()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

// A stale explicit Off may supersede only this migration's exact generated revision.
// A later settings save has a different revision and keeps the normal conflict gate.
pub(crate) async fn enabled_revision(
    db: &Database,
    row: &super::NyxbotChannel,
    expected_revision: &str,
    tx: Option<&mut mongodb::ClientSession>,
) -> AppResult<Option<String>> {
    let binding = row.org_specialist.as_ref().ok_or_else(super::denied)?;
    let collection = db.collection::<Document>(MIGRATIONS);
    let mut filter = doc! {"migration":MIGRATION,"channel_id":&row.id,"binding_revision":&binding.revision,"guest_access_switched_on":true};
    filter.insert(
        "previous_guest_revision",
        if expected_revision.is_empty() {
            Bson::Document(doc! {"$in":["", Bson::Null]})
        } else {
            Bson::String(expected_revision.into())
        },
    );
    let mut query = collection.find_one(filter);
    if let Some(tx) = tx {
        query = query.session(tx);
    }
    Ok(query
        .await?
        .and_then(|row| row.get_str("guest_revision").ok().map(str::to_owned)))
}

async fn notify(db: &Database, id: &str) -> AppResult<()> {
    let channels = db.collection::<super::NyxbotChannel>(CHANNELS);
    let Some(row) = channels
        .find_one(doc! {"_id":id,"org_guest_defaults_notice_pending":true})
        .await?
    else {
        return Ok(());
    };
    let bot = db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":&row.channel_bot_id,"user_id":&row.bot_owner_id,"is_active":true})
        .await?;
    let description = if bot.is_some() {
        Some(guest_notice_text(db, &row).await?)
    } else {
        None
    };
    // Claim durably with the channel hint: old replacements cannot duplicate a notice.
    let context = (db.clone(), id.to_owned());
    let claimed = crate::services::transaction_service::run_db_only(db, &context, |tx, (db, id)| Box::pin(async move {
        let cleared = db.collection::<Document>(CHANNELS).update_one(
            doc! {"_id":id,"org_guest_defaults_notice_pending":true},
            doc! {"$unset":{"org_guest_defaults_notice_pending":""}}).session(&mut *tx).await?;
        if cleared.modified_count == 0 { return Ok(false); }
        Ok(db.collection::<Document>(MIGRATIONS).find_one_and_update(
            doc! {"migration":MIGRATION,"channel_id":id,"guest_access_switched_on":true,"notice_claimed":{"$ne":true}},
            doc! {"$set":{"notice_claimed":true}}).session(&mut *tx).await?.is_some())
    })).await?;
    if claimed && let Some(bot) = bot {
        crate::services::channel_bot_health::notify_guest_defaults(
            db,
            &bot,
            description.as_deref().expect("live bot notice"),
        );
    }
    Ok(())
}

async fn guest_notice_text(db: &Database, row: &super::NyxbotChannel) -> AppResult<String> {
    let binding = row.org_specialist.as_ref().ok_or_else(super::denied)?;
    let agent = db.collection::<crate::models::assistant_agent::AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":&row.agent_id,"user_id":&binding.org_id,"kind":"specialist","destroyed_at":null}).await?;
    let ready =
        crate::services::security_readiness_service::report(db, super::GUEST_GRANTS_READINESS)
            .await?
            .serving_ready;
    let services = match agent {
        Some(agent) => super::management::guest_services(db, &agent)
            .await?
            .into_iter()
            .filter(|service| {
                service.access != "none"
                    && super::guest_service_selected(&agent, &binding.guests, ready, &service.id)
            })
            .map(|service| {
                format!(
                    "{}: {}{}",
                    service.label,
                    service.access,
                    if service.limited_operations {
                        " (selected operations only)"
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };
    let access = if services.is_empty() {
        "model-only chat".into()
    } else {
        format!("model chat and these services: {}", services.join(", "))
    };
    let billing =
        crate::services::channel_bot_health::org_guest_billing(db, &binding.org_id).await?;
    Ok(format!(
        "Guest access is now on by default for this bot. Guests' usage is charged to the organization. Guests can use {access}. To turn it off, open Who can talk to this bot and turn Allow guests off.{}",
        billing
            .map(|reason| format!(
                " {}",
                crate::services::channel_bot_health::guest_billing_advice(reason).text
            ))
            .unwrap_or_default()
    ))
}

pub(crate) async fn erase(db: &Database, filter: Document) -> AppResult<()> {
    let mut filter = filter;
    filter.insert("migration", MIGRATION);
    db.collection::<Document>(MIGRATIONS)
        .delete_many(filter)
        .await?;
    Ok(())
}

pub(crate) async fn erase_in_session(
    db: &Database,
    mut filter: Document,
    tx: &mut mongodb::ClientSession,
) -> AppResult<()> {
    filter.insert("migration", MIGRATION);
    db.collection::<Document>(MIGRATIONS)
        .delete_many(filter)
        .session(tx)
        .await?;
    Ok(())
}
