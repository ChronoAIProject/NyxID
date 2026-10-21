use super::*;

/// Private admission carried from the confirmation gate to concrete egress.
/// It is deliberately not deserializable from HTTP headers or MCP arguments.
#[derive(Clone)]
pub struct Admission {
    pub(super) connection: String,
    pub(super) binding: String,
    pub(super) execution: Option<String>,
    pub(super) execution_credential: bool,
    pub(super) operation: String,
    pub(super) operation_identity: Value,
    pub(super) native_execution: Option<Value>,
    pub(super) read: bool,
    pub(super) api_key_id: String,
    pub(super) summary: String,
    pub(super) job_id: Option<String>,
    pub(super) async_continuation: Option<AsyncContinuation>,
    pub(super) chat: acks::ChatAuthority,
}
impl Admission {
    pub fn bind_ssh(
        &mut self,
        connection: &UserService,
        config: &crate::models::downstream_service::SshServiceConfig,
    ) {
        self.native_execution = Some(crate::services::assistant_native_execution::ssh_projection(
            connection, config,
        ));
    }
    pub fn revalidate<'a>(
        &'a self,
        db: &'a Database,
        connection: &'a str,
        target: Option<&'a crate::services::proxy_service::ProxyTarget>,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(self.revalidate_inner(db, connection, target))
    }

    async fn revalidate_inner(
        &self,
        db: &Database,
        connection: &str,
        target: Option<&crate::services::proxy_service::ProxyTarget>,
    ) -> AppResult<()> {
        if let Some(pool) = self.operation_identity["pool_id"].as_str()
            && let Some(revision) = self.operation_identity["pool_revision"].as_i64()
            && db
                .collection::<Document>("service_pools")
                .find_one(doc! {
                    "_id":pool,"is_active":true,"config_revision":revision,
                    "members":{"$elemMatch":{"user_service_id":connection,"enabled":true}},
                })
                .await?
                .is_none()
        {
            return Err(AppError::Conflict("Confirmed service pool changed".into()));
        }
        if let Some(job) = &self.job_id {
            let service =
                crate::services::user_service_service::find_user_service_by_id(db, connection)
                    .await?
                    .ok_or(AppError::MachineAuthorityStale)?;
            crate::services::assistant_job_service_use::validate(
                db,
                job,
                connection,
                &service.user_id,
            )
            .await?;
        }
        if let Some(projection) = &self.native_execution {
            let row =
                crate::services::user_service_service::find_user_service_by_id(db, connection)
                    .await?
                    .ok_or_else(|| AppError::NotFound("SSH connection unavailable".into()))?;
            let catalog = row
                .catalog_service_id
                .as_deref()
                .ok_or_else(|| AppError::Conflict("SSH catalog changed".into()))?;
            let config = crate::services::ssh_service::get_ssh_service(db, catalog).await?;
            if *projection
                != crate::services::assistant_native_execution::ssh_projection(&row, &config)
            {
                return Err(AppError::Conflict("Resolved SSH execution changed".into()));
            }
        }
        if let Some(watch) = &self.async_continuation {
            watch.revalidate(db, &self.chat).await?;
        } else if self.job_id.is_none() {
            personal_authority(
                db,
                &self.chat,
                "assistant.service_egress",
                Some(connection),
                !matches!(
                    self.operation_identity["kind"].as_str(),
                    Some("raw_inference" | "legacy_inference")
                ),
            )
            .await?;
        }
        let service =
            crate::services::user_service_service::find_user_service_by_id(db, connection)
                .await?
                .ok_or_else(|| AppError::NotFound("Service unavailable".into()))?;
        let access = crate::services::org_service::resolve_owner_access(
            db,
            &self.chat.user_id,
            &service.user_id,
        )
        .await?;
        let eligible = match &access {
            crate::services::org_service::OwnerAccess::Direct
            | crate::services::org_service::OwnerAccess::AsOrgAdmin { .. } => true,
            crate::services::org_service::OwnerAccess::AsOrgMember { role, .. } => {
                role.can_proxy() && !service.admin_only
            }
            crate::services::org_service::OwnerAccess::Forbidden => false,
        };
        if !eligible || !access.allows_resource(connection) {
            return Err(AppError::Forbidden("Service access revoked".into()));
        }
        if self.connection != connection
            || self.binding != connection_binding(db, connection, &self.api_key_id).await?
        {
            return Err(AppError::Conflict(
                "Service selection changed; request a new exact confirmation".into(),
            ));
        }
        if self.execution.as_ref()
            != target
                .map(|target| execution_projection_digest(target, self.execution_credential))
                .as_ref()
        {
            return Err(AppError::Conflict(
                "Resolved service execution changed".into(),
            ));
        }
        Ok(())
    }
}
