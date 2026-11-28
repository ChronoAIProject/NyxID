use super::*;

pub(super) struct Plan<'a, 'b> {
    pub store: &'a Store<'b>,
    pub state: &'a mut Checkpoint,
}

#[async_trait::async_trait]
impl Observations for Plan<'_, '_> {
    fn dirty_limit(&self) -> usize {
        DIRTY_LIMIT
    }
    async fn previous_dependents(&self, _: &Cut, parent: &Row) -> AppResult<Dependents> {
        let token = self.state.token(parent)?;
        let mut children = Vec::new();
        for (i, family) in observation::families().enumerate() {
            if !family
                .parent_joins()
                .iter()
                .any(|(c, _)| *c == parent.collection)
            {
                continue;
            }
            let report: FamilyReport = self.store.get("family", &self.state.baseline[i]).await?;
            if report.pages > 4 {
                return Ok(Dependents::Rescan);
            }
            let mut link = report.head;
            while let Some(hash) = link {
                let page: Partition = self.store.get("rows", &hash).await?;
                children.extend(
                    page.rows
                        .into_iter()
                        .filter(|r| r.parents.contains(&token))
                        .map(|r| r.row),
                );
                if children.len() > DIRTY_LIMIT {
                    return Ok(Dependents::Rescan);
                }
                link = page.previous;
            }
        }
        Ok(Dependents::Rows(children))
    }
    async fn current_dependents(&self, at: &Snapshot<'_>, parent: &Row) -> AppResult<Dependents> {
        let mut children = vec![];
        for family in observation::families() {
            if family
                .rescan_parents()
                .contains(&parent.collection.as_str())
            {
                return Ok(Dependents::Rescan);
            }
            for &(collection, path) in family.parent_joins() {
                if collection != parent.collection {
                    continue;
                }
                let mut rows = at
                    .find(
                        family.collection(),
                        doc! {path:{"$eq":&parent.id}},
                        doc! {"_id":1},
                    )
                    .await?;
                while let Some(mut row) = rows.try_next().await? {
                    children.push(Row {
                        collection: family.collection().into(),
                        id: row.remove("_id").ok_or_else(malformed)?,
                    });
                    if children.len() > DIRTY_LIMIT {
                        return Ok(Dependents::Rescan);
                    }
                }
            }
        }
        Ok(Dependents::Rows(children))
    }
    async fn replace(&mut self, _: &Snapshot<'_>, row: &Row, _: Option<Document>) -> AppResult<()> {
        if !self.state.rescan {
            self.state.dirty.insert(self.state.token(row)?, row.clone());
            if self.state.dirty.len() > DIRTY_LIMIT
                || bson::to_vec(&self.state.dirty)
                    .map_err(|_| malformed())?
                    .len()
                    > MAX_PAGE_BYTES
            {
                self.state.dirty.clear();
                self.state.rescan = true;
            }
        }
        Ok(())
    }
    async fn rescan(&mut self, _: &Snapshot<'_>) -> AppResult<()> {
        self.state.dirty.clear();
        self.state.rescan = true;
        Ok(())
    }
}

async fn record(
    store: &Store<'_>,
    state: &Checkpoint,
    at: &Snapshot<'_>,
    family: &Family,
    doc: &Document,
) -> AppResult<Receipt> {
    let row = Row {
        collection: family.collection().into(),
        id: doc.get("_id").ok_or_else(malformed)?.clone(),
    };
    let observed = family.observe_at(store.keys, doc, at).await?;
    Receipt::observe(state, &row, observed)
}

async fn save(store: &Store<'_>, state: &mut Checkpoint, rows: Vec<Receipt>) -> AppResult<()> {
    if rows.is_empty() {
        return Ok(());
    }
    for row in &rows {
        state.walk.summary.include(row)?;
    }
    let part = Partition {
        previous: state.walk.summary.head.take(),
        rows,
    };
    state.walk.summary.head = Some(store.put("rows", &part, state.expires_at).await?);
    state.walk.summary.pages = state
        .walk
        .summary
        .pages
        .checked_add(1)
        .ok_or_else(malformed)?;
    Ok(())
}

pub(super) async fn source_page(
    store: &Store<'_>,
    state: &mut Checkpoint,
    at: &Snapshot<'_>,
    family: &Family,
) -> AppResult<bool> {
    // Aggregation comparison spans BSON types. Match-language $gt would skip
    // malformed IDs of a different type at a page boundary.
    let filter = match &state.walk.cursor {
        Some(cursor) => {
            doc! {"$expr":{"$gt":["$_id",{"$literal":cursor.get("_id").ok_or_else(malformed)?}]}}
        }
        None => Document::new(),
    };
    let mut docs = at
        .find(family.collection(), filter, doc! {})
        .await?
        .take(ROWS);
    let mut rows = vec![];
    let mut bytes = 0;
    while let Some(doc) = docs.try_next().await? {
        let receipt = record(store, state, at, family, &doc).await?;
        let size = bson::to_vec(&receipt).map_err(|_| malformed())?.len();
        if size > MAX_PAGE_BYTES {
            return Err(refused("Census receipt exceeds partition capacity"));
        }
        if size > MAX_PAGE_BYTES - bytes {
            break;
        }
        bytes += size;
        state.walk.cursor = Some(doc! {"_id":&receipt.row.id});
        rows.push(receipt);
    }
    // An additional empty read witnesses EOF, including after byte truncation.
    let done = rows.is_empty();
    save(store, state, rows).await?;
    Ok(done)
}

async fn merge_page(
    store: &Store<'_>,
    state: &mut Checkpoint,
    at: &Snapshot<'_>,
    family: &Family,
) -> AppResult<bool> {
    if !state.walk.started {
        let base: FamilyReport = store
            .get("family", &state.baseline[state.walk.family])
            .await?;
        state.walk.link = base.head;
        state.walk.started = true;
    }
    if let Some(hash) = state.walk.link.take() {
        let page: Partition = store.get("rows", &hash).await?;
        state.walk.link = page.previous;
        let rows = page
            .rows
            .into_iter()
            .filter(|r| !state.dirty.contains_key(&r.token))
            .collect();
        save(store, state, rows).await?;
        return Ok(false);
    }
    let changed: Vec<_> = state
        .dirty
        .values()
        .filter(|r| r.collection == family.collection())
        .skip(state.walk.delta)
        .take(ROWS)
        .cloned()
        .collect();
    // One replacement per partition also bounds large, heterogeneous records.
    for row in &changed {
        if let Some(doc) = at.row(row).await? {
            let receipt = record(store, state, at, family, &doc).await?;
            save(store, state, vec![receipt]).await?;
        }
    }
    state.walk.delta += changed.len();
    Ok(changed.len() < ROWS)
}

pub(super) async fn walk(
    store: &Store<'_>,
    state: &mut Checkpoint,
    at: &Snapshot<'_>,
    reconcile: bool,
) -> AppResult<bool> {
    let families: Vec<_> = observation::families().collect();
    // Empty families cost a bounded number of point reads/writes per request.
    for _ in 0..8 {
        let Some(family) = families.get(state.walk.family) else {
            state.walk = Walk::default();
            return Ok(true);
        };
        state.walk.summary.family = family.name();
        state.walk.summary.collection = family.collection().into();
        let rescan = !reconcile
            || state.rescan
            || (family.depends_on_clock() && state.start.time.time != at.cut.time.time);
        let root = if !rescan
            && !state
                .dirty
                .values()
                .any(|r| r.collection == family.collection())
        {
            state.baseline[state.walk.family].clone()
        } else {
            let done = if rescan {
                source_page(store, state, at, family).await?
            } else {
                merge_page(store, state, at, family).await?
            };
            if !done {
                return Ok(false);
            }
            store
                .put("family", &state.walk.summary, state.expires_at)
                .await?
        };
        if reconcile {
            state.published.push(root);
        } else {
            state.baseline.push(root);
        }
        state.walk = Walk {
            family: state.walk.family + 1,
            ..Default::default()
        };
    }
    Ok(false)
}

/// Re-open and recount every linked partition before publishing. Family
/// labels and all zero families participate in the sealed root in registry order.
pub(super) async fn verify(store: &Store<'_>, state: &mut Checkpoint) -> AppResult<bool> {
    let families: Vec<_> = observation::families().collect();
    if state.published.len() != families.len() {
        return Err(malformed());
    }
    for _ in 0..8 {
        let Some(family) = families.get(state.walk.family) else {
            state.walk = Walk::default();
            return Ok(true);
        };
        let report: FamilyReport = store
            .get("family", &state.published[state.walk.family])
            .await?;
        if report.family != family.name() || report.collection != family.collection() {
            return Err(malformed());
        }
        if !state.walk.started {
            state.walk.link = report.head.clone();
            state.walk.started = true;
        }
        if let Some(hash) = state.walk.link.take() {
            let part: Partition = store.get("rows", &hash).await?;
            for receipt in part.rows {
                if receipt.row.collection != family.collection()
                    || state.token(&receipt.row)? != receipt.token
                {
                    return Err(malformed());
                }
                state.walk.summary.include(&receipt)?;
            }
            state.walk.summary.pages += 1;
            state.walk.link = part.previous;
            return Ok(false);
        }
        if state.walk.summary.totals != report.totals
            || state.walk.summary.pages != report.pages
            || state.walk.summary.buckets != report.buckets
        {
            return Err(malformed());
        }
        state.totals.include(&report.totals)?;
        state.walk = Walk {
            family: state.walk.family + 1,
            ..Default::default()
        };
    }
    Ok(false)
}
