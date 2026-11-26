use super::*;
use std::time::Duration;

const FAILPOINT: &str = "hangAfterCollectionInserts";
const BLOCKER: &str = "approval_causal_blocker";
const DEADLINE: Duration = Duration::from_secs(30);

#[tokio::test]
async fn decisions_see_the_pending_approval_during_a_concurrent_insert() {
    let web = decision_during_insert(true).await;
    let programmatic = decision_during_insert(false).await;
    let approved = (Ok(()), "approved".to_string(), 1);
    assert_eq!([web, programmatic], [approved.clone(), approved]);
}

async fn decision_during_insert(web: bool) -> (Result<(), String>, String, usize) {
    let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
    let f = Fixture::with_command_observer(move |event| {
        if let CommandEvent::Started(e) = event {
            let after = e
                .command
                .get_document("readConcern")
                .ok()
                .and_then(|rc| rc.get_timestamp("afterClusterTime").ok());
            if e.command.get_str("findAndModify") == Ok(REQUESTS)
                || e.command_name == "commitTransaction"
            {
                let _ = tx.send((after, e.command_name == "commitTransaction"));
            }
        }
    })
    .await;
    Web.prepare(&f, false).await;
    let db = &f.state.db;
    let card = db
        .collection::<Document>(REQUESTS)
        .find_one_and_delete(doc! {"_id": &f.request_id})
        .await
        .unwrap()
        .unwrap();
    db.create_collection(BLOCKER).await.unwrap();
    let admin = db.client().database("admin");
    let armed = admin
        .run_command(doc! {
            "configureFailPoint": FAILPOINT, "mode": "alwaysOn",
            "data": {"collectionNS": format!("{}.{BLOCKER}", db.name())}
        })
        .await
        .unwrap();
    let previous_hits = armed
        .get_i32("count")
        .map(i64::from)
        .or_else(|_| armed.get_i64("count"))
        .unwrap();
    let blocker_db = db.clone();
    let blocker = tokio::spawn(async move {
        blocker_db
            .run_command(doc! {"insert": BLOCKER, "documents": [{"_id": "held"}]})
            .await
    });
    let decide = async {
        if web {
            Web.call(&f, f.auth()).await
        } else {
            approval_service::process_decision(
                db,
                &f.state.config,
                &f.state.http_client,
                None,
                None,
                &f.request_id,
                true,
                None,
                None,
                "web",
            )
            .await
            .map(|_| ())
        }
    };
    let result = AssertUnwindSafe(tokio::time::timeout(DEADLINE, async {
        admin.run_command(doc! {"waitForFailPoint": FAILPOINT, "timesEntered": previous_hits + 1, "maxTimeMS": 30000}).await.unwrap();
        let inserted = db.run_command(doc! {"insert": REQUESTS, "documents": [card], "writeConcern": {"w": 1}}).await.unwrap();
        assert_eq!(inserted.get_i32("n"), Ok(1));
        let insert_time = inserted.get_timestamp("operationTime").unwrap();
        let release = async {
            // A causal transaction can now wait for this insert. An unanchored
            // transaction has already chosen its snapshot when it tries to commit.
            while let Some((after, committing)) = commands.recv().await {
                if after.is_some_and(|t| t >= insert_time) || committing {
                    break;
                }
            }
            admin.run_command(doc! {"configureFailPoint": FAILPOINT, "mode": "off"}).await.unwrap();
        };
        tokio::join!(REQUEST.scope((), decide), release).0
    })).catch_unwind().await;
    // Clear the server-wide failpoint even after a failed assertion or timeout.
    admin
        .run_command(doc! {"configureFailPoint": FAILPOINT, "mode": "off"})
        .await
        .unwrap();
    blocker.await.unwrap().unwrap();
    let result = result
        .unwrap()
        .expect("decision did not reach its transaction boundary");
    let stored = approval_service::get_request(db, &f.request_id)
        .await
        .unwrap();
    let intents = f.rows(INTENTS).await.len();
    crate::test_utils::drop_test_database(db).await;
    (result.map_err(|e| e.to_string()), stored.status, intents)
}
