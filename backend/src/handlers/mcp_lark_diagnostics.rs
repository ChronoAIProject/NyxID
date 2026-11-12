//! One result boundary for direct and universal attachment calls. Early
//! refusals take the same metadata-only activity/audit path as provider results.
use super::*;
use crate::services::lark_attachment_tool::{self as attachment, diagnostics};

pub(super) async fn call(
    state: &AppState,
    auth: &McpAuthContext,
    request: &JsonRpcRequest,
    // Require the large child to be boxed before entering either await branch.
    work: futures::future::BoxFuture<'_, Response>,
) -> Response {
    let params = request.params.as_ref();
    let name = params.and_then(|p| p["name"].as_str());
    let via = name == Some("nyx__call_tool");
    let name = if via {
        params.and_then(|p| p["arguments"]["tool_name"].as_str())
    } else {
        name
    };
    let Some(name) = name.filter(|name| {
        name.rsplit_once("__").is_some_and(|(_, op)| {
            matches!(
                op,
                attachment::COMPOSITE_OPERATION
                    | attachment::UPLOAD_OPERATION
                    | attachment::FIELDS_OPERATION
            )
        })
    }) else {
        return work.await;
    };
    let handle = diagnostics::Handle::default();
    // A coincidentally named custom operation keeps ordinary audit semantics.
    // Only the resolved typed Lark fields call enables this optional boundary.
    if name
        .rsplit_once("__")
        .is_some_and(|(_, op)| op == attachment::FIELDS_OPERATION)
    {
        handle.lock().expect("attachment diagnostic").enabled = false;
    }
    // Drop also records an interrupted call. The dispatch phase determines
    // whether a create might have reached Lark; cancellation never implies success.
    let audit = ResultAudit {
        state,
        auth,
        name,
        via,
        handle: handle.clone(),
    };
    let mut response = diagnostics::scope(handle.clone(), work).await;
    decorate(
        &mut response,
        &mut handle.lock().expect("attachment diagnostic"),
    );
    drop(audit);
    response
}

struct ResultAudit<'a> {
    state: &'a AppState,
    auth: &'a McpAuthContext,
    name: &'a str,
    via: bool,
    handle: diagnostics::Handle,
}
impl Drop for ResultAudit<'_> {
    fn drop(&mut self) {
        let mut diagnostic = self.handle.lock().expect("attachment diagnostic");
        if !diagnostic.enabled {
            return;
        }
        if diagnostic.reason.is_none() && diagnostic.fields()["outcome"] != "succeeded" {
            diagnostic.reason = Some(diagnostics::Reason::Internal);
        }
        let mut fields = diagnostic.fields();
        fields["tool"] = self.name.into();
        fields["service_id"] = diagnostic.service_id.clone().into();
        fields["agent_role"] = serde_json::json!(self.auth.chat.as_ref().map(|chat| chat.role));
        if self.via {
            fields["via"] = "nyx__call_tool".into();
        }
        audit_service::log_async(
            self.state.db.clone(),
            Some(self.auth.user_id.clone()),
            "mcp_tool_call".into(),
            Some(fields),
            self.auth.ip_address.clone(),
            self.auth.user_agent.clone(),
            self.auth.api_key_id.clone(),
            self.auth.api_key_name.clone(),
        );
    }
}

pub(super) fn decorate_current(response: &mut Response) {
    if let Some(handle) = diagnostics::current() {
        decorate(response, &mut handle.lock().expect("attachment diagnostic"));
    }
}
fn decorate(response: &mut Response, diagnostic: &mut diagnostics::Diagnostic) {
    if !diagnostic.enabled {
        return;
    }
    if !tool_succeeded(response) {
        let reason = diagnostic
            .reason
            .get_or_insert(diagnostics::Reason::AdmissionRefused)
            .code();
        let mut failure =
            tool_failure(response).unwrap_or(crate::models::assistant_conversation::ToolFailure {
                kind: "tool_error".into(),
                reason: None,
                status: None,
            });
        failure.reason = Some(reason);
        failure.status = diagnostic.status.or(failure.status);
        response.extensions_mut().insert(failure);
    }
}
