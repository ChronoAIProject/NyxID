import type { ComponentType } from "react";
import { AssistantAccountDeleteDialog } from "@/components/assistant/assistant-account-delete-dialog";
import { AssistantAccountMfaSetupDialog } from "@/components/assistant/assistant-account-mfa-setup-dialog";
import { AssistantAccountProfileUpdateDialog } from "@/components/assistant/assistant-account-profile-update-dialog";
import { AssistantAccountRevokeConsentDialog } from "@/components/assistant/assistant-account-revoke-consent-dialog";
import { AssistantApprovalConfigureDialog } from "@/components/assistant/assistant-approval-configure-dialog";
import { AssistantApprovalDisableDialog } from "@/components/assistant/assistant-approval-disable-dialog";
import { AssistantApprovalEnableDialog } from "@/components/assistant/assistant-approval-enable-dialog";
import { AssistantApprovalRevokeGrantDialog } from "@/components/assistant/assistant-approval-revoke-grant-dialog";
import { AssistantConnectionRevokeDialog } from "@/components/assistant/assistant-connection-revoke-dialog";
import { AssistantDeveloperAppActionDialog } from "@/components/assistant/assistant-developer-app-action-dialog";
import { AssistantDeviceOnboardDialog } from "@/components/assistant/assistant-device-onboard-dialog";
import { AssistantEndpointDeleteDialog } from "@/components/assistant/assistant-endpoint-delete-dialog";
import { AssistantEndpointUpdateDialog } from "@/components/assistant/assistant-endpoint-update-dialog";
import { AssistantExternalKeyDeleteDialog } from "@/components/assistant/assistant-external-key-delete-dialog";
import { AssistantExternalKeyRotateDialog } from "@/components/assistant/assistant-external-key-rotate-dialog";
import { AssistantKeyBindDialog } from "@/components/assistant/assistant-key-bind-dialog";
import { AssistantKeyCreateDialog } from "@/components/assistant/assistant-key-create-dialog";
import { AssistantKeyDeleteDialog } from "@/components/assistant/assistant-key-delete-dialog";
import { AssistantKeyRotateDialog } from "@/components/assistant/assistant-key-rotate-dialog";
import { AssistantKeyScopeDialog } from "@/components/assistant/assistant-key-scope-dialog";
import { AssistantKeyUpdateDialog } from "@/components/assistant/assistant-key-update-dialog";
import { AssistantNodeDeleteDialog } from "@/components/assistant/assistant-node-delete-dialog";
import { AssistantNodeInjectCredentialDialog } from "@/components/assistant/assistant-node-inject-credential-dialog";
import { AssistantNodeRegisterTokenDialog } from "@/components/assistant/assistant-node-register-token-dialog";
import { AssistantNodeRotateTokenDialog } from "@/components/assistant/assistant-node-rotate-token-dialog";
import { AssistantNodeTransferDialog } from "@/components/assistant/assistant-node-transfer-dialog";
import { AssistantNotificationsActionDialog } from "@/components/assistant/assistant-notifications-action-dialog";
import { AssistantOrgActionDialog } from "@/components/assistant/assistant-org-action-dialog";
import { AssistantOrgIntegrationActionDialog } from "@/components/assistant/assistant-org-integration-action-dialog";
import { AssistantPendingCredentialCancelDialog } from "@/components/assistant/assistant-pending-credential-cancel-dialog";
import { AssistantPendingCredentialPushDialog } from "@/components/assistant/assistant-pending-credential-push-dialog";
import { AssistantProviderDisconnectDialog } from "@/components/assistant/assistant-provider-disconnect-dialog";
import { AssistantProviderSetAppCredentialsDialog } from "@/components/assistant/assistant-provider-set-app-credentials-dialog";
import { AssistantServiceAccountActionDialog } from "@/components/assistant/assistant-service-account-action-dialog";
import { AssistantServiceAccessReviewDialog } from "@/components/assistant/assistant-service-access-review-dialog";
import { AssistantServiceDeleteDialog } from "@/components/assistant/assistant-service-delete-dialog";
import { AssistantServiceRotateCredentialDialog } from "@/components/assistant/assistant-service-rotate-credential-dialog";
import { AssistantServiceRouteDialog } from "@/components/assistant/assistant-service-route-dialog";
import { AssistantServiceUpdateDialog } from "@/components/assistant/assistant-service-update-dialog";
import type { ActionCardParams } from "@/schemas/assistant-actions";

export type DialogVariant = Exclude<
  ActionCardParams["variant"],
  "catalog" | "custom" | "service_reauthorize" | "unknown"
>;
type ParamsOf<V extends DialogVariant> = Extract<
  ActionCardParams,
  { variant: V }
>;
export type DialogParams = ParamsOf<DialogVariant>;

interface AssistantDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly onComplete: (completion: unknown) => void;
  readonly action?: unknown;
  readonly params?: unknown;
  // Dialog-specific fragments include discriminators such as `action`.
  readonly [key: string]: unknown;
}

export interface DialogBinding<V extends DialogVariant> {
  readonly Dialog: ComponentType<AssistantDialogProps>;
  readonly toProps: (params: ParamsOf<V>) => Readonly<Record<string, unknown>>;
}

export const ACTION_DIALOGS: {
  readonly [V in DialogVariant]: DialogBinding<V>;
} = {
  key_create: {
    Dialog: AssistantKeyCreateDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        name: params.name,
        platform: params.platform,
        allowedServiceIds: params.allowed_service_ids,
      },
    }),
  },
  key_rotate: {
    Dialog: AssistantKeyRotateDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { keyId: params.key_id } }),
  },
  key_update: {
    Dialog: AssistantKeyUpdateDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        keyId: params.key_id,
        name: params.name,
        platform: params.platform,
        description: params.description,
      },
    }),
  },
  key_delete: {
    Dialog: AssistantKeyDeleteDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { keyId: params.key_id } }),
  },
  key_extend_scope: {
    Dialog: AssistantKeyScopeDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        keyId: params.key_id,
        addServiceIds: params.add_service_ids,
      },
    }),
  },
  key_bind_credential: {
    Dialog: AssistantKeyBindDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        keyId: params.key_id,
        userServiceId: params.user_service_id,
        externalKeyId: params.external_key_id,
      },
    }),
  },
  service_update: {
    Dialog: AssistantServiceUpdateDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        userServiceId: params.user_service_id,
        name: params.name,
        endpointUrl: params.endpoint_url,
        authMethod: params.auth_method,
        authKeyName: params.auth_key_name,
      },
    }),
  },
  service_access_review: {
    Dialog:
      AssistantServiceAccessReviewDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        userServiceId: params.user_service_id,
        serviceSlug: params.service_slug,
        resourceUri: params.resource_uri,
      },
    }),
  },
  service_delete: {
    Dialog: AssistantServiceDeleteDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: { userServiceId: params.user_service_id },
    }),
  },
  service_route: {
    Dialog: AssistantServiceRouteDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        userServiceId: params.user_service_id,
        viaNodeId: params.via_node_id,
      },
    }),
  },
  service_rotate_credential: {
    Dialog:
      AssistantServiceRotateCredentialDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: { userServiceId: params.user_service_id },
    }),
  },
  endpoint_update: {
    Dialog:
      AssistantEndpointUpdateDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        endpointId: params.endpoint_id,
        label: params.label,
        endpointUrl: params.endpoint_url,
        openapiSpecUrl: params.openapi_spec_url,
      },
    }),
  },
  endpoint_delete: {
    Dialog:
      AssistantEndpointDeleteDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { endpointId: params.endpoint_id } }),
  },
  external_key_rotate: {
    Dialog:
      AssistantExternalKeyRotateDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: { externalKeyId: params.external_key_id },
    }),
  },
  external_key_delete: {
    Dialog:
      AssistantExternalKeyDeleteDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: { externalKeyId: params.external_key_id },
    }),
  },
  connection_revoke: {
    Dialog:
      AssistantConnectionRevokeDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { serviceId: params.service_id } }),
  },
  provider_disconnect: {
    Dialog:
      AssistantProviderDisconnectDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { providerId: params.provider_id } }),
  },
  provider_set_app_credentials: {
    Dialog:
      AssistantProviderSetAppCredentialsDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { providerId: params.provider_id } }),
  },
  node_register_token: {
    Dialog:
      AssistantNodeRegisterTokenDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: { name: params.name, targetOrgId: params.target_org_id },
    }),
  },
  node_rotate_token: {
    Dialog:
      AssistantNodeRotateTokenDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { nodeId: params.node_id } }),
  },
  node_delete: {
    Dialog: AssistantNodeDeleteDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { nodeId: params.node_id } }),
  },
  node_transfer: {
    Dialog: AssistantNodeTransferDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        nodeId: params.node_id,
        newOwnerUserId: params.new_owner_user_id,
      },
    }),
  },
  node_inject_credential: {
    Dialog:
      AssistantNodeInjectCredentialDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        nodeId: params.node_id,
        serviceSlug: params.service_slug,
        injectionMethod: params.injection_method,
        fieldName: params.field_name,
        targetUrl: params.target_url,
        label: params.label,
      },
    }),
  },
  pending_credential_push: {
    Dialog:
      AssistantPendingCredentialPushDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        nodeId: params.node_id,
        serviceSlug: params.service_slug,
        injectionMethod: params.injection_method,
        fieldName: params.field_name,
        targetUrl: params.target_url,
        label: params.label,
      },
    }),
  },
  pending_credential_cancel: {
    Dialog:
      AssistantPendingCredentialCancelDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        nodeId: params.node_id,
        pendingCredentialId: params.pending_credential_id,
      },
    }),
  },
  device_onboard: {
    Dialog: AssistantDeviceOnboardDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        label: params.label,
        targetOrgId: params.target_org_id,
        defaultServiceIds: params.default_service_ids,
      },
    }),
  },
  org_create: {
    Dialog: AssistantOrgActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "create",
      params: {
        displayName: params.display_name,
        contactEmail: params.contact_email,
        avatarUrl: params.avatar_url,
      },
    }),
  },
  org_update: {
    Dialog: AssistantOrgActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "update",
      params: {
        orgId: params.org_id,
        displayName: params.display_name,
        slug: params.slug,
        contactEmail: params.contact_email,
        avatarUrl: params.avatar_url,
      },
    }),
  },
  org_delete: {
    Dialog: AssistantOrgActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "delete",
      params: { orgId: params.org_id },
    }),
  },
  org_member_add: {
    Dialog: AssistantOrgActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "member_add",
      params: {
        orgId: params.org_id,
        userId: params.user_id,
        role: params.role,
        allowedServiceIds: params.allowed_service_ids,
      },
    }),
  },
  org_member_remove: {
    Dialog: AssistantOrgActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "member_remove",
      params: { orgId: params.org_id, memberId: params.member_id },
    }),
  },
  org_member_update_role: {
    Dialog: AssistantOrgActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "member_update_role",
      params: {
        orgId: params.org_id,
        memberId: params.member_id,
        role: params.role,
      },
    }),
  },
  org_invite: {
    Dialog: AssistantOrgActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "invite",
      params: {
        orgId: params.org_id,
        role: params.role,
        allowedServiceIds: params.allowed_service_ids,
      },
    }),
  },
  org_set_primary: {
    Dialog: AssistantOrgActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "set_primary",
      params: { orgId: params.org_id },
    }),
  },
  account_profile_update: {
    Dialog:
      AssistantAccountProfileUpdateDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      params: {
        displayName: params.display_name,
        avatarUrl: params.avatar_url,
      },
    }),
  },
  account_revoke_consent: {
    Dialog:
      AssistantAccountRevokeConsentDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { clientId: params.client_id } }),
  },
  account_delete: {
    Dialog: AssistantAccountDeleteDialog as ComponentType<AssistantDialogProps>,
    toProps: () => ({}),
  },
  account_mfa_setup: {
    Dialog:
      AssistantAccountMfaSetupDialog as ComponentType<AssistantDialogProps>,
    toProps: () => ({}),
  },
  approval_configure: {
    Dialog:
      AssistantApprovalConfigureDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { serviceId: params.service_id } }),
  },
  approval_enable: {
    Dialog:
      AssistantApprovalEnableDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { serviceId: params.service_id } }),
  },
  approval_disable: {
    Dialog:
      AssistantApprovalDisableDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { serviceId: params.service_id } }),
  },
  approval_revoke_grant: {
    Dialog:
      AssistantApprovalRevokeGrantDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({ params: { grantId: params.grant_id } }),
  },
  notifications_update: {
    Dialog:
      AssistantNotificationsActionDialog as ComponentType<AssistantDialogProps>,
    toProps: () => ({ action: "update", params: {} }),
  },
  notifications_telegram_link: {
    Dialog:
      AssistantNotificationsActionDialog as ComponentType<AssistantDialogProps>,
    toProps: () => ({ action: "telegram_link", params: {} }),
  },
  notifications_telegram_disconnect: {
    Dialog:
      AssistantNotificationsActionDialog as ComponentType<AssistantDialogProps>,
    toProps: () => ({ action: "telegram_disconnect", params: {} }),
  },
  service_account_create: {
    Dialog:
      AssistantServiceAccountActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "create",
      params: {
        name: params.name,
        description: params.description,
        targetOrgId: params.target_org_id,
      },
    }),
  },
  service_account_update: {
    Dialog:
      AssistantServiceAccountActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "update",
      params: {
        serviceAccountId: params.service_account_id,
        name: params.name,
        description: params.description,
      },
    }),
  },
  service_account_delete: {
    Dialog:
      AssistantServiceAccountActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "delete",
      params: { serviceAccountId: params.service_account_id },
    }),
  },
  service_account_rotate_secret: {
    Dialog:
      AssistantServiceAccountActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "rotate_secret",
      params: { serviceAccountId: params.service_account_id },
    }),
  },
  service_account_revoke_tokens: {
    Dialog:
      AssistantServiceAccountActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "revoke_tokens",
      params: { serviceAccountId: params.service_account_id },
    }),
  },
  developer_app_create: {
    Dialog:
      AssistantDeveloperAppActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "create",
      params: { name: params.name, redirectUris: params.redirect_uris },
    }),
  },
  developer_app_update: {
    Dialog:
      AssistantDeveloperAppActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "update",
      params: {
        clientId: params.client_id,
        name: params.name,
        redirectUris: params.redirect_uris,
      },
    }),
  },
  developer_app_delete: {
    Dialog:
      AssistantDeveloperAppActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "delete",
      params: { clientId: params.client_id },
    }),
  },
  developer_app_rotate_secret: {
    Dialog:
      AssistantDeveloperAppActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "rotate_secret",
      params: { clientId: params.client_id },
    }),
  },
  external_key_add_gcp_service_account: {
    Dialog:
      AssistantOrgIntegrationActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "external_key.add_gcp_service_account",
      params: { label: params.label, targetOrgId: params.target_org_id },
    }),
  },
  openclaw_connect: {
    Dialog:
      AssistantOrgIntegrationActionDialog as ComponentType<AssistantDialogProps>,
    toProps: (params) => ({
      action: "openclaw.connect",
      params: { gatewayUrl: params.gateway_url },
    }),
  },
};

export function isDialogVariant(
  variant: ActionCardParams["variant"],
): variant is DialogVariant {
  return Object.prototype.hasOwnProperty.call(ACTION_DIALOGS, variant);
}

export function isDialogParams(
  params: ActionCardParams,
): params is DialogParams {
  return isDialogVariant(params.variant);
}

export function dialogBindingFor<V extends DialogVariant>(
  variant: V,
): DialogBinding<V> {
  return ACTION_DIALOGS[variant];
}
