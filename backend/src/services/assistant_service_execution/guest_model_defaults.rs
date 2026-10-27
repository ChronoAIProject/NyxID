//! Server-owned NyxAgent profile defaults. Never discover a route from a callback.
use super::*;
use crate::models::downstream_service::DownstreamService;

#[derive(serde::Deserialize)]
struct ModelRoute {
    service_slug: String,
    model: String,
    path: String,
}

static ROUTES: std::sync::LazyLock<std::collections::BTreeMap<String, ModelRoute>> =
    std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!("../../../specs/assistant/model-routes.json"))
            .expect("validated server model routes")
    });

pub(super) async fn matches(
    db: &Database,
    chat: &acks::ChatAuthority,
    profile: &str,
    connection: Option<&str>,
    target: &crate::services::proxy_service::ProxyTarget,
    operation: &Value,
) -> AppResult<bool> {
    let Some(configured) = ROUTES.get(profile) else {
        return Ok(false);
    };
    // Lookup is chosen exclusively by the server's profile, before comparing
    // the request-selected target. Unknown/inactive routes never fall back.
    let Some(catalog) = db
        .collection::<DownstreamService>("downstream_services")
        .find_one(doc! {"slug":&configured.service_slug,"is_active":true})
        .await?
    else {
        return Ok(false);
    };
    let master = !catalog.credential_encrypted.is_empty()
        && (crate::services::platform_key_service::has_platform_key(&catalog)
            || (catalog.service_category == "internal" && !catalog.requires_user_credential));
    if !master
        || catalog.inference.is_none()
        || target.service.id != catalog.id
        || target.base_url != catalog.base_url
        || target.target_id.is_some()
        || !target.user_service_default_headers.is_empty()
    {
        return Ok(false);
    }
    // Saved platform connections may reference the same fixed catalog route.
    // BYOK, owner nodes and per-agent credentials never bootstrap. The regular
    // resolver has already enforced live platform ACLs before materialization;
    // this metadata read repeats them for saved connections at egress too.
    let default = crate::services::proxy_service::read_default_model_authority(
        db,
        &chat.user_id,
        &configured.service_slug,
    )
    .await?;
    match (connection, default) {
        (None, None) => {}
        (Some(connection), Some(authority)) => {
            if authority.user_service_id != connection
                || !authority.master_credential
                || authority.node_id.is_some()
                || authority.api_key_id.is_some()
                || authority.pool_selection.is_some()
                || crate::services::proxy_service::read_agent_credential_override_identity(
                    db,
                    &chat.user_id,
                    &chat.api_key_id,
                    connection,
                    target,
                )
                .await?
                .is_some()
            {
                return Ok(false);
            }
        }
        _ => return Ok(false),
    }
    // Match the bridge's generation endpoint/model and semantic headers, never
    // its evolving prompt. Custom account/project/query choices remain refused.
    let mut expected = json!({"kind":"raw_inference","method":"POST",
        "path":configured.path,"query":null,"model":configured.model,"provider":catalog.id});
    let headers = axum::http::HeaderMap::from_iter([
        (
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        ),
        (
            axum::http::header::ACCEPT,
            axum::http::HeaderValue::from_static("*/*"),
        ),
    ]);
    bind_headers(
        &mut expected,
        &HttpProjection::default().headers(target, &headers),
    );
    Ok(selection_fingerprint(operation) == selection_fingerprint(&expected))
}
