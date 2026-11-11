//! Wiki lookup guidance only. No input rewrite, second request or authority grant.
use serde_json::{Value, json};

use super::{
    assistant_acknowledgement_service::ChatAuthority,
    assistant_links::{AGENT_DETAILS_GRANTS, AssistantPage},
    mcp_service::{McpToolEndpoint, McpToolService, McpToolSource},
};

pub const OPERATION: &str = "wiki_get_node";
pub const SLUGS: [&str; 4] = ["api-lark", "api-lark-bot", "api-feishu", "api-feishu-bot"];

/// Only annotate the new stored catalog operation, never a custom/mounted spec.
/// This grants no execution capability; all ordinary proxy gates already ran.
pub fn matches(
    catalog_slug: Option<&str>,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
) -> bool {
    catalog_slug.is_some_and(|slug| SLUGS.contains(&slug))
        && service
            .durable_endpoint_metadata
            .get(&endpoint.endpoint_id)
            .is_some_and(|metadata| metadata.catalog_contract && metadata.operation_generation > 0)
        && endpoint.name == OPERATION
        && endpoint.method == "GET"
        && matches!(
            endpoint.path.as_str(),
            "/wiki/v2/spaces/get_node" | "/open-apis/wiki/v2/spaces/get_node"
        )
}

fn setup_page(chat: Option<&ChatAuthority>, frontend: &str) -> String {
    match chat.filter(|chat| !chat.guest && !chat.is_orchestrator()) {
        Some(chat) => AssistantPage::AgentDetails {
            agent: &chat.agent_id,
            tab: AGENT_DETAILS_GRANTS,
        },
        None => AssistantPage::Agent { agent: None },
    }
    .url(frontend)
}

/// Provider messages may contain URLs, tokens, titles or arbitrary text. Keep
/// only a numeric code and closed metadata in failure guidance.
pub fn response(
    status: u16,
    text: &str,
    slug: &str,
    chat: Option<&ChatAuthority>,
    frontend: &str,
) -> (u16, String) {
    let value: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let code = value["code"].as_i64();
    let node = &value["data"]["node"];
    let kind = node["obj_type"].as_str();
    let (error, message, http) = match code {
        Some(99991672) => (
            "lark_scope_missing",
            "Enable wiki:wiki:readonly in the selected service application's Developer Console > Permissions & Scopes, publish the application and obtain admin approval. Feishu also accepts wiki:node:read. For user OAuth, request this scope through the hosted connection or scope-management flow and consent. A scope does not grant document access.",
            403,
        ),
        Some(131006) => (
            "lark_document_acl_denied",
            "This service connection's user or tenant application cannot read this wiki node. Ask the document owner to grant that exact identity node/document read access (and wiki membership if required), then retry. The chat bot's access is separate. Do not make the document public.",
            403,
        ),
        Some(131005) => (
            "lark_wiki_node_not_found",
            "The wiki node was not found for this connection. Ask the owner to check that the link still exists and belongs to the selected Lark/Feishu account, then resend the standard wiki link.",
            404,
        ),
        Some(0) if (200..300).contains(&status) => {
            if kind.is_none_or(str::is_empty) {
                (
                    "lark_wiki_invalid_response",
                    "The provider returned incomplete node metadata. No document read was attempted; retry the lookup later.",
                    502,
                )
            } else if !matches!(kind, Some("docx" | "bitable")) {
                (
                    "lark_wiki_unsupported_type",
                    "This wiki node's underlying object has no reader in the Lark/Feishu catalog. Tell the owner the unsupported type; do not guess its content or use its token with docx. Use a supported Document or Base, or ask for a reader to be added.",
                    422,
                )
            } else if node["obj_token"]
                .as_str()
                .is_none_or(|s| s.is_empty() || s.len() > 999)
            {
                (
                    "lark_wiki_invalid_response",
                    "The provider returned incomplete node metadata. No document read was attempted; retry the lookup later.",
                    502,
                )
            } else {
                let docx = kind == Some("docx");
                let mut value = value;
                value["nyxid_read"] = json!({
                    "operation": if docx { "docx_raw_content" } else { "bitable_tables_list" },
                    "token_parameter": if docx { "document_id" } else { "app_token" },
                    "same_connection_required": true,
                    "instructions": "Use data.node.obj_token with the indicated operation on this same connection. Each read still requires its own live grant, operation scope and document ACL. Resolution is not content or a permission grant."
                });
                if docx {
                    // The legacy docx transport/result stays byte-compatible.
                    // Give the model closed interpretations before that call,
                    // without inventing a scope/ACL failure before it happens.
                    value["nyxid_read"]["failure_guidance"] = json!({
                        "99991672": {"code":"lark_scope_missing", "message":"If docx returns 99991672, enable docx:document:readonly in this service application's Developer Console > Permissions & Scopes, publish it and renew user OAuth consent if needed."},
                        "1770032": {"code":"lark_document_acl_denied", "message":"If docx returns 1770032, ask the document owner to grant read access to this exact connected user or tenant application, then retry. Never request public sharing."},
                        "1770002": {"code":"lark_document_not_found", "message":"If docx returns 1770002, ask the owner to check that the document still exists and resend its standard link."},
                        "1770003": {"code":"lark_document_not_found", "message":"If docx returns 1770003, the resource was deleted; ask the owner for the current standard document link."}
                    });
                }
                return (status, value.to_string());
            }
        }
        _ => (
            "lark_wiki_lookup_failed",
            "Wiki lookup did not succeed. No document read was attempted. Check the selected service connection and retry only after resolving the provider failure.",
            if status >= 400 { status } else { 502 },
        ),
    };
    let mut result = json!({"error":error,"code":error,"message":message,"provider_code":code,"request_sent":true,"identity_type":if slug.ends_with("-bot") {"tenant_application"} else {"oauth_user"}});
    if error == "lark_scope_missing" {
        result["scope"] = json!("wiki:wiki:readonly");
        result["developer_console_url"] = json!(if slug.starts_with("api-feishu") {
            "https://open.feishu.cn/app"
        } else {
            "https://open.larksuite.com/app"
        });
    }
    if error == "lark_wiki_unsupported_type" {
        result["obj_type"] = json!(
            kind.filter(|kind| matches!(*kind, "doc" | "sheet" | "mindnote" | "file" | "slides"))
                .unwrap_or("unknown")
        );
    }
    if !chat.is_some_and(|chat| chat.guest) {
        result["setup_url"] = json!(setup_page(chat, frontend));
    }
    (http, result.to_string())
}

pub fn relevant_query(query: &str) -> bool {
    let q = query.to_ascii_lowercase();
    q.contains("wiki_get_node") || q.contains("lark") || q.contains("feishu") || q.contains("飞书")
}

/// Owner-only discovery assistance, using the already authorized connection
/// inventory. Never assert that a hidden connection is absent to a guest.
pub async fn discovery(
    db: &mongodb::Database,
    services: &[McpToolService],
    chat: &ChatAuthority,
    allowed: &[String],
    allowed_platform: &[String],
    frontend: &str,
) -> crate::errors::AppResult<Option<Value>> {
    use futures::TryStreamExt;
    use mongodb::bson::{Document, doc};
    if chat.guest {
        return Ok(None);
    }
    let catalogs: Vec<Document> = db
        .collection::<Document>("downstream_services")
        .find(doc! {"slug":{"$in":SLUGS.to_vec()},"is_active":true})
        .projection(doc! {"_id":1})
        .await?
        .try_collect()
        .await?;
    let connected: Vec<_> = services
        .iter()
        .filter(|service| {
            let id = match &service.source {
                McpToolSource::UserManaged {
                    catalog_service_id: Some(id),
                    ..
                } => id,
                McpToolSource::Platform { .. } => &service.service_id,
                _ => return false,
            };
            catalogs
                .iter()
                .any(|row| row.get_str("_id").ok() == Some(id.as_str()))
        })
        .collect();
    if connected.iter().any(|service| {
        chat.is_orchestrator()
            || match service.source {
                McpToolSource::Platform { .. } => allowed_platform.contains(&service.service_id),
                _ => allowed.contains(&service.service_id),
            }
    }) {
        return Ok(None);
    }
    let (code, message) = if chat.is_orchestrator() && connected.is_empty() {
        (
            "lark_connection_required",
            "No Lark/Feishu service connection is available in this turn. Use nyx__discover_services to select the correct region and user-OAuth or tenant identity, then nyx__connect_service to give the owner its hosted connect_url. For user OAuth, include wiki:wiki:readonly in scopes; reading a Document also needs docx:document:readonly. The owner must enable and publish these permissions in the app's Developer Console and consent through the hosted link. A channel bot is not a document-service connection.",
        )
    } else {
        (
            "lark_specialist_grant_required",
            "This specialist has no granted Lark/Feishu connection available. The owner must grant the intended connection and wiki_get_node plus the supported read operation in this agent's Grants page. If it is not connected yet, ask NyxBot for a hosted connection link first. Do not change accounts or request public sharing.",
        )
    };
    Ok(Some(
        json!({"code":code,"message":message,"setup_url":setup_page(Some(chat),frontend)}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_guidance_is_only_for_lark_discovery() {
        for query in ["wiki_get_node", "Lark documents", "Feishu wiki", "飞书文档"] {
            assert!(relevant_query(query));
        }
        for query in ["Wikipedia", "Notion wiki", "github search"] {
            assert!(!relevant_query(query));
        }
    }

    #[test]
    fn failures_are_stable_metadata_and_region_specific_setup() {
        for slug in SLUGS {
            for (code, expected) in [
                (99991672, "lark_scope_missing"),
                (131006, "lark_document_acl_denied"),
                (131005, "lark_wiki_node_not_found"),
                (131007, "lark_wiki_lookup_failed"),
            ] {
                let (status, text) = response(
                    400,
                    &json!({"code":code,"msg":"private-title","error":{"secret":"private-url"}})
                        .to_string(),
                    slug,
                    None,
                    "https://nyxid.test",
                );
                assert!(status >= 400);
                let value: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(value["error"], expected);
                assert_eq!(value["provider_code"], code);
                assert!(!text.contains("private"));
                if code == 99991672 {
                    assert_eq!(value["scope"], "wiki:wiki:readonly");
                    assert!(value["developer_console_url"].as_str().unwrap().contains(
                        if slug.contains("feishu") {
                            "feishu.cn"
                        } else {
                            "larksuite.com"
                        }
                    ));
                }
            }
        }
    }

    #[test]
    fn supported_read_needs_object_identity_not_optional_space_metadata() {
        let (status, text) = response(
            200,
            &json!({"code":0,"data":{"node":{
                "obj_type":"docx","obj_token":"document-token"
            }}})
            .to_string(),
            "api-lark",
            None,
            "https://nyxid.test",
        );
        assert_eq!(status, 200);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["nyxid_read"]["operation"], "docx_raw_content");
    }

    #[test]
    fn unsupported_types_never_supply_a_docx_token_or_echo_unknown_text() {
        for kind in [
            "sheet",
            "doc",
            "mindnote",
            "file",
            "slides",
            "secret arbitrary text",
        ] {
            let (status,text)=response(200,&json!({"code":0,"data":{"node":{"obj_type":kind,"obj_token":"private-token","title":"private-title","space_id":"private-space"}}}).to_string(),"api-lark",None,"https://nyxid.test");
            assert_eq!(status, 422);
            assert!(text.contains("lark_wiki_unsupported_type"));
            assert!(!text.contains("private"));
            assert!(!text.contains("secret arbitrary text"));
        }
    }
}
