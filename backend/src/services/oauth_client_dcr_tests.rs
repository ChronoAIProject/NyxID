use super::*;
use crate::models::oauth_client::ScopeProvenance;
use crate::test_utils::connect_test_database;

async fn dcr_client(db: &mongodb::Database, provenance: ScopeProvenance) -> OauthClient {
    create_client(
        db,
        "DCR migration",
        &[],
        "public",
        "dynamic_registration",
        "",
        "openid email",
        provenance,
        false,
        None,
        None,
        &[],
    )
    .await
    .unwrap()
    .0
}

#[tokio::test]
async fn dcr_migration_only_widens_durable_defaults() {
    let db = connect_test_database("dcr_migration_provenance")
        .await
        .expect("MongoDB required");
    let defaulted = dcr_client(&db, ScopeProvenance::Defaulted).await;
    let explicit = dcr_client(&db, ScopeProvenance::Explicit).await;
    let legacy = dcr_client(&db, ScopeProvenance::UnknownLegacy).await;
    let absent = dcr_client(&db, ScopeProvenance::UnknownLegacy).await;
    db.collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! {"_id": &absent.id},
            doc! {"$unset": {"scope_provenance": ""}},
        )
        .await
        .unwrap();
    for _ in 0..2 {
        migrate_dynamic_clients_grant_default_mcp_scopes(&db)
            .await
            .unwrap();
        for client in [&explicit, &legacy, &absent] {
            assert_eq!(
                get_client(&db, &client.id).await.unwrap().allowed_scopes,
                "openid email"
            );
        }
        let upgraded = get_client(&db, &defaulted.id).await.unwrap();
        for scope in DEFAULT_MCP_ALLOWED_SCOPES.split_whitespace() {
            assert!(
                upgraded
                    .allowed_scopes
                    .split_whitespace()
                    .any(|s| s == scope)
            );
        }
        assert_eq!(upgraded.scope_provenance, ScopeProvenance::Defaulted);
    }
}

#[tokio::test]
async fn dcr_scope_edits_are_explicit_but_metadata_edits_keep_provenance() {
    let db = connect_test_database("dcr_migration_edits")
        .await
        .expect("MongoDB required");
    for admin in [false, true] {
        let client = dcr_client(&db, ScopeProvenance::Defaulted).await;
        for scopes in [None, Some("openid")] {
            let edited = if admin {
                admin_update_client(
                    &db,
                    &client.id,
                    AdminUpdateClient {
                        client_name: Some("Renamed"),
                        allowed_scopes: scopes,
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
            } else {
                update_client_for_creator(
                    &db,
                    &client.id,
                    "dynamic_registration",
                    Some("Renamed"),
                    None,
                    None,
                    scopes,
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .unwrap()
            };
            assert_eq!(
                edited.scope_provenance,
                if scopes.is_some() {
                    ScopeProvenance::Explicit
                } else {
                    ScopeProvenance::Defaulted
                }
            );
        }
        migrate_dynamic_clients_grant_default_mcp_scopes(&db)
            .await
            .unwrap();
        assert_eq!(
            get_client(&db, &client.id).await.unwrap().allowed_scopes,
            "openid"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dcr_migration_does_not_overwrite_a_concurrent_policy_edit() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;
    let armed = Arc::new(AtomicBool::new(false));
    let reached = Arc::new(tokio::sync::Notify::new());
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let handler = {
        let armed = armed.clone();
        let reached = reached.clone();
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && event.command.get_str("update") == Ok(OAUTH_CLIENTS)
                && armed.swap(false, Ordering::SeqCst)
            {
                reached.notify_one();
                release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(15))
                    .expect("resume migration write");
            }
        })
    };
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "dcr_migration_race",
        handler,
    )
    .await
    .expect("MongoDB required");
    for legacy_writer in [false, true] {
        let client = dcr_client(&db, ScopeProvenance::Defaulted).await;
        armed.store(true, Ordering::SeqCst);
        let task = {
            let db = db.clone();
            tokio::spawn(async move { migrate_dynamic_clients_grant_default_mcp_scopes(&db).await })
        };
        tokio::time::timeout(Duration::from_secs(10), reached.notified())
            .await
            .unwrap();
        let edited_scopes = if legacy_writer {
            // An older writer can change scopes without recording provenance.
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .update_one(
                    doc! {"_id": &client.id},
                    doc! {"$set": {"allowed_scopes": "openid"}},
                )
                .await
                .unwrap();
            "openid"
        } else {
            // An explicit save of the very same scope string must fence the sweep too.
            admin_update_client(
                &db,
                &client.id,
                AdminUpdateClient {
                    allowed_scopes: Some("openid email"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            "openid email"
        };
        release_tx.send(()).unwrap();
        task.await.unwrap().unwrap();
        let stored = get_client(&db, &client.id).await.unwrap();
        assert_eq!(stored.allowed_scopes, edited_scopes);
        if !legacy_writer {
            assert_eq!(stored.scope_provenance, ScopeProvenance::Explicit);
        }
    }
}

#[test]
fn dcr_scope_resolution_defaults_only_without_known_scopes() {
    for requested in [
        None,
        Some(""),
        Some(" \t\n"),
        Some("claudeai"),
        Some("unknown other"),
    ] {
        let (scopes, provenance) = resolve_dcr_allowed_scopes(requested).unwrap();
        assert_eq!(scopes, DEFAULT_MCP_ALLOWED_SCOPES);
        assert_eq!(provenance, ScopeProvenance::Defaulted);
    }
    for (requested, expected) in [
        ("email unknown email", "openid email"),
        ("openid claudeai", "openid"),
        ("proxy", "openid proxy"),
    ] {
        let (scopes, provenance) = resolve_dcr_allowed_scopes(Some(requested)).unwrap();
        assert_eq!(scopes, expected);
        assert_eq!(provenance, ScopeProvenance::Explicit);
    }
    assert!(resolve_dcr_allowed_scopes(Some(&"openid ".repeat(64))).is_ok());
    assert!(resolve_dcr_allowed_scopes(Some(&"openid ".repeat(65))).is_err());
    assert!(resolve_dcr_allowed_scopes(Some(&"x".repeat(256))).is_ok());
    assert!(resolve_dcr_allowed_scopes(Some(&"x".repeat(257))).is_err());
    assert!(validate_allowed_scopes("email unknown").is_err());
}
