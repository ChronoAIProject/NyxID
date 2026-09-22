use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    options::{ReturnDocument, UpdateModifications},
    results::{DeleteResult, InsertOneResult, UpdateResult},
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    future::{Future, IntoFuture},
    marker::PhantomData,
    pin::Pin,
};

use super::{
    context,
    projection::{EventIds, record_changes},
};
use crate::models::service_change_event::HistoryContext;

use crate::errors::BackingReferenceOwnerChanged;

/// Serialize new references with backing edits. Credentials that can move
/// independently must still belong to the referencing owner. Preserve legacy
/// endpoint references and missing backing rows.
pub(crate) async fn fence_backing_reference(
    db: &Database,
    collection: &str,
    id: &str,
    owner: &str,
    session: &mut ClientSession,
) -> mongodb::error::Result<bool> {
    debug_assert!(matches!(collection, "user_endpoints" | "user_api_keys"));
    let mut filter = doc! { "_id": id };
    if collection == "user_api_keys" {
        filter.insert("user_id", owner);
    }
    let result = db
        .collection::<Document>(collection)
        .update_one(
            filter,
            doc! { "$inc": { "service_history_ref_epoch": 1_i64 } },
        )
        .session(&mut *session)
        .await?;
    if result.matched_count == 1 {
        return Ok(true);
    }
    if collection == "user_api_keys"
        && db
            .collection::<Document>(collection)
            .find_one(doc! { "_id": id })
            .projection(doc! { "_id": 1 })
            .session(&mut *session)
            .await?
            .is_some()
    {
        return Err(mongodb::error::Error::custom(BackingReferenceOwnerChanged));
    }
    Ok(false)
}

/// Only the three instance collections may enter this mutation boundary.
pub struct Collection<T: Send + Sync> {
    db: Database,
    raw: mongodb::Collection<T>,
}

pub fn collection<T: Send + Sync>(db: &Database, name: &str) -> Collection<T> {
    assert!(matches!(
        name,
        "user_services" | "user_endpoints" | "user_api_keys"
    ));
    Collection {
        db: db.clone(),
        raw: db.collection(name),
    }
}

/// Usage timestamps are deliberately a single operational write. No caller-supplied
/// fields or update document can bypass the journal through this helper.
pub async fn touch_credential_usage(db: &Database, key_id: &str) -> mongodb::error::Result<()> {
    let now = bson::DateTime::now();
    db.collection::<Document>("user_api_keys")
        .update_one(
            doc! { "_id": key_id },
            doc! { "$set": { "last_used_at": now, "updated_at": now } },
        )
        .await?;
    Ok(())
}

impl<T: Send + Sync + DeserializeOwned> Collection<T> {
    pub fn find(&self, filter: Document) -> mongodb::action::Find<'_, T> {
        self.raw.find(filter)
    }
    pub fn find_one(&self, filter: Document) -> mongodb::action::FindOne<'_, T> {
        self.raw.find_one(filter)
    }
    pub fn count_documents(&self, filter: Document) -> mongodb::action::CountDocuments<'_> {
        self.raw.count_documents(filter)
    }
    pub fn distinct(
        &self,
        field: impl AsRef<str>,
        filter: Document,
    ) -> mongodb::action::Distinct<'_> {
        self.raw.distinct(field, filter)
    }
}

#[derive(Clone)]
enum Mutation {
    Insert(Document),
    Update {
        filter: Document,
        update: UpdateModifications,
        many: bool,
    },
    Delete {
        filter: Document,
        many: bool,
    },
}

pub struct Write<'a, T> {
    db: Database,
    name: String,
    mutation: mongodb::error::Result<Mutation>,
    session: Option<&'a mut ClientSession>,
    upsert: bool,
    routine_refresh: bool,
    return_document: ReturnDocument,
    context: HistoryContext,
    ids: EventIds,
    result: PhantomData<T>,
}

impl<T: Send + Sync + Serialize + DeserializeOwned> Collection<T> {
    fn write<R>(&self, mutation: mongodb::error::Result<Mutation>) -> Write<'static, R> {
        Write {
            db: self.db.clone(),
            name: self.raw.name().into(),
            mutation,
            session: None,
            upsert: false,
            routine_refresh: false,
            return_document: ReturnDocument::Before,
            context: context::current().unwrap_or_else(|| context::system("service_maintenance")),
            ids: context::event_ids(),
            result: PhantomData,
        }
    }
    pub fn insert_one(
        &self,
        value: impl std::borrow::Borrow<T>,
    ) -> Write<'static, InsertOneResult> {
        self.write(
            bson::to_document(value.borrow())
                .map(Mutation::Insert)
                .map_err(Into::into),
        )
    }
    pub fn update_one(
        &self,
        filter: Document,
        update: impl Into<UpdateModifications>,
    ) -> Write<'static, UpdateResult> {
        self.write(Ok(Mutation::Update {
            filter,
            update: update.into(),
            many: false,
        }))
    }
    pub fn update_many(
        &self,
        filter: Document,
        update: impl Into<UpdateModifications>,
    ) -> Write<'static, UpdateResult> {
        self.write(Ok(Mutation::Update {
            filter,
            update: update.into(),
            many: true,
        }))
    }
    pub fn find_one_and_update(
        &self,
        filter: Document,
        update: impl Into<UpdateModifications>,
    ) -> Write<'static, Option<T>> {
        self.write(Ok(Mutation::Update {
            filter,
            update: update.into(),
            many: false,
        }))
    }
    pub fn delete_one(&self, filter: Document) -> Write<'static, DeleteResult> {
        self.write(Ok(Mutation::Delete {
            filter,
            many: false,
        }))
    }
    pub fn delete_many(&self, filter: Document) -> Write<'static, DeleteResult> {
        self.write(Ok(Mutation::Delete { filter, many: true }))
    }
    pub fn find_one_and_delete(&self, filter: Document) -> Write<'static, Option<T>> {
        self.write(Ok(Mutation::Delete {
            filter,
            many: false,
        }))
    }
}

impl<'a, T> Write<'a, T> {
    pub fn session<'b>(self, session: &'b mut super::transaction::Transaction) -> Write<'b, T> {
        Write {
            db: self.db,
            name: self.name,
            mutation: self.mutation,
            session: Some(session.into()),
            upsert: self.upsert,
            routine_refresh: self.routine_refresh,
            return_document: self.return_document.clone(),
            context: self.context,
            ids: self.ids,
            result: PhantomData,
        }
    }
    pub fn routine_refresh(mut self) -> Self {
        self.routine_refresh = true;
        self
    }
    pub fn return_document(mut self, mode: ReturnDocument) -> Self {
        self.return_document = mode;
        self
    }
}

pub struct Outcome {
    insert: Option<InsertOneResult>,
    update: Option<UpdateResult>,
    delete: Option<DeleteResult>,
    document: Option<Document>,
}
impl Outcome {
    fn empty(mutation: &Mutation) -> Self {
        Self {
            insert: None,
            update: matches!(mutation, Mutation::Update { .. }).then(UpdateResult::default),
            delete: matches!(mutation, Mutation::Delete { .. }).then(DeleteResult::default),
            document: None,
        }
    }
    fn merge(&mut self, other: Self) {
        if let (Some(total), Some(batch)) = (&mut self.update, other.update) {
            total.matched_count += batch.matched_count;
            total.modified_count += batch.modified_count;
        }
        if let (Some(total), Some(batch)) = (&mut self.delete, other.delete) {
            total.deleted_count += batch.deleted_count;
        }
    }
}
pub trait WriteResult: Sized {
    fn extract(outcome: Outcome) -> mongodb::error::Result<Self>;
}
impl WriteResult for InsertOneResult {
    fn extract(o: Outcome) -> mongodb::error::Result<Self> {
        o.insert
            .ok_or_else(|| mongodb::error::Error::custom("Expected insert result"))
    }
}
impl WriteResult for UpdateResult {
    fn extract(o: Outcome) -> mongodb::error::Result<Self> {
        o.update
            .ok_or_else(|| mongodb::error::Error::custom("Expected update result"))
    }
}
impl WriteResult for DeleteResult {
    fn extract(o: Outcome) -> mongodb::error::Result<Self> {
        o.delete
            .ok_or_else(|| mongodb::error::Error::custom("Expected delete result"))
    }
}
impl<T: DeserializeOwned> WriteResult for Option<T> {
    fn extract(o: Outcome) -> mongodb::error::Result<Self> {
        o.document
            .map(bson::from_document)
            .transpose()
            .map_err(Into::into)
    }
}

impl<'a, T: WriteResult + Send + 'a> IntoFuture for Write<'a, T> {
    type Output = mongodb::error::Result<T>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let mutation = self.mutation?;
            let op = LocalCommit {
                db: self.db,
                name: self.name,
                mutation,
                upsert: self.upsert,
                return_document: self.return_document,
                routine_refresh: self.routine_refresh,
                context: self.context,
                ids: self.ids,
            };
            let outcome = if let Some(session) = self.session {
                op.run(session).await?
            } else {
                op.run_owned().await?
            };
            T::extract(outcome)
        })
    }
}

struct LocalCommit {
    db: Database,
    name: String,
    mutation: Mutation,
    upsert: bool,
    routine_refresh: bool,
    return_document: ReturnDocument,
    context: HistoryContext,
    ids: EventIds,
}
impl LocalCommit {
    async fn run_owned(&self) -> mongodb::error::Result<Outcome> {
        let filter = match &self.mutation {
            Mutation::Update {
                filter, many: true, ..
            }
            | Mutation::Delete { filter, many: true } => filter,
            _ => return self.run_owned_transaction().await,
        };
        let raw = self.db.collection::<Document>(&self.name);
        // Keyset pages preserve bounded memory and never revisit an already selected ID.
        // Each batch has its own atomic journal commit, all under one operation group.
        let ceiling = raw
            .find_one(filter.clone())
            .sort(doc! { "_id": -1 })
            .projection(doc! { "_id": 1 })
            .await?
            .and_then(|d| d.get("_id").cloned());
        let mut total = Outcome::empty(&self.mutation);
        let Some(ceiling) = ceiling else {
            return Ok(total);
        };
        let mut last = None;
        loop {
            let mut range = doc! { "$lte": &ceiling };
            if let Some(last) = &last {
                range.insert("$gt", last);
            }
            let selected: Vec<Document> = raw
                .find(doc! { "$and": [filter.clone(), { "_id": range }] })
                .sort(doc! { "_id": 1 })
                .projection(doc! { "_id": 1 })
                .limit(128)
                .await?
                .try_collect()
                .await?;
            if selected.is_empty() {
                break;
            }
            last = selected.last().and_then(|d| d.get("_id").cloned());
            let restricted = doc! { "$and": [filter.clone(), { "_id": { "$in": selected.iter().filter_map(|d| d.get("_id").cloned()).collect::<Vec<_>>() } }] };
            let mutation = match &self.mutation {
                Mutation::Update { update, .. } => Mutation::Update {
                    filter: restricted,
                    update: update.clone(),
                    many: true,
                },
                _ => Mutation::Delete {
                    filter: restricted,
                    many: true,
                },
            };
            let batch = LocalCommit {
                db: self.db.clone(),
                name: self.name.clone(),
                mutation,
                upsert: false,
                routine_refresh: self.routine_refresh,
                return_document: self.return_document.clone(),
                context: self.context.clone(),
                ids: self.ids.clone(),
            };
            total.merge(batch.run_owned_transaction().await?);
        }
        Ok(total)
    }

    async fn run_owned_transaction(&self) -> mongodb::error::Result<Outcome> {
        let mut session = self.db.client().start_session().await?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            session.start_transaction().await?;
            let result = self.run(&mut session).await;
            match result {
                Ok(outcome) => loop {
                    match session.commit_transaction().await {
                        Ok(()) => return Ok(outcome),
                        Err(error)
                            if error.contains_label("UnknownTransactionCommitResult")
                                && tokio::time::Instant::now() < deadline =>
                        {
                            continue;
                        }
                        Err(error)
                            if error.contains_label("TransientTransactionError")
                                && tokio::time::Instant::now() < deadline =>
                        {
                            break;
                        }
                        Err(error) => return Err(error),
                    }
                },
                Err(error) => {
                    let _ = session.abort_transaction().await;
                    if !super::transaction::retryable(&error)
                        || tokio::time::Instant::now() >= deadline
                    {
                        return Err(error);
                    }
                }
            }
            tokio::task::yield_now().await;
        }
    }

    async fn run(&self, session: &mut ClientSession) -> mongodb::error::Result<Outcome> {
        if let Mutation::Update {
            filter, many: true, ..
        }
        | Mutation::Delete { filter, many: true } = &self.mutation
        {
            let collection = self.db.collection::<Document>(&self.name);
            let mut cursor = collection
                .find(filter.clone())
                .sort(doc! { "_id": 1 })
                .batch_size(128)
                .session(&mut *session)
                .await?;
            let mut total = Outcome::empty(&self.mutation);
            let mut batch = Vec::new();
            while cursor.advance(&mut *session).await? {
                batch.push(cursor.deserialize_current()?);
                if batch.len() == 128 {
                    total.merge(self.apply(session, std::mem::take(&mut batch)).await?);
                }
            }
            if !batch.is_empty() {
                total.merge(self.apply(session, batch).await?);
            }
            return Ok(total);
        }
        let before = match &self.mutation {
            Mutation::Update { filter, .. } | Mutation::Delete { filter, .. } => self
                .db
                .collection::<Document>(&self.name)
                .find_one(filter.clone())
                .session(&mut *session)
                .await?
                .into_iter()
                .collect(),
            Mutation::Insert(_) => vec![],
        };
        self.apply(session, before).await
    }

    async fn apply(
        &self,
        session: &mut ClientSession,
        before: Vec<Document>,
    ) -> mongodb::error::Result<Outcome> {
        let collection = self.db.collection::<Document>(&self.name);
        let mut after = Vec::new();
        let mut outcome = Outcome {
            insert: None,
            update: None,
            delete: None,
            document: None,
        };
        match &self.mutation {
            Mutation::Insert(document) => {
                outcome.insert = Some(
                    collection
                        .insert_one(document)
                        .session(&mut *session)
                        .await?,
                );
                after.push(document.clone());
            }
            Mutation::Update {
                filter,
                update,
                many,
            } => {
                // Restrict to the documents read inside this snapshot. The write fences every pre-image.
                let selected = if before.is_empty() {
                    filter.clone()
                } else {
                    doc! { "$and": [filter.clone(), { "_id": { "$in": before.iter().filter_map(|d| d.get("_id").cloned()).collect::<Vec<_>>() } }] }
                };
                let result = if *many {
                    collection
                        .update_many(selected, update.clone())
                        .upsert(self.upsert)
                        .session(&mut *session)
                        .await?
                } else {
                    collection
                        .update_one(selected, update.clone())
                        .upsert(self.upsert)
                        .session(&mut *session)
                        .await?
                };
                let mut ids = before
                    .iter()
                    .filter_map(|d| d.get("_id").cloned())
                    .collect::<Vec<_>>();
                ids.extend(result.upserted_id.clone());
                let mut cursor = collection
                    .find(doc! { "_id": { "$in": ids } })
                    .session(&mut *session)
                    .await?;
                while cursor.advance(&mut *session).await? {
                    after.push(cursor.deserialize_current()?);
                }
                outcome.update = Some(result);
            }
            Mutation::Delete { filter, many } => {
                let selected = doc! { "$and": [filter.clone(), { "_id": { "$in": before.iter().filter_map(|d| d.get("_id").cloned()).collect::<Vec<_>>() } }] };
                outcome.delete = Some(if *many {
                    collection
                        .delete_many(selected)
                        .session(&mut *session)
                        .await?
                } else {
                    collection
                        .delete_one(selected)
                        .session(&mut *session)
                        .await?
                });
            }
        }
        if self.name == "user_services" {
            // Snapshot isolation alone cannot see a reference inserted concurrently
            // with a backing-resource edit. Writing its fence makes those commits
            // conflict and retry with a snapshot that includes the new reference.
            let mut references = std::collections::BTreeSet::new();
            for service in &after {
                let old = before
                    .iter()
                    .find(|old| old.get("_id") == service.get("_id"));
                let owner = service.get_str("user_id").map_err(|_| {
                    mongodb::error::Error::custom("Service history reference missing owner")
                })?;
                for (field, backing) in [
                    ("endpoint_id", "user_endpoints"),
                    ("api_key_id", "user_api_keys"),
                ] {
                    let Some(id) = service.get_str(field).ok() else {
                        continue;
                    };
                    if old.and_then(|old| old.get_str(field).ok()) != Some(id) {
                        references.insert((backing, id, owner));
                    }
                }
            }
            for (backing, id, owner) in references {
                fence_backing_reference(&self.db, backing, id, owner, session).await?;
            }
        }
        record_changes(
            &self.db,
            session,
            &self.name,
            &before,
            &after,
            &self.context,
            &self.ids,
            self.routine_refresh,
        )
        .await?;
        outcome.document = match self.return_document {
            ReturnDocument::After => after.into_iter().next(),
            _ => before.into_iter().next(),
        };
        Ok(outcome)
    }
}
