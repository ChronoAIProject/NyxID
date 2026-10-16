use super::*;

/// Private, borrowed authority carried all the way to the actual send/connect.
/// Credential preparation can await external services; it does not admit egress.
pub(crate) struct HttpEgressGuard<'a> {
    admission: &'a Admission,
    db: &'a Database,
    keys: &'a crate::crypto::aes::EncryptionKeys,
    target: &'a crate::services::proxy_service::ProxyTarget,
    method: String,
    path: String,
    projection: Option<&'a HttpProjection>,
    route: Option<Option<&'a crate::services::node_routing_service::NodeRoute>>,
}
impl Admission {
    pub(crate) fn http_egress<'a>(
        &'a self,
        db: &'a Database,
        keys: &'a crate::crypto::aes::EncryptionKeys,
        target: &'a crate::services::proxy_service::ProxyTarget,
        method: &str,
        path: &str,
    ) -> HttpEgressGuard<'a> {
        HttpEgressGuard {
            admission: self,
            db,
            keys,
            target,
            method: method.to_owned(),
            path: path.to_owned(),
            projection: None,
            route: None,
        }
    }
}
impl<'a> HttpEgressGuard<'a> {
    pub(crate) fn projected(mut self, projection: &'a HttpProjection) -> Self {
        self.projection = Some(projection);
        self
    }
    pub(crate) fn routed(
        mut self,
        route: Option<&'a crate::services::node_routing_service::NodeRoute>,
    ) -> Self {
        self.route = Some(route);
        self
    }
    pub(crate) fn revalidate(&self) -> futures::future::BoxFuture<'_, AppResult<()>> {
        Box::pin(self.revalidate_inner())
    }

    async fn revalidate_inner(&self) -> AppResult<()> {
        let projection = HttpProjection::default();
        verify_resolved_projected(
            self.db,
            self.keys,
            &self.admission.chat,
            &self.admission.connection,
            self.target,
            &self.method,
            &self.path,
            self.projection.unwrap_or(&projection),
        )
        .await?;
        if let Some(route) = self.route {
            verify_node_route(
                self.db,
                &self.admission.chat,
                &self.admission.connection,
                route,
                self.target,
            )
            .await?;
        }
        self.admission
            .revalidate(self.db, &self.admission.connection, Some(self.target))
            .await
    }
}
