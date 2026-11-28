use super::{
    replay::{Dependents, Observations, Snapshot},
    *,
};
use crate::test_utils::{connect_transaction_test_database, test_encryption_keys};
use std::collections::BTreeMap;

const CHILD: &str = "census_test_children";
const PARENT: &str = "census_test_parents";

fn scope() -> Scope {
    Scope {
        registry_digest: "two-fixture-families-v1".into(),
        restore_epoch: "original".into(),
        deployment_revision: "deployment-one".into(),
        namespaces: [CHILD.into(), PARENT.into()].into(),
    }
}

async fn fixture() -> Database {
    let db = connect_transaction_test_database("census_stream").await;
    for name in [PARENT, CHILD] {
        db.create_collection(name).await.unwrap();
    }
    db
}

// CI starts mongod with zero retained snapshot history. Serialize the small
// set of history-dependent tests and restore the server setting on unwind.
pub(in crate::services) async fn with_history(
    db: &Database,
    seconds: i32,
    body: impl Future<Output = ()>,
) {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = LOCK.lock().await;
    with_parameter(db, "minSnapshotHistoryWindowInSeconds", seconds, body).await;
}

async fn with_parameter(db: &Database, name: &str, value: i32, body: impl Future<Output = ()>) {
    use futures::FutureExt;
    let admin = db.client().database("admin");
    let old = admin
        .run_command(doc! {"setParameter":1, name:value})
        .await
        .unwrap()
        .get("was")
        .unwrap()
        .clone();
    let result = std::panic::AssertUnwindSafe(body).catch_unwind().await;
    admin
        .run_command(doc! {"setParameter":1, name:old})
        .await
        .unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

fn control_keys() -> EncryptionKeys {
    let mut keys = test_encryption_keys();
    keys.set_contextual_record_writes(true);
    keys
}

async fn witness(census: &Census<'_>, marker: &str) -> Cut {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(cut) = census.capture().await.unwrap()
                && cut.marker == marker
            {
                return cut;
            }
        }
    })
    .await
    .expect("marker must arrive")
}

// Each child has one material instance. A missing parent is one unresolved
// finding. Retain the previous edges even when their parent was absent.
#[derive(Clone, Default)]
struct Fixture {
    rows: BTreeMap<String, (String, Option<i32>)>,
}

impl Fixture {
    async fn evaluate(at: &Snapshot<'_>, source: &Document) -> AppResult<(String, Option<i32>)> {
        let parent = source.get_str("parent").unwrap();
        let related = at
            .find(
                PARENT,
                doc! {"$or":[{"_id":parent},{"alias":parent}]},
                doc! {},
            )
            .await?
            .try_next()
            .await?;
        Ok(match related {
            Some(row) => (
                row.get_str("_id").unwrap().into(),
                Some(row.get_i32("key").unwrap()),
            ),
            None => (parent.into(), None),
        })
    }

    async fn full(at: &Snapshot<'_>) -> AppResult<Self> {
        let mut rows = BTreeMap::new();
        let mut cursor = at.find(CHILD, doc! {}, doc! {}).await?;
        while let Some(source) = cursor.try_next().await? {
            rows.insert(
                source.get_str("_id").unwrap().into(),
                Self::evaluate(at, &source).await?,
            );
        }
        Ok(Self { rows })
    }

    fn totals(&self) -> BTreeMap<Option<i32>, usize> {
        let mut counts = BTreeMap::new();
        for (_, key) in self.rows.values() {
            *counts.entry(*key).or_insert(0) += 1;
        }
        counts
    }
}

#[async_trait::async_trait]
impl Observations for Fixture {
    async fn previous_dependents(&self, _: &Cut, parent: &Row) -> AppResult<Dependents> {
        let rows = self
            .rows
            .iter()
            .filter(|(_, v)| {
                parent.collection == PARENT && Some(v.0.as_str()) == parent.id.as_str()
            })
            .map(|(id, _)| Row {
                collection: CHILD.into(),
                id: id.clone().into(),
            })
            .collect();
        Ok(Dependents::Rows(rows))
    }
    async fn current_dependents(&self, at: &Snapshot<'_>, parent: &Row) -> AppResult<Dependents> {
        let mut rows = vec![];
        if parent.collection == PARENT {
            let mut join_keys = vec![parent.id.clone()];
            if let Some(row) = at.row(parent).await?
                && let Some(alias) = row.get("alias")
            {
                join_keys.push(alias.clone());
            }
            let mut cursor = at
                .find(CHILD, doc! {"parent":{"$in":join_keys}}, doc! {"_id":1})
                .await?;
            while let Some(d) = cursor.try_next().await? {
                rows.push(Row {
                    collection: CHILD.into(),
                    id: d["_id"].clone(),
                });
            }
        }
        Ok(Dependents::Rows(rows))
    }
    async fn replace(
        &mut self,
        at: &Snapshot<'_>,
        row: &Row,
        source: Option<Document>,
    ) -> AppResult<()> {
        if row.collection == CHILD {
            let id = row.id.as_str().unwrap();
            if let Some(d) = source {
                self.rows.insert(id.into(), Self::evaluate(at, &d).await?);
            } else {
                self.rows.remove(id);
            }
        }
        Ok(())
    }
    async fn rescan(&mut self, at: &Snapshot<'_>) -> AppResult<()> {
        *self = Self::full(at).await?;
        Ok(())
    }
}

#[tokio::test]
async fn stream_replays_the_exact_cut_after_crashes_and_split_transaction_batches() {
    let db = fixture().await;
    with_history(&db, 120, async {
        let keys = control_keys();
        let parents = db.collection::<Document>(PARENT);
        let children = db.collection::<Document>(CHILD);
        parents.insert_one(doc! {"_id":"a","key":1}).await.unwrap();
        children
            .insert_many([
                doc! {"_id":"middle","parent":"a"},
                doc! {"_id":"z","parent":"missing"},
                doc! {"_id":"relink","parent":"a"},
                doc! {"_id":"disappears","parent":"missing"},
            ])
            .await
            .unwrap();
        let (census, start) = Census::start(&db, &keys, scope()).await.unwrap();
        let mut staged = Fixture::full(&census.snapshot(&start).await.unwrap())
            .await
            .unwrap();
        assert_eq!(staged.totals(), [(None, 2), (Some(1), 2)].into());
        let mut tx = db.client().start_session().await.unwrap();
        tx.start_transaction()
            .write_concern(WriteConcern::majority())
            .await
            .unwrap();
        children
            .insert_many((0..(PAGE + 2)).map(|i| doc! {"_id":format!("00-{i}"),"parent":"a"}))
            .session(&mut tx)
            .await
            .unwrap();
        parents
            .replace_one(doc! {"_id":"a"}, doc! {"key":2})
            .session(&mut tx)
            .await
            .unwrap();
        transaction_service::commit(&mut tx).await.unwrap();
        children.delete_one(doc! {"_id":"middle"}).await.unwrap();
        children
            .insert_one(doc! {"_id":"middle","parent":"b"})
            .await
            .unwrap();
        parents
            .insert_one(doc! {"_id":"missing","key":3})
            .await
            .unwrap();
        children
            .update_one(doc! {"_id":"relink"}, doc! {"$set":{"parent":"missing"}})
            .await
            .unwrap();
        children
            .delete_one(doc! {"_id":"disappears"})
            .await
            .unwrap();
        let marker = census.request_cut().await.unwrap();
        // One worker reads a page and dies before committing its new token.
        let state = census.state().await.unwrap();
        assert!(
            census
                .collect_page(
                    census.read_page(0).await.unwrap().token,
                    state.get_document("catalog").unwrap(),
                    1,
                    false
                )
                .await
                .unwrap()
                .unwrap()
                .cut
                .is_none()
        );
        assert!(census.capture().await.unwrap().is_none());
        let resumed = Census::resume(&db, &keys, census.generation(), scope())
            .await
            .unwrap();
        let end = witness(&resumed, &marker).await;
        // Simulate the committed marker acknowledgement being lost.
        assert_eq!(census.cut(&marker).await.unwrap().unwrap().time, end.time);
        parents.delete_one(doc! {"_id":"a"}).await.unwrap();
        census.reconcile(&start, &end, &mut staged).await.unwrap();
        let independent = Fixture::full(&census.snapshot(&end).await.unwrap())
            .await
            .unwrap();
        assert_eq!(staged.rows, independent.rows);
        assert_eq!(
            staged.totals(),
            [(None, 1), (Some(2), (PAGE + 2) as usize), (Some(3), 2)].into()
        );
        let latest = witness(&census, &census.request_cut().await.unwrap()).await;
        census.reconcile(&end, &latest, &mut staged).await.unwrap();
        assert_eq!(
            staged.totals(),
            [(None, (PAGE + 3) as usize), (Some(3), 2)].into()
        );
        parents
            .insert_one(doc! {"_id":"alias-owner","alias":"before","key":4})
            .await
            .unwrap();
        children
            .insert_many([
                doc! {"_id":"old-dependent","parent":"before"},
                doc! {"_id":"new-dependent","parent":"after"},
            ])
            .await
            .unwrap();
        let before = witness(&census, &census.request_cut().await.unwrap()).await;
        census
            .reconcile(&latest, &before, &mut staged)
            .await
            .unwrap();
        assert_eq!(staged.rows["old-dependent"].1, Some(4));
        assert_eq!(staged.rows["new-dependent"].1, None);
        parents
            .update_one(doc! {"_id":"alias-owner"}, doc! {"$set":{"alias":"after"}})
            .await
            .unwrap();
        let after = witness(&census, &census.request_cut().await.unwrap()).await;
        census
            .reconcile(&before, &after, &mut staged)
            .await
            .unwrap();
        assert_eq!(staged.rows["old-dependent"].1, None);
        assert_eq!(staged.rows["new-dependent"].1, Some(4));
        assert_eq!(
            staged.rows,
            Fixture::full(&census.snapshot(&after).await.unwrap())
                .await
                .unwrap()
                .rows
        );
        parents.insert_one(doc! {"_id": 7_i32}).await.unwrap();
        parents.delete_one(doc! {"_id": 7_i32}).await.unwrap();
        parents.insert_one(doc! {"_id": 7_i64}).await.unwrap();
        let typed = witness(&census, &census.request_cut().await.unwrap()).await;
        let at = census.snapshot(&typed).await.unwrap();
        let prior = Row {
            collection: PARENT.into(),
            id: 7_i32.into(),
        };
        assert!(at.row(&prior).await.unwrap().is_none());
        assert!(
            at.row(&Row {
                id: 7_i64.into(),
                ..prior
            })
            .await
            .unwrap()
            .is_some()
        );
    })
    .await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn catalog_and_epoch_changes_persistently_invalidate_generations() {
    let db = fixture().await;
    let keys = control_keys();
    let (census, _) = Census::start(&db, &keys, scope()).await.unwrap();
    db.collection::<Document>(CHILD).drop().await.unwrap();
    db.create_collection(CHILD).await.unwrap();
    assert!(census.capture().await.is_err());
    assert!(census.request_cut().await.is_err());
    let (census, _) = Census::start(&db, &keys, scope()).await.unwrap();
    let mut restored = scope();
    restored.restore_epoch = "restored".into();
    assert!(
        Census::resume(&db, &keys, census.generation(), restored)
            .await
            .is_err()
    );
    assert!(
        Census::resume(&db, &keys, census.generation(), scope())
            .await
            .is_err()
    );
    let (census, _) = Census::start(&db, &keys, scope()).await.unwrap();
    census
        .log()
        .update_one(
            doc! {"_id": census.generation()},
            doc! {"$set":{"database_identity.database":"before-import"}},
        )
        .await
        .unwrap();
    assert!(
        Census::resume(&db, &keys, census.generation(), scope())
            .await
            .is_err()
    );
    assert!(census.request_cut().await.is_err());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn ttl_deletes_reconcile_each_commit_order() {
    let db = fixture().await;
    with_history(
        &db,
        15,
        with_parameter(&db, "ttlMonitorSleepSecs", 1, async {
            let keys = control_keys();
            for name in [PARENT, CHILD] {
                db.collection::<Document>(name)
                    .create_index(
                        IndexModel::builder()
                            .keys(doc! {"expires_at":1})
                            .options(IndexOptions::builder().expire_after(Duration::ZERO).build())
                            .build(),
                    )
                    .await
                    .unwrap();
            }
            let parents = db.collection::<Document>(PARENT);
            let children = db.collection::<Document>(CHILD);
            for id in ["parent-first", "child-first"] {
                parents.insert_one(doc! {"_id":id,"key":1}).await.unwrap();
                children
                    .insert_one(doc! {"_id":id,"parent":id})
                    .await
                    .unwrap();
            }
            let (census, start) = Census::start(&db, &keys, scope()).await.unwrap();
            let mut staged = Fixture::full(&census.snapshot(&start).await.unwrap())
                .await
                .unwrap();
            for (name, id) in [(PARENT, "parent-first"), (CHILD, "child-first")] {
                db.collection::<Document>(name)
                    .update_one(
                        doc! {"_id":id},
                        doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
                    )
                    .await
                    .unwrap();
            }
            tokio::time::timeout(Duration::from_secs(100), async {
                while parents
                    .find_one(doc! {"_id":"parent-first"})
                    .await
                    .unwrap()
                    .is_some()
                    || children
                        .find_one(doc! {"_id":"child-first"})
                        .await
                        .unwrap()
                        .is_some()
                {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            })
            .await
            .unwrap();
            let end = witness(&census, &census.request_cut().await.unwrap()).await;
            census.reconcile(&start, &end, &mut staged).await.unwrap();
            assert_eq!(staged.totals(), [(None, 1)].into());
            assert_eq!(
                staged.rows,
                Fixture::full(&census.snapshot(&end).await.unwrap())
                    .await
                    .unwrap()
                    .rows
            );
        }),
    )
    .await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lost_resume_token_and_missing_page_refuse_replay() {
    let db = fixture().await;
    let keys = control_keys();
    let (census, start) = Census::start(&db, &keys, scope()).await.unwrap();
    let mut page = census.read_page(0).await.unwrap();
    page.token = bson::from_bson(Bson::Document(doc! {"_data":"invalid"})).unwrap();
    census
        .log()
        .update_one(
            doc! {"_id":census.page_id(0)},
            doc! {"$set":{"batch":census.seal(0,&page).await.unwrap()}},
        )
        .await
        .unwrap();
    assert!(census.capture().await.is_err());
    assert!(census.snapshot(&start).await.is_err());
    let (census, _) = Census::start(&db, &keys, scope()).await.unwrap();
    census
        .log()
        .update_one(doc! {"_id":census.page_id(0)}, doc! {"$unset":{"batch":""}})
        .await
        .unwrap();
    assert!(census.capture().await.is_err());
    assert!(census.request_cut().await.is_err());
    let (census, start) = Census::start(&db, &keys, scope()).await.unwrap();
    let middle = witness(&census, &census.request_cut().await.unwrap()).await;
    let end = witness(&census, &census.request_cut().await.unwrap()).await;
    census
        .log()
        .delete_one(doc! {"_id":census.page_id(middle.page)})
        .await
        .unwrap();
    assert!(
        census
            .reconcile(&start, &end, &mut Fixture::default())
            .await
            .is_err()
    );
    assert!(census.request_cut().await.is_err());
    db.drop().await.unwrap();
}

mod load;
mod topology;

#[tokio::test]
async fn expired_baseline_refuses_and_invalidates_even_before_replay() {
    let db = fixture().await;
    let keys = control_keys();
    with_history(&db, 0, async {
        let (census, start) = Census::start(&db, &keys, scope()).await.unwrap();
        let at = census.snapshot(&start).await.unwrap();
        tokio::time::timeout(Duration::from_secs(120), async {
            for i in 0.. {
                db.collection::<Document>(CHILD)
                    .insert_one(doc! {"_id":i})
                    .await
                    .unwrap();
                if at
                    .row(&Row {
                        collection: CHILD.into(),
                        id: 0.into(),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        assert!(census.request_cut().await.is_err());
    })
    .await;
    db.drop().await.unwrap();
}
