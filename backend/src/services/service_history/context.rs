use std::{
    future::Future,
    sync::{Arc, Mutex},
};

use axum::{extract::Request, middleware::Next, response::Response};
use bson::doc;
use uuid::Uuid;

use crate::{
    models::{
        service_change_event::{HistoryActor, HistoryActorKind, HistoryContext},
        user::{User, UserType},
    },
    mw::auth::{AuthMethod, AuthUser},
};

tokio::task_local! {
    static CONTEXT: RequestContext;
}

struct RequestContext {
    attribution: Mutex<Option<HistoryContext>>,
    ids: super::projection::EventIds,
}
impl RequestContext {
    fn new(attribution: Option<HistoryContext>) -> Self {
        Self {
            attribution: Mutex::new(attribution),
            ids: Arc::new(Mutex::new(Default::default())),
        }
    }
}

pub(super) fn event_ids() -> super::projection::EventIds {
    CONTEXT
        .try_with(|slot| slot.ids.clone())
        .unwrap_or_else(|_| Arc::new(Mutex::new(Default::default())))
}

pub async fn middleware(request: Request, next: Next) -> Response {
    CONTEXT
        .scope(RequestContext::new(None), next.run(request))
        .await
}

pub fn current() -> Option<HistoryContext> {
    CONTEXT
        .try_with(|slot| {
            slot.attribution
                .lock()
                .expect("history context poisoned")
                .clone()
        })
        .ok()
        .flatten()
}

pub fn system(component: &str) -> HistoryContext {
    HistoryContext {
        actor: HistoryActor {
            kind: HistoryActorKind::System,
            id: component.into(),
            name: "NyxID".into(),
            person_id: None,
            api_key_id: None,
            app_id: None,
        },
        change_group_id: Uuid::new_v4().to_string(),
        operation: component.into(),
    }
}

pub async fn scope<T>(context: HistoryContext, future: impl Future<Output = T>) -> T {
    CONTEXT
        .scope(RequestContext::new(Some(context)), future)
        .await
}

/// Verified REST and MCP authentication adapters bind attribution once per request.
pub async fn authenticated(
    db: &mongodb::Database,
    auth: &AuthUser,
) -> crate::errors::AppResult<()> {
    if CONTEXT.try_with(|_| ()).is_err() || current().is_some() {
        return Ok(());
    }
    let user = db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! { "_id": auth.user_id.to_string() })
        .await?;
    let person = user
        .as_ref()
        .is_some_and(|u| u.user_type == UserType::Person);
    let kind = if auth.api_key_id.is_some() {
        HistoryActorKind::ApiKey
    } else if auth.auth_method == AuthMethod::ServiceAccount {
        HistoryActorKind::ServiceAccount
    } else if person {
        HistoryActorKind::Person
    } else {
        HistoryActorKind::App
    };
    let name = auth
        .api_key_name
        .as_deref()
        .or_else(|| user.as_ref().and_then(|u| u.display_name.as_deref()))
        .filter(|s| !s.is_empty())
        .unwrap_or(match kind {
            HistoryActorKind::Person => "User",
            HistoryActorKind::ApiKey => "API key",
            HistoryActorKind::ServiceAccount => "Service account",
            _ => "Application",
        });
    let context = HistoryContext {
        actor: HistoryActor {
            kind,
            id: auth
                .api_key_id
                .clone()
                .unwrap_or_else(|| auth.user_id.to_string()),
            name: name.chars().take(120).collect(),
            person_id: person.then(|| auth.user_id.to_string()),
            api_key_id: auth.api_key_id.clone(),
            app_id: auth
                .acting_client_id
                .clone()
                .or_else(|| auth.oauth_client_id.clone()),
        },
        change_group_id: Uuid::new_v4().to_string(),
        operation: "authenticated_service_management".into(),
    };
    replace(context);
    Ok(())
}

/// Restore only server-persisted initiation context, never request payload metadata.
pub fn replace(context: HistoryContext) {
    let _ = CONTEXT.try_with(|slot| {
        *slot.attribution.lock().expect("history context poisoned") = Some(context)
    });
}

/// Receipt IDs are server-generated and retain grouping across recovery requests.
pub fn receipt(id: &str) {
    if let Some(mut context) = current() {
        context.change_group_id = id.into();
        context.operation = "assistant_service_management".into();
        replace(context);
    }
}
