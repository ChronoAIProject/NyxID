use super::*;

#[tokio::test]
#[ignore = "contention probe; run through cargo-slot.sh: cargo +1.98.1 test -p nyxid --bin nyxid-server sa_bulk_revoke_scale -- --ignored --nocapture --test-threads 1"]
async fn sa_bulk_revoke_scale() {
    for repetition in 1..=3 {
        let f = fixture().await;
        let db = &f.state.db;
        db.collection::<bson::Document>(SA_TOKENS)
            .delete_many(doc! {})
            .await
            .unwrap();
        let ids: Vec<_> = (0..20_000).map(|_| Uuid::new_v4().to_string()).collect();
        db.collection::<bson::Document>(SA_TOKENS).insert_many(ids.iter().map(|id|doc!{
            "_id":id,"jti":id,"service_account_id":&f.sa.id,"scope":"proxy","revoked":false,"credential_generation":0_i64,
            "created_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::hours(1))
        })).await.unwrap();
        for (keys, options) in [
            (doc! {"service_account_id":1}, None),
            (
                doc! {"jti":1},
                Some(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                ),
            ),
            (
                doc! {"expires_at":1},
                Some(
                    mongodb::options::IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                ),
            ),
        ] {
            db.collection::<bson::Document>(SA_TOKENS)
                .create_index(
                    mongodb::IndexModel::builder()
                        .keys(keys)
                        .options(options)
                        .build(),
                )
                .await
                .unwrap();
        }
        let stop = tokio_util::sync::CancellationToken::new();
        let done = stop.clone();
        let writer_db = db.clone();
        let subset: Vec<_> = (0..1000)
            .map(|i| ids[(i * 7919) % 20_000].clone())
            .collect();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let writer = crate::services::shutdown::process().spawn(async move {
            let mut ready = Some(ready_tx);
            let mut writes = 0;
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(40));
            loop {
                tokio::select! {_=done.cancelled()=>break,_=interval.tick()=>{}}
                writer_db
                    .collection::<bson::Document>(SA_TOKENS)
                    .update_one(
                        doc! {"_id":&subset[writes%1000]},
                        doc! {"$inc":{"probe_touch":1_i64}},
                    )
                    .await
                    .unwrap();
                writes += 1;
                if let Some(ready) = ready.take() {
                    let _ = ready.send(());
                }
            }
            writes
        });
        ready_rx.await.unwrap();
        let actor = audit_service::AuditActor::from_auth_user(&auth(&f));
        let start = std::time::Instant::now();
        let result = REQUEST
            .scope((), revoke_all_tokens(db, &f.sa.id, true, Some(&actor)))
            .await;
        let elapsed = start.elapsed();
        stop.cancel();
        let writes = writer.await.unwrap();
        let attempts = f
            .monitor
            .lock()
            .unwrap()
            .commands
            .iter()
            .filter(|(name, collection)| name == "update" && collection == SERVICE_ACCOUNTS)
            .count();
        println!(
            "SA_REVOKE_SCALE repetition={repetition} tokens=20000 elapsed={elapsed:?} retries={} concurrent_writes={writes} result={result:?}",
            attempts.saturating_sub(1)
        );
        assert_eq!(result.unwrap(), 20_000);
        assert_eq!(
            get_service_account(db, &f.sa.id)
                .await
                .unwrap()
                .credential_generation,
            1
        );
        assert_eq!(
            db.collection::<bson::Document>(SA_TOKENS)
                .count_documents(doc! {"revoked":false})
                .await
                .unwrap(),
            0
        );
        assert_eq!(rows(db, INTENTS).await.len(), 1);
        db.drop().await.unwrap();
    }
}
