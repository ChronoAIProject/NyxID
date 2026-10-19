use crate::{
    errors::{AppError, AppResult},
    models::machine_desktop::{COLLECTION_NAME, MachineDesktop},
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use nyxid_machine::desktop::Display;

pub fn supports_context_desktop(node: &crate::models::node::Node) -> bool {
    node.machine.as_ref().is_some_and(|profile| {
        profile.authority_v2() && profile.separated.as_ref().is_some_and(|s| s.available)
    })
}

/// Context desktops are personal browser sessions, even on org-owned hardware.
/// A personal machine owner also retains their existing physical authority.
pub async fn authorize_context(
    db: &Database,
    node: &crate::models::node::Node,
    viewer: &str,
    context_id: Option<&str>,
) -> AppResult<()> {
    let Some(context_id) = context_id else {
        return Ok(());
    };
    // A persisted context can outlive a node downgrade. Never send its desktop
    // selector to an old runtime that could treat it as the shared display.
    if !supports_context_desktop(node) {
        return Err(AppError::MachineAuthorityUnsupported);
    }
    let mut filter = doc! {
        "_id": context_id,
        "node_id": &node.id,
        "owner_id": &node.user_id,
        "mode": "separated"
    };
    if node.user_id != viewer {
        filter.insert("actor_id", viewer);
    }
    if db
        .collection::<bson::Document>(crate::models::machine_access::CONTEXTS)
        .find_one(filter)
        .projection(doc! {"_id": 1})
        .await?
        .is_none()
    {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(())
}

pub async fn get(db: &Database, node: &str) -> AppResult<Option<MachineDesktop>> {
    let mut row = db
        .collection::<MachineDesktop>(COLLECTION_NAME)
        .find_one(doc! {"_id":node})
        .await?;
    if let Some(row) = &mut row
        && row.node_id.is_empty()
    {
        row.node_id = row.id.clone();
    }
    Ok(row)
}

pub async fn agent_display_allowed(db: &Database, node: &str, display: Display) -> AppResult<()> {
    agent_display_allowed_for_context(db, node, display, None).await
}

pub async fn agent_display_allowed_for_context(
    db: &Database,
    node: &str,
    display: Display,
    context_id: Option<&str>,
) -> AppResult<()> {
    let mut filter = doc! {
        "node_id": node,
        "display": match display { Display::Secure => "secure", Display::Dev => "dev" },
        "status": {"$in": ["owner", "requested", "taking", "returning"]}
    };
    match context_id {
        Some(context_id) => {
            filter.insert("context_id", context_id);
        }
        None => {
            filter.insert(
                "$or",
                vec![
                    doc! {"context_id": bson::Bson::Null},
                    doc! {"context_id": {"$exists": false}},
                ],
            );
        }
    }
    if db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(filter)
        .await?
        .is_some()
    {
        return Err(AppError::MachineOwnerInControl);
    }
    Ok(())
}

pub async fn agent_allowed(db: &Database, node: &str) -> AppResult<()> {
    agent_display_allowed(db, node, Display::Secure).await?;
    agent_display_allowed(db, node, Display::Dev).await
}

pub async fn agent_allowed_for_context(
    db: &Database,
    node: &str,
    context_id: Option<&str>,
) -> AppResult<()> {
    agent_display_allowed_for_context(db, node, Display::Secure, context_id).await?;
    agent_display_allowed_for_context(db, node, Display::Dev, context_id).await
}

#[cfg(test)]
pub async fn open(
    db: &Database,
    owner: &str,
    node: &str,
    conversation: Option<&str>,
) -> AppResult<MachineDesktop> {
    open_display(db, owner, node, conversation, Display::Secure).await
}

#[cfg_attr(not(test), allow(dead_code))]
pub async fn open_display(
    db: &Database,
    owner: &str,
    node: &str,
    conversation: Option<&str>,
    display: Display,
) -> AppResult<MachineDesktop> {
    open_display_for_context(db, owner, node, conversation, None, display).await
}

pub async fn open_display_for_context(
    db: &Database,
    owner: &str,
    node: &str,
    conversation: Option<&str>,
    context_id: Option<&str>,
    display: Display,
) -> AppResult<MachineDesktop> {
    // Check before inserting or refreshing any state, including a brand-new
    // desktop. The stored context, never a caller's desktop user_id, is authority.
    if context_id.is_some() {
        let machine = super::node_service::get_node_by_id(db, node)
            .await?
            .ok_or(AppError::MachineNotAllowed)?;
        authorize_context(db, &machine, owner, context_id).await?;
    }
    let id = desktop_id(node, context_id, display);
    let fresh = MachineDesktop {
        id: id.clone(),
        display,
        context_id: context_id.map(str::to_owned),
        node_id: node.into(),
        session_id: uuid::Uuid::new_v4().to_string(),
        user_id: owner.into(),
        conversation_id: conversation.map(str::to_owned),
        status: "agent".into(),
        revision: 0,
        controller: None,
        reason: None,
        handback_note: None,
        updated_at: Utc::now(),
        controller_expires_at: None,
    };
    let encoded = bson::to_document(&fresh)
        .map_err(|_| AppError::Internal("Desktop state encoding failed".into()))?;
    db.collection::<MachineDesktop>(COLLECTION_NAME)
        .update_one(doc! {"_id":&id}, doc! {"$setOnInsert":encoded})
        .upsert(true)
        .await?;
    // A previous owner's idle session may be retired; active owner control
    // never expires into agent access without an explicit hand-back.
    db.collection::<MachineDesktop>(COLLECTION_NAME).update_one(
        doc!{
            "_id":&id,
            "status":{
                "$in":["agent","closed"]
            },
            "updated_at":{
                "$lt":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(40))
            }
        },
        doc!{
            "$set":bson::to_document(&fresh).map_err(|_|AppError::Internal("Desktop metadata encoding failed".into()))?
        },
    ).await?;
    let mut row = get(db, &id)
        .await?
        .ok_or(AppError::MachineBrowserUnavailable)?;
    if row.user_id != owner && context_id.is_none() {
        return Err(AppError::MachineNotAllowed);
    }
    // A hardware owner may observe another actor's context, but must not
    // rebind that actor's conversation metadata to their own thread.
    if row.status == "agent" && row.user_id == owner {
        let mut set = doc! {"updated_at":bson::DateTime::now()};
        if let Some(conversation) = conversation {
            set.insert("conversation_id", conversation);
        }
        db.collection::<MachineDesktop>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&id,"session_id":&row.session_id,"status":"agent"},
                doc! {"$set":set},
            )
            .await?;
        if let Some(conversation) = conversation {
            row.conversation_id = Some(conversation.into());
        }
    }
    Ok(row)
}

pub fn desktop_id(node: &str, context_id: Option<&str>, display: Display) -> String {
    match context_id {
        Some(context) => format!("{}:{}", display.key(node), context),
        None => display.key(node),
    }
}

pub async fn list(
    db: &Database,
    owner: &str,
    conversation: Option<&str>,
) -> AppResult<Vec<MachineDesktop>> {
    let mut filter = doc! {
        "user_id":owner,
        "status":{
            "$ne":"closed"
        },
        "$or":[{
            "status":{
                "$ne":"agent"
            }
        },{
            "updated_at":{
                "$gt":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(40))
            }
        }]
    };
    if let Some(conversation) = conversation {
        filter.insert("conversation_id", conversation);
    }
    let mut rows: Vec<MachineDesktop> = db
        .collection::<MachineDesktop>(COLLECTION_NAME)
        .find(filter)
        .limit(32)
        .await?
        .try_collect()
        .await?;
    for row in &mut rows {
        if row.node_id.is_empty() {
            row.node_id = row.id.clone();
        }
    }
    Ok(rows)
}

/// A stale desktop row's user_id is not authority over a context. Filter the
/// bounded list against current nodes and Context rows in two batched queries;
/// the pre-context listing behavior is unchanged.
pub async fn visible_context_rows(
    db: &Database,
    viewer: &str,
    rows: Vec<MachineDesktop>,
) -> AppResult<Vec<MachineDesktop>> {
    let ids: Vec<_> = rows
        .iter()
        .filter_map(|row| row.context_id.as_deref())
        .collect();
    if ids.is_empty() {
        return Ok(rows);
    }
    let contexts: Vec<crate::models::machine_access::Context> = db
        .collection(crate::models::machine_access::CONTEXTS)
        .find(doc! {"_id": {"$in": ids}, "mode": "separated"})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    let node_ids: Vec<_> = contexts.iter().map(|context| &context.node_id).collect();
    let owners = super::machine_service::usable_owners(db, viewer).await?;
    let nodes: Vec<crate::models::node::Node> = db
        .collection(crate::models::node::COLLECTION_NAME)
        .find(doc! {"_id": {"$in": node_ids}, "user_id": {"$in": owners}})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .filter(|row| {
            row.context_id.as_deref().is_none_or(|id| {
                contexts.iter().any(|context| {
                    context.id == id
                        && context.node_id == row.node_id
                        && nodes.iter().any(|node| {
                            node.id == context.node_id
                                && node.user_id == context.owner_id
                                && supports_context_desktop(node)
                                && (node.user_id == viewer || context.actor_id == viewer)
                        })
                })
            })
        })
        .collect())
}

pub async fn request(
    db: &Database,
    row: &MachineDesktop,
    reason: &str,
) -> AppResult<MachineDesktop> {
    if reason.is_empty() || reason.len() > 1000 {
        return Err(AppError::ValidationError(
            "Give a short reason for owner control".into(),
        ));
    }
    transition(
        db,
        row,
        doc! {"status":"agent"},
        doc! {
            "status":"requested",
            "reason":reason,
            "handback_note":bson::Bson::Null
        },
    )
    .await
}

pub async fn take(db: &Database, row: &MachineDesktop, viewer: &str) -> AppResult<MachineDesktop> {
    transition(
        db,
        row,
        doc! {
            "$or":[{
                "status":{
                    "$in":["agent","requested"]
                }
            },{
                "status":{
                    "$in":["owner","taking","returning"]
                },"controller_expires_at":{
                    "$lte":bson::DateTime::now()
                }
            },{
                "status":"owner","controller":viewer
            }]
        },
        doc! {
            "status":"taking",
            "controller":viewer,
            "controller_expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(40))
        },
    )
    .await
}

pub async fn controlled(
    db: &Database,
    row: &MachineDesktop,
    viewer: &str,
) -> AppResult<MachineDesktop> {
    acknowledge(
        db,
        row,
        doc! {"status":"taking","controller":viewer},
        doc! {"status":"owner"},
    )
    .await
}

pub async fn release(
    db: &Database,
    row: &MachineDesktop,
    viewer: &str,
    note: &str,
) -> AppResult<MachineDesktop> {
    if note.len() > 2000 {
        return Err(AppError::ValidationError(
            "Hand-back note is too long".into(),
        ));
    }
    transition(
        db,
        row,
        doc! {"status":"owner","controller":viewer},
        doc! {"status":"returning","handback_note":note},
    )
    .await
}

pub async fn returned(db: &Database, row: &MachineDesktop) -> AppResult<MachineDesktop> {
    acknowledge(
        db,
        row,
        doc! {"status":"returning"},
        doc! {
            "status":"agent",
            "controller":bson::Bson::Null,
            "controller_expires_at":bson::Bson::Null
        },
    )
    .await
}

pub async fn touch(db: &Database, row: &MachineDesktop) -> AppResult<()> {
    db.collection::<MachineDesktop>(COLLECTION_NAME)
        .update_one(
            doc! {
                "_id":&row.id,
                "session_id":&row.session_id,
                "user_id":&row.user_id
            },
            doc! {"$set":{"updated_at":bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

pub async fn refresh(db: &Database, row: &MachineDesktop, viewer: &str) -> AppResult<()> {
    db.collection::<MachineDesktop>(COLLECTION_NAME).update_one(doc!{
        "_id":&row.id,
        "session_id":&row.session_id,
        "controller":viewer,
        "status":"owner"
    },doc!{
        "$set":{
            "updated_at":bson::DateTime::now(),
            "controller_expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(40))
        }
    }).await?;
    Ok(())
}

async fn transition(
    db: &Database,
    row: &MachineDesktop,
    mut filter: bson::Document,
    mut set: bson::Document,
) -> AppResult<MachineDesktop> {
    filter.insert("_id", &row.id);
    filter.insert("session_id", &row.session_id);
    filter.insert("user_id", &row.user_id);
    set.insert("updated_at", bson::DateTime::now());
    db.collection::<MachineDesktop>(COLLECTION_NAME)
        .find_one_and_update(filter, doc! {"$set":set,"$inc":{"revision":1}})
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Conflict("Desktop controller changed; refresh the panel".into()))
}

/// Completing a node-acknowledged transition retains the revision sent to the
/// node. Browser input must carry that same fence, and a delayed acknowledgement
/// must never finish a newer controller's transition.
async fn acknowledge(
    db: &Database,
    row: &MachineDesktop,
    mut filter: bson::Document,
    mut set: bson::Document,
) -> AppResult<MachineDesktop> {
    filter.insert("_id", &row.id);
    filter.insert("session_id", &row.session_id);
    filter.insert("user_id", &row.user_id);
    filter.insert("revision", row.revision);
    set.insert("updated_at", bson::DateTime::now());
    db.collection::<MachineDesktop>(COLLECTION_NAME)
        .find_one_and_update(filter, doc! {"$set":set})
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Conflict("Desktop controller changed; refresh the panel".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn display_control_is_independent_and_legacy_rows_keep_node_identity() {
        let db =
            crate::test_utils::connect_transaction_test_database("machine_display_control").await;
        let secure = open(&db, "owner", "machine", Some("thread")).await.unwrap();
        let dev = open_display(&db, "owner", "machine", Some("thread"), Display::Dev)
            .await
            .unwrap();
        let (_, token, _) = super::super::node_service::create_registration_token(
            &db,
            "owner",
            "context-machine",
            100,
            300,
        )
        .await
        .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        let (mut node, _, _) = super::super::node_service::register_node(&db, &keys, &token, None)
            .await
            .unwrap();
        node.id = "machine".into();
        node.machine = Some(nyxid_machine::MachineProfile {
            authority_versions: vec![2],
            separated: Some(nyxid_machine::context::Support {
                available: true,
                ..Default::default()
            }),
            ..Default::default()
        });
        db.collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
            .insert_one(node)
            .await
            .unwrap();
        for context in ["context-a", "context-b"] {
            db.collection::<bson::Document>(crate::models::machine_access::CONTEXTS)
                .insert_one(
                    doc! {"_id": context, "node_id": "machine", "owner_id": "owner",
                    "actor_id": "owner", "mode": "separated"},
                )
                .await
                .unwrap();
        }
        let context_secure = open_display_for_context(
            &db,
            "owner",
            "machine",
            Some("thread"),
            Some("context-a"),
            Display::Secure,
        )
        .await
        .unwrap();
        let context_dev = open_display_for_context(
            &db,
            "owner",
            "machine",
            Some("thread"),
            Some("context-b"),
            Display::Secure,
        )
        .await
        .unwrap();
        assert_ne!(secure.session_id, dev.session_id);
        assert_ne!(context_secure.id, context_dev.id);
        assert!(context_secure.id.contains("context-a"));
        assert_eq!(secure.id, Display::Secure.key("machine"));
        let context_a_owner = take(&db, &context_secure, "context-a-owner").await.unwrap();
        controlled(&db, &context_a_owner, "context-a-owner")
            .await
            .unwrap();
        assert!(
            agent_display_allowed_for_context(&db, "machine", Display::Secure, Some("context-a"))
                .await
                .is_err()
        );
        agent_display_allowed_for_context(&db, "machine", Display::Secure, Some("context-b"))
            .await
            .unwrap();
        assert_eq!(dev.node_id, "machine");
        let taken = take(&db, &dev, "viewer").await.unwrap();
        controlled(&db, &taken, "viewer").await.unwrap();
        agent_display_allowed(&db, "machine", Display::Secure)
            .await
            .unwrap();
        assert!(
            agent_display_allowed(&db, "machine", Display::Dev)
                .await
                .is_err()
        );
        assert!(
            agent_allowed(&db, "machine").await.is_err(),
            "shell/files must protect both desktops"
        );
        db.collection::<MachineDesktop>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":"machine"},
                doc! {"$unset":{"node_id":"","display":""}},
            )
            .await
            .unwrap();
        let rows = list(&db, "owner", Some("thread")).await.unwrap();
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|row| row.node_id == "machine"));
        assert_eq!(
            rows.iter().filter(|row| row.context_id.is_none()).count(),
            2
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn controller_races_recover_without_releasing_agent_authority() {
        let db =
            crate::test_utils::connect_transaction_test_database("machine_desktop_control").await;
        let row = open(&db, "owner", "machine", Some("thread")).await.unwrap();
        agent_allowed(&db, "machine").await.unwrap();
        let requested = request(&db, &row, "Please sign in").await.unwrap();
        assert!(matches!(
            agent_allowed(&db, "machine").await,
            Err(AppError::MachineOwnerInControl)
        ));
        assert!(open(&db, "other", "machine", None).await.is_err());
        let (a, b) = tokio::join!(
            take(&db, &requested, "tab-a"),
            take(&db, &requested, "tab-b")
        );
        assert_ne!(a.is_ok(), b.is_ok(), "one controller wins atomically");
        let taken = a.or(b).unwrap();
        let viewer = taken.controller.as_deref().unwrap();
        assert!(controlled(&db, &taken, "wrong-tab").await.is_err());
        let owner = controlled(&db, &taken, viewer).await.unwrap();
        assert_eq!(
            owner.revision, taken.revision,
            "input uses the node's acknowledged revision"
        );
        assert!(release(&db, &owner, "wrong-tab", "done").await.is_err());
        let releasing = release(&db, &owner, viewer, "logged in").await.unwrap();
        assert!(agent_allowed(&db, "machine").await.is_err());
        let returned = returned(&db, &releasing).await.unwrap();
        assert_eq!(returned.revision, releasing.revision);
        assert_eq!(returned.handback_note.as_deref(), Some("logged in"));
        assert!(returned.revision > taken.revision);
        agent_allowed(&db, "machine").await.unwrap();
        // A replica dying between the durable claim and the node ack must be
        // recoverable by the human, while the agent remains locked out.
        let stalled = take(&db, &returned, "dead-tab").await.unwrap();
        db.collection::<MachineDesktop>(COLLECTION_NAME).update_one(doc!{"_id":"machine"},doc!{
            "$set":{
                "controller_expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))
            }
        }).await.unwrap();
        assert!(agent_allowed(&db, "machine").await.is_err());
        let recovered = take(&db, &stalled, "new-tab").await.unwrap();
        assert_eq!(recovered.controller.as_deref(), Some("new-tab"));
        assert!(recovered.revision > stalled.revision);
        assert!(controlled(&db, &stalled, "dead-tab").await.is_err());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn idle_viewers_do_not_reserve_an_org_machine_forever() {
        let db = crate::test_utils::connect_transaction_test_database("machine_desktop_idle").await;
        let row = open(&db, "admin-one", "machine", Some("thread-one"))
            .await
            .unwrap();
        db.collection::<MachineDesktop>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":"machine"},
                doc! {
                    "$set":{
                        "updated_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(41))
                    }
                },
            )
            .await
            .unwrap();
        assert!(list(&db, "admin-one", None).await.unwrap().is_empty());
        let next = open(&db, "admin-two", "machine", Some("thread-two"))
            .await
            .unwrap();
        assert_ne!(row.session_id, next.session_id);
        let taking = take(&db, &next, "tab").await.unwrap();
        controlled(&db, &taking, "tab").await.unwrap();
        db.collection::<MachineDesktop>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":"machine"},
                doc! {
                    "$set":{
                        "updated_at":bson::DateTime::from_chrono(Utc::now()-Duration::hours(1))
                    }
                },
            )
            .await
            .unwrap();
        assert!(
            open(&db, "admin-one", "machine", None).await.is_err(),
            "idle owner sessions never silently return to the agent"
        );
        db.drop().await.unwrap();
    }
}
