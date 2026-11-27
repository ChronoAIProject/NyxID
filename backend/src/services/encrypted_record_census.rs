//! Majority change-stream journal for the encrypted-record census. The runner
//! supplies the deployed adapter contract and admin-recorded restore epoch.
//! Only a marker witnessed on this database stream can define a snapshot cut.
use std::{collections::BTreeSet, time::Duration};

use bson::{Bson, Document, Timestamp, doc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    change_stream::{ChangeStream, event::ResumeToken},
    options::{IndexOptions, ReadConcern, ReturnDocument, WriteConcern},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::transaction_service;
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
};

pub(crate) mod replay;

const LOG: &str = "encrypted_record_census_log";
const PROTOCOL: i32 = 1;
const PAGE: u32 = 128;
const MAX_PAGE_BYTES: usize = 1024 * 1024;
const MAX_PAGES: i64 = 65_536;
const RETENTION: Duration = Duration::from_secs(7 * 86400);

fn refused(reason: &'static str) -> AppError {
    AppError::Conflict(reason.into())
}
fn malformed() -> AppError {
    refused("Invalid census checkpoint")
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Scope {
    pub registry_digest: String,
    pub restore_epoch: String,
    pub deployment_revision: String,
    pub namespaces: BTreeSet<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Row {
    pub collection: String,
    pub id: Bson,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Cut {
    generation: String,
    page: i64,
    pub time: Timestamp,
    marker: String,
}

#[derive(Serialize, Deserialize)]
struct Batch {
    token: ResumeToken,
    dirty: Vec<Row>,
    cut: Option<Cut>,
}

pub(crate) struct Census<'a> {
    db: Database,
    keys: &'a EncryptionKeys,
    id: String,
    scope: Scope,
}

impl<'a> Census<'a> {
    fn log(&self) -> mongodb::Collection<Document> {
        self.db.collection::<Document>(LOG)
    }

    fn marker_id(&self, marker: &str) -> Document {
        doc! {"generation": &self.id, "marker": marker}
    }

    fn page_id(&self, page: i64) -> Document {
        doc! {"generation": &self.id, "page": page}
    }

    async fn catalog(&self) -> AppResult<Document> {
        let names: Vec<_> = self
            .scope
            .namespaces
            .iter()
            .cloned()
            .chain([LOG.into()])
            .collect();
        let mut rows = self
            .db
            .list_collections()
            .filter(doc! {"name":{"$in":names}})
            .await?;
        let mut ids = std::collections::BTreeMap::new();
        while let Some(row) = rows.try_next().await? {
            ids.insert(row.name, row.info.uuid.ok_or_else(malformed)?);
        }
        // An absent source is an empty family at S. Creating it is a watched DDL event.
        Ok(ids
            .into_iter()
            .map(|(name, uuid)| (name, Bson::Binary(uuid)))
            .collect())
    }

    async fn watch(&self, token: Option<ResumeToken>) -> AppResult<ChangeStream<Document>> {
        let names: Vec<_> = self.scope.namespaces.iter().cloned().collect();
        let pipeline = [
            doc! {"$match":{"$or":[
                {"ns.coll":{"$in":&names}},
                {"to.coll":{"$in":&names}},
                {"ns.coll":LOG,"operationType":"insert","documentKey._id.generation":&self.id,
                    "documentKey._id.marker":{"$exists":true}},
                {"operationType":{"$nin":["insert","update","replace","delete"]},
                    "$or":[{"ns.coll":LOG},{"ns.coll":{"$exists":false}},{"to.coll":LOG}]}
            ]}},
            doc! {"$project":{"_id":1,"operationType":1,"ns":1,"to":1,"collectionUUID":1,
            "clusterTime":1,"documentKey._id":1}},
        ];
        Ok(self
            .db
            .watch()
            .pipeline(pipeline)
            .show_expanded_events(true)
            .batch_size(PAGE)
            .max_await_time(Duration::from_millis(100))
            .resume_after(token)
            .await?
            .with_type())
    }

    // Control-plane keys are supplied by the caller. The record identity includes
    // the generation and page so ciphertext from another checkpoint cannot replay.
    async fn seal(&self, page: i64, batch: &Batch) -> AppResult<Bson> {
        let slot = page.to_string();
        let fields = ["journal", slot.as_str()];
        let ctx = RecordContext::new(LOG, &self.id, "platform", &fields)?;
        let plain = Zeroizing::new(bson::to_vec(batch).map_err(|_| malformed())?);
        let bytes = self.keys.encrypt_record(&plain, &ctx).await?;
        Ok(Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes,
        }))
    }

    async fn read_page(&self, page: i64) -> AppResult<Batch> {
        let Some(row) = self.log().find_one(doc! {"_id":self.page_id(page)}).await? else {
            self.invalidate("missing_page").await?;
            return Err(refused("Census journal page unavailable"));
        };
        let slot = page.to_string();
        let fields = ["journal", slot.as_str()];
        let ctx = RecordContext::new(LOG, &self.id, "platform", &fields)?;
        let opened = async {
            let ciphertext = row.get_binary_generic("batch").map_err(|_| malformed())?;
            let plain = self.keys.decrypt_record(ciphertext, &ctx).await?;
            bson::from_slice(&plain).map_err(|_| malformed())
        }
        .await;
        if opened.is_err() {
            self.invalidate("unreadable_page").await?;
        }
        opened
    }

    pub(crate) async fn start(
        db: &Database,
        keys: &'a EncryptionKeys,
        scope: Scope,
    ) -> AppResult<(Self, Cut)> {
        if scope.namespaces.is_empty()
            || scope.namespaces.contains(LOG)
            || scope.registry_digest.is_empty()
            || scope.restore_epoch.is_empty()
            || scope.deployment_revision.is_empty()
        {
            return Err(refused("Incomplete census scope"));
        }
        let census = Self {
            db: transaction_service::durable_database(db),
            keys,
            id: Uuid::new_v4().to_string(),
            scope,
        };
        let scope_revision = census
            .scope_revision()
            .await?
            .ok_or_else(|| refused("Census lifecycle scope changed"))?;
        // The journal and its TTL index must exist before opening the stream.
        census
            .log()
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"expires_at":1})
                    .options(IndexOptions::builder().expire_after(Duration::ZERO).build())
                    .build(),
            )
            .await?;
        let hello = census.db.run_command(doc! {"hello":1}).await?;
        if !hello.contains_key("setName") || hello.contains_key("msg") {
            return Err(refused("Census requires an ordered replica-set stream"));
        }
        let identity = transaction_service::replica_database_identity(&census.db).await?;
        let mut stream = census.watch(None).await?;
        let catalog = census.catalog().await?;
        let deadline = bson::DateTime::from_millis(
            bson::DateTime::now().timestamp_millis() + RETENTION.as_millis() as i64,
        );
        let marker = Uuid::new_v4().to_string();
        census.write_marker(&marker, deadline).await?;
        let cut = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let event = stream
                    .try_next()
                    .await?
                    .ok_or_else(|| refused("Census stream ended"))?;
                match census.classify(&event, &catalog)? {
                    Change::Marker(id) if id == marker => {
                        return Ok::<_, AppError>(Cut {
                            generation: census.id.clone(),
                            page: 0,
                            time: event
                                .get_timestamp("clusterTime")
                                .map_err(|_| malformed())?,
                            marker: id,
                        });
                    }
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| refused("Census start marker unavailable"))??;
        if census.catalog().await? != catalog
            || transaction_service::replica_database_identity(&census.db).await? != identity
        {
            return Err(refused("Census collection identity changed"));
        }
        let token = stream.resume_token().ok_or_else(malformed)?;
        let batch = census
            .seal(
                0,
                &Batch {
                    token,
                    dirty: vec![],
                    cut: Some(cut.clone()),
                },
            )
            .await?;
        let mut tx = census.db.client().start_session().await?;
        tx.start_transaction()
            .write_concern(WriteConcern::majority())
            .await?;
        census.log().insert_many([
            doc! {"_id":&census.id,"protocol":PROTOCOL,"scope_revision":scope_revision,"scope":bson::to_bson(&census.scope).map_err(|_|malformed())?,
                "database_identity":identity,"catalog":catalog,"checkpointed_at":bson::DateTime::now(),"head":0_i64,"expires_at":deadline,"lease_until":bson::DateTime::from_millis(0)},
            doc! {"_id":census.page_id(0),"batch":batch,"expires_at":deadline}
        ]).session(&mut tx).await?;
        census
            .log()
            .update_one(
                doc! {"_id":census.marker_id(&cut.marker)},
                doc! {"$set":{"page":0_i64}},
            )
            .session(&mut tx)
            .await?;
        transaction_service::commit(&mut tx).await?;
        census.state().await?;
        Ok((census, cut))
    }

    async fn write_marker(&self, marker: &str, expires: bson::DateTime) -> AppResult<()> {
        self.log()
            .insert_one(doc! {"_id":self.marker_id(marker),"expires_at":expires})
            .await?;
        Ok(())
    }

    pub(crate) async fn resume(
        db: &Database,
        keys: &'a EncryptionKeys,
        id: &str,
        scope: Scope,
    ) -> AppResult<Self> {
        let census = Self {
            db: transaction_service::durable_database(db),
            keys,
            id: id.to_owned(),
            scope,
        };
        let state = census.state().await?;
        if state.get_document("database_identity").ok()
            != Some(&transaction_service::replica_database_identity(&census.db).await?)
            || state.get_document("catalog").ok() != Some(&census.catalog().await?)
        {
            census.invalidate("database_identity_changed").await?;
            return Err(refused("Census database identity changed"));
        }
        Ok(census)
    }

    pub(crate) fn generation(&self) -> &str {
        &self.id
    }

    async fn invalidate(&self, reason: &'static str) -> AppResult<()> {
        self.log()
            .update_one(
                doc! {"_id":&self.id,"invalid":{"$exists":false}},
                doc! {"$set":{"invalid":reason}},
            )
            .await?;
        Ok(())
    }

    async fn scope_revision(&self) -> AppResult<Option<i64>> {
        super::key_lifecycle::census_scope_revision(
            &self.db,
            &self.scope.deployment_revision,
            &self.scope.restore_epoch,
        )
        .await
    }

    async fn state(&self) -> AppResult<Document> {
        let state = self
            .log()
            .find_one(doc! {"_id":&self.id,"$expr":{"$gt":["$expires_at","$$NOW"]}})
            .await?
            .ok_or_else(|| refused("Census generation unavailable"))?;
        if state.contains_key("invalid") {
            return Err(refused("Census generation invalidated"));
        }
        let stored_revision = match state.get("scope_revision") {
            None => 0, // A pre-enrollment PR 7b journal.
            Some(Bson::Int64(rev)) => *rev,
            _ => return Err(malformed()),
        };
        if self.scope_revision().await? != Some(stored_revision) {
            self.invalidate("lifecycle_scope_changed").await?;
            return Err(refused("Census lifecycle scope changed"));
        }
        if state.get_i32("protocol") != Ok(PROTOCOL)
            || state.get("scope") != Some(&bson::to_bson(&self.scope).map_err(|_| malformed())?)
        {
            self.invalidate("scope_changed").await?;
            return Err(refused("Census scope changed"));
        }
        Ok(state)
    }

    pub(crate) async fn request_cut(&self) -> AppResult<String> {
        let state = self.state().await?;
        let marker = Uuid::new_v4().to_string();
        self.write_marker(
            &marker,
            *state.get_datetime("expires_at").map_err(|_| malformed())?,
        )
        .await?;
        Ok(marker)
    }

    fn classify(&self, event: &Document, catalog: &Document) -> AppResult<Change> {
        let op = event.get_str("operationType").map_err(|_| malformed())?;
        if !matches!(op, "insert" | "replace" | "delete" | "update") {
            return Err(refused("Census catalog changed"));
        }
        let ns = event.get_document("ns").map_err(|_| malformed())?;
        let coll = ns.get_str("coll").map_err(|_| malformed())?;
        if ns.get_str("db") != Ok(self.db.name())
            || event.get("collectionUUID").is_none()
            || event.get("collectionUUID") != catalog.get(coll)
        {
            return Err(refused("Census collection identity changed"));
        }
        let key = event
            .get_document("documentKey")
            .map_err(|_| malformed())?
            .get("_id")
            .ok_or_else(malformed)?;
        if coll == LOG {
            let marker = key.as_document().ok_or_else(malformed)?;
            if op != "insert" || marker.get_str("generation") != Ok(self.id.as_str()) {
                return Err(malformed());
            }
            return Ok(Change::Marker(
                marker.get_str("marker").map_err(|_| malformed())?.into(),
            ));
        }
        if !self.scope.namespaces.contains(coll) {
            return Err(malformed());
        }
        Ok(Change::Row(Row {
            collection: coll.into(),
            id: key.clone(),
        }))
    }
}

enum Change {
    Row(Row),
    Marker(String),
}

#[cfg(test)]
pub(super) mod tests;

impl Census<'_> {
    /// Read one bounded page starting at the durable token. Discard the local
    /// cursor on cancellation, conflict or unknown commit. Retrying opens at
    /// the last *committed* page, including after a primary change.
    pub(crate) async fn capture(&self) -> AppResult<Option<Cut>> {
        let state = self.state().await?;
        let head = state.get_i64("head").map_err(|_| malformed())?;
        if !(0..MAX_PAGES).contains(&head) {
            self.invalidate("journal_limit").await?;
            return Err(refused("Census journal capacity exhausted"));
        }
        let old = self.read_page(head).await?;
        let claim = Uuid::new_v4().to_string();
        let claimed = self.log().find_one_and_update(doc! {
            "_id":&self.id,"head":head,"invalid":{"$exists":false},
            "$expr":{"$and":[{"$lte":["$lease_until","$$NOW"]},{"$gt":["$expires_at","$$NOW"]}]}
        },vec![doc! {"$set":{"claim":&claim,"lease_until":{"$add":["$$NOW",30_000]},"save_idle":{"$gte":[{"$subtract":["$$NOW","$checkpointed_at"]},60_000]}}}])
            .return_document(ReturnDocument::After).await?;
        let claimed = claimed.ok_or_else(|| refused("Census capture lease changed"))?;
        let next = self
            .collect_page(
                old.token,
                state.get_document("catalog").map_err(|_| malformed())?,
                head + 1,
                claimed.get_bool("save_idle").map_err(|_| malformed())?,
            )
            .await;
        let batch = match next {
            Ok(Some(batch)) => batch,
            Ok(None) => {
                self.log().update_one(doc!{"_id":&self.id,"claim":&claim},doc!{"$unset":{"claim":""},"$set":{"lease_until":bson::DateTime::from_millis(0)}}).await?;
                return Ok(None);
            }
            Err(error) => {
                if continuity_lost(&error) {
                    self.invalidate("stream_discontinuity").await?;
                }
                return Err(refused(
                    "Census stream unavailable; inspect generation status",
                ));
            }
        };
        let sealed = self.seal(head + 1, &batch).await?;
        let mut tx = self.db.client().start_session().await?;
        tx.start_transaction()
            .read_concern(ReadConcern::snapshot())
            .write_concern(WriteConcern::majority())
            .await?;
        let changed=self.log().update_one(doc!{"_id":&self.id,"head":head,"claim":&claim,"invalid":{"$exists":false},
            "$expr":{"$and":[{"$gt":["$lease_until","$$NOW"]},{"$gt":["$expires_at","$$NOW"]}]}},
            doc!{"$currentDate":{"checkpointed_at":true},"$inc":{"head":1_i64},"$unset":{"claim":""},"$set":{"lease_until":bson::DateTime::from_millis(0)}}).session(&mut tx).await?;
        if changed.matched_count != 1 {
            return Err(refused("Census capture lease changed"));
        }
        self.log()
            .insert_one(doc! {"_id":self.page_id(head+1),"batch":sealed,
            "expires_at":state.get_datetime("expires_at").map_err(|_|malformed())?})
            .session(&mut tx)
            .await?;
        if let Some(cut) = &batch.cut {
            let indexed = self
                .log()
                .update_one(
                    doc! {"_id":self.marker_id(&cut.marker)},
                    doc! {"$set":{"page":cut.page}},
                )
                .session(&mut tx)
                .await?;
            if indexed.matched_count != 1 {
                return Err(malformed());
            }
        }
        transaction_service::commit(&mut tx).await?;
        Ok(batch.cut)
    }

    async fn collect_page(
        &self,
        token: ResumeToken,
        catalog: &Document,
        page: i64,
        save_idle: bool,
    ) -> AppResult<Option<Batch>> {
        let mut stream = self.watch(Some(token.clone())).await?;
        let mut dirty = vec![];
        let mut cut = None;
        let mut bytes = 0;
        let mut previous = token.clone();
        for _ in 0..PAGE {
            let Some(event) = stream.next_if_any().await? else {
                if !stream.is_alive() {
                    return Err(refused("Census stream ended"));
                }
                break;
            };
            match self.classify(&event, catalog)? {
                Change::Marker(marker) => {
                    cut = Some(Cut {
                        generation: self.id.clone(),
                        page,
                        marker,
                        time: event
                            .get_timestamp("clusterTime")
                            .map_err(|_| malformed())?,
                    });
                    break;
                }
                Change::Row(row) => {
                    let size = bson::to_vec(&row).map_err(|_| malformed())?.len();
                    if size > MAX_PAGE_BYTES {
                        return Err(refused("Census document key too large"));
                    }
                    if size > MAX_PAGE_BYTES - bytes {
                        // The next cursor must return this unjournaled event again.
                        return Ok(Some(Batch {
                            token: previous,
                            dirty,
                            cut: None,
                        }));
                    }
                    bytes += size;
                    dirty.push(row);
                    previous = stream.resume_token().ok_or_else(malformed)?;
                    if bytes >= MAX_PAGE_BYTES {
                        break;
                    }
                }
            }
        }
        let next = stream.resume_token().ok_or_else(malformed)?;
        if dirty.is_empty() && cut.is_none() && (!save_idle || next == token) {
            return Ok(None);
        }
        Ok(Some(Batch {
            token: next,
            dirty,
            cut,
        }))
    }

    /// Recover a witnessed marker even when the response to its checkpoint
    /// transaction was lost. The indexable page locator is only a hint.
    pub(crate) async fn cut(&self, marker: &str) -> AppResult<Option<Cut>> {
        self.state().await?;
        let Some(index) = self
            .log()
            .find_one(doc! {"_id":self.marker_id(marker)})
            .await?
        else {
            return Err(refused("Census marker unavailable"));
        };
        let Ok(page) = index.get_i64("page") else {
            return Ok(None);
        };
        let cut = self.read_page(page).await?.cut.ok_or_else(malformed)?;
        if cut.marker != marker || cut.generation != self.id || cut.page != page {
            return Err(malformed());
        }
        Ok(Some(cut))
    }
}

fn continuity_lost(error: &AppError) -> bool {
    match error {
        AppError::DatabaseError(db_error) => {
            if db_error.contains_label("ResumableChangeStreamError") {
                return false;
            }
            !matches!(
                db_error.kind.as_ref(),
                mongodb::error::ErrorKind::Io(_)
                    | mongodb::error::ErrorKind::ServerSelection { .. }
                    | mongodb::error::ErrorKind::ConnectionPoolCleared { .. }
            )
        }
        _ => true,
    }
}
