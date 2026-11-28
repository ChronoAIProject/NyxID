//! A dirty closure is computed against the last complete cut and a single
//! database time W. The caller owns generation-private, idempotent staging.
use super::*;
use futures::{StreamExt, stream::BoxStream};
use std::collections::BTreeMap;

pub(crate) struct Snapshot<'a> {
    census: &'a Census<'a>,
    pub cut: &'a Cut,
}

impl Snapshot<'_> {
    async fn checked<T>(&self, value: mongodb::error::Result<T>) -> AppResult<T> {
        match value {
            Ok(value) => Ok(value),
            Err(error) => {
                if let mongodb::error::ErrorKind::Command(command) = error.kind.as_ref()
                    && matches!(command.code, 239 | 246 | 286)
                {
                    self.census.invalidate("snapshot_unavailable").await?;
                    return Err(refused("Census snapshot unavailable"));
                }
                Err(error.into())
            }
        }
    }

    /// All adapter source and companion reads go through this handle. A
    /// cursor returned by MongoDB retains the snapshot for subsequent getMore.
    pub(crate) async fn find(
        &self,
        collection: &str,
        filter: Document,
        projection: Document,
    ) -> AppResult<BoxStream<'_, AppResult<Document>>> {
        if !self.census.scope.namespaces.contains(collection) {
            return Err(refused("Unregistered census namespace"));
        }
        let mut command = doc! {"find":collection,"filter":filter,"sort":{"_id":1},
        "batchSize":PAGE,"readConcern":{"level":"snapshot","atClusterTime":self.cut.time}};
        if !projection.is_empty() {
            command.insert("projection", projection);
        }
        let cursor = self
            .checked(self.census.db.run_cursor_command(command).await)
            .await?;
        Ok(cursor.then(|result| self.checked(result)).boxed())
    }

    pub(crate) async fn row(&self, row: &Row) -> AppResult<Option<Document>> {
        let document = self
            .find(&row.collection, doc! {"_id":{"$eq":&row.id}}, doc! {})
            .await?
            .try_next()
            .await?;
        // MongoDB equality can match another numeric type or a collation
        // equivalent string. The previous contribution uses the exact BSON key.
        if let Some(found) = &document {
            let key = found.get("_id").ok_or_else(malformed)?;
            let bytes = |id: &Bson| bson::to_vec(&doc! {"_id":id}).map_err(|_| malformed());
            if bytes(key)? != bytes(&row.id)? {
                return Ok(None);
            }
        }
        Ok(document)
    }
}

pub(crate) enum Dependents {
    Rows(Vec<Row>),
    Rescan,
}

#[async_trait::async_trait]
pub(crate) trait Observations: Sync + Send {
    fn dirty_limit(&self) -> usize {
        MAX_DIRTY_ROWS
    }

    /// Include missing-parent edges recorded by the last complete generation.
    async fn previous_dependents(&self, from: &Cut, parent: &Row) -> AppResult<Dependents>;
    /// Include newly linked sources using the reviewed reverse join at W.
    async fn current_dependents(&self, at: &Snapshot<'_>, parent: &Row) -> AppResult<Dependents>;
    /// Replace the staged contribution for this identity, including absence.
    async fn replace(
        &mut self,
        at: &Snapshot<'_>,
        row: &Row,
        source: Option<Document>,
    ) -> AppResult<()>;
    /// Reconstruct the candidate from scratch at W when a join cannot be
    /// bounded. The baseline's published partitions remain visible.
    async fn rescan(&mut self, at: &Snapshot<'_>) -> AppResult<()>;
}

const MAX_DIRTY_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIRTY_ROWS: usize = 16_384;

struct Closure {
    seen: BTreeMap<Vec<u8>, Row>,
    pending: Vec<Vec<u8>>,
    bytes: usize,
}

impl Closure {
    fn add(&mut self, scope: &Scope, row: Row, limit: usize) -> AppResult<bool> {
        if !scope.namespaces.contains(&row.collection) {
            return Err(malformed());
        }
        let key = bson::to_vec(&row).map_err(|_| malformed())?;
        if !self.seen.contains_key(&key) {
            self.bytes += key.len();
            if self.bytes > MAX_DIRTY_BYTES || self.seen.len() >= limit {
                return Ok(false);
            }
            self.pending.push(key.clone());
            self.seen.insert(key, row);
        }
        Ok(true)
    }
}

impl Census<'_> {
    pub(crate) async fn snapshot<'a>(&'a self, cut: &'a Cut) -> AppResult<Snapshot<'a>> {
        if cut.generation != self.id {
            return Err(refused("Census cut belongs to another generation"));
        }
        let witnessed = self.cut(&cut.marker).await?.ok_or_else(malformed)?;
        if witnessed.page != cut.page || witnessed.time != cut.time {
            return Err(malformed());
        }
        Ok(Snapshot { census: self, cut })
    }

    /// On failure the caller discards or retries the private candidate. Only a
    /// successful return is eligible for the later manifest publication CAS.
    pub(crate) async fn reconcile(
        &self,
        from: &Cut,
        to: &Cut,
        observer: &mut impl Observations,
    ) -> AppResult<()> {
        self.reconcile_range(from, to, from.page + 1..=to.page, observer)
            .await
    }

    pub(super) async fn reconcile_range(
        &self,
        from: &Cut,
        to: &Cut,
        pages: std::ops::RangeInclusive<i64>,
        observer: &mut impl Observations,
    ) -> AppResult<()> {
        self.snapshot(from).await?;
        let snapshot = self.snapshot(to).await?;
        if pages.is_empty()
            || *pages.start() <= from.page
            || *pages.end() > to.page
            || from.time >= to.time
        {
            return Err(refused("Census cuts are out of order"));
        }
        let result = self.apply(from, &snapshot, pages, observer).await;
        if let Err(e) = &result
            && continuity_lost(e)
        {
            self.invalidate("replay_incomplete").await?;
        }
        result?;
        self.state().await?;
        Ok(())
    }

    async fn apply(
        &self,
        from: &Cut,
        to: &Snapshot<'_>,
        pages: std::ops::RangeInclusive<i64>,
        observer: &mut impl Observations,
    ) -> AppResult<()> {
        let mut closure = Closure {
            seen: BTreeMap::new(),
            pending: Vec::new(),
            bytes: 0,
        };
        let mut full_scan = false;
        for page in pages {
            let batch = self.read_page(page).await?;
            if !full_scan {
                for row in batch.dirty {
                    if !closure.add(&self.scope, row, observer.dirty_limit())? {
                        full_scan = true;
                        break;
                    }
                }
            }
        }
        if full_scan {
            return observer.rescan(to).await;
        }
        while let Some(key) = closure.pending.pop() {
            let row = closure.seen.get(&key).ok_or_else(malformed)?.clone();
            for related in [
                observer.previous_dependents(from, &row).await?,
                observer.current_dependents(to, &row).await?,
            ] {
                let Dependents::Rows(rows) = related else {
                    return observer.rescan(to).await;
                };
                for child in rows {
                    if !closure.add(&self.scope, child, observer.dirty_limit())? {
                        return observer.rescan(to).await;
                    }
                }
            }
        }
        for row in closure.seen.values() {
            let source = to.row(row).await?;
            observer.replace(to, row, source).await?;
        }
        Ok(())
    }
}
