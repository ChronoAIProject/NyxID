//! Prepare dependent bot fingerprints before rewrapping a shared signing producer.
use super::channel_managed::PlatformCredentialBacking;
use crate::errors::{AppError, AppResult};
use crate::models::{channel_bot, platform_credential, platform_settings, provider_config};
use bson::{Bson, Document, doc};
use mongodb::{ClientSession, Database};
use std::sync::Arc;

use super::platform_entry_credentials::EntryCredentialSnapshot;
use crate::models::channel_bot::ChannelBot;
use futures::TryStreamExt;
use hmac::Mac;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) const SIGNING_PREPARATION_FIELD: &str = "channel_signing_preparation";
const PAGE: i64 = 8;
pub(super) type Generations = BTreeMap<String, String>;

fn blocked() -> AppError {
    AppError::Conflict("Channel signing credential requires fingerprint preparation".into())
}

#[derive(Serialize, Deserialize)]
struct Preparation {
    version: u8,
    generations: Generations,
    after: Option<String>,
    complete: bool,
    tag: String,
}

fn physical(collection: &str, row: &Document) -> AppResult<Generations> {
    let mut fields = Generations::new();
    if collection == provider_config::COLLECTION_NAME {
        for name in ["client_id", "client_secret"] {
            if let Some(value) = row.get(format!("{name}_encrypted"))
                && let Some(bytes) = super::credential_record_service::stored_bytes(value)?
            {
                fields.insert(name.into(), hex::encode(Sha256::digest(bytes)));
            }
        }
    } else if let Ok(secrets) = row.get_document("secrets") {
        for (name, value) in secrets {
            let Bson::Binary(value) = value else {
                return Err(blocked());
            };
            let mut digest = Sha256::new();
            digest.update([u8::from(value.subtype)]);
            digest.update(&value.bytes);
            fields.insert(name.clone(), hex::encode(digest.finalize()));
        }
    }
    Ok(fields)
}

impl Preparation {
    fn initial(coll: &str, row: &Document) -> AppResult<Self> {
        Ok(Self {
            version: 1,
            generations: physical(coll, row)?,
            after: None,
            complete: false,
            tag: String::new(),
        })
    }

    fn mac(&self, coll: &str, row: &Document) -> AppResult<hmac::Hmac<Sha256>> {
        if self.version != 1 {
            return Err(blocked());
        }
        let scope = if coll == provider_config::COLLECTION_NAME {
            "slug"
        } else {
            "provider"
        };
        let material = serde_json::to_vec(&(
            self.version,
            coll,
            row.get_str("_id").map_err(|_| blocked())?,
            row.get_str(scope).map_err(|_| blocked())?,
            physical(coll, row)?,
            row.get("secret_context_versions"),
            &self.generations,
            &self.after,
            self.complete,
        ))
        .map_err(|_| blocked())?;
        super::channel_credential_continuity::continuity_mac(
            b"nyxid-channel-shared-signing:v1",
            &material,
        )
    }

    fn read(coll: &str, row: &Document) -> AppResult<Option<Self>> {
        let Some(value) = row.get(SIGNING_PREPARATION_FIELD) else {
            return Ok(None);
        };
        let state: Self = bson::from_bson(value.clone()).map_err(|_| blocked())?;
        state
            .mac(coll, row)?
            .verify_slice(&hex::decode(&state.tag).map_err(|_| blocked())?)
            .map_err(|_| blocked())?;
        Ok(Some(state))
    }

    fn seal(mut self, coll: &str, row: &Document) -> AppResult<Bson> {
        self.tag = hex::encode(self.mac(coll, row)?.finalize().into_bytes());
        bson::to_bson(&self).map_err(|_| blocked())
    }
}

pub(super) fn verify_state(coll: &str, row: &Document) -> AppResult<()> {
    if Preparation::read(coll, row)?.is_some_and(|state| !state.complete) {
        return Err(blocked());
    }
    Ok(())
}

pub(super) fn generations(coll: &str, row: &Document) -> AppResult<Generations> {
    match Preparation::read(coll, row)? {
        Some(state) => Ok(state.generations),
        None => physical(coll, row),
    }
}

/// Preserve every non-storage admission predicate, including updated_at.
pub(super) async fn fence_projection(
    db: &Database,
    tx: &mut ClientSession,
    coll: &str,
    mut filter: Document,
    expected: &Generations,
    revision: Option<&str>,
) -> AppResult<bool> {
    for field in ["client_id_encrypted", "client_secret_encrypted", "secrets"] {
        filter.remove(field);
    }
    let rows = db.collection::<Document>(coll);
    let Some(current) = rows.find_one(filter).session(&mut *tx).await? else {
        return Ok(false);
    };
    if generations(coll, &current)? != *expected {
        return Ok(false);
    }
    if let Some(revision) = revision {
        return Ok(rows
            .update_one(
                super::record_storage::exact_document_predicate(&current),
                doc! {"$inc":{revision:1_i64}},
            )
            .session(tx)
            .await?
            .matched_count
            == 1);
    }
    Ok(true)
}

/// A mutation of a prepared producer authenticates its successor in the same transaction.
/// Absent fields keep their replacement generation so clear/re-add cannot revive an old pin.
fn replacement(coll: &str, source: &Document, target: &Document) -> AppResult<Option<Bson>> {
    let Some(mut state) = Preparation::read(coll, source)? else {
        return Ok(None);
    };
    let before = physical(coll, source)?;
    let after = physical(coll, target)?;
    for name in before
        .keys()
        .chain(after.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        if before.get(name) != after.get(name) {
            state
                .generations
                .insert(name.clone(), uuid::Uuid::new_v4().to_string());
            state.after = None;
        }
    }
    state.seal(coll, target).map(Some)
}

// A composite can return to an absent auxiliary map after a real remove.
// The provider retains the membership generation across that transition.
pub(super) async fn auxiliary_membership_changed(
    db: &Database,
    tx: &mut ClientSession,
    provider: &Document,
) -> AppResult<()> {
    let coll = provider_config::COLLECTION_NAME;
    let mut state = Preparation::read(coll, provider)?.ok_or_else(blocked)?;
    state.generations.insert(
        "auxiliary_membership".into(),
        uuid::Uuid::new_v4().to_string(),
    );
    state.after = None;
    db.collection::<Document>(coll)
        .update_one(
            doc! {"_id":provider.get("_id")},
            doc! {"$set":{SIGNING_PREPARATION_FIELD:state.seal(coll,provider)?}},
        )
        .session(tx)
        .await?;
    Ok(())
}

/// Use the caller's transaction for both the business mutation and the new evidence.
pub(super) async fn update_in_session(
    db: &Database,
    session: &mut ClientSession,
    coll: &str,
    filter: Document,
    update: impl Into<mongodb::options::UpdateModifications>,
    upsert: bool,
) -> AppResult<Option<Document>> {
    let rows = db.collection::<Document>(coll);
    let mut prepared = filter.clone();
    prepared.insert(SIGNING_PREPARATION_FIELD, doc! {"$exists":true});
    let source = rows.find_one(prepared).session(&mut *session).await?;
    let Some(mut target) = rows
        .find_one_and_update(filter, update)
        .upsert(upsert)
        .return_document(mongodb::options::ReturnDocument::After)
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    if let Some(source) = source
        && let Some(state) = replacement(coll, &source, &target)?
    {
        rows.update_one(
            doc! {"_id":target.get("_id")},
            doc! {"$set":{SIGNING_PREPARATION_FIELD:&state}},
        )
        .session(session)
        .await?;
        target.insert(SIGNING_PREPARATION_FIELD, state);
    }
    Ok(Some(target))
}

/// Untouched providers retain their single-document update. Enrollment between
/// observation and publication routes the business write through the prepared path.
pub(super) async fn update_provider(
    db: &Database,
    filter: Document,
    update: Document,
) -> AppResult<Option<Document>> {
    let coll = provider_config::COLLECTION_NAME;
    let mut ordinary = filter.clone();
    ordinary.insert(SIGNING_PREPARATION_FIELD, doc! {"$exists":false});
    let row = db
        .collection::<Document>(coll)
        .find_one_and_update(ordinary, update.clone())
        .return_document(mongodb::options::ReturnDocument::After)
        .await?;
    if row.is_some() {
        return Ok(row);
    }
    super::transaction_service::run_db_only(
        db,
        &(db, filter, update),
        |tx, (db, filter, update)| {
            Box::pin(async move {
                update_in_session(db, tx, coll, filter.clone(), update.clone(), false).await
            })
        },
    )
    .await
}

fn consumers(collection: &str, source: &Document) -> Vec<Document> {
    if collection != provider_config::COLLECTION_NAME
        && collection != platform_credential::COLLECTION_NAME
    {
        return vec![];
    }
    let cache = Arc::new(super::provider_token_exchange_service::TokenExchangeCache::new());
    let mut consumers = Vec::new();
    for adapter in super::channel_adapters::registered_adapters(&cache) {
        let Some(descriptor) = adapter.platform_credentials() else {
            continue;
        };
        let contributes = if collection == provider_config::COLLECTION_NAME {
            matches!(descriptor.backing, PlatformCredentialBacking::ProviderOAuth { provider_slug }
                if source.get_str("slug") == Ok(provider_slug))
                && (source.contains_key(SIGNING_PREPARATION_FIELD)
                    || ["client_id_encrypted", "client_secret_encrypted"]
                        .iter()
                        .any(|field| source.get(*field).is_some_and(|value| value != &Bson::Null)))
        } else if source.get_str("provider") == Ok(descriptor.provider) {
            source.contains_key(SIGNING_PREPARATION_FIELD)
                || source
                    .get_document("secrets")
                    .is_ok_and(|secrets| match descriptor.backing {
                        PlatformCredentialBacking::Stored => !secrets.is_empty(),
                        PlatformCredentialBacking::ProviderOAuth { .. } => {
                            descriptor.fields.iter().any(|field| {
                                !matches!(field.name, "client_id" | "client_secret")
                                    && secrets.contains_key(field.name)
                            })
                        }
                    })
        } else {
            false
        };
        if contributes {
            let mut sources = vec!["platform"];
            if adapter.platform_webhook()
                && matches!(
                    adapter.credential_resolution(),
                    super::channel_platform::CredentialResolution::OAuthConnection { .. }
                )
            {
                sources.push("connection");
            }
            consumers
                .push(doc! {"platform":adapter.platform_id(),"credential_source":{"$in":sources}});
        }
    }
    consumers
}

pub(super) async fn registration_fence(
    db: &Database,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(platform_settings::COLLECTION_NAME)
        .update_one(
            doc! {"_id":platform_settings::PLATFORM_SETTINGS_ID},
            doc! {"$inc":{"channel_bot_registration_revision":1_i64}},
        )
        .upsert(true)
        .session(&mut *session)
        .await?;
    Ok(())
}

fn adapter(bot: &ChannelBot) -> AppResult<Box<dyn super::channel_platform::PlatformAdapter>> {
    let cache = Arc::new(super::provider_token_exchange_service::TokenExchangeCache::new());
    super::channel_adapters::resolve_adapter(&bot.platform, &cache)
}

async fn capture(
    db: &Database,
    tx: &mut ClientSession,
    bot: &ChannelBot,
    enrolling: Option<(&str, &str)>,
) -> AppResult<crate::models::channel_bot::CredentialContinuity> {
    let adapter = adapter(bot)?;
    let entry = EntryCredentialSnapshot::load(db, adapter.as_ref(), bot).await?;
    let source = entry.signing_producer().ok_or_else(blocked)?;
    if enrolling.is_some_and(|(coll, id)| source.source_id(coll) != Some(id))
        || !entry.fence_sources_for_rewrap(db, tx).await?
    {
        return Err(blocked());
    }
    // The first prepared aggregate also needs its other producer to track real
    // replacements before that producer's independent migration walk reaches it.
    let mut proofs = BTreeMap::new();
    for coll in [
        provider_config::COLLECTION_NAME,
        platform_credential::COLLECTION_NAME,
    ] {
        if let Some(id) = source.source_id(coll) {
            let rows = db.collection::<Document>(coll);
            let current = rows
                .find_one(doc! {"_id":id})
                .session(&mut *tx)
                .await?
                .ok_or_else(blocked)?;
            if Preparation::read(coll, &current)?.is_none() {
                let state = Preparation::initial(coll, &current)?.seal(coll, &current)?;
                rows.update_one(
                    doc! {"_id":id},
                    doc! {"$set":{SIGNING_PREPARATION_FIELD:state}},
                )
                .session(&mut *tx)
                .await?;
            }
            proofs.insert(coll.to_owned(), id.to_owned());
        }
    }
    super::channel_credential_continuity::capture(&entry, Some(proofs))
}

/// Registration already holds the registration fence. An attachment behind a
/// durable cursor publishes its pins before its row becomes visible.
pub(super) async fn capture_attachment(
    db: &Database,
    tx: &mut ClientSession,
    bot: &ChannelBot,
) -> AppResult<Option<crate::models::channel_bot::CredentialContinuity>> {
    if !matches!(bot.credential_source.as_str(), "platform" | "connection") {
        return Ok(None);
    }
    let cache = Arc::new(super::provider_token_exchange_service::TokenExchangeCache::new());
    let Some(adapter) = super::channel_adapters::registered_adapters(&cache)
        .into_iter()
        .find(|a| a.platform_id() == bot.platform)
    else {
        return Ok(None);
    };
    let Some(desc) = adapter.platform_credentials() else {
        return Ok(None);
    };
    if bot.credential_source == "connection" && !adapter.platform_webhook() {
        return Ok(None);
    }
    let mut active = BTreeMap::new();
    let mut selectors = vec![(
        platform_credential::COLLECTION_NAME,
        doc! {"provider":desc.provider,SIGNING_PREPARATION_FIELD:{"$exists":true}},
    )];
    if let PlatformCredentialBacking::ProviderOAuth { provider_slug } = desc.backing {
        selectors.push((
            provider_config::COLLECTION_NAME,
            doc! {"slug":provider_slug,SIGNING_PREPARATION_FIELD:{"$exists":true}},
        ));
    }
    for (coll, filter) in selectors {
        if let Some(row) = db
            .collection::<Document>(coll)
            .find_one_and_update(filter, doc! {"$inc":{"channel_observation_version":1_i64}})
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?
        {
            active.insert(
                coll.to_owned(),
                row.get_str("_id").map_err(|_| blocked())?.to_owned(),
            );
        }
    }
    let previous = super::channel_credential_continuity::verified(bot)?;
    if let Some(previous) = previous {
        for (coll, id) in &previous.shared_producers {
            if !active.contains_key(coll)
                && db
                    .collection::<Document>(coll)
                    .find_one(doc! {"_id":id,SIGNING_PREPARATION_FIELD:{"$exists":false}})
                    .projection(doc! {"_id":1})
                    .session(&mut *tx)
                    .await?
                    .is_some()
            {
                return Err(blocked());
            }
        }
    }
    if !active.is_empty() && !previous.is_some_and(|e| e.shared_producers == active) {
        let captured = capture(db, tx, bot, None).await?;
        if !active
            .iter()
            .all(|(coll, id)| captured.shared_producers.get(coll) == Some(id))
        {
            return Err(blocked());
        }
        return Ok(Some(captured));
    }
    Ok(None)
}

/// Join the publication transaction after it won its original bot fence.
pub(super) async fn prepare_observed(
    db: &Database,
    tx: &mut ClientSession,
    id: &str,
) -> AppResult<()> {
    let rows = db.collection::<ChannelBot>(channel_bot::COLLECTION_NAME);
    let bot = rows
        .find_one(doc! {"_id":id})
        .session(&mut *tx)
        .await?
        .ok_or_else(blocked)?;
    if let Some(evidence) = capture_attachment(db, tx, &bot).await? {
        let value = bson::to_bson(&evidence).map_err(|_| blocked())?;
        rows.update_one(
            doc! {"_id":id},
            doc! {"$set":{"credential_continuity":value}},
        )
        .session(tx)
        .await?;
    }
    Ok(())
}

/// Returns true after committing a page. The walker yields its lease with
/// the source row still pending; a later call reads the new producer snapshot.
pub(super) async fn prepare_page(db: &Database, coll: &str, source: &Document) -> AppResult<bool> {
    let consumers = consumers(coll, source);
    if consumers.is_empty() {
        return Ok(false);
    }
    let existing = Preparation::read(coll, source)?;
    if existing.as_ref().is_some_and(|s| s.complete) {
        return Ok(false);
    }
    let mut state = match existing {
        Some(state) => state,
        None => Preparation::initial(coll, source)?,
    };
    let db = super::transaction_service::durable_database(db);
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await?;
    let result = async {
        registration_fence(&db, &mut tx).await?;
        let rows = db.collection::<Document>(coll);
        if rows
            .find_one(super::record_storage::exact_document_predicate(source))
            .session(&mut tx)
            .await?
            .is_none()
        {
            return Ok(true);
        }
        let bots = db.collection::<Document>(channel_bot::COLLECTION_NAME);
        let mut query = doc! {"$or":consumers};
        if let Some(after) = &state.after {
            query.insert("_id", doc! {"$gt":after});
        }
        let page = bots
            .find(query)
            .sort(doc! {"_id":1})
            .limit(PAGE + 1)
            .session(&mut tx)
            .await?
            .stream(&mut tx)
            .try_collect::<Vec<_>>()
            .await?;
        if page.is_empty() && !source.contains_key(SIGNING_PREPARATION_FIELD) {
            return Ok(false);
        }
        for row in page.iter().take(PAGE as usize) {
            let bot: ChannelBot = bson::from_document(row.clone()).map_err(|_| blocked())?;
            let pins = bson::to_bson(
                &capture(
                    &db,
                    &mut tx,
                    &bot,
                    Some((coll, source.get_str("_id").map_err(|_| blocked())?)),
                )
                .await?,
            )
            .map_err(|_| blocked())?;
            if bots
                .update_one(
                    super::record_storage::exact_document_predicate(row),
                    doc! {"$set":{"credential_continuity":pins}},
                )
                .session(&mut tx)
                .await?
                .matched_count
                != 1
            {
                return Err(blocked());
            }
            state.after = Some(bot.id);
        }
        state.complete = page.len() <= PAGE as usize;
        if state.complete {
            state.after = None;
        }
        // The snapshot and write conflict protect the exact source across our
        // own observation-counter increments in capture().
        rows.update_one(
            doc! {"_id":source.get("_id")},
            doc! {"$set":{SIGNING_PREPARATION_FIELD:state.seal(coll,source)?}},
        )
        .session(&mut tx)
        .await?;
        Ok(true)
    }
    .await;
    if matches!(&result,Err(AppError::DatabaseError(error)) if error.contains_label("TransientTransactionError"))
    {
        tx.abort_transaction().await?;
        return Ok(true);
    }
    super::org_membership_authority::finish(tx, result).await
}

/// Called before the exact-source CAS in the same transaction. Absence of
/// dependents is serialized with registration even on the first rewrite.
pub(super) async fn authorize_rewrap(
    db: &Database,
    session: &mut ClientSession,
    coll: &str,
    source: &Document,
    set: &mut Document,
) -> AppResult<()> {
    let consumers = consumers(coll, source);
    if let Some(state) = Preparation::read(coll, source)? {
        if !state.complete {
            return Err(blocked());
        }
        let mut target = source.clone();
        target.extend(set.clone());
        set.insert(SIGNING_PREPARATION_FIELD, state.seal(coll, &target)?);
    } else if !consumers.is_empty() {
        registration_fence(db, session).await?;
        if db
            .collection::<Document>(channel_bot::COLLECTION_NAME)
            .find_one(doc! {"$or":consumers})
            .projection(doc! {"_id":1})
            .session(session)
            .await?
            .is_some()
        {
            return Err(blocked());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_bot_attached_after_the_migration_snapshot_conflicts_with_the_registration_fence() {
        let db = crate::test_utils::connect_transaction_test_database(
            "channel_signing_registration_race",
        )
        .await;
        let settings = db.collection::<Document>(platform_settings::COLLECTION_NAME);
        let id = doc! {"_id":platform_settings::PLATFORM_SETTINGS_ID};
        settings.insert_one(doc!{"_id":platform_settings::PLATFORM_SETTINGS_ID,"channel_bot_registration_revision":0_i64}).await.unwrap();
        let mut migrate = db.client().start_session().await.unwrap();
        migrate
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .await
            .unwrap();
        assert!(
            settings
                .find_one(id.clone())
                .session(&mut migrate)
                .await
                .unwrap()
                .is_some()
        );
        let mut register = db.client().start_session().await.unwrap();
        register.start_transaction().await.unwrap();
        settings
            .update_one(
                id,
                doc! {"$inc":{"channel_bot_registration_revision":1_i64}},
            )
            .session(&mut register)
            .await
            .unwrap();
        db.collection::<Document>(channel_bot::COLLECTION_NAME)
            .insert_one(doc! {"_id":"attached","platform":"x","credential_source":"connection"})
            .session(&mut register)
            .await
            .unwrap();
        register.commit_transaction().await.unwrap();
        let source = doc! {"slug":"twitter","client_secret_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1]}};
        let result = authorize_rewrap(
            &db,
            &mut migrate,
            provider_config::COLLECTION_NAME,
            &source,
            &mut Document::new(),
        )
        .await;
        assert!(
            matches!(result,Err(AppError::DatabaseError(e)) if e.contains_label("TransientTransactionError"))
        );
        migrate.abort_transaction().await.unwrap();
        let mut fresh = db.client().start_session().await.unwrap();
        fresh.start_transaction().await.unwrap();
        assert!(matches!(
            authorize_rewrap(
                &db,
                &mut fresh,
                provider_config::COLLECTION_NAME,
                &source,
                &mut Document::new()
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        fresh.abort_transaction().await.unwrap();
    }
}
