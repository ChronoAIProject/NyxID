use crate::db::DbHandle;
use crate::errors::{AppError, AppResult};
use crate::models::usage_meter::CredentialClass;
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

/// Only a live server-verified organization channel guest selects OrgWallet
/// for platform credentials. Ordinary execution keeps the billing principal
/// and Personal classification, including for organization-owned API keys.
#[derive(Clone, Copy)]
pub enum ExecutionLane<'a> {
    Ordinary,
    OrgChannelGuest(&'a crate::services::org_channel_service::GuestPayer),
}

impl BillingOwnerResolver {
    pub fn new(db: DbHandle) -> Self {
        Self { db }
    }

    /// Caller-pays is exclusive to a server-resolved shared-agent lane.
    /// Owner, org member and personal guest executions retain their existing rules;
    /// org-channel guests use the separately verified billing exception below.
    pub async fn resolve_for_assistant_execution(
        &self,
        principal: &str,
        resource_owner: &str,
        credential_class: CredentialClass,
        access: Option<&crate::services::org_agent_service::RequestAccess>,
    ) -> AppResult<ResolvedBillingOwner> {
        if let Some(shared) = access.and_then(|a| a.shared()) {
            if shared.actor() != principal
                || (resource_owner != shared.owner() && resource_owner != principal)
            {
                return Err(AppError::Forbidden(
                    "Shared billing authority mismatch".into(),
                ));
            }
            return Ok(ResolvedBillingOwner {
                owner_id: principal.into(),
                pays: PaysFrom::Personal,
            });
        }
        let guest = access
            .and_then(|a| a.channel())
            .and_then(|a| a.guest_payer());
        self.resolve_for_execution(
            principal,
            resource_owner,
            credential_class,
            guest
                .as_ref()
                .map_or(ExecutionLane::Ordinary, ExecutionLane::OrgChannelGuest),
        )
        .await
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

    /// Resolve the payer after resource authorization and final credential selection.
    ///
    /// A platform-key grant authorizes use of NyxID's credential; it does not
    /// make the granting organization pay. Master-key usage keeps the billing
    /// principal and Personal classification without reading its owner type,
    /// including for organization-owned API keys. All other classes retain
    /// resource-owner billing and its organization ACL.
    pub async fn resolve_for_ordinary_execution(
        &self,
        principal: &str,
        resource_owner: &str,
        credential_class: CredentialClass,
    ) -> AppResult<ResolvedBillingOwner> {
        self.resolve_for_execution(
            principal,
            resource_owner,
            credential_class,
            ExecutionLane::Ordinary,
        )
        .await
    }

    pub async fn resolve_for_execution(
        &self,
        billing_principal_user_id: &str,
        resource_owner_id: &str,
        credential_class: CredentialClass,
        lane: ExecutionLane<'_>,
    ) -> AppResult<ResolvedBillingOwner> {
        if let ExecutionLane::OrgChannelGuest(guest) = lane {
            if billing_principal_user_id != guest.org_id() || resource_owner_id != guest.org_id() {
                return Err(AppError::Forbidden(
                    "Organization guest billing identity mismatch".into(),
                ));
            }
            crate::services::org_channel_service::budget::require_payer(&self.db, guest).await?;
            return Ok(ResolvedBillingOwner {
                owner_id: guest.org_id().into(),
                pays: PaysFrom::OrgWallet {
                    org_id: guest.org_id().into(),
                },
            });
        }
        if credential_class == CredentialClass::NyxidManagedMaster {
            return Ok(ResolvedBillingOwner {
                owner_id: billing_principal_user_id.to_string(),
                pays: PaysFrom::Personal,
            });
        }
        self.resolve_for_resource(billing_principal_user_id, resource_owner_id)
            .await
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

    #[tokio::test]
    async fn ordinary_platform_keys_preserve_org_key_principal_without_database_reads() {
        use super::{ExecutionLane, ResolvedBillingOwner};
        use crate::models::{usage_meter::CredentialClass, user::UserType};
        use crate::mw::auth::AuthMethod;
        use crate::test_utils::{
            connect_test_database_with_command_handler, test_auth_user, test_user,
        };
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{Arc, Mutex};

        let reads = Arc::new(Mutex::new(Vec::new()));
        let observed = reads.clone();
        let db = connect_test_database_with_command_handler(
            "billing_ordinary_platform_key_no_reads",
            EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event
                    && matches!(
                        event.command_name.as_str(),
                        "find" | "aggregate" | "count" | "distinct"
                    )
                {
                    observed.lock().unwrap().push(event.command);
                }
            }),
        )
        .await
        .unwrap();
        let resolver = BillingOwnerResolver::new(db.clone());
        let resource_owner = uuid::Uuid::new_v4().to_string();
        for owner_type in [UserType::Org, UserType::Person] {
            let owner = uuid::Uuid::new_v4().to_string();
            db.collection("users")
                .insert_one(test_user(&owner, owner_type))
                .await
                .unwrap();
            // An authenticated org-owned API key acts as its org, not the
            // admin who created it. Measure only payer resolution, not auth.
            let mut auth = test_auth_user(&owner);
            auth.auth_method = AuthMethod::ApiKey;
            auth.api_key_id = Some(uuid::Uuid::new_v4().to_string());
            let principal = auth.proxy_resolution_user_id();
            assert_eq!(principal, owner);
            let expected = ResolvedBillingOwner {
                owner_id: owner,
                pays: PaysFrom::Personal,
            };
            reads.lock().unwrap().clear();
            assert_eq!(
                resolver
                    .resolve_for_execution(
                        &principal,
                        &resource_owner,
                        CredentialClass::NyxidManagedMaster,
                        ExecutionLane::Ordinary
                    )
                    .await
                    .unwrap(),
                expected
            );
            assert_eq!(
                resolver
                    .resolve_for_ordinary_execution(
                        &principal,
                        &resource_owner,
                        CredentialClass::NyxidManagedMaster
                    )
                    .await
                    .unwrap(),
                expected
            );
            assert_eq!(
                resolver
                    .resolve_for_assistant_execution(
                        &principal,
                        &resource_owner,
                        CredentialClass::NyxidManagedMaster,
                        None
                    )
                    .await
                    .unwrap(),
                expected
            );
            assert!(
                reads.lock().unwrap().is_empty(),
                "ordinary platform-key payer resolution must not read users or any other collection"
            );
        }
        db.drop().await.unwrap();
    }

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
