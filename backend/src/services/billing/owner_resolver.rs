use crate::db::DbHandle;
use crate::errors::{AppError, AppResult};
use crate::services::org_service::{self, OwnerAccess};

#[derive(Clone)]
pub struct BillingOwnerResolver {
    db: DbHandle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaysFrom {
    Personal,
    OrgWallet { org_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedBillingOwner {
    pub owner_id: String,
    pub pays: PaysFrom,
}

impl BillingOwnerResolver {
    pub fn new(db: DbHandle) -> Self {
        Self { db }
    }

    /// Resolve the payer for an already-authorized resource operation.
    ///
    /// `resource_owner_id` must come from the resolved resource, never from
    /// request input. Members may consume org resources, so this path accepts
    /// both org-admin and org-member access.
    pub async fn resolve_for_resource(
        &self,
        billing_principal_user_id: &str,
        resource_owner_id: &str,
    ) -> AppResult<ResolvedBillingOwner> {
        let access = org_service::resolve_owner_access(
            &self.db,
            billing_principal_user_id,
            resource_owner_id,
        )
        .await?;
        Self::from_owner_access(billing_principal_user_id, resource_owner_id, &access)
    }

    /// Resolve an owner selected on a billing-benefit read surface.
    ///
    /// Unlike resource execution, grants and allowances have no resource row
    /// to establish the owner. The request may select an organization, so this
    /// method deliberately accepts that owner id while retaining the same
    /// direct/admin/member consumption ACL as [`Self::resolve_for_resource`].
    pub async fn resolve_for_benefit_read(
        &self,
        actor_user_id: &str,
        requested_owner_id: Option<&str>,
    ) -> AppResult<ResolvedBillingOwner> {
        self.resolve_for_resource(actor_user_id, requested_owner_id.unwrap_or(actor_user_id))
            .await
    }

    /// Resolve a wallet targeted by an explicit management operation.
    ///
    /// Personal owners and org admins may mutate wallets. Org members retain
    /// consumption access through [`Self::resolve_for_resource`] but cannot
    /// provision, top up, or otherwise manage the org wallet.
    pub async fn resolve_for_wallet_management(
        &self,
        actor_user_id: &str,
        requested_owner_id: Option<&str>,
    ) -> AppResult<ResolvedBillingOwner> {
        let owner_id = requested_owner_id.unwrap_or(actor_user_id);
        let access = org_service::resolve_owner_access(&self.db, actor_user_id, owner_id).await?;
        Self::from_owner_access_for_wallet_management(actor_user_id, owner_id, &access)
    }

    pub fn from_owner_access(
        actor_user_id: &str,
        owner_id: &str,
        access: &OwnerAccess,
    ) -> AppResult<ResolvedBillingOwner> {
        match access {
            OwnerAccess::Direct => Ok(ResolvedBillingOwner {
                owner_id: actor_user_id.to_string(),
                pays: PaysFrom::Personal,
            }),
            OwnerAccess::AsOrgAdmin { org_user_id, .. }
            | OwnerAccess::AsOrgMember { org_user_id, .. } => Ok(ResolvedBillingOwner {
                owner_id: org_user_id.clone(),
                pays: PaysFrom::OrgWallet {
                    org_id: org_user_id.clone(),
                },
            }),
            OwnerAccess::Forbidden => Err(AppError::Forbidden(format!(
                "User is not allowed to bill owner '{owner_id}'"
            ))),
        }
    }

    fn from_owner_access_for_wallet_management(
        actor_user_id: &str,
        owner_id: &str,
        access: &OwnerAccess,
    ) -> AppResult<ResolvedBillingOwner> {
        if !access.can_write() {
            return Err(AppError::Forbidden(format!(
                "User is not allowed to manage billing owner '{owner_id}'"
            )));
        }

        Self::from_owner_access(actor_user_id, owner_id, access)
    }
}

#[cfg(test)]
mod tests {
    use super::{BillingOwnerResolver, PaysFrom};
    use crate::models::org_membership::OrgRole;
    use crate::services::org_service::OwnerAccess;

    #[test]
    fn direct_access_bills_personal_owner() {
        let resolved =
            BillingOwnerResolver::from_owner_access("actor", "actor", &OwnerAccess::Direct)
                .expect("direct access");

        assert_eq!(resolved.owner_id, "actor");
        assert_eq!(resolved.pays, PaysFrom::Personal);
    }

    #[test]
    fn org_access_bills_org_wallet() {
        let resolved = BillingOwnerResolver::from_owner_access(
            "member",
            "org",
            &OwnerAccess::AsOrgMember {
                org_user_id: "org".to_string(),
                membership_id: "membership".to_string(),
                role: OrgRole::Member,
                allowed_service_ids: None,
            },
        )
        .expect("org access");

        assert_eq!(resolved.owner_id, "org");
        assert_eq!(
            resolved.pays,
            PaysFrom::OrgWallet {
                org_id: "org".to_string()
            }
        );
    }

    #[test]
    fn org_member_can_consume_but_cannot_manage_org_wallet() {
        let access = OwnerAccess::AsOrgMember {
            org_user_id: "org".to_string(),
            membership_id: "membership".to_string(),
            role: OrgRole::Member,
            allowed_service_ids: None,
        };

        let consumption = BillingOwnerResolver::from_owner_access("member", "org", &access)
            .expect("org member consumption should bill the org");
        let management =
            BillingOwnerResolver::from_owner_access_for_wallet_management("member", "org", &access)
                .expect_err("org member must not manage the org wallet");

        assert_eq!(consumption.owner_id, "org");
        assert!(matches!(management, crate::errors::AppError::Forbidden(_)));
    }

    #[test]
    fn org_admin_can_manage_org_wallet() {
        let access = OwnerAccess::AsOrgAdmin {
            org_user_id: "org".to_string(),
            membership_id: "membership".to_string(),
            allowed_service_ids: None,
        };

        let resolved =
            BillingOwnerResolver::from_owner_access_for_wallet_management("admin", "org", &access)
                .expect("org admin should manage the org wallet");

        assert_eq!(resolved.owner_id, "org");
        assert_eq!(
            resolved.pays,
            PaysFrom::OrgWallet {
                org_id: "org".to_string()
            }
        );
    }

    #[test]
    fn direct_owner_can_manage_personal_wallet() {
        let resolved = BillingOwnerResolver::from_owner_access_for_wallet_management(
            "actor",
            "actor",
            &OwnerAccess::Direct,
        )
        .expect("direct owner should manage personal wallet");

        assert_eq!(resolved.owner_id, "actor");
        assert_eq!(resolved.pays, PaysFrom::Personal);
    }

    #[test]
    fn forbidden_access_is_not_rewritten_to_personal_billing() {
        let err =
            BillingOwnerResolver::from_owner_access("actor", "other", &OwnerAccess::Forbidden)
                .expect_err("forbidden owner access must fail");

        assert!(matches!(err, crate::errors::AppError::Forbidden(_)));
    }
}
