use super::*;
use crate::services::{assistant_team_service as team, org_channel_service as org};

async fn notice_member(f: &Fixture) -> String {
    let actor = Uuid::new_v4().to_string();
    let db = &f.state.db;
    db.collection(USERS)
        .insert_one(test_user(&actor, UserType::Person))
        .await
        .unwrap();
    db.collection("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            f.row.bot_owner_id.as_deref().unwrap(),
            &actor,
            crate::models::org_membership::OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    let challenge = org::links::issue(db, &actor, &f.row).await.unwrap();
    assert!(
        org::links::receive(
            db,
            &f.row,
            "human",
            challenge["code"].as_str().unwrap(),
            None
        )
        .await
        .unwrap()
    );
    org::links::confirm(db, &actor, &f.row, challenge["id"].as_str().unwrap())
        .await
        .unwrap();
    actor
}

async fn configure_group(f: &Fixture) {
    let parent = chats::record_chat(
        &f.state,
        &f.row,
        &chats::group_partition("oc_chat", None),
        &chats::ChatFacts {
            kind: "group",
            chat_id: "oc_chat".into(),
            thread_id: None,
            owner: true,
            title: None,
        },
        None,
    )
    .await
    .unwrap();
    f.state.db.collection::<bson::Document>(THREADS).update_one(doc!{"_id":&parent.id},
        doc!{"$set":{"members":"everyone","reply_mode":"mention","owner_seen":true,"threads":"follow"}}).await.unwrap();
}

#[tokio::test]
async fn org_notice_2625_member_service_refusal_legacy_live_reaches_exact_root() {
    Box::pin(async {
        let f = Box::pin(Fixture::with_org("org_notice_legacy", true)).await;
        notice_member(&f).await;
        configure_group(&f).await;
        let (body, source) = f
            .input(
                "/service missing-connection\nRead my messages",
                ThreadAddress::Mention,
                false,
                true,
            )
            .await;
        let response = Box::pin(respond(&f.state, &f.key, &body, &source)).await;
        assert!(!response.headers().contains_key("x-cma-response-id"));
        let lifecycle = frames(&body_text(response).await);
        Box::pin(deliver_prepared_notice(&f, &source, &lifecycle)).await;

        let replica = Box::pin(Fixture::with_org("org_notice_service_replica", true)).await;
        notice_member(&replica).await;
        configure_group(&replica).await;
        advertised(&replica).await;
        let captured = Box::pin(capture(&replica, SERVICE_REQUEST, Consumer::Replica)).await;
        let text = Box::pin(recovered(&replica, &captured)).await;
        assert!(text.starts_with("connection_unavailable:"));
        assert_kind(&replica, &captured, "service_selection").await;
        Box::pin(deliver(&replica, &captured, &text)).await;
    })
    .await;
}

const SERVICE_REQUEST: &str = "/service missing-connection\nRead my messages";
const FOLLOW_BUSY: &str = "I'm busy right now. Please try again in a moment.";
const GUEST_BUSY: &str = "I'm answering another message right now. Please try again in a moment.";
const START_FAILURE: &str = "I could not start that. Please try again.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Consumer {
    Live,
    Replica,
}

struct Captured {
    source: String,
    id: String,
    request: Value,
    live: Option<String>,
    created: Value,
}

fn notice_output(response: &Value) -> String {
    assert_eq!(response["status"], "completed");
    assert_eq!(response["output"].as_array().unwrap().len(), 1);
    let content = &response["output"][0]["content"];
    assert_eq!(content.as_array().unwrap().len(), 1);
    assert_eq!(content[0]["type"], "output_text");
    content[0]["text"].as_str().unwrap().to_owned()
}

async fn capture(f: &Fixture, input: &str, consumer: Consumer) -> Captured {
    let (mut request, source) = f.input(input, ThreadAddress::Mention, false, true).await;
    metadata(&mut request, &source);
    let response = Box::pin(respond(&f.state, &f.key, &request, &source)).await;
    let (parts, mut body) = response.into_parts();
    let accepted = submit_frozen_fixture(
        parts.status,
        &parts.headers,
        &first(&mut body).await,
        &request["metadata"],
    );
    assert_eq!(accepted["thread_reply"], true);
    let id = accepted["id"].as_str().unwrap().to_owned();
    let live = if consumer == Consumer::Replica {
        drop(body);
        None
    } else {
        let rest = frames(&body_text(Response::new(body)).await);
        Some(notice_output(&rest.last().unwrap()["response"]))
    };
    let row: NyxbotEvent = f
        .state
        .db
        .collection(EVENTS)
        .find_one(doc! {"event_id":&source, "channel_id":&f.row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "refused");
    assert!(row.turn_id.is_none() && row.conversation_id.is_none());
    let origin = &row.delivery.as_ref().unwrap().origin;
    assert!(origin.org_execution.is_some());
    assert_eq!(origin.thread.as_ref().unwrap().source_message_id, source);
    let created = json!({"sequence_number":0,"type":"response.created","response":accepted});
    Captured {
        source,
        id,
        request,
        live,
        created,
    }
}

async fn recovered(f: &Fixture, c: &Captured) -> String {
    let result = snapshot(f, &c.id).await;
    let replay = native_responses::events(
        State(f.state.clone()),
        Path(c.id.clone()),
        axum::extract::Query(native_responses::Cursor {
            starting_after: None,
        }),
        bearer(&f.key),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let history = frames(&body_text(replay).await);
    assert_eq!(history[0]["response"]["thread_reply"], true);
    assert_eq!(history.last().unwrap()["response"], result);
    let repeated = Box::pin(respond(&f.state, &f.key, &c.request, &c.source)).await;
    assert_eq!(repeated.status(), StatusCode::OK);
    assert_eq!(frames(&body_text(repeated).await), history);
    notice_output(&result)
}

async fn platform_reply(f: &Fixture, text: &str, count: usize) {
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages/om_root/reply"))
        .and(body_partial_json(
            json!({"reply_in_thread":true,"content":json!({"text":text}).to_string()}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"data":{"message_id":"om_notice"}})),
        )
        .expect(count as u64)
        .mount(&f.platform)
        .await;
}

async fn callback(f: &Fixture, source: &str, reply: Value, headers: &HeaderMap) -> StatusCode {
    let response = Box::pin(post_thread_body(
        &f.state, &f.row, source, headers, reply, false,
    ))
    .await;
    response.status()
}

async fn deliver(f: &Fixture, c: &Captured, text: &str) {
    platform_reply(f, text, 1).await;
    let (token, _) = thread_reply_headers(&f.state, &f.row, &c.source);
    let reply = if c.live.is_some() {
        json!({"text":text})
    } else {
        json!({"text":text,"metadata":{"text_format":"commonmark"}})
    };
    assert_eq!(
        callback(f, &c.source, reply.clone(), &token).await,
        StatusCode::OK
    );
    let after = reply_state(f, &c.source).await;
    assert_eq!(after.get_array("receipts").unwrap().len(), 1);
    assert!(after.get_array("audits").unwrap().is_empty());
    assert!(!callback(f, &c.source, reply, &token).await.is_success());
    assert_eq!(reply_state(f, &c.source).await, after);
    f.platform.verify().await;
}

async fn assert_closed(f: &Fixture, source: &str) {
    let bot = channel_bot_service::get_bot(&f.state.db, &f.row.channel_bot_id)
        .await
        .unwrap();
    assert_eq!(
        bot.diagnostics
            .last_outbound_error
            .as_ref()
            .and_then(|e| e.thread_reason.as_deref()),
        Some("thread_target_rejected")
    );
    let state = reply_state(f, source).await;
    assert!(state.get_array("receipts").unwrap().is_empty());
    assert_eq!(state.get_array("audits").unwrap().len(), 1);
    assert!(
        f.platform
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !r.url.path().ends_with("/reply"))
    );
}

async fn provision_guest(f: &mut Fixture) {
    let org_id = f.row.bot_owner_id.clone().unwrap();
    f.state.billing = Arc::new(crate::billing_integration_tests::billing_service(
        &f.state.db,
        Arc::new(crate::billing_integration_tests::FakeLago::default()),
        1000,
    ));
    f.state.billing.ensure_wallet(&org_id).await.unwrap();
    org::management::update(
        &f.state.db,
        OWNER,
        &f.row,
        org::management::GuestSettings {
            enabled: true,
            expected_revision: None,
            service_ids: Some(vec![]),
            daily_credits: Some(crate::models::credits::Credits::from_whole(1)),
        },
        None,
    )
    .await
    .unwrap();
    f.row = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
    let agent = team::agent(&f.state.db, &org_id, f.row.agent_id.as_deref().unwrap())
        .await
        .unwrap();
    // A previous hidden setup is already in progress. The source arriving while
    // the org channel pool is full must get a fixed refusal without entering the queue.
    let until = bson::DateTime::from_chrono(Utc::now() + ChronoDuration::minutes(10));
    f.state.db.collection::<bson::Document>(crate::services::assistant_service_execution::COLLECTION)
        .insert_one(doc!{"_id":Uuid::new_v4().to_string(),"kind":"guest_model_setup","user_id":org_id,
            "agent_id":&agent.id,"profile":agent.model,"status":"setting_up","until":until,"expires_at":until}).await.unwrap();
}

async fn pool_full(f: &Fixture, actor: &str) -> Vec<crate::mw::rate_limit::DirectChatPermit> {
    let mut permits = Vec::new();
    for _ in 0..team::CHANNEL_TURNS_PER_OWNER {
        permits.push(
            f.state
                .direct_chat_limiter
                .try_acquire_pool("assistant_channel", actor, team::CHANNEL_TURNS_PER_OWNER)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    permits
}

async fn guest_busy(f: &Fixture, consumer: Consumer) -> Captured {
    let org_id = f.row.bot_owner_id.as_deref().unwrap();
    let permits = pool_full(f, org_id).await;
    let c = Box::pin(capture(f, "An addressed question", consumer)).await;
    drop(permits);
    c
}

#[tokio::test]
async fn org_notice_2625_member_and_guest_follow_busy_reaches_root_for_both_consumers() {
    Box::pin(async {
        for guest in [false, true] {
            for consumer in [Consumer::Live, Consumer::Replica] {
                let mut f = Box::pin(Fixture::with_org("org_notice_follow_busy", true)).await;
                let actor = if guest {
                    Box::pin(provision_guest(&mut f)).await;
                    f.row.bot_owner_id.clone().unwrap()
                } else {
                    notice_member(&f).await
                };
                configure_group(&f).await;
                advertised(&f).await;
                let permits = pool_full(&f, &actor).await;
                let c = Box::pin(capture(&f, "Hello", consumer)).await;
                drop(permits);
                let text = if let Some(text) = c.live.clone() {
                    text
                } else {
                    Box::pin(recovered(&f, &c)).await
                };
                assert_eq!(text, FOLLOW_BUSY);
                assert_kind(&f, &c, "follow_busy").await;
                Box::pin(deliver(&f, &c, &text)).await;
            }
        }
    })
    .await;
}

#[tokio::test]
async fn org_notice_2625_guest_wallet_suspension_or_deletion_allows_the_saved_notice() {
    Box::pin(async {
        for absent in [false, true] {
            let mut f = Box::pin(Fixture::with_org("org_notice_wallet", true)).await;
            Box::pin(provision_guest(&mut f)).await;
            configure_group(&f).await;
            advertised(&f).await;
            let c = Box::pin(guest_busy(&f, Consumer::Replica)).await;
            let wallets = f.state.db.collection::<bson::Document>("billing_wallet");
            let filter = doc! {"owner_id":&f.row.bot_owner_id};
            if absent {
                assert_eq!(wallets.delete_one(filter).await.unwrap().deleted_count, 1);
            } else {
                assert_eq!(
                    wallets
                        .update_one(filter, doc! {"$set":{"suspended":true}})
                        .await
                        .unwrap()
                        .modified_count,
                    1
                );
            }
            let text = Box::pin(recovered(&f, &c)).await;
            assert_eq!(text, FOLLOW_BUSY);
            Box::pin(deliver(&f, &c, &text)).await;
        }
    })
    .await;
}

#[tokio::test]
async fn org_notice_2625_member_removal_and_disabled_guests_complete_reads_and_refuse_delivery() {
    Box::pin(async {
        for guest in [false, true] {
            let mut f = Box::pin(Fixture::with_org("org_notice_actor_removed", true)).await;
            configure_group(&f).await;
            advertised(&f).await;
            let c = if guest {
                Box::pin(provision_guest(&mut f)).await;
                let c = Box::pin(guest_busy(&f, Consumer::Replica)).await;
                org::management::update(
                    &f.state.db,
                    OWNER,
                    &f.row,
                    serde_json::from_value(json!({"enabled":false})).unwrap(),
                    None,
                )
                .await
                .unwrap();
                c
            } else {
                let actor = notice_member(&f).await;
                let c = Box::pin(capture(&f, SERVICE_REQUEST, Consumer::Replica)).await;
                assert_eq!(
                    f.state
                        .db
                        .collection::<bson::Document>("org_memberships")
                        .update_one(
                            doc! {"org_user_id":&f.row.bot_owner_id,"member_user_id":actor},
                            doc! {"$set":{"revoked_at":bson::DateTime::now()}}
                        )
                        .await
                        .unwrap()
                        .modified_count,
                    1
                );
                c
            };
            let text = Box::pin(recovered(&f, &c)).await;
            let (token, jti) = thread_reply_headers(&f.state, &f.row, &c.source);
            assert!(
                !callback(&f, &c.source, json!({"text":text}), &token)
                    .await
                    .is_success()
            );
            assert_eq!(
                f.state
                    .db
                    .collection::<bson::Document>("reply_token_uses")
                    .count_documents(doc! {"_id":jti})
                    .await
                    .unwrap(),
                0
            );
            assert_closed(&f, &c.source).await;
        }
    })
    .await;
}

#[tokio::test]
async fn org_notice_2625_changed_bytes_markup_and_image_refuse_before_dispatch() {
    Box::pin(async {
        for mutation in ["whitespace","empty","markup","image","formatted_markup"] {
            let f=Box::pin(Fixture::with_org("org_notice_mismatch",true)).await;
            notice_member(&f).await;
            configure_group(&f).await;
            advertised(&f).await;
            let c=Box::pin(capture(&f,SERVICE_REQUEST,Consumer::Live)).await;
            let text=c.live.unwrap();
            let reply=match mutation {
                "whitespace"=>json!({"text":format!("{text} ")}),
                "empty"=>json!({"text":""}),
                "formatted_markup"=>json!({"text":text,"metadata":{"text_format":"commonmark","card":{}}}),
                "markup"=>json!({"text":text,"metadata":{"reply_markup":{"inline_keyboard":[]}}}),
                _=>json!({"text":text,"attachments":[{"kind":"image","source":{"type":"base64","data":"aGVsbG8="}}]}),
            };
            let (token,jti)=thread_reply_headers(&f.state,&f.row,&c.source);
            assert!(!callback(&f,&c.source,reply,&token).await.is_success(),"{mutation}");
            assert_closed(&f,&c.source).await;
            assert_eq!(f.state.db.collection::<bson::Document>("reply_token_uses").count_documents(doc!{"_id":jti}).await.unwrap(),0);
        }
    }).await;
}

// The setup fixture retains the provider key only. Give its assigned route
// key fresh test bytes without changing any of the event's route identities.
async fn assigned_key(f: &Fixture) -> HeaderMap {
    let (prefix, raw, hash) = crate::crypto::token::generate_api_key();
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("api_keys")
            .update_one(
                doc! {"_id":&f.row.route_api_key_id},
                doc! {"$set":{"key_prefix":prefix,"key_hash":hash}}
            )
            .await
            .unwrap()
            .matched_count,
        1
    );
    bearer(&raw)
}

#[tokio::test]
async fn org_notice_2625_token_and_assigned_key_race_over_live_and_replayed_notice() {
    Box::pin(async {
        let f = Box::pin(Fixture::with_org("org_notice_race", true)).await;
        notice_member(&f).await;
        configure_group(&f).await;
        advertised(&f).await;
        let key = assigned_key(&f).await;
        let c = Box::pin(capture(&f, SERVICE_REQUEST, Consumer::Live)).await;
        assert_kind(&f, &c, "service_selection").await;
        let alias = "separate-provider-command";
        let alias_response = Box::pin(respond(&f.state, &f.key, &c.request, alias)).await;
        assert_eq!(alias_response.status(), StatusCode::OK);
        let alias_result =
            frames(&body_text(alias_response).await).pop().unwrap()["response"].clone();
        assert_eq!(alias_result["output"], json!([]));
        assert_eq!(
            snapshot(&f, &dispatch_identity::relay_response(&f.row.id, alias)).await,
            alias_result
        );
        let live = c.live.as_ref().unwrap();
        let replay = Box::pin(recovered(&f, &c)).await;
        assert_eq!(live, &replay);
        platform_reply(&f, live, 1).await;
        let (token, jti) = thread_reply_headers(&f.state, &f.row, &c.source);
        let (a, b) = tokio::join!(
            Box::pin(callback(&f, &c.source, json!({"text":live}), &token)),
            Box::pin(callback(
                &f,
                &c.source,
                json!({"text":replay,"metadata":{"text_format":"commonmark"}}),
                &key
            ))
        );
        assert_ne!(a.is_success(), b.is_success(), "{a}, {b}");
        let uses = f
            .state
            .db
            .collection::<bson::Document>("reply_token_uses")
            .count_documents(doc! {"_id":jti})
            .await
            .unwrap();
        assert_eq!(uses, u64::from(a.is_success()));
        let before = reply_state(&f, &c.source).await;
        let receipts = before.get_array("receipts").unwrap();
        assert_eq!(receipts.len(), 1);
        let receipt_id = receipts[0].as_document().unwrap().get_str("_id").unwrap();
        assert_eq!(Uuid::parse_str(receipt_id).unwrap().get_version_num(), 5);
        assert!(before.get_array("audits").unwrap().is_empty());
        let (fresh, _) = thread_reply_headers(&f.state, &f.row, &c.source);
        for auth in [&fresh, &key, &token] {
            assert!(
                !callback(&f, &c.source, json!({"text":live}), auth)
                    .await
                    .is_success()
            );
        }
        assert_eq!(reply_state(&f, &c.source).await, before);
        assert!(
            !callback(&f, &c.source, json!({"text":format!("{live} ")}), &key)
                .await
                .is_success()
        );
        let changed = reply_state(&f, &c.source).await;
        assert_eq!(changed.get_array("receipts").unwrap(), receipts);
        assert_eq!(changed.get_array("audits").unwrap().len(), 1);
        let bot = channel_bot_service::get_bot(&f.state.db, &f.row.channel_bot_id)
            .await
            .unwrap();
        assert_eq!(
            bot.diagnostics
                .last_outbound_error
                .unwrap()
                .thread_reason
                .as_deref(),
            Some("thread_target_rejected")
        );
        f.platform.verify().await;
    })
    .await;
}

#[tokio::test]
async fn org_notice_2625_committed_turn_same_source_keeps_fresh_ordinary_receipts() {
    Box::pin(async {
        let f = Box::pin(Fixture::with_org("org_notice_turn", true)).await;
        notice_member(&f).await;
        configure_group(&f).await;
        let (body, source) = f
            .input(
                "Question for the specialist",
                ThreadAddress::Mention,
                false,
                true,
            )
            .await;
        assert!(f.send(&body, &source).await.contains("Here is your answer"));
        let events = f.state.db.collection::<bson::Document>(EVENTS);
        let row = events
            .find_one(doc! {"event_id":&source})
            .await
            .unwrap()
            .unwrap();
        assert!(!row.contains_key("org_notice"));
        let conversation = row.get_str("conversation_id").unwrap();
        let filter = doc! {"_id":row.get_str("_id").unwrap()};
        // Even the exact words of a fixed refusal cannot convert a turn into a notice.
        let reply = json!({"text":FOLLOW_BUSY});
        events
            .update_one(filter.clone(), doc! {"$unset":{"conversation_id":""}})
            .await
            .unwrap();
        let (token, _) = thread_reply_headers(&f.state, &f.row, &source);
        assert!(
            !callback(&f, &source, reply.clone(), &token)
                .await
                .is_success()
        );
        events
            .update_one(filter, doc! {"$set":{"conversation_id":conversation}})
            .await
            .unwrap();
        let answer = "Here is your answer";
        let reply = json!({"text":answer});
        platform_reply(&f, answer, 2).await;
        let key = assigned_key(&f).await;
        assert_eq!(
            callback(&f, &source, reply.clone(), &key).await,
            StatusCode::OK
        );
        assert_eq!(callback(&f, &source, reply, &token).await, StatusCode::OK);
        let state = reply_state(&f, &source).await;
        let receipts = state.get_array("receipts").unwrap();
        assert_eq!(receipts.len(), 2);
        for receipt in receipts {
            let id = receipt.as_document().unwrap().get_str("_id").unwrap();
            assert_eq!(Uuid::parse_str(id).unwrap().get_version_num(), 4);
        }
        f.platform.verify().await;
    })
    .await;
}

async fn read_events(f: &Fixture, c: &Captured, cursor: Option<&str>) -> Response {
    native_responses::events(
        State(f.state.clone()),
        Path(c.id.clone()),
        axum::extract::Query(native_responses::Cursor {
            starting_after: cursor.map(str::to_owned),
        }),
        bearer(&f.key),
    )
    .await
}

async fn closed_history(response: Response, private_text: &str) -> Vec<Value> {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = body_text(response).await;
    assert!(!bytes.contains(private_text));
    let history = frames(&bytes);
    assert_eq!(history.len(), 4);
    for (i, frame) in history.iter().enumerate() {
        assert_eq!(frame["sequence_number"], i);
        assert_eq!(frame["response"]["output"], json!([]));
    }
    assert_eq!(history[2]["type"], "response.in_progress");
    assert_eq!(history[3]["type"], "response.completed");
    history
}

#[tokio::test]
async fn org_notice_2625_bot_or_route_closed_reads_complete_empty_and_record_health() {
    Box::pin(async {
        // Detecting a deactivated bot retires its provider key. Give each
        // entry point an authenticated first read on its own such binding.
        for (bot, first_read) in [
            (false, "snapshot"),
            (true, "snapshot"),
            (true, "events"),
            (true, "post"),
        ] {
            let f = Box::pin(Fixture::with_org("org_notice_transport_closed", true)).await;
            configure_group(&f).await;
            notice_member(&f).await;
            advertised(&f).await;
            let c = Box::pin(capture(&f, SERVICE_REQUEST, Consumer::Live)).await;
            let text = c.live.as_deref().unwrap();
            let saved = read_events(&f, &c, Some("1")).await;
            assert_eq!(saved.status(), StatusCode::OK);
            let mut stream = saved.into_body();
            let frame = frames(&first(&mut stream).await).remove(0);
            assert_eq!(frame["sequence_number"], 2);
            assert_eq!(frame["item"]["content"][0]["text"], text);
            drop(stream);
            let (collection, id) = if bot {
                (
                    crate::models::channel_bot::COLLECTION_NAME,
                    f.row.channel_bot_id.as_str(),
                )
            } else {
                ("channel_conversations", f.row.route_id.as_deref().unwrap())
            };
            assert_eq!(
                f.state
                    .db
                    .collection::<bson::Document>(collection)
                    .update_one(doc! {"_id":id}, doc! {"$set":{"is_active":false}})
                    .await
                    .unwrap()
                    .modified_count,
                1
            );
            let empty = match first_read {
                "snapshot" => snapshot(&f, &c.id).await,
                "events" => closed_history(read_events(&f, &c, None).await, text)
                    .await
                    .pop()
                    .unwrap()["response"]
                    .clone(),
                _ => {
                    let post = Box::pin(respond(&f.state, &f.key, &c.request, &c.source)).await;
                    closed_history(post, text).await.pop().unwrap()["response"].clone()
                }
            };
            assert_eq!(empty["status"], "completed");
            assert_eq!(empty["output"], json!([]));
            let events = read_events(&f, &c, None).await;
            let repeated = Box::pin(respond(&f.state, &f.key, &c.request, &c.source)).await;
            if bot {
                assert_eq!(events.status(), StatusCode::UNAUTHORIZED);
                assert_eq!(repeated.status(), StatusCode::UNAUTHORIZED);
            } else {
                let history = closed_history(events, text).await;
                assert_eq!(history[3]["response"], empty);
                assert_eq!(closed_history(repeated, text).await, history);
                let resumed = read_events(&f, &c, Some("2")).await;
                assert_eq!(resumed.status(), StatusCode::OK);
                let bytes = body_text(resumed).await;
                assert!(!bytes.contains(text));
                assert_eq!(frames(&bytes), history[3..]);
            }
            let (token, _) = thread_reply_headers(&f.state, &f.row, &c.source);
            assert!(
                !callback(&f, &c.source, json!({"text":text}), &token)
                    .await
                    .is_success()
            );
            assert_closed(&f, &c.source).await;
        }
    })
    .await;
}

#[tokio::test]
async fn org_notice_2625_retirement_records_health_without_a_gateway_callback() {
    Box::pin(async {
        use crate::models::nyxbot_gateway_release::ReleaseReason;
        let f = Box::pin(Fixture::with_org("org_notice_retirement", true)).await;
        read::ensure_indexes(&f.state.db).await.unwrap();
        configure_group(&f).await;
        notice_member(&f).await;
        advertised(&f).await;
        let c = Box::pin(capture(&f, SERVICE_REQUEST, Consumer::Replica)).await;
        let original = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
        Box::pin(
            crate::services::nyxbot_gateway_release_service::retire_connection(
                &f.state,
                &original,
                ReleaseReason::OrgAccessLost,
                "failed",
                Some("org_access_lost"),
            ),
        )
        .await
        .unwrap();
        // The bounded trace runs after the retirement transaction. Wait before
        // making any provider or relay call so neither can be responsible for it.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !reply_state(&f, &c.source)
                    .await
                    .get_array("audits")
                    .unwrap()
                    .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_closed(&f, &c.source).await;
        let denied =
            native_responses::snapshot(State(f.state.clone()), Path(c.id), bearer(&f.key)).await;
        assert!(matches!(
            denied.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    })
    .await;
}

#[tokio::test]
async fn org_notice_2625_retained_ingress_never_delivers_after_original_event_expires() {
    Box::pin(async {
        let f = Box::pin(Fixture::with_org("org_notice_retained_source", true)).await;
        configure_group(&f).await;
        notice_member(&f).await;
        advertised(&f).await;
        let c = Box::pin(capture(&f, SERVICE_REQUEST, Consumer::Live)).await;
        let events = f.state.db.collection::<bson::Document>(EVENTS);
        let filter = doc! {"event_id":&c.source,"channel_id":&f.row.id};
        assert_eq!(
            events
                .update_one(
                    filter.clone(),
                    doc! {"$set":{"response_read.expires_at":bson::DateTime::from_millis(0)}}
                )
                .await
                .unwrap()
                .modified_count,
            1
        );
        read::expire(&f.state.db).await.unwrap();
        assert!(
            !events
                .find_one(filter.clone())
                .await
                .unwrap()
                .unwrap()
                .contains_key("org_notice")
        );
        let (token, jti) = thread_reply_headers(&f.state, &f.row, &c.source);
        for remove_event in [false, true] {
            if remove_event {
                assert_eq!(
                    events
                        .delete_one(filter.clone())
                        .await
                        .unwrap()
                        .deleted_count,
                    1
                );
            }
            for text in [c.live.as_deref().unwrap(), "different bytes"] {
                assert!(
                    !callback(&f, &c.source, json!({"text":text}), &token)
                        .await
                        .is_success()
                );
            }
        }
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>("reply_token_uses")
                .count_documents(doc! {"_id":jti})
                .await
                .unwrap(),
            0
        );
        assert!(
            reply_state(&f, &c.source)
                .await
                .get_array("receipts")
                .unwrap()
                .is_empty()
        );
        assert!(
            f.platform
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| !r.url.path().ends_with("/reply"))
        );
    })
    .await;
}

async fn output_for(f: &Fixture, c: &Captured) -> String {
    if let Some(text) = &c.live {
        text.clone()
    } else {
        Box::pin(recovered(f, c)).await
    }
}

async fn assert_kind(f: &Fixture, c: &Captured, kind: &str) {
    assert_eq!(f.state.db.collection::<bson::Document>(EVENTS).count_documents(
        doc!{"channel_id":&f.row.id,"event_id":&c.source,"status":"refused","org_notice.kind":kind,
            "org_notice.version":1,"org_notice.response_id":&c.id,"org_notice.pending":true}
    ).await.unwrap(),1,"{kind}");
}

// Open a real source first to resolve the actor's durable lane. A live guest
// turn bypasses cold model setup; a legacy unreadable conversation models a
// storage failure during start, after this next event's delivery is prepared.
async fn existing_lane(f: &Fixture, c: &Captured, warm: bool) {
    let db = &f.state.db;
    let previous: NyxbotEvent = db
        .collection(EVENTS)
        .find_one(doc! {"event_id":&c.source})
        .await
        .unwrap()
        .unwrap();
    let origin = previous.delivery.unwrap().origin;
    let actor = origin.org_execution.as_ref().unwrap();
    let lane = db
        .collection::<bson::Document>(crate::models::org_channel::LANES)
        .find_one(doc! {"actor_id":&actor.actor_id,"partition":&origin.partition})
        .await
        .unwrap()
        .unwrap();
    let mut conversation =
        doc! {"_id":lane.get_str("conversation_id").unwrap(),"user_id":&actor.actor_id};
    if warm {
        assert!(actor.guest);
        let now = bson::DateTime::now();
        let agent = team::agent(db, &actor.org_id, &actor.agent_id)
            .await
            .unwrap();
        conversation.extend(doc!{"agent_id":&agent.id,"role":"subagent","model":agent.model,
            "channel":bson::to_bson(&origin).unwrap(),"guest_turn":true,"title":"Existing guest conversation",
            "credential_api_key_id":Uuid::new_v4().to_string(),"message_count":0_i64,"created_at":now,"updated_at":now,
            "active_turn":{"turn_id":Uuid::new_v4().to_string(),"origin":"channel","started_at":now}});
        let _: crate::models::assistant_conversation::AssistantConversation =
            bson::from_document(conversation.clone()).unwrap();
    }
    db.collection::<bson::Document>("assistant_conversations")
        .insert_one(conversation)
        .await
        .unwrap();
}

#[tokio::test]
async fn org_notice_2625_start_failure_and_addressed_guest_busy_for_both_consumers() {
    Box::pin(async {
        for (guest, warm, expected, kind) in [
            (false, false, START_FAILURE, "start_failure"),
            (true, false, START_FAILURE, "start_failure"),
            (true, true, GUEST_BUSY, "guest_busy"),
        ] {
            for consumer in [Consumer::Live, Consumer::Replica] {
                let mut f = Box::pin(Fixture::with_org("org_notice_start_outcome", true)).await;
                configure_group(&f).await;
                advertised(&f).await;
                let first = if guest {
                    Box::pin(provision_guest(&mut f)).await;
                    Box::pin(guest_busy(&f, Consumer::Live)).await
                } else {
                    notice_member(&f).await;
                    Box::pin(capture(&f, SERVICE_REQUEST, Consumer::Live)).await
                };
                existing_lane(&f, &first, warm).await;
                let _permits = if warm {
                    pool_full(&f, f.row.bot_owner_id.as_deref().unwrap()).await
                } else {
                    vec![]
                };
                let c = Box::pin(capture(&f, "Another addressed question", consumer)).await;
                let text = Box::pin(output_for(&f, &c)).await;
                assert_eq!(text, expected);
                assert_kind(&f, &c, kind).await;
                Box::pin(deliver(&f, &c, &text)).await;
            }
        }
    })
    .await;
}

// Unreachable combinations exercise the typed storage/delivery boundary only.
// Production followed ingress maps guest billing errors to StartFailure, leaves
// service selection to members, and only produces GuestBusy for addressed guests.
#[tokio::test]
async fn org_notice_2625_typed_outcomes_without_followed_producers_preserve_exact_text() {
    Box::pin(async {
        for (guest,kind,name,text) in [
            (false,NoticeKind::GuestUnavailable,"guest_unavailable",org::UNAVAILABLE),
            (true,NoticeKind::GuestUnavailable,"guest_unavailable",org::UNAVAILABLE),
            (false,NoticeKind::GuestBusy,"guest_busy",GUEST_BUSY),
            (true,NoticeKind::ServiceSelection,"service_selection","ambiguous_selection: This connection name identifies multiple accounts. Choose the exact connection UUID from Connections; no account was selected."),
        ] {
            for consumer in [Consumer::Live, Consumer::Replica] {
                let mut f=Box::pin(Fixture::with_org("org_notice_typed",true)).await;
                configure_group(&f).await;
                advertised(&f).await;
                let mut c=if guest {
                    Box::pin(provision_guest(&mut f)).await;
                    Box::pin(guest_busy(&f,Consumer::Replica)).await
                } else {
                    notice_member(&f).await;
                    Box::pin(capture(&f,SERVICE_REQUEST,Consumer::Replica)).await
                };
                let events=f.state.db.collection::<bson::Document>(EVENTS);
                let original:NyxbotEvent=f.state.db.collection(EVENTS).find_one(doc!{"event_id":&c.source}).await.unwrap().unwrap();
                let origin=original.delivery.unwrap().origin;
                let facts:ChannelThreadFacts=serde_json::from_value(c.request["event_context"]["activity"]["thread"].clone()).unwrap();
                let row=load_channel(&f.state,OWNER,&f.row.id).await.unwrap();
                let mut refusal=InboundReply::fixed(kind,text);
                refusal.followed=Some(Box::new(response_read::SelectedThread{
                    binding:*origin.thread.unwrap(),binding_id:row.binding_id.clone().unwrap(),
                    partition:origin.partition,target_sha256:crate::services::nyxbot_response_read::notice_target_digest(&facts),
                }));
                assert_eq!(events.update_one(doc!{"_id":&original.id},doc!{"$unset":{"org_notice":"","response_reply":""},"$set":{"status":"running"}}).await.unwrap().modified_count,1);
                let response=Box::pin(response_read::reply(f.state.clone(),&row,original.id,c.id.clone(),c.created.clone(),refusal,true)).await.unwrap();
                c.live=if consumer==Consumer::Live {
                    let history=frames(&body_text(response).await);
                    Some(notice_output(&history.last().unwrap()["response"]))
                } else {drop(response);None};
                assert_kind(&f,&c,name).await;
                assert_eq!(Box::pin(output_for(&f,&c)).await,text);
                Box::pin(deliver(&f,&c,text)).await;
            }
        }
    }).await;
}

#[tokio::test]
async fn org_notice_2625_changed_native_root_or_follow_revision_refuses_saved_notice() {
    Box::pin(async {
        for root in [true, false] {
            let f = Box::pin(Fixture::with_org("org_notice_source_changed", true)).await;
            configure_group(&f).await;
            notice_member(&f).await;
            advertised(&f).await;
            let c = Box::pin(capture(&f, SERVICE_REQUEST, Consumer::Live)).await;
            let change = if root {
                f.state
                    .db
                    .collection::<bson::Document>(MESSAGES)
                    .update_one(
                        doc! {"_id":&c.source},
                        doc! {"$set":{"thread_context.root_id":"om_other_root"}},
                    )
                    .await
            } else {
                let child = f.children().await.remove(0);
                f.state
                    .db
                    .collection::<bson::Document>(THREADS)
                    .update_one(
                        doc! {"_id":&child.id},
                        doc! {"$inc":{"follow_revision":1_i64}},
                    )
                    .await
            };
            assert_eq!(change.unwrap().modified_count, 1);
            let (headers, _) = thread_reply_headers(&f.state, &f.row, &c.source);
            assert!(
                !callback(&f, &c.source, json!({"text":c.live.unwrap()}), &headers)
                    .await
                    .is_success()
            );
            assert_closed(&f, &c.source).await;
        }
    })
    .await;
}

async fn member_notice_for_dispatch(name: &str) -> (Fixture, Captured) {
    let f = Box::pin(Fixture::with_org(name, true)).await;
    notice_member(&f).await;
    configure_group(&f).await;
    advertised(&f).await;
    let c = Box::pin(capture(&f, SERVICE_REQUEST, Consumer::Live)).await;
    (f, c)
}

async fn assert_notice_claim(f: &Fixture, c: &Captured, jti: &str, claimed: bool) {
    let db = &f.state.db;
    assert_eq!(
        db.collection::<bson::Document>(EVENTS)
            .count_documents(doc! {
                "channel_id":&f.row.id, "event_id":&c.source, "org_notice.pending":!claimed,
            })
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<bson::Document>(crate::models::reply_token_use::COLLECTION_NAME)
            .count_documents(doc! {"_id":jti})
            .await
            .unwrap(),
        u64::from(claimed)
    );
    let receipts =
        db.collection::<bson::Document>(crate::models::channel_reply_delivery::COLLECTION_NAME);
    assert_eq!(
        receipts
            .count_documents(doc! {"inbound_message_id":&c.source})
            .await
            .unwrap(),
        u64::from(claimed)
    );
}

#[derive(Clone, Copy)]
enum BeforeDispatchFailure {
    TenantToken,
    UncertainReceipt,
}

async fn retry_before_dispatch(failure: BeforeDispatchFailure) {
    let (f, c) = Box::pin(member_notice_for_dispatch("org_notice_before_dispatch")).await;
    let (token, jti) = thread_reply_headers(&f.state, &f.row, &c.source);
    let text = c.live.as_deref().unwrap();
    let reply = json!({"text":text});
    let receipts = crate::models::channel_reply_delivery::COLLECTION_NAME;
    match failure {
        BeforeDispatchFailure::TenantToken => {
            // Opening the followed thread can warm the adapter's token cache.
            let cold =
                crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
                    &f.platform.uri(),
                    "lark",
                );
            thread_follow::TEST_ADAPTERS
                .lock()
                .unwrap()
                .insert(f.row.channel_bot_id.clone(), Arc::new(cold));
            Mock::given(method("POST"))
                .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
                .respond_with(ResponseTemplate::new(503))
                .with_priority(1)
                .up_to_n_times(1)
                .expect(1)
                .mount(&f.platform)
                .await;
        }
        BeforeDispatchFailure::UncertainReceipt => {
            crate::services::channel_reply_delivery::ensure_indexes(&f.state.db)
                .await
                .unwrap();
            // Permit ordinary receipt inserts but reject the status transition
            // required before bytes can leave this notice's send transaction.
            f.state
                .db
                .run_command(
                    doc! {"collMod":receipts,"validationLevel":"strict","validationAction":"error",
                    "validator":{"parts.status":{"$ne":"uncertain"}}},
                )
                .await
                .unwrap();
        }
    }
    platform_reply(&f, text, 1).await;
    assert!(
        !callback(&f, &c.source, reply.clone(), &token)
            .await
            .is_success()
    );
    assert_notice_claim(&f, &c, &jti, false).await;
    assert!(
        f.platform
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !r.url.path().ends_with("/reply"))
    );
    if matches!(failure, BeforeDispatchFailure::UncertainReceipt) {
        f.state
            .db
            .run_command(doc! {"collMod":receipts,"validator":{}})
            .await
            .unwrap();
    }
    assert_eq!(
        callback(&f, &c.source, reply.clone(), &token).await,
        StatusCode::OK
    );
    assert_notice_claim(&f, &c, &jti, true).await;
    assert!(!callback(&f, &c.source, reply, &token).await.is_success());
    f.platform.verify().await;
}

#[tokio::test]
async fn org_notice_2625_tenant_token_failure_allows_same_token_retry_before_platform_post() {
    Box::pin(retry_before_dispatch(BeforeDispatchFailure::TenantToken)).await;
}

#[tokio::test]
async fn org_notice_2625_uncertain_receipt_write_failure_rolls_back_before_platform_post() {
    Box::pin(retry_before_dispatch(
        BeforeDispatchFailure::UncertainReceipt,
    ))
    .await;
}

#[tokio::test]
async fn org_notice_2625_ambiguous_platform_post_keeps_claim_and_refuses_key_and_token_retries() {
    Box::pin(async {
        let (f, c) = Box::pin(member_notice_for_dispatch("org_notice_unknown_send")).await;
        let text = c.live.as_deref().unwrap();
        let reply = json!({"text":text});
        Mock::given(method("POST"))
            .and(path("/open-apis/im/v1/messages/om_root/reply"))
            .and(body_partial_json(
                json!({"reply_in_thread":true,"content":json!({"text":text}).to_string()}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string("unconfirmed"))
            .expect(1)
            .mount(&f.platform)
            .await;
        let key = assigned_key(&f).await;
        let (token, jti) = thread_reply_headers(&f.state, &f.row, &c.source);
        assert!(
            !callback(&f, &c.source, reply.clone(), &token)
                .await
                .is_success()
        );
        assert_notice_claim(&f, &c, &jti, true).await;
        let before = reply_state(&f, &c.source).await;
        let receipt = before.get_array("receipts").unwrap()[0]
            .as_document()
            .unwrap();
        assert_eq!(receipt.get_bool("complete"), Ok(false));
        assert_eq!(
            receipt.get_array("parts").unwrap()[0]
                .as_document()
                .unwrap()
                .get_str("status"),
            Ok("uncertain")
        );
        let (fresh, fresh_jti) = thread_reply_headers(&f.state, &f.row, &c.source);
        for auth in [&key, &token, &fresh] {
            assert!(
                !callback(&f, &c.source, reply.clone(), auth)
                    .await
                    .is_success()
            );
        }
        assert_eq!(reply_state(&f, &c.source).await, before);
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(crate::models::reply_token_use::COLLECTION_NAME)
                .count_documents(doc! {"_id":fresh_jti})
                .await
                .unwrap(),
            0
        );
        f.platform.verify().await;
    })
    .await;
}
