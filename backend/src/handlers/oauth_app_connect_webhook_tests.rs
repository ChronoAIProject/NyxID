use super::*;
use crate::models::connect_link::ConnectLinkWebhookStatus;
use crate::models::platform_settings::AppConnectRollout;
use crate::services::app_connect_webhook_service as webhooks;
use crate::services::connect_link_service::{
    WEBHOOK_MAX_DISPATCH_CYCLES, WEBHOOK_REDISPATCH_STALE_SECS,
};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;

async fn receiver(f: &Fixture) -> mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = mpsc::unbounded_channel();
    let router = axum::Router::new().route(
        "/events",
        axum::routing::post(move |headers: HeaderMap, body: axum::body::Bytes| {
            let tx = tx.clone();
            async move {
                let envelope: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(
                    headers["X-NyxID-Delivery-Id"],
                    envelope["event_id"].as_str().unwrap()
                );
                assert!(headers.contains_key("X-NyxID-Signature"));
                tx.send(envelope).unwrap();
                StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let encrypted = f
        .state
        .encryption_keys
        .encrypt(b"app-outbox-fixture-secret")
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("oauth_clients")
        .update_one(
            doc! { "_id": &f.app.id },
            doc! { "$set": {
                "connection_webhook_url": format!("http://{address}/events"),
                "connection_webhook_secret_encrypted": bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted,
                },
                "connection_webhook_enabled": true, "connection_webhook_key_id": "fixture-key",
            } },
        )
        .await
        .unwrap();
    rx
}

async fn received(rx: &mut mpsc::UnboundedReceiver<Value>) -> Value {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn no_event(rx: &mut mpsc::UnboundedReceiver<Value>) {
    assert!(
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err()
    );
}
async fn stored(f: &Fixture, id: &str) -> AppConnectLink {
    f.state
        .db
        .collection::<AppConnectLink>(LINKS)
        .find_one(doc! { "_id": id })
        .await
        .unwrap()
        .unwrap()
}
async fn settled(f: &Fixture, id: &str, expected: ConnectLinkWebhookStatus) -> AppConnectLink {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let link = stored(f, id).await;
            if link.webhook_event_status == Some(expected) {
                return link;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
async fn repair(f: &Fixture) -> AppConnectLink {
    let created = app_links::start_from_app(
        &f.state,
        &f.app.id,
        &f.auth.user_id.to_string(),
        "https://app.example/callback",
        "private-callback-state",
    )
    .await
    .unwrap();
    let capability = url::Url::parse(&created.connect_url)
        .unwrap()
        .fragment()
        .unwrap()
        .strip_prefix("t=")
        .unwrap()
        .to_string();
    app_links::redeem(
        &f.state,
        &created.link.id,
        &created.link.user_id,
        &capability,
    )
    .await
    .unwrap()
}
async fn terminal_without_reservation(f: &Fixture, id: &str, status: &str) {
    f.state.db.collection::<Document>(LINKS).update_one(doc! { "_id": id },
        doc! { "$set": { "status": status, "completed_at": bson::DateTime::now(),
            "failure_reason": if status == "failed" { Some("requirement_unsatisfiable") } else { None },
        }, "$inc": { "revision": 1 } },
    ).await.unwrap();
}
async fn stale(f: &Fixture, id: &str, attempts: u32) {
    f.state
        .db
        .collection::<Document>(LINKS)
        .update_one(
            doc! { "_id": id },
            doc! { "$set": { "webhook_event_status": "pending", "webhook_event_delivered_at": null,
                "webhook_event_attempts": i64::from(attempts),
                "webhook_event_reserved_at": bson::DateTime::from_chrono(
                    Utc::now() - chrono::Duration::seconds(WEBHOOK_REDISPATCH_STALE_SECS + 1)),
            } },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn app_connect_webhooks_db_terminal_payloads_both_origins_and_duplicate_reservations() {
    let Some(f) = fixture("app_outbox_payloads").await else {
        return;
    };
    gated(&f, false, true).await;
    let mut rx = receiver(&f).await;
    for origin in ["app", "authorize"] {
        for status in ["completed", "cancelled", "expired", "failed"] {
            let link = if origin == "app" {
                repair(&f).await
            } else {
                session(&f, &params(&f)).await
            };
            terminal_without_reservation(&f, &link.id, status).await;
            let ((), ()) = tokio::join!(
                webhooks::dispatch_terminal_webhook_if_needed(&f.state, &link.id),
                webhooks::dispatch_terminal_webhook_if_needed(&f.state, &link.id),
            );
            let event = received(&mut rx).await;
            assert_eq!(event["event_type"], format!("app_connect_link.{status}"));
            assert_eq!(
                event["data"],
                json!({
                    "user_id": link.user_id, "app_connect_link_id": link.id, "origin": origin,
                    "requirements_version": link.manifest_version, "status": status,
                    "failure_reason": if status == "failed" { Some("requirement_unsatisfiable") } else { None },
                    "grant_update_required": false,
                    "items": [{ "requirement_id": "required", "state": "met",
                        "user_service_id": link.items[0].user_service_id, "slug": "personal-github" }],
                })
            );
            let saved = settled(&f, &link.id, ConnectLinkWebhookStatus::Delivered).await;
            assert_eq!(saved.webhook_event_attempts, 1);
            assert_eq!(
                saved.webhook_event_id.as_deref(),
                event["event_id"].as_str()
            );
            webhooks::redispatch_terminal_webhooks(&f.state)
                .await
                .unwrap();
            no_event(&mut rx).await;
        }
    }
}

#[tokio::test]
async fn app_connect_webhooks_db_ready_for_consent_is_silent_until_code_commits() {
    let Some(f) = fixture("app_outbox_consent").await else {
        return;
    };
    gated(&f, false, true).await;
    let mut rx = receiver(&f).await;
    let (link, form) = ready(&f, &params(&f)).await;
    webhooks::redispatch_terminal_webhooks(&f.state)
        .await
        .unwrap();
    no_event(&mut rx).await;
    assert!(stored(&f, &link.id).await.webhook_event_id.is_none());
    assert_eq!(count(&f, CODES).await, 0);
    let replay = serde_json::from_value(json!({
        "response_type": "code", "client_id": form.client_id, "redirect_uri": form.redirect_uri,
        "scope": form.scope, "decision": "allow", "consent_request": form.consent_request,
        "allowed_service_ids": form.allowed_service_ids, "resource": form.resource,
    }))
    .unwrap();
    decide(&f, form).await.unwrap();
    let event = received(&mut rx).await;
    assert_eq!(event["event_type"], "app_connect_link.completed");
    assert_eq!(event["data"]["origin"], "authorize");
    assert_eq!(count(&f, CODES).await, 1);
    assert_eq!(
        stored(&f, &link.id).await.status,
        AppConnectStatus::Completed
    );
    assert!(decide(&f, replay).await.is_err());
    settled(&f, &link.id, ConnectLinkWebhookStatus::Delivered).await;
    no_event(&mut rx).await;
}

#[tokio::test]
async fn app_connect_webhooks_db_repair_ready_cancel_expiry_and_failure_dispatch() {
    let Some(f) = fixture("app_outbox_transitions").await else {
        return;
    };
    gated(&f, false, true).await;
    let mut rx = receiver(&f).await;
    let link = repair(&f).await;
    app_links::ready(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    let event = received(&mut rx).await;
    assert_eq!(event["event_type"], "app_connect_link.completed");
    assert_eq!(event["data"]["grant_update_required"], true);
    let cancelled = repair(&f).await;
    app_links::cancel(&f.state, &cancelled.id, &cancelled.user_id)
        .await
        .unwrap();
    app_links::cancel(&f.state, &cancelled.id, &cancelled.user_id)
        .await
        .unwrap();
    assert_eq!(
        received(&mut rx).await["event_type"],
        "app_connect_link.cancelled"
    );
    let expired = session(&f, &params(&f)).await;
    f.state.db.collection::<Document>(LINKS).update_one(doc! { "_id": &expired.id },
        doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1)) } },
    ).await.unwrap();
    app_links::expire_sessions(&f.state).await.unwrap();
    assert_eq!(
        received(&mut rx).await["event_type"],
        "app_connect_link.expired"
    );
    let failed = session(&f, &params(&f)).await;
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_many(doc! {}, doc! { "$set": { "is_active": false } })
        .await
        .unwrap();
    app_links::ready(&f.state, &failed.id, &failed.user_id)
        .await
        .unwrap();
    let event = received(&mut rx).await;
    assert_eq!(event["event_type"], "app_connect_link.failed");
    assert_eq!(event["data"]["failure_reason"], "requirement_unsatisfiable");
    no_event(&mut rx).await;
}

#[tokio::test]
async fn app_connect_webhooks_db_sweep_recovers_transition_before_reservation() {
    let Some(f) = fixture("app_outbox_recovery").await else {
        return;
    };
    gated(&f, false, true).await;
    let mut rx = receiver(&f).await;
    let link = repair(&f).await;
    terminal_without_reservation(&f, &link.id, "completed").await;
    assert!(stored(&f, &link.id).await.webhook_event_id.is_none());
    tokio::try_join!(
        app_links::expire_sessions(&f.state),
        app_links::expire_sessions(&f.state)
    )
    .unwrap();
    assert_eq!(
        received(&mut rx).await["event_type"],
        "app_connect_link.completed"
    );
    settled(&f, &link.id, ConnectLinkWebhookStatus::Delivered).await;
    no_event(&mut rx).await;
}

#[tokio::test]
async fn app_connect_webhooks_db_stale_cycle_reuses_id_and_frozen_payload() {
    let Some(f) = fixture("app_outbox_retry").await else {
        return;
    };
    gated(&f, false, true).await;
    let mut rx = receiver(&f).await;
    let link = repair(&f).await;
    app_links::cancel(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    let first = received(&mut rx).await;
    settled(&f, &link.id, ConnectLinkWebhookStatus::Delivered).await;
    // Receiver accepted, then the process died before recording delivery.
    stale(&f, &link.id, 1).await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_many(
            doc! {},
            doc! { "$set": { "slug": "renamed-after-reservation" } },
        )
        .await
        .unwrap();
    tokio::try_join!(
        webhooks::redispatch_terminal_webhooks(&f.state),
        webhooks::redispatch_terminal_webhooks(&f.state)
    )
    .unwrap();
    let retry = received(&mut rx).await;
    assert_eq!(retry["event_id"], first["event_id"]);
    assert_eq!(retry["data"], first["data"]);
    assert_eq!(retry["occurred_at"], first["occurred_at"]);
    let saved = stored(&f, &link.id).await;
    assert_eq!(
        first["occurred_at"],
        serde_json::to_value(saved.webhook_event_occurred_at).unwrap()
    );
    assert_eq!(
        settled(&f, &link.id, ConnectLinkWebhookStatus::Delivered)
            .await
            .webhook_event_attempts,
        2
    );
    no_event(&mut rx).await;
}

#[tokio::test]
async fn app_connect_webhooks_db_exhausted_cycle_is_abandoned_once() {
    let Some(f) = fixture("app_outbox_abandon").await else {
        return;
    };
    gated(&f, false, true).await;
    let mut rx = receiver(&f).await;
    let link = repair(&f).await;
    app_links::cancel(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    received(&mut rx).await;
    settled(&f, &link.id, ConnectLinkWebhookStatus::Delivered).await;
    stale(&f, &link.id, WEBHOOK_MAX_DISPATCH_CYCLES).await;
    tokio::try_join!(
        webhooks::redispatch_terminal_webhooks(&f.state),
        webhooks::redispatch_terminal_webhooks(&f.state)
    )
    .unwrap();
    let link = settled(&f, &link.id, ConnectLinkWebhookStatus::Abandoned).await;
    assert_eq!(link.webhook_event_attempts, WEBHOOK_MAX_DISPATCH_CYCLES);
    no_event(&mut rx).await;
}

#[tokio::test]
async fn app_connect_webhooks_db_rollout_disabled_suppresses_new_and_reserved_events() {
    let Some(f) = fixture("app_outbox_disabled").await else {
        return;
    };
    gated(&f, false, true).await;
    let mut rx = receiver(&f).await;
    let pending = repair(&f).await;
    app_links::cancel(&f.state, &pending.id, &pending.user_id)
        .await
        .unwrap();
    received(&mut rx).await;
    settled(&f, &pending.id, ConnectLinkWebhookStatus::Delivered).await;
    stale(&f, &pending.id, 1).await;
    let fresh = repair(&f).await;
    terminal_without_reservation(&f, &fresh.id, "expired").await;
    let mut policy = f.state.app_connect_policy();
    policy.revision += 1;
    policy.rollout = AppConnectRollout::Disabled;
    f.state.set_app_connect_policy_if_fresh(policy);
    app_links::expire_sessions(&f.state).await.unwrap();
    settled(&f, &pending.id, ConnectLinkWebhookStatus::Abandoned).await;
    settled(&f, &fresh.id, ConnectLinkWebhookStatus::Abandoned).await;
    no_event(&mut rx).await;
    policy.revision += 1;
    policy.rollout = AppConnectRollout::Allowlist;
    f.state.set_app_connect_policy_if_fresh(policy);
    app_links::expire_sessions(&f.state).await.unwrap();
    no_event(&mut rx).await;
}

#[tokio::test]
async fn app_connect_webhooks_db_unredeemed_expiry_discloses_only_transaction_without_consent() {
    let Some(f) = fixture("app_outbox_private_expiry").await else {
        return;
    };
    let selected = gated(&f, false, true).await.unwrap();
    let mut rx = receiver(&f).await;
    for prior_consent in [false, true] {
        if prior_consent {
            consent_service::grant_consent_with_services(
                &f.state.db,
                &f.auth.user_id.to_string(),
                &f.app.id,
                "openid proxy",
                Some(vec![selected.clone()]),
            )
            .await
            .unwrap();
        }
        let mut p = params(&f);
        p.nyx_connect = Some("force".into());
        let response = authorize_response(&f, &p).await;
        let url = url::Url::parse(&location(&response)).unwrap();
        let id = url.path_segments().unwrap().next_back().unwrap();
        let link = stored(&f, id).await;
        assert!(link.redeemed_at.is_none());
        assert_eq!(
            link.items[0].user_service_id.as_deref(),
            Some(selected.as_str())
        );
        f.state
            .db
            .collection::<Document>(LINKS)
            .update_one(
                doc! { "_id": id },
                doc! { "$set": { "expires_at": bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::seconds(1)) } },
            )
            .await
            .unwrap();
        app_links::expire_sessions(&f.state).await.unwrap();
        let event = received(&mut rx).await;
        assert_eq!(event["event_type"], "app_connect_link.expired");
        if prior_consent {
            assert_eq!(event["data"]["user_id"], link.user_id);
            assert_eq!(event["data"]["items"][0]["slug"], "personal-github");
        } else {
            assert_eq!(
                event["data"],
                json!({
                    "app_connect_link_id": id, "origin": "authorize", "status": "expired",
                })
            );
            let saved = settled(&f, id, ConnectLinkWebhookStatus::Delivered).await;
            assert_eq!(
                serde_json::to_value(saved.webhook_event_data).unwrap(),
                event["data"]
            );
        }
        settled(&f, id, ConnectLinkWebhookStatus::Delivered).await;
        no_event(&mut rx).await;
    }
}
