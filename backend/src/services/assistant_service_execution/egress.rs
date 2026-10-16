use super::*;

/// Private, borrowed authority carried all the way to the actual send/connect.
/// Credential preparation can await external services; it does not admit egress.
pub(crate) struct HttpEgressGuard<'a> {
    authority: EgressAuthority<'a>,
}

enum EgressAuthority<'a> {
    Assistant(AssistantEgressGuard<'a>),
    ExternalJob {
        db: &'a Database,
        job: &'a str,
        declared: &'a str,
    },
    AssistantCatalogJob {
        db: &'a Database,
        job: &'a str,
        catalog: &'a str,
    },
}

struct AssistantEgressGuard<'a> {
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
            authority: EgressAuthority::Assistant(AssistantEgressGuard {
                admission: self,
                db,
                keys,
                target,
                method: method.to_owned(),
                path: path.to_owned(),
                projection: None,
                route: None,
            }),
        }
    }
}
impl<'a> HttpEgressGuard<'a> {
    pub(crate) fn for_proxy(
        admission: Option<&'a Admission>,
        ingress: Option<&'a crate::services::machine_gateway_service::Ingress>,
        db: &'a Database,
        keys: &'a crate::crypto::aes::EncryptionKeys,
        target: &'a crate::services::proxy_service::ProxyTarget,
        method: &str,
        path: &str,
    ) -> Option<Self> {
        if let Some(admission) = admission {
            return Some(admission.http_egress(db, keys, target, method, path));
        }
        let ingress = ingress?;
        if let Some(job) = ingress.assistant_job_id() {
            return Some(Self {
                authority: EgressAuthority::AssistantCatalogJob {
                    db,
                    job,
                    catalog: &target.service.id,
                },
            });
        }
        Some(Self {
            authority: EgressAuthority::ExternalJob {
                db,
                job: ingress.external_job_id()?,
                declared: &ingress.declared_id,
            },
        })
    }

    pub(crate) fn projected(mut self, projection: &'a HttpProjection) -> Self {
        if let EgressAuthority::Assistant(guard) = &mut self.authority {
            guard.projection = Some(projection);
        }
        self
    }
    pub(crate) fn routed(
        mut self,
        route: Option<&'a crate::services::node_routing_service::NodeRoute>,
    ) -> Self {
        if let EgressAuthority::Assistant(guard) = &mut self.authority {
            guard.route = Some(route);
        }
        self
    }
    pub(crate) fn revalidate(&self) -> futures::future::BoxFuture<'_, AppResult<()>> {
        match &self.authority {
            EgressAuthority::Assistant(guard) => Box::pin(guard.revalidate_inner()),
            EgressAuthority::ExternalJob { db, job, declared } => {
                crate::services::machine_gateway_service::validate_external_job(db, job, declared)
            }
            EgressAuthority::AssistantCatalogJob { db, job, catalog } => {
                crate::services::assistant_job_service_use::validate_catalog(db, job, catalog)
            }
        }
    }
}

impl AssistantEgressGuard<'_> {
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
