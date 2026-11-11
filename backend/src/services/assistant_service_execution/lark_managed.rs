//! One owner decision admits a deterministic Lark expansion. This guard uses
//! connection credentials (including overrides), never catalog master authority.
use super::*;
use crate::models::lark_base_write::{LarkBaseWrite, WritePrincipal, WriteState};
use crate::services::{lark_attachment_tool as attachment, lark_base_writes as writes};

pub(crate) struct LarkManagedGuard<'a> {
    db: &'a Database,
    keys: &'a crate::crypto::aes::EncryptionKeys,
    admission: &'a Admission,
    target: &'a crate::services::proxy_service::ProxyTarget,
    service: &'a mcp_service::McpToolService,
    endpoint: &'a mcp_service::McpToolEndpoint,
    prepared: &'a writes::Prepared,
    path: &'a str,
    body: &'a [u8],
}

impl<'a> LarkManagedGuard<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        db: &'a Database,
        keys: &'a crate::crypto::aes::EncryptionKeys,
        admission: &'a Admission,
        target: &'a crate::services::proxy_service::ProxyTarget,
        service: &'a mcp_service::McpToolService,
        endpoint: &'a mcp_service::McpToolEndpoint,
        prepared: &'a writes::Prepared,
        path: &'a str,
        body: &'a [u8],
    ) -> AppResult<Self> {
        if admission.chat.api_key_id != prepared.chat.api_key_id
            || admission.chat.turn_id != prepared.chat.turn_id
            || admission.chat.conversation_id != prepared.chat.conversation_id
            || prepared.chat.guest
        {
            return Err(attachment::refused());
        }
        Ok(Self {
            db,
            keys,
            admission,
            target,
            service,
            endpoint,
            prepared,
            path,
            body,
        })
    }
    pub(crate) async fn principal(&self) -> AppResult<WritePrincipal> {
        let row = crate::services::user_service_service::find_user_service_by_id(
            self.db,
            &self.admission.connection,
        )
        .await?
        .filter(|row| row.is_active)
        .ok_or_else(attachment::refused)?;
        let key = self
            .db
            .collection::<ApiKey>("api_keys")
            .find_one(doc! {"_id":&self.prepared.chat.api_key_id,"is_active":true})
            .await?
            .ok_or_else(attachment::refused)?;
        let override_identity = if key
            .assistant_agent_owner_id
            .as_deref()
            .is_none_or(|owner| owner == self.prepared.chat.user_id)
        {
            crate::services::proxy_service::read_agent_credential_override_identity(
                self.db,
                &self.prepared.chat.user_id,
                &self.prepared.chat.api_key_id,
                &row.id,
                self.target,
            )
            .await?
        } else {
            None
        };
        if let Some(identity) = override_identity {
            return Ok(WritePrincipal {
                user_service_id: row.id,
                credential_override: Some(identity.api_key_id.clone()),
                credential_id: identity.api_key_id,
                epoch: identity.credential_epoch,
            });
        }
        let id = row.api_key_id.as_ref().ok_or_else(attachment::refused)?;
        let credential = self
            .db
            .collection::<UserApiKey>("user_api_keys")
            .find_one(doc! {
                "_id":id,"user_id":&row.user_id,"status":"active"
            })
            .await?
            .ok_or_else(attachment::refused)?;
        Ok(WritePrincipal {
            user_service_id: row.id,
            credential_id: credential.id,
            epoch: credential.credential_epoch,
            credential_override: None,
        })
    }
    pub(crate) fn revalidate(&self) -> futures::future::BoxFuture<'_, AppResult<()>> {
        Box::pin(async move {
            crate::services::assistant_resource_service::require_live(self.db, &self.prepared.chat)
                .await?;
            self.admission
                .http_egress(self.db, self.keys, self.target, "POST", self.path)
                .routed(None)
                .revalidate()
                .await?;
            let key = self
                .db
                .collection::<ApiKey>("api_keys")
                .find_one(doc! {
                    "_id":&self.prepared.chat.api_key_id,"is_active":true
                })
                .await?
                .ok_or_else(attachment::refused)?;
            let path =
                crate::services::proxy_authorization::CanonicalPath::from_mcp_built(self.path)?;
            let catalog =
                crate::services::agent_operation_scope_service::mcp_catalog_id(self.service);
            crate::services::agent_operation_scope_service::authorize(
                &key.assistant_operation_scopes,
                &self.service.service_id,
                catalog,
                Some(&self.endpoint.endpoint_id),
                "POST",
                &path,
                false,
                false,
            )?;
            crate::services::agent_operation_scope_service::check_inputs(
                &key.assistant_operation_scopes,
                &self.service.service_id,
                catalog,
                Some(&self.endpoint.endpoint_id),
                "POST",
                &path,
                None,
                self.body,
                Some("application/json"),
            )?;
            if writes::contracts(self.db, self.service, self.endpoint).await?
                != self.prepared.contract_digests
            {
                return Err(attachment::refused());
            }
            // All preceding work can await remote KMS/DB providers. Close the
            // authority check again before the attempt gate at the actual send.
            crate::services::assistant_resource_service::require_live(self.db, &self.prepared.chat)
                .await?;
            Ok(())
        })
    }
    pub(crate) fn effect<'b>(
        &'b self,
        row: &'b LarkBaseWrite,
        phase: WriteState,
    ) -> HttpEgressGuard<'b> {
        HttpEgressGuard::for_lark(self, row, phase)
    }
    pub(crate) async fn revalidate_effect(
        &self,
        row: &LarkBaseWrite,
        phase: WriteState,
    ) -> AppResult<()> {
        self.revalidate().await?;
        if self.principal().await? != row.principal {
            return Err(attachment::refused());
        }
        writes::effect_gate(self.db, row, phase).await
    }
}
