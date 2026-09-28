//! Role-to-profile routing for NyxBot agents. Admins author the routes now;
//! turns keep using the conversation's own profile until routing is enabled.
use chrono::Utc;
use mongodb::{Database, bson::doc, options::ReturnDocument};
use std::collections::BTreeMap;

use crate::{
    errors::{AppError, AppResult},
    models::assistant_profile_route::{AssistantProfileRoutes, COLLECTION_NAME, DOCUMENT_ID},
};

/// Routing is implemented but not yet in use: every agent keeps the model it
/// was created with (a subagent inherits its orchestrator's). Flip only after
/// NyxAgent publishes the target profiles and the routes are reviewed.
pub const ROUTING_ACTIVE: bool = false;
pub const MAX_SPECIALTIES: usize = 16;

#[derive(Clone, Copy, Debug)]
pub enum RouteRole<'a> {
    Orchestrator,
    Subagent { specialty: Option<&'a str> },
    Channel,
}

pub async fn get(db: &Database) -> AppResult<Option<AssistantProfileRoutes>> {
    Ok(db
        .collection::<AssistantProfileRoutes>(COLLECTION_NAME)
        .find_one(doc! {"_id": DOCUMENT_ID})
        .await?)
}

pub fn valid_specialty(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

#[derive(Clone, Debug, Default)]
pub struct RoutesUpdate {
    pub orchestrator: Option<String>,
    pub subagent: Option<String>,
    pub channel: Option<String>,
    pub subagent_roles: BTreeMap<String, String>,
}

/// Replace the routes. Every profile must be a NyxAgent profile id.
pub async fn set(
    db: &Database,
    admin_user_id: &str,
    update: RoutesUpdate,
) -> AppResult<AssistantProfileRoutes> {
    let valid = |value: &Option<String>| {
        value
            .as_deref()
            .is_none_or(super::assistant_nyxagent::valid_model)
    };
    if !valid(&update.orchestrator) || !valid(&update.subagent) || !valid(&update.channel) {
        return Err(AppError::ValidationError(
            "Profiles must be NyxAgent profile ids (nyxagent/<name>)".into(),
        ));
    }
    if update.subagent_roles.len() > MAX_SPECIALTIES
        || update.subagent_roles.iter().any(|(role, profile)| {
            !valid_specialty(role) || !super::assistant_nyxagent::valid_model(profile)
        })
    {
        return Err(AppError::ValidationError(format!(
            "subagent_roles allows at most {MAX_SPECIALTIES} lowercase role names mapped to \
            NyxAgent profile ids"
        )));
    }
    let row = AssistantProfileRoutes {
        id: DOCUMENT_ID.into(),
        orchestrator: update.orchestrator,
        subagent: update.subagent,
        channel: update.channel,
        subagent_roles: update.subagent_roles,
        updated_by: admin_user_id.into(),
        updated_at: Utc::now(),
    };
    db.collection::<AssistantProfileRoutes>(COLLECTION_NAME)
        .find_one_and_replace(doc! {"_id": DOCUMENT_ID}, &row)
        .upsert(true)
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Internal("Profile routes unavailable".into()))
}

/// The profile a new agent runs with. While routing is inactive this is
/// always `inherited` (the caller's current profile).
pub fn resolve(
    routes: Option<&AssistantProfileRoutes>,
    role: RouteRole<'_>,
    inherited: &str,
    active: bool,
) -> String {
    let Some(routes) = routes.filter(|_| active) else {
        return inherited.into();
    };
    let routed = match role {
        RouteRole::Orchestrator => routes.orchestrator.as_deref(),
        RouteRole::Channel => routes.channel.as_deref(),
        RouteRole::Subagent { specialty } => specialty
            .and_then(|specialty| routes.subagent_roles.get(specialty).map(String::as_str))
            .or(routes.subagent.as_deref()),
    };
    routed.unwrap_or(inherited).to_owned()
}

pub async fn model_for(db: &Database, role: RouteRole<'_>, inherited: &str) -> String {
    if !ROUTING_ACTIVE {
        return inherited.into();
    }
    match get(db).await {
        Ok(routes) => resolve(routes.as_ref(), role, inherited, true),
        Err(_) => inherited.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routes() -> AssistantProfileRoutes {
        AssistantProfileRoutes {
            id: DOCUMENT_ID.into(),
            orchestrator: Some("nyxagent/chat".into()),
            subagent: Some("nyxagent/worker".into()),
            channel: Some("nyxagent/channel".into()),
            subagent_roles: BTreeMap::from([("research".into(), "nyxagent/research".into())]),
            updated_by: "admin".into(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn inactive_routing_always_inherits() {
        const { assert!(!ROUTING_ACTIVE) };
        for role in [
            RouteRole::Orchestrator,
            RouteRole::Channel,
            RouteRole::Subagent {
                specialty: Some("research"),
            },
        ] {
            assert_eq!(
                resolve(Some(&routes()), role, "nyxagent/chat", false),
                "nyxagent/chat"
            );
        }
    }

    #[test]
    fn active_routing_prefers_specialty_then_role_then_inherited() {
        let routes = routes();
        let subagent = |specialty| RouteRole::Subagent { specialty };
        assert_eq!(
            resolve(Some(&routes), subagent(Some("research")), "x", true),
            "nyxagent/research"
        );
        assert_eq!(
            resolve(Some(&routes), subagent(Some("writer")), "x", true),
            "nyxagent/worker"
        );
        assert_eq!(
            resolve(Some(&routes), RouteRole::Channel, "x", true),
            "nyxagent/channel"
        );
        assert_eq!(resolve(None, RouteRole::Channel, "x", true), "x");
    }
}
