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
    async fn record(self, db: &Database, chat: &acks::ChatAuthority) {
        match self {
            Self::Approval(receipt) => {
                acks::record_auto_allowed(
                    db,
                    chat,
                    "service_approval",
                    &receipt.arguments,
                    &receipt.auto,
                    &receipt.summary,
                )
                .await
            }
            Self::Service(admission, auto) => admission.receipt(db, chat, Some(auto)).await,
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
        if let Some(auto) = auto {
            acks::record_auto_allowed(db, chat, "nyxid.service_execution", &json!({
                "connection_id": self.connection, "execution_binding": self.binding, "operation_digest": self.operation,
            }), &auto, &self.summary).await;
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
                receipt.record(&db, &chat).await;
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
                receipt.record(&db, &chat).await;
            };
            axum::response::Response::from_parts(parts, axum::body::Body::from_stream(stream))
        })
    }
}
