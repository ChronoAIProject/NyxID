//! Native evidence is minted only after the existing permission engine verifies
//! a plan. Verification requests never receive the primary request's context.
use super::*;
use crate::models::permission_policy::PermissionPolicy;

pub(super) struct Native {
    binding: PermissionPolicy,
    request: nyxid_permissions::Request,
    policy_digest: String,
    dispatch: nyxid_permissions::DispatchMark,
}

impl Native {
    pub(super) fn policy(&self) -> &nyxid_permissions::Policy {
        &self.binding.policy
    }
    pub(super) fn source(&self) -> crate::models::execution_proposal::ProposalSource {
        crate::models::execution_proposal::ProposalSource::Native {
            policy_id: self.binding.id.clone(),
            revision: self.binding.revision,
        }
    }

    pub(super) fn require_effect(&self, effect: &Effect<'_>) -> AppResult<()> {
        let query =
            nyxid_permissions::parse_query(effect.query.unwrap_or("")).map_err(|_| refused())?;
        if effect.connection != self.binding.user_service_id
            || effect.method != self.request.method.as_str()
            || effect.path.trim_start_matches('/') != self.request.path.trim_start_matches('/')
            || query != self.request.query
            || effect.body != self.request.body
            || effect.content_type != self.request.content_type.as_deref()
        {
            return Err(refused());
        }
        Ok(())
    }

    pub(super) fn evidence<'a>(
        &'a self,
        digest: &'a str,
        binding: &'a ConnectionBinding,
    ) -> contract::ResourceEvidence<'a> {
        contract::ResourceEvidence {
            policy_digest: &self.policy_digest,
            prepared_digest: digest,
            authority_digest: &binding.authority_digest,
            allowed: true,
        }
    }

    pub(super) async fn check(
        &self,
        db: &Database,
        session: Option<&mut ClientSession>,
    ) -> AppResult<()> {
        if let Some(session) = session {
            if db.collection::<Document>(crate::models::permission_policy::COLLECTION_NAME).update_one(
                doc! {"_id":&self.binding.id,"user_id":&self.binding.user_id,"revision":self.binding.revision,"paused":false},
                doc! {"$inc":{"execution_dispatch_fence":1_i64}},
            ).session(session).await?.matched_count != 1 { return Err(refused()); }
        } else {
            super::super::permission_policy_service::check_live(db, &self.binding).await?;
        }
        Ok(())
    }
}

impl RequestContext {
    pub(crate) fn for_native(
        &self,
        binding: &PermissionPolicy,
        verified: &nyxid_permissions::VerifiedPlan<'_>,
        dispatch: nyxid_permissions::DispatchMark,
    ) -> AppResult<Arc<Self>> {
        let Principal::ApiKey(key) = &self.principal else {
            return Err(refused());
        };
        if binding.user_id != self.actor || binding.id != *key {
            return Err(refused());
        }
        let mut fresh = Arc::try_unwrap(
            Self::enrolled(Some(VERSION), &self.actor, key, None).ok_or_else(refused)?,
        )
        .map_err(|_| refused())?;
        fresh.operation_id.clone_from(&self.operation_id);
        fresh.resume = self.resume().cloned();
        fresh.async_requested.store(
            self.async_requested
                .load(std::sync::atomic::Ordering::Relaxed),
            std::sync::atomic::Ordering::Relaxed,
        );
        if let Some(token) = self.cancellation.get() {
            fresh.bind_cancellation(token.clone());
        }
        if let Some(value) = self.generated_correlation.get() {
            fresh.bind_generated_correlation(value);
        }
        if let Some(v) = self.ingress.get() {
            let _ = fresh.ingress.set(v.clone());
        }
        if let Some(v) = self.idempotency.get() {
            let _ = fresh.idempotency.set(v.clone());
        }
        if let Some(v) = self.source_credential.get() {
            let _ = fresh.source_credential.set(v.clone());
        }

        fresh.native = Some(Arc::new(Native {
            policy_digest: super::super::mcp_service::canonical_sha256(
                serde_json::to_value(&binding.policy).map_err(encoding)?,
            ),
            request: verified.plan().request.clone(),
            binding: binding.clone(),
            dispatch,
        }));
        if let Some(key) = self.authenticated_key.get() {
            let _ = fresh.authenticated_key.set(key.clone());
        }
        Ok(Arc::new(fresh))
    }

    pub(crate) fn native_dispatched(&self) {
        if let Some(native) = &self.native {
            native.dispatch.dispatched();
        }
    }
}
