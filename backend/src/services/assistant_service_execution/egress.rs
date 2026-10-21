use super::*;

/// Private, borrowed authority carried all the way to the actual send/connect.
/// Credential preparation can await external services; it does not admit egress.
pub(crate) struct HttpEgressGuard<'a> {
    attachment: Option<AttachmentEgress<'a>>,
    authority: EgressAuthority<'a>,
}

struct AttachmentEgress<'a> {
    db: &'a Database,
    service: &'a crate::services::mcp_service::McpToolService,
    endpoint: &'a crate::services::mcp_service::McpToolEndpoint,
    binding: &'a crate::services::lark_attachment_tool::Binding,
}

enum EgressAuthority<'a> {
    Lark {
        guard: &'a super::LarkManagedGuard<'a>,
        row: &'a crate::models::lark_base_write::LarkBaseWrite,
        phase: crate::models::lark_base_write::WriteState,
    },
    AttachmentOnly,
    Managed(
        &'a OwnedHttpEgressGuard,
        &'a crate::services::proxy_service::ProxyTarget,
    ),
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

/// A detached managed exchange retains the exact server-issued authority while
/// owning its target and sharing the existing encryption provider.
pub(crate) struct OwnedHttpEgressGuard {
    db: Database,
    keys: std::sync::Arc<crate::crypto::aes::EncryptionKeys>,
    authority: Option<OwnedEgressAuthority>,
    catalog: Option<String>,
    resource_owner: String,
    method: String,
    path: String,
}

enum OwnedEgressAuthority {
    Assistant(Box<Admission>),
    AssistantCatalog(Box<acks::ChatAuthority>),
    ExternalJob { job: String, declared: String },
    AssistantCatalogJob { job: String, catalog: String },
}

impl OwnedHttpEgressGuard {
    pub(crate) fn for_managed(
        admission: Option<&Admission>,
        ingress: Option<&crate::services::machine_gateway_service::Ingress>,
        db: &Database,
        keys: std::sync::Arc<crate::crypto::aes::EncryptionKeys>,
        target: &crate::services::proxy_service::ProxyTarget,
        operation: (&str, &str),
        context: (
            &crate::services::billing::BillingRouteContext,
            &str,
            Option<&acks::ChatAuthority>,
        ),
    ) -> Self {
        let authority = if let Some(admission) = admission {
            Some(OwnedEgressAuthority::Assistant(Box::new(admission.clone())))
        } else if let Some(ingress) = ingress {
            if let Some(job) = ingress.assistant_job_id() {
                Some(OwnedEgressAuthority::AssistantCatalogJob {
                    job: job.to_owned(),
                    catalog: target.service.id.clone(),
                })
            } else {
                ingress
                    .external_job_id()
                    .map(|job| OwnedEgressAuthority::ExternalJob {
                        job: job.to_owned(),
                        declared: ingress.declared_id.clone(),
                    })
            }
        } else {
            context
                .2
                .map(|chat| OwnedEgressAuthority::AssistantCatalog(Box::new(chat.clone())))
        };
        Self {
            db: db.clone(),
            keys,
            authority,
            catalog: context.0.catalog_service_id.clone(),
            resource_owner: context.1.to_owned(),
            method: operation.0.to_owned(),
            path: operation.1.to_owned(),
        }
    }

    pub(crate) fn borrowed<'a>(
        &'a self,
        target: &'a crate::services::proxy_service::ProxyTarget,
    ) -> HttpEgressGuard<'a> {
        HttpEgressGuard {
            attachment: None,
            authority: EgressAuthority::Managed(self, target),
        }
    }

    fn revalidate<'a>(
        &'a self,
        target: &'a crate::services::proxy_service::ProxyTarget,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let catalog = self
                .catalog
                .as_deref()
                .ok_or_else(|| AppError::Conflict("Managed catalog unavailable".into()))?;
            let current = self
                .db
                .collection::<crate::models::downstream_service::DownstreamService>(
                    "downstream_services",
                )
                .find_one(doc! {"_id":catalog,"is_active":true})
                .await?
                .ok_or_else(|| AppError::NotFound("Managed service unavailable".into()))?;
            crate::services::telnyx_platform_service::validate_catalog(&current)?;
            let actor =
                crate::services::proxy_service::EffectiveActor::from_user_id(&self.resource_owner);
            let authorized = crate::services::proxy_service::authorize_master_credential(
                &self.db, &current, &actor,
            )
            .await?;
            let (method, key) =
                crate::services::platform_key_service::effective_auth(&self.db, &current).await?;
            if current.billing != target.service.billing
                || current.base_url.trim_end_matches('/') != target.base_url.trim_end_matches('/')
                || method != target.auth_method
                || key != target.auth_key_name
            {
                return Err(AppError::Conflict(
                    "Managed service execution changed".into(),
                ));
            }
            let credential = zeroize::Zeroizing::new(authorized.decrypt(&self.keys).await?);
            if credential.as_slice() != target.credential.as_bytes() {
                return Err(AppError::Conflict(
                    "Managed service credential changed".into(),
                ));
            }
            // KMS decryption can await a remote provider. Re-read the catalog
            // after it, then apply the live ACL before the final actor/job fence.
            let latest = self
                .db
                .collection::<crate::models::downstream_service::DownstreamService>(
                    "downstream_services",
                )
                .find_one(doc! {"_id":catalog,"is_active":true})
                .await?
                .ok_or_else(|| AppError::NotFound("Managed service unavailable".into()))?;
            crate::services::telnyx_platform_service::validate_catalog(&latest)?;
            if latest.billing != target.service.billing
                || latest.base_url.trim_end_matches('/') != target.base_url.trim_end_matches('/')
                || latest.credential_encrypted != current.credential_encrypted
                || latest.secret_context_versions != current.secret_context_versions
                || latest.auth_method != current.auth_method
                || latest.auth_key_name != current.auth_key_name
                || latest.auth_scheme != current.auth_scheme
                || latest.provider_config_id != current.provider_config_id
                || latest.proxy_operation_policy != target.service.proxy_operation_policy
            {
                return Err(AppError::Conflict(
                    "Managed service execution changed".into(),
                ));
            }
            crate::services::proxy_service::authorize_master_credential(&self.db, &latest, &actor)
                .await?;
            let base = match &self.authority {
                Some(OwnedEgressAuthority::Assistant(admission)) => Some(
                    admission
                        .http_egress(&self.db, &self.keys, target, &self.method, &self.path)
                        .routed(None),
                ),
                Some(OwnedEgressAuthority::AssistantCatalog(chat)) => {
                    personal_authority(
                        &self.db,
                        chat,
                        "assistant.managed_catalog_egress",
                        Some(catalog),
                        true,
                    )
                    .await?;
                    None
                }
                Some(OwnedEgressAuthority::ExternalJob { job, declared }) => {
                    Some(HttpEgressGuard {
                        attachment: None,
                        authority: EgressAuthority::ExternalJob {
                            db: &self.db,
                            job,
                            declared,
                        },
                    })
                }
                Some(OwnedEgressAuthority::AssistantCatalogJob { job, catalog }) => {
                    Some(HttpEgressGuard {
                        attachment: None,
                        authority: EgressAuthority::AssistantCatalogJob {
                            db: &self.db,
                            job,
                            catalog,
                        },
                    })
                }
                None => None,
            };
            if let Some(base) = base {
                base.revalidate().await?;
            }
            Ok(())
        })
    }
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
            attachment: None,
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
    pub(super) fn for_lark(
        guard: &'a super::LarkManagedGuard<'a>,
        row: &'a crate::models::lark_base_write::LarkBaseWrite,
        phase: crate::models::lark_base_write::WriteState,
    ) -> Self {
        Self {
            attachment: None,
            authority: EgressAuthority::Lark { guard, row, phase },
        }
    }
    pub(crate) fn with_attachment(
        base: Option<Self>,
        db: &'a Database,
        service: &'a crate::services::mcp_service::McpToolService,
        endpoint: &'a crate::services::mcp_service::McpToolEndpoint,
        binding: Option<&'a crate::services::lark_attachment_tool::Binding>,
    ) -> Option<Self> {
        let Some(binding) = binding else { return base };
        let mut guard = base.unwrap_or(Self {
            authority: EgressAuthority::AttachmentOnly,
            attachment: None,
        });
        guard.attachment = Some(AttachmentEgress {
            db,
            service,
            endpoint,
            binding,
        });
        Some(guard)
    }

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
                attachment: None,
                authority: EgressAuthority::AssistantCatalogJob {
                    db,
                    job,
                    catalog: &target.service.id,
                },
            });
        }
        Some(Self {
            attachment: None,
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
        Box::pin(async move {
            match &self.authority {
                EgressAuthority::Lark { guard, row, phase } => {
                    guard.revalidate_effect(row, *phase).await
                }
                EgressAuthority::AttachmentOnly => Ok(()),
                EgressAuthority::Managed(guard, target) => guard.revalidate(target).await,
                EgressAuthority::Assistant(guard) => guard.revalidate_inner().await,
                EgressAuthority::ExternalJob { db, job, declared } => {
                    crate::services::machine_gateway_service::validate_external_job(
                        db, job, declared,
                    )
                    .await
                }
                EgressAuthority::AssistantCatalogJob { db, job, catalog } => {
                    crate::services::assistant_job_service_use::validate_catalog(db, job, catalog)
                        .await
                }
            }?;
            if let Some(attachment) = &self.attachment {
                crate::services::lark_attachment_tool::validate_binding(
                    attachment.db,
                    attachment.service,
                    attachment.endpoint,
                    attachment.binding,
                )
                .await?;
            }
            Ok(())
        })
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
