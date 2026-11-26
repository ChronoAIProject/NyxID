use super::*;

// Requires the disposable three-member rehearsal set. Only its designated
// primary is stepped down. Ordinary CI shares one mongod among concurrent tests.
#[tokio::test]
#[ignore = "opt-in primary handoff on a disposable three-member replica set"]
async fn resumes_on_a_different_elected_primary() {
    let db = fixture().await;
    let admin = db.client().database("admin");
    let before = admin.run_command(doc! {"hello":1}).await.unwrap();
    assert_eq!(before.get_array("hosts").unwrap().len(), 3);
    let keys = control_keys();
    let (census, start) = Census::start(&db, &keys, scope()).await.unwrap();
    db.collection::<Document>(PARENT)
        .insert_one(doc! {"_id":"a","key":1})
        .await
        .unwrap();
    let _ = admin
        .run_command(doc! {"replSetStepDown":45,"secondaryCatchUpPeriodSecs":5})
        .await;
    tokio::time::timeout(Duration::from_secs(40), async {
        loop {
            if let Ok(hello) = admin.run_command(doc! {"hello":1}).await
                && hello.get_bool("isWritablePrimary") == Ok(true)
                && hello
                    .get_str("primary")
                    .is_ok_and(|host| Some(host) != before.get_str("primary").ok())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("another member won election");
    db.collection::<Document>(CHILD)
        .insert_one(doc! {"_id":"z","parent":"a"})
        .await
        .unwrap();
    let census = Census::resume(&db, &keys, census.generation(), scope())
        .await
        .unwrap();
    let end = witness(&census, &census.request_cut().await.unwrap()).await;
    let mut staged = Fixture::default();
    census.reconcile(&start, &end, &mut staged).await.unwrap();
    assert_eq!(staged.totals(), [(Some(1), 1)].into());
    db.drop().await.unwrap();
}

// The runner provisions a small oplog. Verify its capacity before generating
// disposable fixture writes and confirm that the baseline actually fell off.
#[tokio::test]
#[ignore = "opt-in oplog rollover on a disposable small-oplog replica set"]
async fn oplog_rollover_invalidates_instead_of_restarting_at_now() {
    let db = fixture().await;
    let local = db.client().database("local");
    let stats = local
        .run_command(doc! {"collStats":"oplog.rs"})
        .await
        .unwrap();
    let cap = stats
        .get("maxSize")
        .unwrap()
        .as_i64()
        .unwrap_or_else(|| i64::from(stats.get_i32("maxSize").unwrap()));
    assert!(
        cap <= 64 * 1024 * 1024,
        "requires disposable small-oplog replica set"
    );
    let keys = control_keys();
    let (census, start) = Census::start(&db, &keys, scope()).await.unwrap();
    let filler = db.collection::<Document>("census_test_oplog_filler");
    let value = Bson::Binary(bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes: vec![123; 1024 * 1024],
    });
    let op = local.collection::<Document>("oplog.rs");
    tokio::time::timeout(Duration::from_secs(420), async {
        for batch in 0.. {
            let oldest = op
                .find_one(doc! {})
                .sort(doc! {"$natural":1})
                .projection(doc! {"ts":1})
                .await
                .unwrap()
                .unwrap();
            if oldest.get_timestamp("ts").unwrap() > start.time {
                return;
            }
            if batch < 48 {
                filler
                    .insert_many((0..4).map(|_| doc! {"payload":&value}))
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .unwrap();
    assert!(census.capture().await.is_err());
    assert!(census.request_cut().await.is_err());
    db.drop().await.unwrap();
}
