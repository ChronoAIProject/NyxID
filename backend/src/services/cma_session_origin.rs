//! Personal CMA observation receipts. Inventory and browser routes never author attribution.
use crate::{
    errors::{AppError, AppResult},
    models::{
        async_service_operation::{AsyncServiceOperation, CmaOperation},
        cma_session_origin::{COLLECTION_NAME, CmaSessionOrigin},
        user_service::UserService,
    },
};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, doc},
    options::IndexOptions,
};
use std::collections::HashMap;

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<CmaSessionOrigin>(COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"user_id":1,"service_id":1,"remote_agent_id":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id":1,"_id":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id":1,"service_id":1,"conversation_id":1})
                .build(),
        ])
        .await?;
    Ok(())
}

/// Bind default personal routing, including destination and credential epoch.
/// No raw credential is loaded. An agent override cannot use the human REST proxy.
pub fn personal_binding<'a>(
    db: &'a Database,
    user: &'a str,
    service: &'a str,
    key: Option<&'a str>,
) -> futures::future::BoxFuture<'a, AppResult<Option<String>>> {
    Box::pin(async move {
        let Some(row) = db
            .collection::<UserService>("user_services")
            .find_one(doc! {"_id":service,"user_id":user,"is_active":true,"deleted_at":null})
            .await?
        else {
            return Ok(None);
        };
        if row.node_id.is_some() || row.service_type != "http" {
            return Ok(None);
        }
        if let Some(key) = key
            && db
                .collection::<bson::Document>("agent_service_bindings")
                .find_one(doc! {"api_key_id":key,"user_service_id":service})
                .await?
                .is_some()
        {
            return Ok(None);
        }
        let Some(endpoint) = db
            .collection::<bson::Document>("user_endpoints")
            .find_one(doc! {"_id":&row.endpoint_id,"user_id":user})
            .await?
        else {
            return Ok(None);
        };
        let Some(catalog_id) = row.catalog_service_id.as_deref() else {
            return Ok(None);
        };
        if db
            .collection::<bson::Document>("node_service_bindings")
            .find_one(doc! {"user_id":user,"service_id":catalog_id,"is_active":true})
            .projection(doc! {"_id":1})
            .await?
            .is_some()
        {
            return Ok(None);
        }
        let Some(catalog) = db.collection::<bson::Document>("downstream_services")
        .find_one(doc! {"_id":catalog_id,"is_active":true,"slug":"cma"}).projection(doc! {
            "_id":1,"auth_method":1,"auth_key_name":1,"auth_scheme":1,"default_request_headers":1,"token_exchange_config":1,
            "requires_user_credential":1,"provider_config_id":1,"platform_key":1,"base_url":1,"service_category":1,"destination_targets":1,
        }).await? else { return Ok(None) };
        let platform_key_configured = catalog
            .get("platform_key")
            .is_some_and(|v| v != &bson::Bson::Null);
        // Match the proxy's platform-route selection. Legacy auto-provisioned
        // no-auth connections have a "platform" label but still use the owner's
        // default route when no platform key is configured (e.g. signed CMA identity).
        // Explicit platform selections and configured catalog keys remain excluded.
        if (super::platform_key_service::binding(&row) == "platform"
            && (row.auth_method != "none"
                || row.credential_binding.as_deref() == Some("platform")
                || platform_key_configured))
            || (platform_key_configured && catalog.get_str("auth_method").ok() == Some("none"))
        {
            return Ok(None);
        }
        // A missing personal credential must not silently become a shared catalog key.
        if row.api_key_id.is_none() && row.auth_method != "none" {
            return Ok(None);
        }
        let credential = if let Some(id) = &row.api_key_id {
            let Some(credential) = db.collection::<bson::Document>("user_api_keys")
            .find_one(doc! {"_id":id,"user_id":user}).projection(doc! {
                "_id":1,"credential_epoch":1,"provider_config_id":1,"connection_id":1,"credential_source":1,"status":1,
            }).await? else { return Ok(None) };
            if credential
                .get_str("status")
                .is_ok_and(|status| status != "active")
            {
                return Ok(None);
            }
            let epoch = match credential.get("credential_epoch") {
                None => 1,
                Some(bson::Bson::Int64(value)) => *value,
                Some(bson::Bson::Int32(value)) => i64::from(*value),
                _ => return Ok(None),
            };
            Some(serde_json::json!({
                "id":id,"epoch":epoch,
                "provider":credential.get_str("provider_config_id").ok(),
                "connection":credential.get_str("connection_id").ok(),
                "source":credential.get_str("credential_source").ok(),
            }))
        } else {
            None
        };
        use sha2::{Digest, Sha256};
        let projection = serde_json::json!({
        "connection":row.id,"owner":user,"endpoint":row.endpoint_id,
        "base_url":endpoint.get_str("url").ok(),"credential":credential,
        "credential_binding":super::platform_key_service::binding(&row),
        "custom_user_agent":row.custom_user_agent,"service_type":row.service_type,
        "catalog":{
            "id":catalog_id,"base_url":catalog.get_str("base_url").ok(),
            "platform_key":catalog.get("platform_key").unwrap_or(&bson::Bson::Null),
            "destinations":catalog.get("destination_targets").unwrap_or(&bson::Bson::Null),
            "category":catalog.get_str("service_category").ok(),"auth_method":catalog.get_str("auth_method").ok(),
            "auth_key_name":catalog.get_str("auth_key_name").ok(),
            "auth_scheme":catalog.get_str("auth_scheme").ok(),
            "headers":catalog.get("default_request_headers").unwrap_or(&bson::Bson::Null),
            "token_exchange":catalog.get("token_exchange_config").unwrap_or(&bson::Bson::Null),
            "requires_user_credential":catalog.get_bool("requires_user_credential").unwrap_or(true),
            "provider":catalog.get_str("provider_config_id").ok(),
        },"auth":[row.auth_method,row.auth_key_name],
        "headers":row.default_request_headers,
        "identity":[row.identity_propagation_mode, row.identity_jwt_audience, row.delegation_token_scope],
        "identity_fields":[row.identity_include_user_id,row.identity_include_email,row.identity_include_name,
            row.forward_access_token,row.inject_delegation_token],
        });
        let bytes = serde_json::to_vec(&projection)
            .map_err(|_| AppError::Internal("CMA connection projection failed".into()))?;
        Ok(Some(hex::encode(Sha256::digest(bytes))))
    })
}

pub fn record<'a>(
    db: &'a Database,
    watch: &'a AsyncServiceOperation,
    cma: &'a CmaOperation,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let (Some(binding), Some(remote)) = (&cma.observer_binding, &cma.agent_id) else {
            return Ok(());
        };
        if watch.shared_execution.is_some()
            || watch.execution_binding.is_none()
            || watch.delivery.group_id.is_some()
            || watch.delivery.invocation_id.is_some()
            || !valid_agent(remote)
        {
            return Ok(());
        }
        // Revalidate both ownership and exact personal routing before retaining a link.
        if personal_binding(
            db,
            &watch.user_id,
            &watch.service_id,
            Some(&watch.api_key_id),
        )
        .await?
        .as_ref()
            != Some(binding)
        {
            return Ok(());
        }
        let conversation = match Box::pin(super::assistant_nyxagent::get(
            db,
            &watch.user_id,
            &watch.conversation_id,
        ))
        .await
        {
            Ok(row) => row,
            Err(AppError::NotFound(_) | AppError::Forbidden(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
        if conversation.group_id.is_some()
            || conversation.shared_agent_thread
            || conversation.collaboration.is_some()
        {
            return Ok(());
        }
        let origin = CmaSessionOrigin {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: watch.user_id.clone(),
            service_id: watch.service_id.clone(),
            remote_agent_id: remote.clone(),
            connection_binding: binding.clone(),
            conversation_id: watch.conversation_id.clone(),
            turn_id: watch.turn_id.clone(),
            agent_id: watch.agent_id.clone(),
            callback_url: super::cma_operation::card_url(cma),
            created_at: watch.created_at,
        };
        let serialized = bson::to_document(&origin)
            .map_err(|_| AppError::Internal("CMA origin serialization failed".into()))?;
        // Resolve observations arriving out of order to the earliest verified
        // invocation. A later reuse can never move the source conversation.
        let earlier = doc! {"$or":[
            {"$eq":[{"$type":"$created_at"},"missing"]},
            {"$gt":["$created_at",bson::DateTime::from_chrono(origin.created_at)]},
        ]};
        let mut fields = doc! {
            "_id":{"$ifNull":["$_id",{"$literal":&origin.id}]},
            "callback_url":{"$literal":&origin.callback_url},
        };
        for field in ["conversation_id", "turn_id", "agent_id", "created_at"] {
            fields.insert(field, doc! {"$cond":[earlier.clone(),{"$literal":serialized.get(field).cloned().unwrap_or(bson::Bson::Null)},format!("${field}")]});
        }
        #[cfg(test)]
        pause_record(&watch.id, "before_tx").await;
        let context = (db.clone(), watch.clone(), origin, fields);
        super::transaction_service::run_db_only(db, &context, |tx, (db, watch, origin, fields)| {
            Box::pin(persist_in_session(db, watch, origin, fields, tx))
        })
        .await
    })
}

async fn persist_in_session(
    db: &Database,
    watch: &AsyncServiceOperation,
    origin: &CmaSessionOrigin,
    fields: &bson::Document,
    tx: &mut mongodb::ClientSession,
) -> AppResult<()> {
    super::erasure_admission::fence_in_session(db, &watch.user_id, tx).await?;
    let person = db
        .collection::<crate::models::user::User>("users")
        .find_one(
            doc! {"_id":&watch.user_id,"is_active":true,"platform_personal_deleting":{"$ne":true}},
        )
        .session(&mut *tx)
        .await?;
    if person.is_none_or(|person| !person.user_type.is_person()) {
        return Ok(());
    }
    // Writing the source row conflicts with deletion; a snapshot read alone
    // could recreate an orphan after delete_in_session removed its receipts.
    let source = db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(
            "assistant_conversations",
        )
        .find_one(doc! {"_id":&watch.conversation_id,"user_id":&watch.user_id})
        .session(&mut *tx)
        .await?;
    let Some(source) = source else { return Ok(()) };
    if source.agent_id.as_deref() != Some(watch.agent_id.as_str())
        || source.group_id.is_some()
        || source.shared_agent_thread
        || source.collaboration.is_some()
    {
        return Ok(());
    }
    #[cfg(test)]
    pause_record(&watch.id, "source_read").await;
    db.collection::<bson::Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&source.id,"user_id":&watch.user_id},
            doc! {"$inc":{"cma_origin_revision":1_i64}},
        )
        .session(&mut *tx)
        .await?;
    let existing=db.collection::<CmaSessionOrigin>(COLLECTION_NAME).find_one(doc! {"user_id":&origin.user_id,"service_id":&origin.service_id,"remote_agent_id":&origin.remote_agent_id}).session(&mut *tx).await?;
    // A replacement credential cannot adopt an old remote identity.
    if existing.is_some_and(|old| old.connection_binding != origin.connection_binding) {
        return Ok(());
    }
    db.collection::<bson::Document>(COLLECTION_NAME).update_one(doc! {"user_id":&origin.user_id,"service_id":&origin.service_id,"remote_agent_id":&origin.remote_agent_id,"connection_binding":&origin.connection_binding},vec![doc! {"$set":fields.clone()}]).upsert(true).session(tx).await?;
    Ok(())
}

fn valid_agent(id: &str) -> bool {
    id.strip_prefix("agt_").is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 128
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    })
}

pub struct OriginPage {
    pub origins: Vec<CmaSessionOrigin>,
    pub next_cursor: Option<String>,
}

/// Bounded reads, no CMA calls/decryption, live connection and private-history ACLs.
#[cfg(test)]
pub async fn page(db: &Database, user: &str, after: Option<&str>) -> AppResult<OriginPage> {
    page_filtered(db, user, after, doc! {}).await
}

#[cfg(test)]
pub async fn page_scoped(
    db: &Database,
    user: &str,
    after: Option<&str>,
    conversation: Option<&str>,
    pins: Option<&str>,
) -> AppResult<OriginPage> {
    page_candidates(db, user, after, conversation, pins, None).await
}

fn scope_filter(conversation: Option<&str>, pins: Option<&str>) -> AppResult<bson::Document> {
    let mut scopes = Vec::new();
    if let Some(conversation) = conversation {
        if conversation.is_empty()
            || conversation.len() > 200
            || conversation.chars().any(char::is_control)
        {
            return Err(AppError::BadRequest(
                "Invalid CMA source conversation".into(),
            ));
        }
        scopes.push(doc! {"conversation_id":conversation});
    }
    if let Some(pins) = pins {
        if pins.len() > 8500 {
            return Err(AppError::BadRequest("Too many CMA pins".into()));
        }
        for (i, pin) in pins.split(',').enumerate() {
            if i >= 50 {
                return Err(AppError::BadRequest("Too many CMA pins".into()));
            }
            let Some((service, agent)) = pin.split_once(':') else {
                return Err(AppError::BadRequest("Invalid CMA pin".into()));
            };
            if uuid::Uuid::parse_str(service).is_err() || !valid_agent(agent) {
                return Err(AppError::BadRequest("Invalid CMA pin".into()));
            }
            scopes.push(doc! {"service_id":service,"remote_agent_id":agent});
        }
    }
    Ok(doc! {"$or":scopes})
}

pub async fn page_candidates(
    db: &Database,
    user: &str,
    after: Option<&str>,
    conversation: Option<&str>,
    pins: Option<&str>,
    agents: Option<&str>,
) -> AppResult<OriginPage> {
    let mut scope = scope_filter(conversation, pins)?;
    if scope.get_array("$or").is_ok_and(|v| v.is_empty()) {
        return Ok(OriginPage {
            origins: Vec::new(),
            next_cursor: None,
        });
    }
    if let Some(agents) = agents {
        let candidates = scope_filter(None, Some(agents))?;
        scope = doc! {"$and":[scope,candidates]};
    }
    page_filtered(db, user, after, scope).await
}

pub struct Connection {
    pub id: String,
    pub name: String,
    pub slug: String,
}
pub struct ConnectionPage {
    pub connections: Vec<Connection>,
    pub next_cursor: Option<String>,
}

/// Scan at most 50 *connections*, never a page of historical agent receipts.
/// Inventory can propose candidates; only page_candidates can attach an origin.
pub async fn connections_page(
    db: &Database,
    user: &str,
    after: Option<&str>,
    conversation: Option<&str>,
    pins: Option<&str>,
) -> AppResult<ConnectionPage> {
    if !owner_available(db, user).await? {
        return Ok(ConnectionPage {
            connections: Vec::new(),
            next_cursor: None,
        });
    }
    let scope = scope_filter(conversation, pins)?;
    if scope.get_array("$or").is_ok_and(|v| v.is_empty()) {
        return Ok(ConnectionPage {
            connections: Vec::new(),
            next_cursor: None,
        });
    }
    if let Some(conversation) = conversation {
        let allowed = match Box::pin(super::assistant_nyxagent::get(db, user, conversation)).await {
            Ok(chat) => {
                chat.group_id.is_none() && !chat.shared_agent_thread && chat.collaboration.is_none()
            }
            Err(AppError::NotFound(_) | AppError::Forbidden(_)) => false,
            Err(error) => return Err(error),
        };
        if !allowed {
            return Ok(ConnectionPage {
                connections: Vec::new(),
                next_cursor: None,
            });
        }
    }
    let Some(catalog) = db
        .collection::<bson::Document>("downstream_services")
        .find_one(doc! {"slug":"cma","is_active":true})
        .projection(doc! {"_id":1})
        .await?
    else {
        return Ok(ConnectionPage {
            connections: Vec::new(),
            next_cursor: None,
        });
    };
    let mut filter = doc! {"user_id":user,"catalog_service_id":catalog.get_str("_id").unwrap_or(""),"is_active":true,"deleted_at":null};
    if let Some(after) = after {
        uuid::Uuid::parse_str(after)
            .map_err(|_| AppError::BadRequest("Invalid CMA connection cursor".into()))?;
        filter.insert("_id", doc! {"$gt":after});
    }
    let mut rows: Vec<UserService> = db
        .collection("user_services")
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(51)
        .max_time(std::time::Duration::from_secs(2))
        .await?
        .try_collect()
        .await?;
    let more = rows.len() > 50;
    rows.truncate(50);
    let next_cursor = more.then(|| rows.last().expect("full page").id.clone());
    let mut connections = Vec::new();
    for row in rows {
        let Some(binding) = personal_binding(db, user, &row.id, None).await? else {
            continue;
        };
        let filter = doc! {"$and":[scope.clone(), {"user_id":user,"service_id":&row.id,"connection_binding":binding}]};
        if db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(filter)
            .projection(doc! {"_id":1})
            .max_time(std::time::Duration::from_secs(2))
            .await?
            .is_none()
        {
            continue;
        }
        let endpoint = db
            .collection::<bson::Document>("user_endpoints")
            .find_one(doc! {"_id":&row.endpoint_id,"user_id":user})
            .projection(doc! {"label":1})
            .await?;
        connections.push(Connection {
            id: row.id,
            slug: row.slug,
            name: endpoint
                .as_ref()
                .and_then(|e| e.get_str("label").ok())
                .unwrap_or("CMA")
                .chars()
                .take(300)
                .collect(),
        });
    }
    Ok(ConnectionPage {
        connections,
        next_cursor,
    })
}

async fn page_filtered(
    db: &Database,
    user: &str,
    after: Option<&str>,
    mut filter: bson::Document,
) -> AppResult<OriginPage> {
    if !owner_available(db, user).await? {
        return Ok(OriginPage {
            origins: Vec::new(),
            next_cursor: None,
        });
    }
    filter.insert("user_id", user);
    if let Some(after) = after {
        uuid::Uuid::parse_str(after)
            .map_err(|_| AppError::BadRequest("Invalid CMA origin cursor".into()))?;
        filter.insert("_id", doc! {"$gt":after});
    }
    let mut rows: Vec<CmaSessionOrigin> = db
        .collection(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(51)
        .max_time(std::time::Duration::from_secs(2))
        .await?
        .try_collect()
        .await?;
    let more = rows.len() > 50;
    rows.truncate(50);
    let next_cursor = more.then(|| rows.last().expect("full page").id.clone());
    let mut bindings = HashMap::new();
    let mut conversations = HashMap::new();
    let mut origins = Vec::new();
    for row in rows {
        if !bindings.contains_key(&row.service_id) {
            bindings.insert(
                row.service_id.clone(),
                personal_binding(db, user, &row.service_id, None).await?,
            );
        }
        if bindings[&row.service_id].as_ref() != Some(&row.connection_binding)
            || !valid_agent(&row.remote_agent_id)
        {
            continue;
        }
        if !conversations.contains_key(&row.conversation_id) {
            let allowed = match super::assistant_nyxagent::get(db, user, &row.conversation_id).await
            {
                Ok(chat) => {
                    chat.group_id.is_none()
                        && !chat.shared_agent_thread
                        && chat.collaboration.is_none()
                }
                Err(AppError::NotFound(_) | AppError::Forbidden(_)) => false,
                Err(error) => return Err(error),
            };
            conversations.insert(row.conversation_id.clone(), allowed);
        }
        if conversations[&row.conversation_id] {
            origins.push(row)
        }
    }
    Ok(OriginPage {
        origins,
        next_cursor,
    })
}

/// Optional UI metadata must never change provider completion or delivery.
pub async fn record_best_effort(db: &Database, watch: &AsyncServiceOperation, cma: &CmaOperation) {
    let outcome =
        tokio::time::timeout(std::time::Duration::from_secs(2), record(db, watch, cma)).await;
    if !matches!(outcome, Ok(Ok(()))) {
        metadata_unavailable();
    }
}

pub fn metadata_unavailable() {
    use std::sync::atomic::{AtomicI64, Ordering};
    static LAST: AtomicI64 = AtomicI64::new(0);
    let now = chrono::Utc::now().timestamp();
    let last = LAST.load(Ordering::Relaxed);
    if now - last >= 60
        && LAST
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        // Never log provider bodies, credential projections, or database errors.
        tracing::warn!("CMA card metadata unavailable; a later observation may repair it");
    }
}

pub async fn connection_metadata(
    db: &Database,
    user: &str,
    id: &str,
) -> AppResult<Option<Connection>> {
    let Some(row) = db
        .collection::<UserService>("user_services")
        .find_one(doc! {"_id":id,"user_id":user,"is_active":true,"deleted_at":null})
        .await?
    else {
        return Ok(None);
    };
    let endpoint = db
        .collection::<bson::Document>("user_endpoints")
        .find_one(doc! {"_id":&row.endpoint_id,"user_id":user})
        .projection(doc! {"label":1})
        .await?;
    Ok(endpoint.map(|e| Connection {
        id: row.id,
        slug: row.slug,
        name: e
            .get_str("label")
            .unwrap_or("CMA")
            .chars()
            .take(300)
            .collect(),
    }))
}

#[cfg(test)]
type RecordPause = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);
#[cfg(test)]
fn record_pauses() -> &'static std::sync::Mutex<HashMap<String, RecordPause>> {
    static PAUSES: std::sync::OnceLock<std::sync::Mutex<HashMap<String, RecordPause>>> =
        std::sync::OnceLock::new();
    PAUSES.get_or_init(Default::default)
}
#[cfg(test)]
pub(crate) fn hold_record(
    watch: &str,
    phase: &str,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let (reached, observed) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    record_pauses()
        .lock()
        .unwrap()
        .insert(format!("{watch}:{phase}"), (reached, resume));
    (observed, release)
}
#[cfg(test)]
async fn pause_record(watch: &str, phase: &str) {
    let pause = record_pauses()
        .lock()
        .unwrap()
        .remove(&format!("{watch}:{phase}"));
    if let Some((reached, resume)) = pause {
        let _ = reached.send(());
        let _ = resume.await;
    }
}

async fn owner_available(db: &Database, user: &str) -> AppResult<bool> {
    if db
        .collection::<bson::Document>(super::erasure_admission::COLLECTION)
        .find_one(doc! {"_id":user,"erasure_accepted":true})
        .projection(doc! {"_id":1})
        .await?
        .is_some()
    {
        return Ok(false);
    }
    let person = db
        .collection::<crate::models::user::User>("users")
        .find_one(doc! {"_id":user,"is_active":true,"platform_personal_deleting":{"$ne":true}})
        .await?;
    Ok(person.is_some_and(|p| p.user_type.is_person()))
}
