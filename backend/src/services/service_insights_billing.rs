use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::errors::{AppError, AppResult};
use crate::models::agent_service_binding::{AgentServiceBinding, COLLECTION_NAME as BINDINGS};
use crate::models::api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as AGENT_KEYS};
use crate::models::downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService};
use crate::models::service_billing::{BillingMetric, PricingSyncStatus, ServiceBilling};
use crate::models::usage_meter::CredentialClass;
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::models::user_api_key::{COLLECTION_NAME as CREDENTIALS, UserApiKey};
use crate::models::user_endpoint::{COLLECTION_NAME as ENDPOINTS, UserEndpoint};
use crate::models::user_service::UserService;
use crate::services::billing::{BillingIngress, BillingRouteContext, BillingService, NodeIntent};
use crate::services::{feature_flag_service, org_service, platform_key_service, proxy_service};

#[derive(Clone, Copy, Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingExplanationStatus {
    Resolved,
    Conditional,
    Restricted,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionChargeStatus {
    UsageBased,
    NotCharged,
    Conditional,
    Restricted,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingAccountKind {
    Personal,
    Organization,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ServiceBillingAccount {
    pub id: String,
    pub kind: BillingAccountKind,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ServiceBillingRate {
    /// "platform" or "resale"; both are charges collected by NyxID.
    pub layer: String,
    pub metric: BillingMetric,
    /// Exact decimal credits per one unit; absent when the plan rate is unknown.
    pub credits_per_unit: Option<String>,
    pub currency: String,
    /// "credential_lane", "service_price", or "legacy_plan".
    pub source: String,
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderBillingDisclosure {
    SeparateProviderAccount,
    NyxidCredential,
    NoCredential,
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSupplier {
    Nyxid,
    Own,
    None,
    Unknown,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ServiceBillingExplanation {
    pub status: BillingExplanationStatus,
    pub credential_class: Option<CredentialClass>,
    pub credential_label: String,
    pub account: Option<ServiceBillingAccount>,
    pub charge_status: ConnectionChargeStatus,
    /// Saved usage-charge configuration, independent of availability and caller rollout.
    pub credit_billing_configured: Option<bool>,
    /// Whether any credential class has a configured charge for this service.
    pub service_billing_configured: Option<bool>,
    /// Credential supply is distinct from the execution price-lane classification.
    pub credential_supplier: Option<CredentialSupplier>,
    pub rates: Vec<ServiceBillingRate>,
    pub provider_billing: ProviderBillingDisclosure,
    /// "for_you" uses the viewer's default; "agent_key" includes that key's override.
    pub context: String,
    pub notes: Vec<String>,
}

/// Metadata-only preview for rows already selected by the inventory ACL.
/// Distinct catalog, credential and account rows are loaded once per request.
/// No credential resolution, wallet provisioning, reservation or provider call
/// may be added here: opening AI Services must not execute a service.
pub async fn explain_connections(
    db: &Database,
    billing: &BillingService,
    billing_principal_id: &str,
    actor_user_id: &str,
    services: &[UserService],
) -> AppResult<HashMap<String, ServiceBillingExplanation>> {
    explain_connections_in_context(
        db,
        billing,
        billing_principal_id,
        actor_user_id,
        services,
        None,
    )
    .await
}

/// Inspect a managed agent identity without authenticating as that key or
/// granting its permissions to the viewer. The handler's service ACL remains
/// in force in addition to the key's live execution authority.
pub async fn explain_for_agent_key(
    db: &Database,
    billing: &BillingService,
    actor_id: &str,
    services: &[UserService],
    api_key_id: &str,
) -> AppResult<HashMap<String, ServiceBillingExplanation>> {
    if services.len() > 100 {
        return Err(AppError::ValidationError(
            "At most 100 connections may be requested".into(),
        ));
    }
    let key = db
        .collection::<ApiKey>(AGENT_KEYS)
        .find_one(doc! { "_id": api_key_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Agent key not found".into()))?;
    let management_access = org_service::resolve_owner_access(db, actor_id, &key.user_id).await?;
    if !management_access.can_write() {
        return Err(AppError::NotFound("Agent key not found".into()));
    }
    let active = key.is_active
        && key.purpose == ApiKeyPurpose::General
        && key
            .expires_at
            .is_none_or(|expires| expires > chrono::Utc::now())
        && crate::mw::auth::scope_allows_llm_proxy(&key.scopes);
    let effective_ids = if active {
        crate::services::key_service::effective_allowed_service_ids(db, &key).await?
    } else {
        Vec::new()
    };
    let mut selected = Vec::new();
    let mut result = HashMap::new();
    let mut viewer_accesses = HashMap::new();
    for service in services {
        if !viewer_accesses.contains_key(&service.user_id) {
            viewer_accesses.insert(
                service.user_id.clone(),
                org_service::resolve_owner_access(db, actor_id, &service.user_id).await?,
            );
        }
        let access = &viewer_accesses[&service.user_id];
        let explanation = if !access.can_read() || !access.allows_resource(&service.id) {
            Some(restricted())
        } else if !active {
            Some(unavailable(
                "This agent key is inactive, expired, or lacks proxy permission.",
            ))
        } else if !key.allow_all_services && !effective_ids.contains(&service.id) {
            Some(unavailable(
                "This agent key does not have access to this connection.",
            ))
        } else if !key.allow_all_nodes
            && service
                .node_id
                .as_ref()
                .is_some_and(|id| !key.allowed_node_ids.contains(id))
        {
            Some(unavailable(
                "This agent key does not have access to the connection's node.",
            ))
        } else {
            None
        };
        if let Some(explanation) = explanation {
            result.insert(service.id.clone(), explanation);
        } else {
            selected.push(service.clone());
        }
    }
    result.extend(
        explain_connections_in_context(
            db,
            billing,
            &key.user_id,
            &key.user_id,
            &selected,
            Some(&key),
        )
        .await?,
    );
    for explanation in result.values_mut() {
        explanation.context = "agent_key".into();
    }
    Ok(result)
}

async fn explain_connections_in_context(
    db: &Database,
    billing: &BillingService,
    billing_principal_id: &str,
    actor_user_id: &str,
    services: &[UserService],
    agent_key: Option<&ApiKey>,
) -> AppResult<HashMap<String, ServiceBillingExplanation>> {
    if services.is_empty() {
        return Ok(HashMap::new());
    }
    let bindings: Vec<AgentServiceBinding> = if let Some(key) = agent_key {
        let service_ids: Vec<_> = services.iter().map(|s| s.id.as_str()).collect();
        db.collection::<AgentServiceBinding>(BINDINGS)
            .find(doc! {
                "api_key_id": &key.id, "user_id": &key.user_id,
                "user_service_id": { "$in": service_ids },
            })
            .await?
            .try_collect()
            .await?
    } else {
        Vec::new()
    };
    let overrides: HashMap<_, _> = bindings
        .iter()
        .map(|b| (b.user_service_id.as_str(), b))
        .collect();
    let catalog_ids: Vec<_> = services
        .iter()
        .filter_map(|s| s.catalog_service_id.as_deref())
        .collect();
    let mut credential_ids: Vec<_> = services
        .iter()
        .filter_map(|s| s.api_key_id.as_deref())
        .collect();
    credential_ids.extend(
        bindings
            .iter()
            .map(|binding| binding.user_api_key_id.as_str()),
    );
    let endpoints: HashMap<String, UserEndpoint> = if bindings.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<_> = services
            .iter()
            .filter(|s| overrides.contains_key(s.id.as_str()))
            .map(|s| s.endpoint_id.as_str())
            .collect();
        db.collection::<UserEndpoint>(ENDPOINTS)
            .find(doc! { "_id": { "$in": ids } })
            .await?
            .try_collect::<Vec<_>>()
            .await?
            .into_iter()
            .map(|e| (e.id.clone(), e))
            .collect()
    };
    let mut owner_ids: Vec<_> = services.iter().map(|s| s.user_id.as_str()).collect();
    owner_ids.push(billing_principal_id);
    let (catalog, credentials, owners): (Vec<DownstreamService>, Vec<UserApiKey>, Vec<User>) = tokio::try_join!(
        async {
            db.collection::<DownstreamService>(CATALOG)
                .find(doc! { "_id": { "$in": catalog_ids } })
                .await?
                .try_collect()
                .await
        },
        async {
            db.collection::<UserApiKey>(CREDENTIALS)
                .find(doc! { "_id": { "$in": credential_ids } })
                .await?
                .try_collect()
                .await
        },
        async {
            db.collection::<User>(USERS)
                .find(doc! { "_id": { "$in": owner_ids } })
                .await?
                .try_collect()
                .await
        },
    )?;
    let app_sources =
        super::oauth_app_source::load(db, &credentials.iter().collect::<Vec<_>>()).await?;
    let catalog: HashMap<_, _> = catalog.into_iter().map(|s| (s.id.clone(), s)).collect();
    let credentials: HashMap<_, _> = credentials.into_iter().map(|k| (k.id.clone(), k)).collect();
    let owners: HashMap<_, _> = owners.into_iter().map(|u| (u.id.clone(), u)).collect();
    let needs_platform_grants = services.iter().any(uses_platform_binding);
    let grants = if needs_platform_grants {
        Some(platform_key_service::OwnerGrants::load(db, billing_principal_id).await?)
    } else {
        None
    };
    let providers = if needs_platform_grants {
        platform_key_service::load_providers(db).await?
    } else {
        HashMap::new()
    };
    let mut accesses = HashMap::new();
    let mut materializable = HashMap::new();
    let mut rollout = HashMap::new();
    let mut resolved_owners = HashMap::new();
    let mut explanations = HashMap::new();

    for service in services {
        if !accesses.contains_key(&service.user_id) {
            accesses.insert(
                service.user_id.clone(),
                org_service::resolve_owner_access(db, billing_principal_id, &service.user_id)
                    .await?,
            );
        }
        let access = &accesses[&service.user_id];
        if !access.can_read() || !access.allows_resource(&service.id) {
            explanations.insert(service.id.clone(), restricted());
            continue;
        }
        if matches!(access, org_service::OwnerAccess::AsOrgMember { role, .. } if !role.can_proxy())
            || (service.admin_only
                && matches!(access, org_service::OwnerAccess::AsOrgMember { .. }))
        {
            explanations.insert(
                service.id.clone(),
                unavailable("You do not have execution access to this connection."),
            );
            continue;
        }
        if !service.is_active || service.deleted_at.is_some() {
            explanations.insert(
                service.id.clone(),
                unavailable("This connection is disabled."),
            );
            continue;
        }
        let catalog_service = service
            .catalog_service_id
            .as_ref()
            .and_then(|id| catalog.get(id));
        if service.catalog_service_id.is_some() && catalog_service.is_none() {
            explanations.insert(
                service.id.clone(),
                unavailable("The service billing configuration is unavailable."),
            );
            continue;
        }
        let credential = service
            .api_key_id
            .as_ref()
            .and_then(|id| credentials.get(id));
        let mut effective_auth_method = service.auth_method.clone();
        let default_class = if uses_platform_binding(service) {
            let available = catalog_service.is_some_and(|catalog| {
                grants.as_ref().is_some_and(|grants| {
                    platform_key_service::available_with_grants(
                        catalog,
                        catalog
                            .provider_config_id
                            .as_ref()
                            .and_then(|id| providers.get(id)),
                        &service.user_id,
                        grants,
                    )
                })
            });
            if !available || service.node_id.is_some() {
                explanations.insert(
                    service.id.clone(),
                    unavailable("The NyxID credential is unavailable for this connection."),
                );
                continue;
            }
            // Explicit platform bindings can override the stored connection auth.
            let Some(catalog_service) = catalog_service else {
                explanations.insert(
                    service.id.clone(),
                    unavailable("The service billing configuration is unavailable."),
                );
                continue;
            };
            match platform_key_service::effective_auth(db, catalog_service).await {
                Ok((method, _)) => {
                    effective_auth_method = method;
                    CredentialClass::NyxidManagedMaster
                }
                Err(AppError::ValidationError(_)) => {
                    explanations.insert(
                        service.id.clone(),
                        unavailable("The NyxID credential configuration is incomplete."),
                    );
                    continue;
                }
                Err(error) => return Err(error),
            }
        } else if service.auth_method == "none" {
            if service.node_id.is_some() {
                CredentialClass::NodeManaged
            } else {
                CredentialClass::NoAuth
            }
        } else {
            let Some(credential) = credential.filter(|key| key.user_id == service.user_id) else {
                explanations.insert(
                    service.id.clone(),
                    unavailable("No connection credential is available."),
                );
                continue;
            };
            if !materializable.contains_key(&credential.id) {
                materializable.insert(
                    credential.id.clone(),
                    proxy_service::credential_is_materializable(db, credential).await?,
                );
            }
            let has_server_credential = materializable[&credential.id];
            if service.node_id.is_none()
                && (credential.status != "active" || !has_server_credential)
            {
                explanations.insert(
                    service.id.clone(),
                    unavailable(
                        "Reconnect or replace the connection credential before using this service.",
                    ),
                );
                continue;
            }
            default_credential_class(service, credential, has_server_credential)
        };
        let credential_class = if let Some(binding) = overrides.get(service.id.as_str()) {
            let override_key = credentials.get(&binding.user_api_key_id).filter(|key| {
                key.user_id == billing_principal_id && key.user_id == service.user_id
            });
            let Some(override_key) = override_key else {
                explanations.insert(
                    service.id.clone(),
                    unavailable("The agent's credential override is unavailable."),
                );
                continue;
            };
            if !materializable.contains_key(&override_key.id) {
                materializable.insert(
                    override_key.id.clone(),
                    proxy_service::credential_is_materializable(db, override_key).await?,
                );
            }
            if override_key.status != "active" || !materializable[&override_key.id] {
                explanations.insert(
                    service.id.clone(),
                    unavailable("Reconnect or replace this agent's credential override."),
                );
                continue;
            }
            let target_url = if default_class == CredentialClass::NyxidManagedMaster {
                catalog_service.map(|c| c.base_url.as_str())
            } else {
                endpoints
                    .get(&service.endpoint_id)
                    .filter(|e| e.user_id == service.user_id)
                    .map(|e| e.url.as_str())
            };
            let Some(target_url) = target_url else {
                explanations.insert(
                    service.id.clone(),
                    unavailable("The override's destination is unavailable."),
                );
                continue;
            };
            match crate::services::ifttt_oauth_service::validate_credential_route(
                db,
                override_key.provider_config_id.as_deref(),
                &effective_auth_method,
                target_url,
                None,
            )
            .await
            {
                Ok(()) => {}
                Err(AppError::ValidationError(_)) => {
                    explanations.insert(service.id.clone(), unavailable("This credential override cannot be used with the connection's destination."));
                    continue;
                }
                Err(error) => return Err(error),
            }
            // The proxy classifies a node without a default server credential
            // as node-managed before considering an agent override.
            if default_class == CredentialClass::NodeManaged {
                default_class
            } else {
                CredentialClass::AgentOverrideUserOwned
            }
        } else {
            default_class
        };
        let owner_key = (
            service.user_id.clone(),
            credential_class == CredentialClass::NyxidManagedMaster,
        );
        if !resolved_owners.contains_key(&owner_key) {
            let resolved = billing
                .owner_resolver()
                .resolve_for_execution(billing_principal_id, &service.user_id, credential_class)
                .await?;
            resolved_owners.insert(owner_key.clone(), resolved.owner_id);
        }
        let payer_id = &resolved_owners[&owner_key];
        let Some(payer) = owners.get(payer_id).filter(|u| u.is_active) else {
            explanations.insert(
                service.id.clone(),
                unavailable("The billing account is unavailable."),
            );
            continue;
        };
        if !rollout.contains_key(payer_id) {
            let enabled = billing.billing_enabled()
                && feature_flag_service::billing_rollout_enabled(db, payer_id, actor_user_id)
                    .await?;
            rollout.insert(payer_id.clone(), enabled);
        }
        let account = ServiceBillingAccount {
            id: payer_id.clone(),
            kind: if payer.user_type.is_org() {
                BillingAccountKind::Organization
            } else {
                BillingAccountKind::Personal
            },
            name: if payer.user_type.is_org() {
                payer
                    .display_name
                    .clone()
                    .unwrap_or_else(|| "Organization".into())
            } else {
                "Your personal account".into()
            },
        };
        let mut explanation = project_billing(
            service,
            catalog_service.and_then(|s| s.billing.as_ref()),
            credential_class,
            account,
            rollout[payer_id],
            billing.resale_enabled(),
            billing.lago_configured(),
        );
        explanation.credential_supplier = Some(
            if credential_class == CredentialClass::AgentOverrideUserOwned {
                overrides
                    .get(service.id.as_str())
                    .and_then(|binding| credentials.get(&binding.user_api_key_id))
                    .map_or(CredentialSupplier::Unknown, |key| {
                        resolved_credential_supplier(key, &app_sources)
                    })
            } else {
                let supplier = connection_credential_supplier(service, credential);
                if supplier == CredentialSupplier::Unknown {
                    credential
                        .filter(|key| key.user_id == service.user_id)
                        .map_or(supplier, |key| {
                            resolved_credential_supplier(key, &app_sources)
                        })
                } else {
                    supplier
                }
            },
        );
        annotate_transport_pricing(&mut explanation, catalog_service);
        if credential_class == CredentialClass::NodeManaged {
            explanation.status = BillingExplanationStatus::Conditional;
            explanation
                .notes
                .push("Uses the node credential if the node is available.".into());
        }
        explanations.insert(service.id.clone(), explanation);
    }
    // Keep configured billability visible for disabled or unavailable connections.
    // This reads the already-loaded metadata, without resolving credentials or a payer.
    for service in services {
        let Some(explanation) = explanations.get_mut(&service.id) else {
            continue;
        };
        if agent_key.is_none()
            && explanation.status != BillingExplanationStatus::Restricted
            && explanation.credential_supplier.is_none()
            && !uses_platform_binding(service)
            && service.auth_method != "none"
        {
            explanation.credential_supplier = service
                .api_key_id
                .as_ref()
                .and_then(|id| credentials.get(id))
                .filter(|key| key.user_id == service.user_id)
                .map(|key| resolved_credential_supplier(key, &app_sources));
        }
        annotate_configured_charge(
            explanation,
            service,
            service
                .catalog_service_id
                .as_ref()
                .and_then(|id| catalog.get(id)),
            service
                .api_key_id
                .as_ref()
                .and_then(|id| credentials.get(id)),
            agent_key.is_some(),
        );
    }
    Ok(explanations)
}

fn annotate_configured_charge(
    explanation: &mut ServiceBillingExplanation,
    service: &UserService,
    catalog: Option<&DownstreamService>,
    credential: Option<&UserApiKey>,
    agent_context: bool,
) {
    if explanation.status == BillingExplanationStatus::Restricted
        || (service.catalog_service_id.is_some() && catalog.is_none())
    {
        return;
    }
    let configuration = catalog.and_then(|service| service.billing.as_ref());
    explanation.service_billing_configured = Some(service_billing_configured(configuration));
    if explanation.credential_supplier.is_none() {
        explanation.credential_supplier = Some(if agent_context {
            CredentialSupplier::Unknown
        } else {
            connection_credential_supplier(service, credential)
        });
    }
    if matches!(
        explanation.status,
        BillingExplanationStatus::Resolved | BillingExplanationStatus::Conditional
    ) {
        match explanation.credential_supplier {
            Some(CredentialSupplier::Unknown) => {
                explanation.provider_billing = ProviderBillingDisclosure::Unknown;
                explanation.credential_label = "Credential supplier unverified".into();
            }
            Some(CredentialSupplier::Nyxid) => {
                explanation.provider_billing = ProviderBillingDisclosure::NyxidCredential;
                explanation.credential_label =
                    if explanation.credential_class == Some(CredentialClass::NyxidManagedMaster) {
                        "NyxID key"
                    } else {
                        "NyxID OAuth app"
                    }
                    .into();
            }
            _ => {}
        }
    }
    if agent_context && explanation.credential_class.is_none() {
        return;
    }
    let stored = credential.filter(|key| key.user_id == service.user_id);
    let class = explanation.credential_class.or_else(|| {
        if uses_platform_binding(service) {
            Some(CredentialClass::NyxidManagedMaster)
        } else if service.node_id.is_some() {
            // Without materializing a disabled credential, shared OAuth versus node
            // supply is ambiguous and can change a credential-restricted price.
            if stored.is_some_and(|key| key.credential_source.as_deref() == Some("platform")) {
                None
            } else {
                Some(CredentialClass::NodeManaged)
            }
        } else if service.auth_method == "none" {
            Some(CredentialClass::NoAuth)
        } else {
            stored.map(|key| default_credential_class(service, key, true))
        }
    });
    explanation.credit_billing_configured = if configuration.is_none() {
        Some(false)
    } else {
        class.map(|class| configured_usage_charge(configuration, class))
    };
}

fn connection_credential_supplier(
    service: &UserService,
    credential: Option<&UserApiKey>,
) -> CredentialSupplier {
    if uses_platform_binding(service) {
        return CredentialSupplier::Nyxid;
    }
    if service.auth_method == "none" && service.node_id.is_none() {
        return CredentialSupplier::None;
    }
    credential
        .filter(|key| key.user_id == service.user_id)
        .map_or(CredentialSupplier::Unknown, stored_credential_supplier)
}

fn stored_credential_supplier(key: &UserApiKey) -> CredentialSupplier {
    if matches!(key.credential_type.as_str(), "oauth2" | "device_code") {
        match super::oauth_app_source::from_key(key) {
            Some(super::oauth_app_source::OAuthAppSource::Platform) => CredentialSupplier::Nyxid,
            Some(super::oauth_app_source::OAuthAppSource::Byo) => CredentialSupplier::Own,
            None => CredentialSupplier::Unknown,
        }
    } else if matches!(
        key.credential_type.as_str(),
        "api_key" | "bearer" | "basic" | "token_exchange" | "ssh_certificate" | "node_managed"
    ) {
        CredentialSupplier::Own
    } else {
        CredentialSupplier::Unknown
    }
}

fn resolved_credential_supplier(
    key: &UserApiKey,
    sources: &HashMap<String, super::oauth_app_source::OAuthAppSource>,
) -> CredentialSupplier {
    match sources.get(&key.id) {
        Some(super::oauth_app_source::OAuthAppSource::Platform) => CredentialSupplier::Nyxid,
        Some(super::oauth_app_source::OAuthAppSource::Byo) => CredentialSupplier::Own,
        None => stored_credential_supplier(key),
    }
}

fn service_billing_configured(configuration: Option<&ServiceBilling>) -> bool {
    [
        CredentialClass::NyxidManagedMaster,
        CredentialClass::NyxidPlatformOauthApp,
        CredentialClass::UserOwned,
        CredentialClass::NoAuth,
    ]
    .into_iter()
    .any(|class| configured_usage_charge(configuration, class))
}

fn configured_usage_charge(configuration: Option<&ServiceBilling>, class: CredentialClass) -> bool {
    let Some(billing) = configuration else {
        return false;
    };
    if class == CredentialClass::NyxidManagedMaster && billing.resale_billable {
        return true;
    }
    if billing.platform_charge_nyxid_credentials_only
        && !matches!(
            class,
            CredentialClass::NyxidManagedMaster | CredentialClass::NyxidPlatformOauthApp
        )
    {
        return false;
    }
    let positive = |rate: &str| {
        crate::services::billing::amounts::decimal_to_pico(rate).is_some_and(|rate| rate > 0)
    };
    let legacy = billing.platform_billable
        && billing.platform_pricing.as_ref().is_none_or(|price| {
            price.sync_status != PricingSyncStatus::Synced || positive(&price.credits_per_unit)
        });
    if billing.byok_pricing.is_none() && billing.platform_key_pricing.is_none() {
        return legacy;
    }
    let lane = match class {
        CredentialClass::NyxidManagedMaster => billing.platform_key_pricing.as_ref(),
        CredentialClass::NoAuth => None,
        _ => billing.byok_pricing.as_ref(),
    };
    lane.is_some_and(|lane| {
        positive(&lane.credits_per_unit)
            || lane
                .components
                .iter()
                .any(|rate| positive(&rate.credits_per_unit))
            || (lane.sync_status != PricingSyncStatus::Synced && legacy)
    })
}

fn annotate_transport_pricing(
    explanation: &mut ServiceBillingExplanation,
    catalog: Option<&DownstreamService>,
) {
    let Some(catalog) = catalog else {
        return;
    };
    if catalog
        .capabilities
        .as_ref()
        .is_some_and(|capabilities| capabilities.supports_websocket)
        && catalog
            .billing
            .as_ref()
            .is_none_or(|billing| billing.platform_metric.is_none())
        && explanation.rates.iter().any(|rate| {
            rate.layer == "platform"
                && rate.source != "credential_lane"
                && rate.metric != BillingMetric::Bytes
        })
    {
        explanation.status = BillingExplanationStatus::Conditional;
        explanation.notes.push(
            "Rates shown are for HTTP requests. WebSocket connections use bytes and may have a different rate.".into(),
        );
    }
}

fn uses_platform_binding(service: &UserService) -> bool {
    platform_key_service::binding(service) == "platform"
        && (service.auth_method != "none"
            || service.credential_binding.as_deref() == Some("platform"))
}

fn default_credential_class(
    service: &UserService,
    credential: &UserApiKey,
    has_server_credential: bool,
) -> CredentialClass {
    if service.node_id.is_some() && !has_server_credential {
        CredentialClass::NodeManaged
    } else if credential.credential_source.as_deref() == Some("platform") {
        CredentialClass::NyxidPlatformOauthApp
    } else {
        CredentialClass::UserOwned
    }
}

fn restricted() -> ServiceBillingExplanation {
    ServiceBillingExplanation {
        status: BillingExplanationStatus::Restricted,
        credential_class: None,
        credential_label: "Credential restricted".into(),
        account: None,
        charge_status: ConnectionChargeStatus::Restricted,
        credit_billing_configured: None,
        service_billing_configured: None,
        credential_supplier: None,
        rates: Vec::new(),
        provider_billing: ProviderBillingDisclosure::Unknown,
        context: "for_you".into(),
        notes: Vec::new(),
    }
}

fn unavailable(reason: &str) -> ServiceBillingExplanation {
    ServiceBillingExplanation {
        status: BillingExplanationStatus::Unavailable,
        credential_label: "Credential unavailable".into(),
        charge_status: ConnectionChargeStatus::Unavailable,
        notes: vec![reason.into()],
        ..restricted()
    }
}

#[allow(clippy::too_many_arguments)]
fn project_billing(
    service: &UserService,
    configuration: Option<&ServiceBilling>,
    credential_class: CredentialClass,
    account: ServiceBillingAccount,
    charging_enabled: bool,
    resale_enabled: bool,
    provider_configured: bool,
) -> ServiceBillingExplanation {
    // UserService proxy targets use the connection slug with the catalog's
    // billing block. The catalog slug is not the metering heuristic here.
    let metric = configuration
        .and_then(|b| b.platform_metric)
        .unwrap_or_else(|| {
            if service.service_type == "ssh" {
                BillingMetric::Bytes
            } else if service.slug.starts_with("llm-") {
                BillingMetric::Tokens
            } else {
                BillingMetric::Requests
            }
        });
    let ctx = BillingRouteContext::new(
        BillingIngress::Proxy,
        String::new(),
        account.id.clone(),
        String::new(),
        None,
        Some(service.id.clone()),
        service.catalog_service_id.clone(),
        Some(service.slug.clone()),
        NodeIntent::Direct,
        service.auth_method.clone(),
        credential_class,
        metric,
        configuration,
        resale_enabled,
    );
    let (credential_label, provider_billing) = match credential_class {
        CredentialClass::NyxidManagedMaster => (
            "NyxID credential",
            ProviderBillingDisclosure::NyxidCredential,
        ),
        CredentialClass::NyxidPlatformOauthApp => (
            "NyxID OAuth app",
            ProviderBillingDisclosure::SeparateProviderAccount,
        ),
        CredentialClass::NodeManaged => (
            "Node credential",
            ProviderBillingDisclosure::SeparateProviderAccount,
        ),
        CredentialClass::NoAuth => ("No credential", ProviderBillingDisclosure::NoCredential),
        CredentialClass::AgentOverrideUserOwned => (
            "Credential override",
            ProviderBillingDisclosure::SeparateProviderAccount,
        ),
        CredentialClass::UserOwned if account.kind == BillingAccountKind::Organization => (
            "Organization credential",
            ProviderBillingDisclosure::SeparateProviderAccount,
        ),
        CredentialClass::UserOwned => (
            "Your credential",
            ProviderBillingDisclosure::SeparateProviderAccount,
        ),
    };
    let mut result = ServiceBillingExplanation {
        status: BillingExplanationStatus::Resolved,
        credential_class: Some(credential_class),
        credential_label: credential_label.into(),
        account: Some(account),
        charge_status: ConnectionChargeStatus::NotCharged,
        credit_billing_configured: Some(configured_usage_charge(configuration, credential_class)),
        service_billing_configured: Some(service_billing_configured(configuration)),
        credential_supplier: None,
        rates: Vec::new(),
        provider_billing,
        context: "for_you".into(),
        notes: Vec::new(),
    };
    if !charging_enabled || (!ctx.service_platform_billable && ctx.resale.is_none()) {
        result
            .notes
            .push("No NyxID usage charge applies to this caller and default credential.".into());
        return result;
    }
    result.charge_status = ConnectionChargeStatus::UsageBased;
    if ctx.service_platform_billable {
        for (metric, code) in ctx.platform_specs() {
            result
                .rates
                .push(platform_rate(configuration, credential_class, metric, code));
        }
    }
    if let Some(resale) = &ctx.resale {
        result.rates.push(ServiceBillingRate {
            layer: "resale".into(),
            metric: resale.metric,
            credits_per_unit: None,
            currency: "credits".into(),
            source: "legacy_plan".into(),
        });
    }
    if result
        .rates
        .iter()
        .any(|rate| rate.credits_per_unit.is_none())
    {
        result.status = BillingExplanationStatus::Conditional;
        result.notes.push(
            "A legacy plan rate applies; the exact amount is determined at execution.".into(),
        );
    }
    if !provider_configured {
        result.status = BillingExplanationStatus::Conditional;
        result.charge_status = ConnectionChargeStatus::Conditional;
        result.notes.push(
            "Billing setup is unavailable; execution depends on the server's billing policy."
                .into(),
        );
    }
    result.notes.push("Allowances and grants are applied before wallet credits. Actual debit is recorded after settlement.".into());
    result
}

fn platform_rate(
    configuration: Option<&ServiceBilling>,
    class: CredentialClass,
    metric: BillingMetric,
    code: &str,
) -> ServiceBillingRate {
    let lane = configuration
        .and_then(|b| match class {
            CredentialClass::NyxidManagedMaster => b.platform_key_pricing.as_ref(),
            CredentialClass::NoAuth => None,
            _ => b.byok_pricing.as_ref(),
        })
        .filter(|l| l.sync_status == PricingSyncStatus::Synced);
    let lane_price = lane.and_then(|l| {
        if l.lago_metric_code == code {
            Some(l.credits_per_unit.clone())
        } else {
            l.components
                .iter()
                .find(|c| c.sync_status == PricingSyncStatus::Synced && c.lago_metric_code == code)
                .map(|c| c.credits_per_unit.clone())
        }
    });
    let service_price = configuration
        .and_then(|b| b.platform_pricing.as_ref())
        .filter(|p| p.sync_status == PricingSyncStatus::Synced && p.lago_metric_code == code)
        .map(|p| p.credits_per_unit.clone());
    let source = if lane_price.is_some() {
        "credential_lane"
    } else if service_price.is_some() {
        "service_price"
    } else {
        "legacy_plan"
    };
    ServiceBillingRate {
        layer: "platform".into(),
        metric,
        credits_per_unit: lane_price.or(service_price),
        currency: "credits".into(),
        source: source.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
    use crate::models::service_billing::{LanePriceComponent, LanePricing, ServicePlatformPricing};
    use crate::models::user::UserType;
    use crate::test_utils::{
        connect_test_database, test_app_config, test_membership, test_user, test_user_service,
    };

    fn connection() -> UserService {
        bson::from_document(doc! {
            "_id": "connection", "user_id": "owner", "slug": "openai-work",
            "endpoint_id": "endpoint", "api_key_id": "credential",
            "auth_method": "bearer", "auth_key_name": "Authorization",
            "is_active": true, "created_at": bson::DateTime::now(),
            "updated_at": bson::DateTime::now(),
        })
        .unwrap()
    }

    fn account(kind: BillingAccountKind) -> ServiceBillingAccount {
        ServiceBillingAccount {
            id: "owner".into(),
            kind,
            name: "Account".into(),
        }
    }

    fn lane(metric: BillingMetric, price: &str, code: &str) -> LanePricing {
        LanePricing {
            metric,
            credits_per_unit: price.into(),
            lago_metric_code: code.into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
            components: Vec::new(),
        }
    }

    fn metadata_credential(kind: &str) -> UserApiKey {
        bson::from_document(doc! {
            "_id": "credential", "user_id": "owner", "label": "Connection",
            "credential_type": kind, "status": "active",
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .unwrap()
    }

    #[test]
    fn billing_metadata_service_gate_is_independent_of_twitter_oauth_lane() {
        let config = ServiceBilling {
            platform_billable: true,
            platform_key_pricing: Some(lane(BillingMetric::Requests, "0.05", "twitter-pk")),
            ..Default::default()
        };
        assert!(service_billing_configured(Some(&config)));
        let oauth = project_billing(
            &connection(),
            Some(&config),
            CredentialClass::NyxidPlatformOauthApp,
            account(BillingAccountKind::Personal),
            true,
            false,
            true,
        );
        assert_eq!(oauth.service_billing_configured, Some(true));
        assert_eq!(oauth.credit_billing_configured, Some(false));
        assert_eq!(oauth.charge_status, ConnectionChargeStatus::NotCharged);
        assert!(oauth.rates.is_empty());
        assert!(!service_billing_configured(None));
        assert!(!service_billing_configured(Some(&ServiceBilling {
            platform_key_pricing: Some(lane(BillingMetric::Requests, "0", "zero")),
            ..Default::default()
        })));
    }

    #[test]
    fn billing_metadata_supplier_uses_durable_oauth_provenance() {
        let service = connection();
        let mut key = metadata_credential("oauth2");
        assert_eq!(
            default_credential_class(&service, &key, true),
            CredentialClass::UserOwned
        );
        assert_eq!(
            stored_credential_supplier(&key),
            CredentialSupplier::Unknown
        );
        key.credential_source = Some("byo".into());
        assert_eq!(stored_credential_supplier(&key), CredentialSupplier::Own);
        key.user_oauth_client_id_encrypted = Some(vec![1, 2, 3]);
        key.credential_source = Some("platform".into());
        assert_eq!(stored_credential_supplier(&key), CredentialSupplier::Nyxid);
        key.credential_source = None;
        key.connection_id = Some("connection".into());
        key.provider_config_id = Some("provider".into());
        assert_eq!(stored_credential_supplier(&key), CredentialSupplier::Own);
        key.user_oauth_client_id_encrypted = None;
        assert_eq!(stored_credential_supplier(&key), CredentialSupplier::Nyxid);
        // Legacy selection must use the provider token, including for disabled
        // connections and selected agent overrides, not a retained client hint.
        key.connection_id = None;
        assert_eq!(
            stored_credential_supplier(&key),
            CredentialSupplier::Unknown
        );
        let sources = HashMap::from([(
            key.id.clone(),
            super::super::oauth_app_source::OAuthAppSource::Platform,
        )]);
        assert_eq!(
            resolved_credential_supplier(&key, &sources),
            CredentialSupplier::Nyxid
        );
        assert_eq!(
            stored_credential_supplier(&metadata_credential("api_key")),
            CredentialSupplier::Own
        );
        assert_eq!(
            stored_credential_supplier(&metadata_credential("node_managed")),
            CredentialSupplier::Own
        );
        assert_eq!(
            connection_credential_supplier(&service, None),
            CredentialSupplier::Unknown
        );
        let mut platform = service.clone();
        platform.credential_binding = Some("platform".into());
        assert_eq!(
            connection_credential_supplier(&platform, Some(&key)),
            CredentialSupplier::Nyxid
        );
    }

    #[test]
    fn billing_metadata_disabled_and_agent_preserve_service_gate() {
        let mut service = connection();
        service.is_active = false;
        let mut key = metadata_credential("oauth2");
        key.credential_source = Some("platform".into());
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.billing = Some(ServiceBilling {
            platform_key_pricing: Some(lane(BillingMetric::Requests, "0.05", "twitter-pk")),
            ..Default::default()
        });
        let mut result = unavailable("Disabled");
        annotate_configured_charge(&mut result, &service, Some(&catalog), Some(&key), false);
        assert_eq!(result.service_billing_configured, Some(true));
        assert_eq!(result.credential_supplier, Some(CredentialSupplier::Nyxid));
        let mut agent = unavailable("Override unavailable");
        annotate_configured_charge(&mut agent, &service, Some(&catalog), Some(&key), true);
        assert_eq!(agent.service_billing_configured, Some(true));
        assert_eq!(agent.credential_supplier, Some(CredentialSupplier::Unknown));
        catalog.billing = None;
        annotate_configured_charge(&mut agent, &service, Some(&catalog), Some(&key), true);
        assert_eq!(agent.service_billing_configured, Some(false));
        let mut hidden = restricted();
        annotate_configured_charge(&mut hidden, &service, Some(&catalog), Some(&key), false);
        assert!(hidden.service_billing_configured.is_none());
        assert!(hidden.credential_supplier.is_none());
    }

    #[test]
    fn configured_billability_for_disabled_connections_preserves_access_and_unknown_states() {
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.billing = Some(ServiceBilling {
            platform_key_pricing: Some(lane(BillingMetric::Requests, "1", "pk")),
            ..Default::default()
        });
        let mut service = connection();
        service.catalog_service_id = Some(catalog.id.clone());
        service.is_active = false;
        service.credential_binding = Some("platform".into());
        let mut disabled = unavailable("This connection is disabled.");
        annotate_configured_charge(&mut disabled, &service, Some(&catalog), None, false);
        assert_eq!(disabled.credit_billing_configured, Some(true));
        assert_eq!(disabled.status, BillingExplanationStatus::Unavailable);
        assert!(disabled.account.is_none());
        let mut hidden = restricted();
        annotate_configured_charge(&mut hidden, &service, Some(&catalog), None, false);
        assert_eq!(hidden.credit_billing_configured, None);
        let mut missing_catalog = unavailable("Missing configuration");
        annotate_configured_charge(&mut missing_catalog, &service, None, None, false);
        assert_eq!(missing_catalog.credit_billing_configured, None);
        let mut override_unavailable = unavailable("Agent override unavailable");
        annotate_configured_charge(
            &mut override_unavailable,
            &service,
            Some(&catalog),
            None,
            true,
        );
        assert_eq!(override_unavailable.credit_billing_configured, None);
    }

    #[test]
    fn configured_billability_counts_the_credential_lane_independent_of_rollout() {
        let mut config = ServiceBilling {
            platform_key_pricing: Some(lane(BillingMetric::Requests, "1", "pk")),
            ..Default::default()
        };
        assert!(configured_usage_charge(
            Some(&config),
            CredentialClass::NyxidManagedMaster
        ));
        assert!(!configured_usage_charge(
            Some(&config),
            CredentialClass::UserOwned
        ));
        config.byok_pricing = Some(lane(BillingMetric::Requests, "0", "byok"));
        assert!(!configured_usage_charge(
            Some(&config),
            CredentialClass::UserOwned
        ));
        config
            .byok_pricing
            .as_mut()
            .unwrap()
            .components
            .push(LanePriceComponent {
                metric: BillingMetric::Images,
                credits_per_unit: "0.000000000001".into(),
                lago_metric_code: "images".into(),
                sync_status: PricingSyncStatus::Pending,
                sync_error: None,
            });
        assert!(configured_usage_charge(
            Some(&config),
            CredentialClass::UserOwned
        ));
        let mut service = connection();
        service.is_active = false;
        let explanation = project_billing(
            &service,
            Some(&config),
            CredentialClass::UserOwned,
            ServiceBillingAccount {
                id: "person".into(),
                kind: BillingAccountKind::Personal,
                name: "Personal".into(),
            },
            false,
            false,
            false,
        );
        assert_eq!(explanation.credit_billing_configured, Some(true));
        assert_eq!(
            explanation.charge_status,
            ConnectionChargeStatus::NotCharged
        );
        config.platform_charge_nyxid_credentials_only = true;
        assert!(!configured_usage_charge(
            Some(&config),
            CredentialClass::UserOwned
        ));
        assert!(configured_usage_charge(
            Some(&config),
            CredentialClass::NyxidPlatformOauthApp
        ));
        let mut pending = ServiceBilling {
            platform_billable: true,
            platform_pricing: Some(ServicePlatformPricing {
                credits_per_unit: "0".into(),
                lago_metric_code: "legacy".into(),
                sync_status: PricingSyncStatus::Pending,
                sync_error: None,
            }),
            ..Default::default()
        };
        assert!(configured_usage_charge(
            Some(&pending),
            CredentialClass::UserOwned
        ));
        pending.platform_pricing.as_mut().unwrap().sync_status = PricingSyncStatus::Synced;
        assert!(!configured_usage_charge(
            Some(&pending),
            CredentialClass::UserOwned
        ));
    }

    fn explain(config: &ServiceBilling, class: CredentialClass) -> ServiceBillingExplanation {
        project_billing(
            &connection(),
            Some(config),
            class,
            account(BillingAccountKind::Personal),
            true,
            true,
            true,
        )
    }

    #[test]
    fn platform_credential_and_personal_payer_are_independent() {
        let config = ServiceBilling {
            platform_key_pricing: Some(lane(BillingMetric::InputTokens, "0.000000125", "pk")),
            ..Default::default()
        };
        let result = explain(&config, CredentialClass::NyxidManagedMaster);
        assert_eq!(result.credential_label, "NyxID credential");
        assert_eq!(result.account.unwrap().kind, BillingAccountKind::Personal);
        assert_eq!(result.charge_status, ConnectionChargeStatus::UsageBased);
        assert_eq!(
            result.rates[0].credits_per_unit.as_deref(),
            Some("0.000000125")
        );
        assert_eq!(result.rates[0].metric, BillingMetric::InputTokens);
        assert_eq!(
            result.provider_billing,
            ProviderBillingDisclosure::NyxidCredential
        );
    }

    #[test]
    fn shared_oauth_and_credential_override_use_own_key_prices() {
        let config = ServiceBilling {
            byok_pricing: Some(lane(BillingMetric::Requests, "2", "byok")),
            platform_key_pricing: Some(lane(BillingMetric::Tokens, "3", "pk")),
            ..Default::default()
        };
        for class in [
            CredentialClass::UserOwned,
            CredentialClass::AgentOverrideUserOwned,
            CredentialClass::NyxidPlatformOauthApp,
            CredentialClass::NodeManaged,
        ] {
            let result = explain(&config, class);
            assert_eq!(result.rates[0].credits_per_unit.as_deref(), Some("2"));
            assert_eq!(
                result.provider_billing,
                ProviderBillingDisclosure::SeparateProviderAccount
            );
        }
    }

    #[test]
    fn missing_selected_lane_is_only_free_when_lane_configuration_confirms_it() {
        let config = ServiceBilling {
            platform_billable: true,
            platform_key_pricing: Some(lane(BillingMetric::Tokens, "3", "pk")),
            ..Default::default()
        };
        assert_eq!(
            explain(&config, CredentialClass::UserOwned).charge_status,
            ConnectionChargeStatus::NotCharged
        );
        let legacy = ServiceBilling {
            platform_billable: true,
            ..Default::default()
        };
        let result = explain(&legacy, CredentialClass::UserOwned);
        assert_eq!(result.charge_status, ConnectionChargeStatus::UsageBased);
        assert_eq!(result.status, BillingExplanationStatus::Conditional);
        assert_eq!(result.rates.len(), 1);
        assert_eq!(result.rates[0].credits_per_unit, None);
    }

    #[test]
    fn unsynced_primary_uses_legacy_rate_and_excludes_components() {
        let mut unsynced = lane(BillingMetric::InputTokens, "2", "byok");
        unsynced.sync_status = PricingSyncStatus::Pending;
        unsynced.components.push(LanePriceComponent {
            metric: BillingMetric::OutputTokens,
            credits_per_unit: "4".into(),
            lago_metric_code: "byok_output".into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
        });
        let config = ServiceBilling {
            platform_billable: true,
            platform_pricing: Some(ServicePlatformPricing {
                credits_per_unit: "7".into(),
                lago_metric_code: "legacy".into(),
                sync_status: PricingSyncStatus::Synced,
                sync_error: None,
            }),
            byok_pricing: Some(unsynced),
            ..Default::default()
        };
        let result = explain(&config, CredentialClass::UserOwned);
        assert_eq!(result.rates.len(), 1);
        assert_eq!(result.rates[0].credits_per_unit.as_deref(), Some("7"));
        assert_eq!(result.rates[0].source, "service_price");
        assert_eq!(result.rates[0].metric, BillingMetric::Requests);
    }

    #[test]
    fn only_synced_components_are_exposed_as_effective_rates() {
        let mut pricing = lane(BillingMetric::InputTokens, "1", "primary");
        for (metric, status) in [
            (BillingMetric::OutputTokens, PricingSyncStatus::Synced),
            (BillingMetric::CacheReadTokens, PricingSyncStatus::Pending),
            (BillingMetric::CacheWriteTokens, PricingSyncStatus::Failed),
        ] {
            pricing.components.push(LanePriceComponent {
                metric,
                credits_per_unit: "2".into(),
                lago_metric_code: metric.as_str().into(),
                sync_status: status,
                sync_error: None,
            });
        }
        let config = ServiceBilling {
            byok_pricing: Some(pricing),
            ..Default::default()
        };
        let result = explain(&config, CredentialClass::UserOwned);
        assert_eq!(result.rates.len(), 2);
        assert_eq!(result.rates[1].metric, BillingMetric::OutputTokens);
        assert_eq!(result.rates[1].credits_per_unit.as_deref(), Some("2"));
    }

    #[test]
    fn charge_restriction_overrides_a_synced_own_key_lane() {
        let config = ServiceBilling {
            platform_charge_nyxid_credentials_only: true,
            byok_pricing: Some(lane(BillingMetric::Requests, "1", "byok")),
            ..Default::default()
        };
        for class in [
            CredentialClass::UserOwned,
            CredentialClass::AgentOverrideUserOwned,
            CredentialClass::NodeManaged,
        ] {
            assert_eq!(
                explain(&config, class).charge_status,
                ConnectionChargeStatus::NotCharged
            );
        }
        assert_eq!(
            explain(&config, CredentialClass::NyxidPlatformOauthApp).charge_status,
            ConnectionChargeStatus::UsageBased
        );
    }

    #[test]
    fn rollout_or_global_disable_suppresses_both_charge_layers() {
        let config = ServiceBilling {
            platform_billable: true,
            resale_billable: true,
            lago_resale_metric_code: Some("resale".into()),
            ..Default::default()
        };
        let result = project_billing(
            &connection(),
            Some(&config),
            CredentialClass::NyxidManagedMaster,
            account(BillingAccountKind::Personal),
            false,
            true,
            true,
        );
        assert_eq!(result.charge_status, ConnectionChargeStatus::NotCharged);
        assert!(result.rates.is_empty());
    }

    #[test]
    fn resale_is_independent_of_the_platform_price_lane() {
        let config = ServiceBilling {
            resale_billable: true,
            lago_resale_metric_code: Some("resale".into()),
            ..Default::default()
        };
        let master = explain(&config, CredentialClass::NyxidManagedMaster);
        assert_eq!(master.charge_status, ConnectionChargeStatus::UsageBased);
        assert_eq!(master.rates[0].layer, "resale");
        assert_eq!(master.rates[0].credits_per_unit, None);
        assert_eq!(
            explain(&config, CredentialClass::UserOwned).charge_status,
            ConnectionChargeStatus::NotCharged
        );
    }

    #[test]
    fn billing_provider_missing_is_conditional_not_free() {
        let config = ServiceBilling {
            byok_pricing: Some(lane(BillingMetric::Requests, "1", "byok")),
            ..Default::default()
        };
        let result = project_billing(
            &connection(),
            Some(&config),
            CredentialClass::UserOwned,
            account(BillingAccountKind::Organization),
            true,
            true,
            false,
        );
        assert_eq!(
            result.account.unwrap().kind,
            BillingAccountKind::Organization
        );
        assert_eq!(result.charge_status, ConnectionChargeStatus::Conditional);
        assert_eq!(result.credential_label, "Organization credential");
        assert_eq!(result.rates[0].credits_per_unit.as_deref(), Some("1"));
    }

    #[test]
    fn no_auth_does_not_erase_legacy_opt_in_billing() {
        let legacy = ServiceBilling {
            platform_billable: true,
            ..Default::default()
        };
        assert_eq!(
            explain(&legacy, CredentialClass::NoAuth).charge_status,
            ConnectionChargeStatus::UsageBased
        );
        let lanes = ServiceBilling {
            platform_billable: true,
            byok_pricing: Some(lane(BillingMetric::Requests, "1", "byok")),
            ..Default::default()
        };
        assert_eq!(
            explain(&lanes, CredentialClass::NoAuth).charge_status,
            ConnectionChargeStatus::NotCharged
        );
    }

    #[test]
    fn restricted_and_unavailable_never_masquerade_as_free_or_publish_an_account() {
        for result in [restricted(), unavailable("Credential unavailable")] {
            assert!(result.account.is_none());
            assert!(result.credential_class.is_none());
            assert!(result.rates.is_empty());
            assert_ne!(result.charge_status, ConnectionChargeStatus::NotCharged);
        }
    }

    #[test]
    fn websocket_fallback_pricing_is_explicitly_conditional_but_fixed_lanes_are_not() {
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.capabilities = Some(crate::models::downstream_service::ServiceCapabilities {
            supports_websocket: true,
            ..Default::default()
        });
        let mut config = ServiceBilling {
            platform_billable: true,
            platform_pricing: Some(ServicePlatformPricing {
                credits_per_unit: "2".into(),
                lago_metric_code: "service".into(),
                sync_status: PricingSyncStatus::Synced,
                sync_error: None,
            }),
            ..Default::default()
        };
        catalog.billing = Some(config.clone());
        let mut fallback = explain(&config, CredentialClass::UserOwned);
        assert_eq!(fallback.status, BillingExplanationStatus::Resolved);
        annotate_transport_pricing(&mut fallback, Some(&catalog));
        assert_eq!(fallback.status, BillingExplanationStatus::Conditional);
        assert!(fallback.notes.iter().any(|note| note.contains("WebSocket")));
        assert_eq!(fallback.rates[0].metric, BillingMetric::Requests);

        config.platform_metric = Some(BillingMetric::Requests);
        catalog.billing = Some(config.clone());
        let mut fixed_metric = explain(&config, CredentialClass::UserOwned);
        annotate_transport_pricing(&mut fixed_metric, Some(&catalog));
        assert_eq!(fixed_metric.status, BillingExplanationStatus::Resolved);

        config.platform_metric = None;
        config.byok_pricing = Some(lane(BillingMetric::InputTokens, "1", "byok"));
        catalog.billing = Some(config.clone());
        let mut fixed_lane = explain(&config, CredentialClass::UserOwned);
        annotate_transport_pricing(&mut fixed_lane, Some(&catalog));
        assert_eq!(fixed_lane.status, BillingExplanationStatus::Resolved);
        assert!(
            !fixed_lane
                .notes
                .iter()
                .any(|note| note.contains("WebSocket"))
        );
    }

    fn stored_credential(id: &str, owner: &str) -> UserApiKey {
        bson::from_document(doc! {
            "_id": id, "user_id": owner, "label": "External credential",
            "credential_type": "api_key", "status": "active",
            // Intentionally not decryptable: this projection must only inspect metadata.
            "credential_encrypted": bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1, 2, 3],
            },
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .unwrap()
    }

    async fn seed_accounts(db: &Database, membership: OrgMembership) {
        db.collection::<User>(USERS)
            .insert_many([
                test_user("person", UserType::Person),
                test_user("org", UserType::Org),
            ])
            .await
            .unwrap();
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(membership)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn database_preview_resolves_default_personal_org_and_platform_payers_without_writes() {
        let db = connect_test_database("insights_billing_payers")
            .await
            .expect("database");
        seed_accounts(&db, test_membership("org", "person", OrgRole::Member, None)).await;
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.auth_method = "bearer".into();
        catalog.auth_key_name = "Authorization".into();
        catalog.service_category = "internal".into();
        catalog.credential_encrypted = vec![1, 2, 3];
        db.collection::<DownstreamService>(CATALOG)
            .insert_one(&catalog)
            .await
            .unwrap();
        db.collection::<UserApiKey>(CREDENTIALS)
            .insert_many([
                stored_credential("personal-key", "person"),
                stored_credential("org-key", "org"),
            ])
            .await
            .unwrap();
        let mut personal = test_user_service(
            "personal",
            "person",
            "personal",
            "endpoint",
            Some(&catalog.id),
            None,
        );
        personal.auth_method = "bearer".into();
        personal.api_key_id = Some("personal-key".into());
        let mut organization = test_user_service(
            "organization",
            "org",
            "org-service",
            "endpoint",
            Some(&catalog.id),
            None,
        );
        organization.auth_method = "bearer".into();
        organization.api_key_id = Some("org-key".into());
        let mut platform = organization.clone();
        platform.id = "platform".into();
        platform.api_key_id = None;
        platform.credential_binding = Some("platform".into());
        let billing = BillingService::new(db.clone(), Arc::new(test_app_config()));
        let result = explain_connections(
            &db,
            &billing,
            "person",
            "person",
            &[personal, organization, platform],
        )
        .await
        .unwrap();

        let personal = &result["personal"];
        assert_eq!(personal.credential_class, Some(CredentialClass::UserOwned));
        assert_eq!(personal.account.as_ref().unwrap().id, "person");
        assert_eq!(
            personal.account.as_ref().unwrap().kind,
            BillingAccountKind::Personal
        );
        let organization = &result["organization"];
        assert_eq!(
            organization.credential_class,
            Some(CredentialClass::UserOwned)
        );
        assert_eq!(organization.account.as_ref().unwrap().id, "org");
        assert_eq!(
            organization.account.as_ref().unwrap().kind,
            BillingAccountKind::Organization
        );
        let platform = &result["platform"];
        assert_eq!(
            platform.credential_class,
            Some(CredentialClass::NyxidManagedMaster)
        );
        assert_eq!(
            platform.account.as_ref().unwrap().id,
            "person",
            "org-visible platform credential bills the caller"
        );
        assert_eq!(
            platform.account.as_ref().unwrap().kind,
            BillingAccountKind::Personal
        );
        assert!(
            result
                .values()
                .all(|r| r.charge_status == ConnectionChargeStatus::NotCharged)
        );
        for collection in [
            crate::models::billing_wallet::COLLECTION_NAME,
            crate::models::usage_meter::COLLECTION_NAME,
        ] {
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
        let keys: Vec<UserApiKey> = db
            .collection::<UserApiKey>(CREDENTIALS)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(keys.iter().all(|key| key.last_used_at.is_none()));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn database_missing_or_invalid_credentials_cannot_publish_a_payer_or_free_state() {
        let db = connect_test_database("insights_billing_missing")
            .await
            .expect("database");
        db.collection::<User>(USERS)
            .insert_one(test_user("person", UserType::Person))
            .await
            .unwrap();
        let mut empty = stored_credential("empty", "person");
        empty.credential_encrypted = None;
        let mut revoked = stored_credential("revoked", "person");
        revoked.status = "revoked".into();
        db.collection::<UserApiKey>(CREDENTIALS)
            .insert_many([empty, revoked, stored_credential("foreign", "someone-else")])
            .await
            .unwrap();
        let services: Vec<_> = ["missing", "empty", "revoked", "foreign"]
            .into_iter()
            .map(|id| {
                let mut service = test_user_service(id, "person", id, "endpoint", None, None);
                service.auth_method = "bearer".into();
                service.api_key_id = Some(id.into());
                service
            })
            .collect();
        let billing = BillingService::new(db.clone(), Arc::new(test_app_config()));
        let result = explain_connections(&db, &billing, "person", "person", &services)
            .await
            .unwrap();
        assert_eq!(result.len(), 4);
        for explanation in result.values() {
            assert_eq!(explanation.status, BillingExplanationStatus::Unavailable);
            assert_eq!(
                explanation.charge_status,
                ConnectionChargeStatus::Unavailable
            );
            assert!(explanation.account.is_none());
            assert!(explanation.rates.is_empty());
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn database_scope_and_viewer_restrictions_hide_billing_accounts() {
        let db = connect_test_database("insights_billing_scope")
            .await
            .expect("database");
        seed_accounts(
            &db,
            test_membership(
                "org",
                "person",
                OrgRole::Viewer,
                Some(vec!["visible".into()]),
            ),
        )
        .await;
        let services = [
            test_user_service("visible", "org", "visible", "endpoint", None, None),
            test_user_service("hidden", "org", "hidden", "endpoint", None, None),
        ];
        let billing = BillingService::new(db.clone(), Arc::new(test_app_config()));
        let result = explain_connections(&db, &billing, "person", "person", &services)
            .await
            .unwrap();
        assert_eq!(
            result["visible"].status,
            BillingExplanationStatus::Unavailable
        );
        assert_eq!(
            result["hidden"].status,
            BillingExplanationStatus::Restricted
        );
        assert!(
            result
                .values()
                .all(|r| r.account.is_none() && r.rates.is_empty())
        );
        db.drop().await.unwrap();
    }

    fn stored_agent_key(owner: &str) -> ApiKey {
        bson::from_document(doc! {
            "_id": "agent", "user_id": owner, "name": "Build agent",
            "key_prefix": "nyxid_ag_test", "key_hash": "test-hash",
            "scopes": "proxy", "is_active": true, "created_at": bson::DateTime::now(),
            "allow_all_services": false, "allowed_service_ids": ["service"],
        })
        .unwrap()
    }

    #[tokio::test]
    async fn agent_preview_uses_selected_principal_and_final_override_price_lane() {
        let db = connect_test_database("insights_billing_agent")
            .await
            .expect("database");
        seed_accounts(&db, test_membership("org", "person", OrgRole::Admin, None)).await;
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.auth_method = "bearer".into();
        catalog.auth_key_name = "Authorization".into();
        catalog.service_category = "internal".into();
        catalog.credential_encrypted = vec![1, 2, 3];
        catalog.billing = Some(ServiceBilling {
            byok_pricing: Some(lane(BillingMetric::Requests, "2", "byok")),
            platform_key_pricing: Some(lane(BillingMetric::Requests, "5", "platform")),
            ..Default::default()
        });
        db.collection::<DownstreamService>(CATALOG)
            .insert_one(&catalog)
            .await
            .unwrap();
        let mut service = test_user_service(
            "service",
            "org",
            "platform",
            "endpoint",
            Some(&catalog.id),
            None,
        );
        service.auth_method = "bearer".into();
        service.credential_binding = Some("platform".into());
        db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
            .insert_one(&service)
            .await
            .unwrap();
        db.collection::<ApiKey>(AGENT_KEYS)
            .insert_one(stored_agent_key("org"))
            .await
            .unwrap();
        let services = [service];
        let mut config = test_app_config();
        config.billing_enabled = true;
        let billing = BillingService::new(db.clone(), Arc::new(config));
        let default = explain_connections(&db, &billing, "person", "person", &services)
            .await
            .unwrap();
        assert_eq!(default["service"].account.as_ref().unwrap().id, "person");
        assert_eq!(
            default["service"].rates[0].credits_per_unit.as_deref(),
            Some("5")
        );
        let selected = explain_for_agent_key(&db, &billing, "person", &services, "agent")
            .await
            .unwrap();
        assert_eq!(selected["service"].account.as_ref().unwrap().id, "org");
        assert_eq!(
            selected["service"].credential_class,
            Some(CredentialClass::NyxidManagedMaster)
        );
        assert_eq!(selected["service"].context, "agent_key");

        db.collection::<UserApiKey>(CREDENTIALS)
            .insert_one(stored_credential("override", "org"))
            .await
            .unwrap();
        let now = chrono::Utc::now();
        db.collection::<AgentServiceBinding>(BINDINGS)
            .insert_one(AgentServiceBinding {
                id: "binding".into(),
                api_key_id: "agent".into(),
                user_service_id: "service".into(),
                user_api_key_id: "override".into(),
                user_id: "org".into(),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        let selected = explain_for_agent_key(&db, &billing, "person", &services, "agent")
            .await
            .unwrap();
        let selected = &selected["service"];
        assert_eq!(selected.account.as_ref().unwrap().id, "org");
        assert_eq!(
            selected.account.as_ref().unwrap().kind,
            BillingAccountKind::Organization
        );
        assert_eq!(
            selected.credential_class,
            Some(CredentialClass::AgentOverrideUserOwned)
        );
        assert_eq!(selected.credential_label, "Credential override");
        assert_eq!(selected.rates[0].credits_per_unit.as_deref(), Some("2"));
        assert!(
            db.collection::<UserApiKey>(CREDENTIALS)
                .find_one(doc! { "_id": "override" })
                .await
                .unwrap()
                .unwrap()
                .last_used_at
                .is_none()
        );

        // A configured override never grants access by itself.
        db.collection::<ApiKey>(AGENT_KEYS)
            .update_one(
                doc! { "_id": "agent" },
                doc! { "$set": { "allowed_service_ids": [] } },
            )
            .await
            .unwrap();
        let denied = explain_for_agent_key(&db, &billing, "person", &services, "agent")
            .await
            .unwrap();
        assert_eq!(
            denied["service"].status,
            BillingExplanationStatus::Unavailable
        );
        assert!(denied["service"].account.is_none());

        // The same runtime auto-connected expansion admits an explicit platform binding.
        db.collection::<ApiKey>(AGENT_KEYS)
            .update_one(
                doc! { "_id": "agent" },
                doc! { "$set": { "allow_auto_connected_services": true } },
            )
            .await
            .unwrap();
        let permitted = explain_for_agent_key(&db, &billing, "person", &services, "agent")
            .await
            .unwrap();
        assert_eq!(
            permitted["service"].credential_class,
            Some(CredentialClass::AgentOverrideUserOwned)
        );

        // Invalid overrides fail closed instead of pretending the default key was selected.
        db.collection::<UserApiKey>(CREDENTIALS)
            .update_one(
                doc! { "_id": "override" },
                doc! { "$set": { "status": "revoked" } },
            )
            .await
            .unwrap();
        let invalid = explain_for_agent_key(&db, &billing, "person", &services, "agent")
            .await
            .unwrap();
        assert_eq!(
            invalid["service"].status,
            BillingExplanationStatus::Unavailable
        );
        assert!(invalid["service"].account.is_none());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn agent_preview_preserves_key_management_lifecycle_and_service_scope_acl() {
        let db = connect_test_database("insights_billing_agent_acl")
            .await
            .expect("database");
        seed_accounts(&db, test_membership("org", "person", OrgRole::Member, None)).await;
        db.collection::<ApiKey>(AGENT_KEYS)
            .insert_one(stored_agent_key("org"))
            .await
            .unwrap();
        let services = [test_user_service(
            "service", "org", "service", "endpoint", None, None,
        )];
        let billing = BillingService::new(db.clone(), Arc::new(test_app_config()));
        assert!(matches!(
            explain_for_agent_key(&db, &billing, "person", &services, "agent").await,
            Err(AppError::NotFound(_))
        ));
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .update_one(
                doc! { "org_user_id": "org", "member_user_id": "person" },
                doc! { "$set": { "role": "admin" } },
            )
            .await
            .unwrap();
        for changes in [
            doc! { "is_active": false },
            doc! { "is_active": true, "expires_at": bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::minutes(1)) },
            doc! { "expires_at": null, "purpose": "scheduled_invocation" },
            doc! { "purpose": "general", "scopes": "read" },
        ] {
            db.collection::<ApiKey>(AGENT_KEYS)
                .update_one(doc! { "_id": "agent" }, doc! { "$set": changes })
                .await
                .unwrap();
            let result = explain_for_agent_key(&db, &billing, "person", &services, "agent")
                .await
                .unwrap();
            assert_eq!(
                result["service"].status,
                BillingExplanationStatus::Unavailable
            );
            assert!(result["service"].account.is_none());
        }
        db.collection::<ApiKey>(AGENT_KEYS)
            .update_one(
                doc! { "_id": "agent" },
                doc! { "$set": { "scopes": "proxy", "allow_all_services": true } },
            )
            .await
            .unwrap();
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .update_one(
                doc! { "org_user_id": "org", "member_user_id": "person" },
                doc! { "$set": { "allowed_service_ids": [] } },
            )
            .await
            .unwrap();
        let result = explain_for_agent_key(&db, &billing, "person", &services, "agent")
            .await
            .unwrap();
        assert_eq!(
            result["service"].status,
            BillingExplanationStatus::Restricted
        );
        assert!(result["service"].account.is_none());
        db.drop().await.unwrap();
    }
}
