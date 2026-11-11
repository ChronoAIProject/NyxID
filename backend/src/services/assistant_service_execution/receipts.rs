use super::*;

/// The dispatcher chooses the existing service approval reason over a nested
/// service-action receipt. This metadata never comes from caller input.
pub(crate) struct HttpApprovalReceipt {
    auto: acks::AutoAllowed,
    arguments: Value,
    summary: String,
}
impl HttpApprovalReceipt {
    pub(crate) fn new(
        auto: Option<acks::AutoAllowed>,
        service: &str,
        name: &str,
        descriptor: &super::super::operation_descriptor::OperationDescriptor,
    ) -> Option<Self> {
        auto.map(|auto| {
            let operation = descriptor.operation_summary();
            Self {
                auto,
                arguments: json!({"service_id":service,"operation":operation}),
                summary: format!("{operation} on {name}"),
            }
        })
    }
}

enum HttpReceipt {
    Approval(HttpApprovalReceipt),
    Service(Box<Admission>, acks::AutoAllowed),
}
impl HttpReceipt {
    /// `activity` is captured while the tool call is still in scope; a
    /// streamed body reaches EOF after that scope has ended.
    async fn record(self, db: &Database, chat: &acks::ChatAuthority, activity: Option<String>) {
        match self {
            Self::Approval(receipt) => {
                acks::record_auto_allowed_for_activity(
                    db,
                    chat,
                    "service_approval",
                    &receipt.arguments,
                    &receipt.auto,
                    &receipt.summary,
                    None,
                    activity,
                )
                .await
            }
            Self::Service(admission, auto) => {
                admission
                    .receipt_for_activity(db, chat, Some(auto), activity)
                    .await
            }
        }
    }
}

impl Admission {
    pub async fn receipt(
        &self,
        db: &Database,
        chat: &acks::ChatAuthority,
        auto: Option<acks::AutoAllowed>,
    ) {
        let activity = crate::services::machine_activity_service::activity_id();
        self.receipt_for_activity(db, chat, auto, activity).await;
    }

    async fn receipt_for_activity(
        &self,
        db: &Database,
        chat: &acks::ChatAuthority,
        auto: Option<acks::AutoAllowed>,
        activity: Option<String>,
    ) {
        if let Some(auto) = auto {
            acks::record_auto_allowed_for_activity(db, chat, "nyxid.service_execution", &json!({
                "connection_id": self.connection, "execution_binding": self.binding, "operation_digest": self.operation,
            }), &auto, &self.summary, None, activity).await;
        }
    }
}

impl Decision {
    /// Emit exactly one chosen receipt on completed HTTP success. HEAD and
    /// bodyless responses complete here because the server may never poll their
    /// bodies; other streams must reach a clean EOF without being abandoned.
    pub(crate) fn finish_http(
        self,
        response: axum::response::Response,
        db: Database,
        chat: Option<acks::ChatAuthority>,
        method: &str,
        approval: Option<HttpApprovalReceipt>,
    ) -> futures::future::BoxFuture<'static, axum::response::Response> {
        let head = method == "HEAD";
        Box::pin(async move {
            // Read on first poll, which runs inside the tool call's activity
            // scope; the body's EOF (and the receipt) come after it has ended.
            let activity = crate::services::machine_activity_service::activity_id();
            let Some(chat) = chat else {
                return response;
            };
            if !response.status().is_success() {
                return response;
            }
            let receipt = if let Some(approval) = approval {
                HttpReceipt::Approval(approval)
            } else if let (Some(admission), Some(auto)) = (self.admission, self.automatic) {
                HttpReceipt::Service(Box::new(admission), auto)
            } else {
                return response;
            };
            use axum::body::HttpBody;
            if head
                || matches!(response.status().as_u16(), 204 | 205)
                || response.body().is_end_stream()
            {
                receipt.record(&db, &chat, activity).await;
                return response;
            }
            use futures::StreamExt;
            let (mut parts, body) = response.into_parts();
            // A fixed length lets Hyper stop polling after the last data frame.
            // Drive this wrapper through the upstream's clean EOF before the
            // response completes, including for a known-length provider body.
            parts.headers.remove(axum::http::header::CONTENT_LENGTH);
            let stream = async_stream::stream! {
                let mut source = body.into_data_stream();
                while let Some(chunk) = source.next().await {
                    match chunk {
                        Ok(bytes) => yield Ok::<_, axum::Error>(bytes),
                        Err(error) => { yield Err(error); return; }
                    }
                }
                receipt.record(&db, &chat, activity).await;
            };
            axum::response::Response::from_parts(parts, axum::body::Body::from_stream(stream))
        })
    }
}
