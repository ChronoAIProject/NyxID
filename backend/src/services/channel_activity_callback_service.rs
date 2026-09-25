//! Receiver declarations are opt-in and scoped to a route, key and callback revision.
use bson::doc;
use serde::{Deserialize, Serialize};

use crate::errors::{AppError, AppResult};
use crate::models::{
    api_key::{ApiKey, COLLECTION_NAME as KEYS},
    channel_activity::ActivityCallback,
    channel_conversation::{COLLECTION_NAME as ROUTES, ChannelConversation},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub version: u8,
    pub kinds: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CallbackSupport {
    pub declared: bool,
    pub enabled: bool,
    pub kinds: Vec<String>,
    pub version: Option<u8>,
}

pub fn valid<'a>(route: &'a ChannelConversation, key: &ApiKey) -> Option<&'a ActivityCallback> {
    let declared = route.activity_callback.as_ref()?;
    (route.is_active
        && super::channel_routing_service::ensure_route_agent(key, &route.user_id).is_ok()
        && route.agent_api_key_id == key.id
        && declared.version == 1
        && declared.agent_api_key_id == key.id
        && declared.key_state_version == key.state_version
        && Some(declared.callback_url.as_str()) == key.callback_url.as_deref())
    .then_some(declared)
}

pub fn supports(route: &ChannelConversation, key: &ApiKey, kind: &str) -> bool {
    valid(route, key).is_some_and(|declaration| {
        declaration.enabled && declaration.kinds.iter().any(|value| value == kind)
    })
}

pub async fn status(
    db: &mongodb::Database,
    route: &ChannelConversation,
) -> AppResult<CallbackSupport> {
    let key = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id": &route.agent_api_key_id})
        .await?;
    let declaration = key.as_ref().and_then(|key| valid(route, key));
    Ok(CallbackSupport {
        declared: declaration.is_some(),
        enabled: declaration.is_some_and(|value| value.enabled),
        kinds: declaration.map_or_else(Vec::new, |value| value.kinds.clone()),
        version: declaration.map(|value| value.version),
    })
}

pub async fn declare(
    db: &mongodb::Database,
    route_id: &str,
    owner: &str,
    agent_id: &str,
    mut request: Declaration,
    adapter: &dyn super::channel_platform::PlatformAdapter,
) -> AppResult<()> {
    let kinds = adapter.activity_descriptors();
    if request.version != 1
        || request.kinds.is_empty()
        || request.kinds.len() > kinds.len()
        || request.kinds.iter().enumerate().any(|(i, kind)| {
            !kinds.iter().any(|entry| entry.kind == kind) || request.kinds[..i].contains(kind)
        })
    {
        return Err(AppError::ValidationError(
            "Declare activity version 1 and unique supported activity kinds".into(),
        ));
    }
    request.kinds.sort();
    let key = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id": agent_id})
        .await?
        .ok_or(AppError::Unauthorized("Agent unavailable".into()))?;
    super::channel_routing_service::ensure_route_agent(&key, owner)?;
    let declaration = ActivityCallback {
        version: 1,
        kinds: request.kinds,
        agent_api_key_id: agent_id.into(),
        callback_url: key.callback_url.clone().expect("validated callback"),
        key_state_version: key.state_version,
        enabled: false,
    };
    let value = bson::to_document(&declaration)
        .map_err(|_| AppError::Internal("Invalid activity declaration".into()))?;
    let mut unchanged: Vec<bson::Bson> = value
        .iter()
        .filter(|(key, _)| *key != "enabled")
        .map(|(key, value)| {
            bson::Bson::Document(
                doc! {"$eq": [format!("$activity_callback.{key}"), {"$literal": value}]},
            )
        })
        .collect();
    unchanged.push(bson::Bson::String("$activity_callback.enabled".into()));
    let result = db.collection::<ChannelConversation>(ROUTES).update_one(
        doc! {"_id": route_id, "user_id": owner, "agent_api_key_id": agent_id, "platform": adapter.platform_id(), "is_active": true, "retired_by_transfer": {"$ne": true}},
        vec![doc! {"$set": {"activity_callback": {"$mergeObjects": [
            {"$literal": value}, {"enabled": {"$and": unchanged}}
        ]}}}],
    ).await?;
    if result.matched_count != 1 {
        return Err(AppError::NotFound(
            "Assigned channel route not found".into(),
        ));
    }
    Ok(())
}

pub async fn set_enabled(
    db: &mongodb::Database,
    route: &ChannelConversation,
    enabled: bool,
) -> AppResult<()> {
    let mut filter = doc! {"_id": &route.id, "user_id": &route.user_id, "agent_api_key_id": &route.agent_api_key_id, "retired_by_transfer": {"$ne": true}};
    let update = if enabled {
        let key = db
            .collection::<ApiKey>(KEYS)
            .find_one(doc! {"_id": &route.agent_api_key_id})
            .await?
            .ok_or_else(|| {
                AppError::ValidationError("Agent must declare activity support first".into())
            })?;
        let declaration = valid(route, &key).ok_or_else(|| {
            AppError::ValidationError(
                "Agent must declare activity support for its current callback first".into(),
            )
        })?;
        filter.insert(
            "activity_callback",
            bson::to_bson(declaration)
                .map_err(|_| AppError::Internal("Invalid activity declaration".into()))?,
        );
        filter.insert("is_active", true);
        doc! {"$set": {"activity_callback.enabled": true}}
    } else {
        // Revocation remains available with a stale or deleted key.
        if route.activity_callback.is_none() {
            return Ok(());
        }
        filter.insert(
            "activity_callback",
            bson::to_bson(&route.activity_callback)
                .map_err(|_| AppError::Internal("Invalid activity declaration".into()))?,
        );
        doc! {"$set": {"activity_callback.enabled": false}}
    };
    if db
        .collection::<ChannelConversation>(ROUTES)
        .update_one(filter, update)
        .await?
        .matched_count
        != 1
    {
        return Err(AppError::Conflict(
            "Channel activity declaration changed; reload and retry".into(),
        ));
    }
    Ok(())
}
