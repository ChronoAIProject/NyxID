import {
  ACTION_SCHEMA_VERSION,
  ACTION_SERVICE_SLUG_PATTERN,
  accountProfileUpdateActionParamsSchema,
  accountRevokeConsentActionParamsSchema,
  approvalRevokeGrantActionParamsSchema,
  approvalServiceActionParamsSchema,
  developerAppCreateActionParamsSchema,
  developerAppIdentityActionParamsSchema,
  developerAppUpdateActionParamsSchema,
  deviceOnboardActionParamsSchema,
  endpointDeleteActionParamsSchema,
  endpointUpdateActionParamsSchema,
  externalKeyAddGcpActionParamsSchema,
  externalKeyDeleteActionParamsSchema,
  externalKeyRotateActionParamsSchema,
  keyBindCredentialActionParamsSchema,
  keyCreateActionParamsSchema,
  keyDeleteActionParamsSchema,
  keyExtendScopeActionParamsSchema,
  keyRotateActionParamsSchema,
  keyUpdateActionParamsSchema,
  nodeCredentialActionParamsSchema,
  nodeDeleteActionParamsSchema,
  nodeRegisterTokenActionParamsSchema,
  nodeRotateTokenActionParamsSchema,
  nodeTransferActionParamsSchema,
  openClawConnectActionParamsSchema,
  orgCreateActionParamsSchema,
  orgIdentityActionParamsSchema,
  orgInviteActionParamsSchema,
  orgMemberAddActionParamsSchema,
  orgMemberIdentityActionParamsSchema,
  orgMemberUpdateRoleActionParamsSchema,
  orgUpdateActionParamsSchema,
  pendingCredentialCancelActionParamsSchema,
  serviceConnectActionParamsSchema,
  serviceAccountCreateActionParamsSchema,
  serviceAccountIdentityActionParamsSchema,
  serviceAccountUpdateActionParamsSchema,
  serviceDeleteActionParamsSchema,
  serviceReauthorizeActionParamsSchema,
  serviceRotateCredentialActionParamsSchema,
  serviceRouteActionParamsSchema,
  serviceUpdateActionParamsSchema,
  type ActionCardParams,
  type ActionResource,
  type AssistantActionRequest,
} from "@/schemas/assistant-actions";

export type ActionRisk = "credential_access" | "unsupported";
// The two connect aliases remain observable in existing transport consumers;
// new dialog journeys use their ActionCardParams variant directly.
export type ActionJourney =
  | ActionCardParams["variant"]
  | "catalog_service"
  | "custom_service"
  | null;
export type ActionIcon =
  | "service"
  | "globe"
  | "shield"
  | "key"
  | "org"
  | "bell"
  | "app"
  | "node";

export interface SummaryRow {
  readonly label: string;
  readonly value: string;
  readonly mono?: boolean;
}

export interface ActionDescriptor<
  P extends ActionCardParams = ActionCardParams,
> {
  readonly title: (params: P) => string;
  readonly body: (params: P) => string;
  readonly cta: (params: P) => string;
  readonly risk: ActionRisk;
  readonly normalize: (raw: unknown) => P | null;
  readonly summary: (params: P) => readonly SummaryRow[];
  readonly icon: ActionIcon;
  readonly busyLabel: "Working" | "Authorizing" | "Connecting";
  readonly assurance: string;
  readonly resource: (completion: unknown) => ActionResource;
  readonly wiring:
    | "dialog"
    | "legacy_connect"
    | "legacy_reauthorize"
    | "deferred";
  readonly journey: (params: ActionCardParams) => ActionJourney;
}

const SERVICE_LABELS: Readonly<Record<string, string>> = {
  github: "GitHub",
  "api-github": "GitHub",
  openai: "OpenAI",
  "llm-openai": "OpenAI",
  lark: "Lark",
  "api-lark": "Lark",
};

/**
 * A service label is the one model-supplied fragment NyxID's consent copy has
 * to interpolate, and the wire allows up to 4 KiB of free text there. Collapse
 * it to a single short line so an injected sentence ("… paste your password to
 * verify") can never masquerade as NyxID-authored consent copy.
 */
const MAX_SERVICE_LABEL_CHARS = 32;
const AUTH_KEY_NAME_PATTERN = /^[!#$%&'*+.^_`|~0-9A-Za-z-]{1,256}$/;

export function clampServiceLabel(value: string): string {
  const collapsed = value
    // eslint-disable-next-line no-control-regex -- C0/C1 must never reach the DOM
    .replace(/[\u0000-\u001f\u007f-\u009f]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (collapsed.length <= MAX_SERVICE_LABEL_CHARS) return collapsed;
  return `${collapsed.slice(0, MAX_SERVICE_LABEL_CHARS - 1).trimEnd()}…`;
}

function humanizeServiceSlug(slug: string): string {
  const normalized = slug.trim().toLowerCase();
  const known = SERVICE_LABELS[normalized];
  if (known) return known;
  const bare = normalized.replace(/^(api|llm)-/, "");
  const label = bare
    .split(/[-_]/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
  return clampServiceLabel(label) || "service";
}

export function actionServiceLabel(params: ActionCardParams): string {
  if (params.variant === "catalog") {
    return humanizeServiceSlug(params.service_slug);
  }
  if (params.variant === "custom")
    return clampServiceLabel(params.name) || "custom service";
  return "requested action";
}

function nullableId(value: string): string | null {
  const trimmed = value.trim();
  return trimmed || null;
}

function safeCatalogServiceSlug(value: string): string | null {
  const trimmed = value.trim();
  return ACTION_SERVICE_SLUG_PATTERN.test(trimmed) ? trimmed : null;
}

function safeEndpointUrl(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "https:") return null;
    if (!parsed.hostname) return null;
    if (parsed.username || parsed.password) return null;
    if (parsed.search || parsed.hash) return null;
    return parsed.toString();
  } catch {
    return null;
  }
}

function safeAuthKeyName(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return "";
  return AUTH_KEY_NAME_PATTERN.test(trimmed) ? trimmed : null;
}

function normalizeServiceConnect(raw: unknown): ActionCardParams | null {
  const connected = serviceConnectActionParamsSchema.safeParse(raw);
  if (!connected.success) return null;
  const catalog = connected.data.catalogService;
  const custom = connected.data.customService;
  if (catalog && !custom) {
    const serviceSlug = safeCatalogServiceSlug(catalog.serviceSlug);
    if (!serviceSlug) return null;
    return {
      variant: "catalog",
      service_slug: serviceSlug,
      requested_scopes: catalog.requestedScopes.map((scope) => scope.trim()),
      via_node_id: nullableId(catalog.viaNodeId),
      target_org_id: nullableId(catalog.targetOrgId),
    };
  }
  if (custom && !catalog) {
    const endpointUrl = safeEndpointUrl(custom.endpointUrl);
    const authKeyName = safeAuthKeyName(custom.authKeyName);
    if (!endpointUrl || authKeyName === null) return null;
    return {
      variant: "custom",
      name: custom.name.trim(),
      endpoint_url: endpointUrl,
      auth_method: custom.authMethod.trim(),
      auth_key_name: authKeyName,
      via_node_id: nullableId(custom.viaNodeId),
      target_org_id: nullableId(custom.targetOrgId),
    };
  }
  return null;
}

function normalizeServiceReauthorize(raw: unknown): ActionCardParams | null {
  const parsed = serviceReauthorizeActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "service_reauthorize",
    user_service_id: parsed.data.userServiceId,
    requested_scopes: parsed.data.requestedScopes,
  };
}

function normalizeKeyCreate(raw: unknown): ActionCardParams | null {
  const parsed = keyCreateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "key_create",
    name: parsed.data.name,
    platform: parsed.data.platform,
    allowed_service_ids: parsed.data.allowedServiceIds,
  };
}

function normalizeKeyRotate(raw: unknown): ActionCardParams | null {
  const parsed = keyRotateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "key_rotate",
    key_id: parsed.data.keyId,
  };
}

function normalizeKeyUpdate(raw: unknown): ActionCardParams | null {
  const parsed = keyUpdateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "key_update",
    key_id: parsed.data.keyId,
    name: parsed.data.name,
    platform: parsed.data.platform,
    description: parsed.data.description,
  };
}

function normalizeKeyDelete(raw: unknown): ActionCardParams | null {
  const parsed = keyDeleteActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "key_delete",
    key_id: parsed.data.keyId,
  };
}

function normalizeKeyExtendScope(raw: unknown): ActionCardParams | null {
  const parsed = keyExtendScopeActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "key_extend_scope",
    key_id: parsed.data.keyId,
    add_service_ids: parsed.data.addServiceIds,
  };
}

function normalizeKeyBindCredential(raw: unknown): ActionCardParams | null {
  const parsed = keyBindCredentialActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "key_bind_credential",
    key_id: parsed.data.keyId,
    user_service_id: parsed.data.userServiceId,
    external_key_id: parsed.data.externalKeyId,
  };
}

function normalizeServiceUpdate(raw: unknown): ActionCardParams | null {
  const parsed = serviceUpdateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "service_update",
    user_service_id: parsed.data.userServiceId,
    name: parsed.data.name,
    endpoint_url: parsed.data.endpointUrl,
    auth_method: parsed.data.authMethod,
    auth_key_name: parsed.data.authKeyName,
  };
}

function normalizeServiceDelete(raw: unknown): ActionCardParams | null {
  const parsed = serviceDeleteActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "service_delete",
    user_service_id: parsed.data.userServiceId,
  };
}

function normalizeServiceRoute(raw: unknown): ActionCardParams | null {
  const parsed = serviceRouteActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "service_route",
    user_service_id: parsed.data.userServiceId,
    via_node_id: parsed.data.viaNodeId,
  };
}

function normalizeServiceRotateCredential(
  raw: unknown,
): ActionCardParams | null {
  const parsed = serviceRotateCredentialActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "service_rotate_credential",
    user_service_id: parsed.data.userServiceId,
  };
}

function normalizeEndpointUpdate(raw: unknown): ActionCardParams | null {
  const parsed = endpointUpdateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "endpoint_update",
    endpoint_id: parsed.data.endpointId,
    label: parsed.data.label,
    endpoint_url: parsed.data.endpointUrl,
    openapi_spec_url: parsed.data.openapiSpecUrl,
  };
}

function normalizeEndpointDelete(raw: unknown): ActionCardParams | null {
  const parsed = endpointDeleteActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "endpoint_delete",
    endpoint_id: parsed.data.endpointId,
  };
}

function normalizeExternalKeyRotate(raw: unknown): ActionCardParams | null {
  const parsed = externalKeyRotateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "external_key_rotate",
    external_key_id: parsed.data.externalKeyId,
  };
}

function normalizeExternalKeyDelete(raw: unknown): ActionCardParams | null {
  const parsed = externalKeyDeleteActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "external_key_delete",
    external_key_id: parsed.data.externalKeyId,
  };
}

function normalizeNodeRegisterToken(raw: unknown): ActionCardParams | null {
  const parsed = nodeRegisterTokenActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "node_register_token",
    name: parsed.data.name,
    target_org_id: parsed.data.targetOrgId,
  };
}

function normalizeNodeRotateToken(raw: unknown): ActionCardParams | null {
  const parsed = nodeRotateTokenActionParamsSchema.safeParse(raw);
  return parsed.success
    ? { variant: "node_rotate_token", node_id: parsed.data.nodeId }
    : null;
}

function normalizeNodeDelete(raw: unknown): ActionCardParams | null {
  const parsed = nodeDeleteActionParamsSchema.safeParse(raw);
  return parsed.success
    ? { variant: "node_delete", node_id: parsed.data.nodeId }
    : null;
}

function normalizeNodeTransfer(raw: unknown): ActionCardParams | null {
  const parsed = nodeTransferActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "node_transfer",
    node_id: parsed.data.nodeId,
    new_owner_user_id: parsed.data.newOwnerUserId,
  };
}

function normalizedCredentialParams(
  raw: unknown,
  variant: "node_inject_credential" | "pending_credential_push",
): ActionCardParams | null {
  const parsed = nodeCredentialActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant,
    node_id: parsed.data.nodeId,
    service_slug: parsed.data.serviceSlug,
    injection_method: parsed.data.injectionMethod,
    field_name: parsed.data.fieldName,
    target_url: parsed.data.targetUrl,
    label: parsed.data.label,
  };
}

function normalizeNodeInjectCredential(raw: unknown): ActionCardParams | null {
  return normalizedCredentialParams(raw, "node_inject_credential");
}

function normalizePendingCredentialPush(raw: unknown): ActionCardParams | null {
  return normalizedCredentialParams(raw, "pending_credential_push");
}

function normalizePendingCredentialCancel(
  raw: unknown,
): ActionCardParams | null {
  const parsed = pendingCredentialCancelActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "pending_credential_cancel",
    node_id: parsed.data.nodeId,
    pending_credential_id: parsed.data.pendingCredentialId,
  };
}

function normalizeDeviceOnboard(raw: unknown): ActionCardParams | null {
  const parsed = deviceOnboardActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "device_onboard",
    label: parsed.data.label,
    target_org_id: parsed.data.targetOrgId,
    default_service_ids: parsed.data.defaultServiceIds,
  };
}

function normalizeOrgCreate(raw: unknown): ActionCardParams | null {
  const parsed = orgCreateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "org_create",
    display_name: parsed.data.displayName,
    contact_email: parsed.data.contactEmail,
    avatar_url: parsed.data.avatarUrl,
  };
}

function normalizeOrgUpdate(raw: unknown): ActionCardParams | null {
  const parsed = orgUpdateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "org_update",
    org_id: parsed.data.orgId,
    display_name: parsed.data.displayName,
    slug: parsed.data.slug,
    contact_email: parsed.data.contactEmail,
    avatar_url: parsed.data.avatarUrl,
  };
}

function normalizedOrgIdentity(
  raw: unknown,
  variant: "org_delete" | "org_set_primary",
): ActionCardParams | null {
  const parsed = orgIdentityActionParamsSchema.safeParse(raw);
  return parsed.success ? { variant, org_id: parsed.data.orgId } : null;
}

function normalizeOrgDelete(raw: unknown): ActionCardParams | null {
  return normalizedOrgIdentity(raw, "org_delete");
}

function normalizeOrgMemberAdd(raw: unknown): ActionCardParams | null {
  const parsed = orgMemberAddActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "org_member_add",
    org_id: parsed.data.orgId,
    user_id: parsed.data.userId,
    role: parsed.data.role,
    allowed_service_ids: parsed.data.allowedServiceIds,
  };
}

function normalizedOrgMemberIdentity(
  raw: unknown,
  variant: "org_member_remove",
): ActionCardParams | null {
  const parsed = orgMemberIdentityActionParamsSchema.safeParse(raw);
  return parsed.success
    ? {
        variant,
        org_id: parsed.data.orgId,
        member_id: parsed.data.memberId,
      }
    : null;
}

function normalizeOrgMemberRemove(raw: unknown): ActionCardParams | null {
  return normalizedOrgMemberIdentity(raw, "org_member_remove");
}

function normalizeOrgMemberUpdateRole(raw: unknown): ActionCardParams | null {
  const parsed = orgMemberUpdateRoleActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "org_member_update_role",
    org_id: parsed.data.orgId,
    member_id: parsed.data.memberId,
    role: parsed.data.role,
  };
}

function normalizeOrgInvite(raw: unknown): ActionCardParams | null {
  const parsed = orgInviteActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "org_invite",
    org_id: parsed.data.orgId,
    role: parsed.data.role,
    allowed_service_ids: parsed.data.allowedServiceIds,
  };
}

function normalizeOrgSetPrimary(raw: unknown): ActionCardParams | null {
  return normalizedOrgIdentity(raw, "org_set_primary");
}

function normalizeAccountProfileUpdate(raw: unknown): ActionCardParams | null {
  const parsed = accountProfileUpdateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "account_profile_update",
    display_name: parsed.data.displayName,
    avatar_url: parsed.data.avatarUrl,
  };
}

function normalizeAccountRevokeConsent(raw: unknown): ActionCardParams | null {
  const parsed = accountRevokeConsentActionParamsSchema.safeParse(raw);
  return parsed.success
    ? { variant: "account_revoke_consent", client_id: parsed.data.clientId }
    : null;
}

function normalizeEmptyVariant(
  raw: unknown,
  variant:
    | "account_delete"
    | "account_mfa_setup"
    | "notifications_update"
    | "notifications_telegram_link"
    | "notifications_telegram_disconnect",
): ActionCardParams | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  return Object.keys(raw).length === 0 ? { variant } : null;
}

function normalizeAccountDelete(raw: unknown): ActionCardParams | null {
  return normalizeEmptyVariant(raw, "account_delete");
}

function normalizeAccountMfaSetup(raw: unknown): ActionCardParams | null {
  return normalizeEmptyVariant(raw, "account_mfa_setup");
}

function normalizedApprovalService(
  raw: unknown,
  variant: "approval_configure" | "approval_enable" | "approval_disable",
): ActionCardParams | null {
  const parsed = approvalServiceActionParamsSchema.safeParse(raw);
  return parsed.success ? { variant, service_id: parsed.data.serviceId } : null;
}

function normalizeApprovalConfigure(raw: unknown): ActionCardParams | null {
  return normalizedApprovalService(raw, "approval_configure");
}

function normalizeApprovalEnable(raw: unknown): ActionCardParams | null {
  return normalizedApprovalService(raw, "approval_enable");
}

function normalizeApprovalDisable(raw: unknown): ActionCardParams | null {
  return normalizedApprovalService(raw, "approval_disable");
}

function normalizeApprovalRevokeGrant(raw: unknown): ActionCardParams | null {
  const parsed = approvalRevokeGrantActionParamsSchema.safeParse(raw);
  return parsed.success
    ? { variant: "approval_revoke_grant", grant_id: parsed.data.grantId }
    : null;
}

function normalizeNotificationsUpdate(raw: unknown): ActionCardParams | null {
  return normalizeEmptyVariant(raw, "notifications_update");
}

function normalizeNotificationsTelegramLink(
  raw: unknown,
): ActionCardParams | null {
  return normalizeEmptyVariant(raw, "notifications_telegram_link");
}

function normalizeNotificationsTelegramDisconnect(
  raw: unknown,
): ActionCardParams | null {
  return normalizeEmptyVariant(raw, "notifications_telegram_disconnect");
}

function normalizeServiceAccountCreate(raw: unknown): ActionCardParams | null {
  const parsed = serviceAccountCreateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "service_account_create",
    name: parsed.data.name,
    description: parsed.data.description,
  };
}

function normalizeServiceAccountUpdate(raw: unknown): ActionCardParams | null {
  const parsed = serviceAccountUpdateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "service_account_update",
    service_account_id: parsed.data.serviceAccountId,
    name: parsed.data.name,
    description: parsed.data.description,
  };
}

function normalizedServiceAccountIdentity(
  raw: unknown,
  variant:
    | "service_account_delete"
    | "service_account_rotate_secret"
    | "service_account_revoke_tokens",
): ActionCardParams | null {
  const parsed = serviceAccountIdentityActionParamsSchema.safeParse(raw);
  return parsed.success
    ? { variant, service_account_id: parsed.data.serviceAccountId }
    : null;
}

function normalizeServiceAccountDelete(raw: unknown): ActionCardParams | null {
  return normalizedServiceAccountIdentity(raw, "service_account_delete");
}

function normalizeServiceAccountRotateSecret(
  raw: unknown,
): ActionCardParams | null {
  return normalizedServiceAccountIdentity(raw, "service_account_rotate_secret");
}

function normalizeServiceAccountRevokeTokens(
  raw: unknown,
): ActionCardParams | null {
  return normalizedServiceAccountIdentity(raw, "service_account_revoke_tokens");
}

function normalizeDeveloperAppCreate(raw: unknown): ActionCardParams | null {
  const parsed = developerAppCreateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "developer_app_create",
    name: parsed.data.name,
    redirect_uris: parsed.data.redirectUris,
  };
}

function normalizeDeveloperAppUpdate(raw: unknown): ActionCardParams | null {
  const parsed = developerAppUpdateActionParamsSchema.safeParse(raw);
  if (!parsed.success) return null;
  return {
    variant: "developer_app_update",
    client_id: parsed.data.clientId,
    name: parsed.data.name,
    redirect_uris: parsed.data.redirectUris,
  };
}

function normalizedDeveloperAppIdentity(
  raw: unknown,
  variant: "developer_app_delete" | "developer_app_rotate_secret",
): ActionCardParams | null {
  const parsed = developerAppIdentityActionParamsSchema.safeParse(raw);
  return parsed.success ? { variant, client_id: parsed.data.clientId } : null;
}

function normalizeDeveloperAppDelete(raw: unknown): ActionCardParams | null {
  return normalizedDeveloperAppIdentity(raw, "developer_app_delete");
}

function normalizeDeveloperAppRotateSecret(
  raw: unknown,
): ActionCardParams | null {
  return normalizedDeveloperAppIdentity(raw, "developer_app_rotate_secret");
}

function normalizeExternalKeyAddGcp(raw: unknown): ActionCardParams | null {
  const parsed = externalKeyAddGcpActionParamsSchema.safeParse(raw);
  return parsed.success
    ? {
        variant: "external_key_add_gcp_service_account",
        label: parsed.data.label,
      }
    : null;
}

function normalizeOpenClawConnect(raw: unknown): ActionCardParams | null {
  const parsed = openClawConnectActionParamsSchema.safeParse(raw);
  return parsed.success
    ? { variant: "openclaw_connect", gateway_url: parsed.data.gatewayUrl }
    : null;
}

function completedId(completion: unknown): string {
  if (typeof completion !== "string" || !completion) {
    throw new Error("The action dialog returned an invalid resource identity.");
  }
  return completion;
}

function keyBindingResource(completion: unknown): ActionResource {
  if (!completion || typeof completion !== "object") {
    throw new Error("The key binding dialog returned an invalid resource.");
  }
  const resource = completion as Record<string, unknown>;
  if (
    typeof resource["keyId"] !== "string" ||
    !resource["keyId"] ||
    typeof resource["userServiceId"] !== "string" ||
    !resource["userServiceId"]
  ) {
    throw new Error("The key binding dialog returned an invalid resource.");
  }
  return {
    key: {
      keyId: resource["keyId"],
      userServiceId: resource["userServiceId"],
    },
  };
}

function endpointHost(endpointUrl: string): string {
  try {
    return new URL(endpointUrl).host;
  } catch {
    return "";
  }
}

function connectSummary(params: ActionCardParams): readonly SummaryRow[] {
  if (params.variant === "catalog") {
    return [
      {
        label: "Service",
        value: clampServiceLabel(params.service_slug) || "Custom",
      },
      ...params.requested_scopes
        .filter(Boolean)
        .map((scope) => ({ label: "Scopes", value: scope, mono: true })),
      ...(params.via_node_id
        ? [{ label: "", value: `Node ${params.via_node_id}`, mono: true }]
        : []),
      ...(params.target_org_id
        ? [{ label: "", value: `Org ${params.target_org_id}`, mono: true }]
        : []),
    ];
  }
  if (params.variant === "custom") {
    const host = endpointHost(params.endpoint_url);
    return [
      {
        label: "Service",
        value: clampServiceLabel(params.name) || "Custom",
      },
      ...(host ? [{ label: "Service", value: host }] : []),
      ...(params.via_node_id
        ? [{ label: "", value: `Node ${params.via_node_id}`, mono: true }]
        : []),
      ...(params.target_org_id
        ? [{ label: "", value: `Org ${params.target_org_id}`, mono: true }]
        : []),
    ];
  }
  return [];
}

const serviceConnectDescriptor: ActionDescriptor = {
  title: (params) => `Connect ${actionServiceLabel(params)}`,
  body: (params) =>
    `NyxID will broker access to ${actionServiceLabel(params)} for this assistant request. Your credential stays in NyxID and is never shared with the model.`,
  cta: (params) => `Connect ${actionServiceLabel(params)}`,
  risk: "credential_access",
  normalize: normalizeServiceConnect,
  summary: connectSummary,
  icon: "service",
  busyLabel: "Connecting",
  assurance:
    "You choose the account, routing, and credential. The assistant receives only brokered access after you finish.",
  resource: (completion) => ({
    userService: { userServiceId: completedId(completion) },
  }),
  wiring: "legacy_connect",
  journey: (params) => {
    if (params.variant === "catalog") return "catalog_service";
    if (params.variant === "custom") return "custom_service";
    return null;
  },
};

const serviceReauthorizeDescriptor: ActionDescriptor = {
  title: () => "Re-authorize service",
  body: (params) =>
    params.variant === "service_reauthorize"
      ? "NyxID will re-authorize this connected service with the requested permissions. Your credential stays in NyxID and is never shared with the model."
      : "NyxID will re-authorize one exact connected service.",
  cta: () => "Re-authorize",
  risk: "credential_access",
  normalize: normalizeServiceReauthorize,
  summary: (params) =>
    params.variant === "service_reauthorize"
      ? [
          {
            label: "Service",
            value: params.user_service_id,
            mono: true,
          },
          ...params.requested_scopes.map((scope) => ({
            label: "Requested scopes",
            value: scope,
            mono: true,
          })),
        ]
      : [],
  icon: "shield",
  busyLabel: "Authorizing",
  assurance:
    "NyxID opens the provider authorization flow here. The assistant receives only the service reference after fresh authorization finishes.",
  resource: (completion) => ({
    userService: { userServiceId: completedId(completion) },
  }),
  wiring: "legacy_reauthorize",
  journey: (params) =>
    params.variant === "service_reauthorize" ? "service_reauthorize" : null,
};

const keyCreateDescriptor: ActionDescriptor = {
  title: () => "Create API key",
  body: (params) =>
    params.variant === "key_create"
      ? `NyxID will create ${clampServiceLabel(params.name) || "an API key"} with proxy access limited to the listed services.`
      : "NyxID will create a least-scope API key.",
  cta: () => "Create key",
  risk: "credential_access",
  normalize: normalizeKeyCreate,
  summary: (params) =>
    params.variant === "key_create"
      ? [
          { label: "Key", value: params.name },
          { label: "Key", value: params.platform },
          ...params.allowed_service_ids.map((serviceId) => ({
            label: "Allowed services",
            value: serviceId,
            mono: true,
          })),
        ]
      : [],
  icon: "key",
  busyLabel: "Working",
  assurance:
    "NyxID creates and verifies the key here. The assistant receives only the safe key reference after you finish.",
  resource: (completion) => ({ key: { keyId: completedId(completion) } }),
  wiring: "dialog",
  journey: (params) => (params.variant === "key_create" ? "key_create" : null),
};

const keyRotateDescriptor: ActionDescriptor = {
  title: () => "Rotate API key",
  body: (params) =>
    params.variant === "key_rotate"
      ? "NyxID will replace this exact API key, preserve its authority, and commit an immutable predecessor link."
      : "NyxID will rotate one exact API key.",
  cta: () => "Rotate key",
  risk: "credential_access",
  normalize: normalizeKeyRotate,
  summary: (params) =>
    params.variant === "key_rotate"
      ? [{ label: "Predecessor", value: params.key_id, mono: true }]
      : [],
  icon: "key",
  busyLabel: "Working",
  assurance:
    "NyxID rotates and verifies the exact lineage here. The assistant receives only the replacement key reference after you finish.",
  resource: (completion) => ({ key: { keyId: completedId(completion) } }),
  wiring: "dialog",
  journey: (params) => (params.variant === "key_rotate" ? "key_rotate" : null),
};

const keyUpdateDescriptor: ActionDescriptor = {
  title: () => "Update API key",
  body: () =>
    "NyxID will update the display metadata for this API key without changing its access.",
  cta: () => "Update key",
  risk: "credential_access",
  normalize: normalizeKeyUpdate,
  summary: (params) =>
    params.variant === "key_update"
      ? [
          { label: "Key", value: params.key_id, mono: true },
          ...(params.name ? [{ label: "Name", value: params.name }] : []),
          ...(params.platform
            ? [{ label: "Platform", value: params.platform }]
            : []),
          ...(params.description
            ? [{ label: "Description", value: params.description }]
            : []),
        ]
      : [],
  icon: "key",
  busyLabel: "Working",
  assurance:
    "NyxID verifies the exact key and applies only these metadata changes. The assistant receives only the safe key reference.",
  resource: (completion) => ({ key: { keyId: completedId(completion) } }),
  wiring: "dialog",
  journey: (params) => (params.variant === "key_update" ? "key_update" : null),
};

const keyDeleteDescriptor: ActionDescriptor = {
  title: () => "Delete API key",
  body: () =>
    "NyxID will permanently delete this API key after you confirm the destructive change.",
  cta: () => "Delete key",
  risk: "credential_access",
  normalize: normalizeKeyDelete,
  summary: (params) =>
    params.variant === "key_delete"
      ? [{ label: "Key", value: params.key_id, mono: true }]
      : [],
  icon: "key",
  busyLabel: "Working",
  assurance:
    "NyxID confirms and verifies this deletion here. The assistant receives only the deleted key reference.",
  resource: (completion) => ({ key: { keyId: completedId(completion) } }),
  wiring: "dialog",
  journey: (params) => (params.variant === "key_delete" ? "key_delete" : null),
};

const keyExtendScopeDescriptor: ActionDescriptor = {
  title: () => "Extend API key scope",
  body: () =>
    "NyxID will allow this API key to access the additional listed services after you confirm the wider authority.",
  cta: () => "Extend scope",
  risk: "credential_access",
  normalize: normalizeKeyExtendScope,
  summary: (params) =>
    params.variant === "key_extend_scope"
      ? [
          { label: "Key", value: params.key_id, mono: true },
          ...params.add_service_ids.map((serviceId) => ({
            label: "Add service",
            value: serviceId,
            mono: true,
          })),
        ]
      : [],
  icon: "key",
  busyLabel: "Working",
  assurance:
    "NyxID widens only the listed service scope after your confirmation. The assistant receives only the safe key reference.",
  resource: (completion) => ({ key: { keyId: completedId(completion) } }),
  wiring: "dialog",
  journey: (params) =>
    params.variant === "key_extend_scope" ? "key_extend_scope" : null,
};

const keyBindCredentialDescriptor: ActionDescriptor = {
  title: () => "Bind service credential",
  body: () =>
    "NyxID will bind this API key to the selected stored credential for one exact service.",
  cta: () => "Bind credential",
  risk: "credential_access",
  normalize: normalizeKeyBindCredential,
  summary: (params) =>
    params.variant === "key_bind_credential"
      ? [
          { label: "Key", value: params.key_id, mono: true },
          { label: "Service", value: params.user_service_id, mono: true },
          {
            label: "External key",
            value: params.external_key_id,
            mono: true,
          },
        ]
      : [],
  icon: "key",
  busyLabel: "Working",
  assurance:
    "NyxID confirms and verifies the exact binding here. The assistant receives only safe key and service references.",
  resource: keyBindingResource,
  wiring: "dialog",
  journey: (params) =>
    params.variant === "key_bind_credential" ? "key_bind_credential" : null,
};

const serviceUpdateDescriptor: ActionDescriptor = {
  title: () => "Update connected service",
  body: () =>
    "NyxID will update this connected service's display and routing metadata without exposing its credential.",
  cta: () => "Update service",
  risk: "credential_access",
  normalize: normalizeServiceUpdate,
  summary: (params) =>
    params.variant === "service_update"
      ? [
          { label: "Service", value: params.user_service_id, mono: true },
          ...(params.name ? [{ label: "Name", value: params.name }] : []),
          ...(params.endpoint_url
            ? [{ label: "Endpoint", value: params.endpoint_url, mono: true }]
            : []),
          ...(params.auth_method
            ? [{ label: "Auth method", value: params.auth_method }]
            : []),
          ...(params.auth_key_name
            ? [{ label: "Auth key name", value: params.auth_key_name }]
            : []),
        ]
      : [],
  icon: "service",
  busyLabel: "Working",
  assurance:
    "NyxID verifies the exact service and applies only these configuration changes. The assistant receives only the safe service reference.",
  resource: (completion) => ({
    userService: { userServiceId: completedId(completion) },
  }),
  wiring: "dialog",
  journey: (params) =>
    params.variant === "service_update" ? "service_update" : null,
};

const serviceDeleteDescriptor: ActionDescriptor = {
  title: () => "Delete connected service",
  body: () =>
    "NyxID will permanently disconnect and delete this service after you confirm the destructive change.",
  cta: () => "Delete service",
  risk: "credential_access",
  normalize: normalizeServiceDelete,
  summary: (params) =>
    params.variant === "service_delete"
      ? [{ label: "Service", value: params.user_service_id, mono: true }]
      : [],
  icon: "service",
  busyLabel: "Working",
  assurance:
    "NyxID confirms and verifies this deletion here. The assistant receives only the deleted service reference.",
  resource: (completion) => ({
    userService: { userServiceId: completedId(completion) },
  }),
  wiring: "dialog",
  journey: (params) =>
    params.variant === "service_delete" ? "service_delete" : null,
};

const serviceRouteDescriptor: ActionDescriptor = {
  title: () => "Change service routing",
  body: (params) =>
    params.variant === "service_route" && params.via_node_id
      ? "NyxID will route this connected service through the selected credential node."
      : "NyxID will clear node routing and connect to this service directly.",
  cta: () => "Change routing",
  risk: "credential_access",
  normalize: normalizeServiceRoute,
  summary: (params) =>
    params.variant === "service_route"
      ? [
          { label: "Service", value: params.user_service_id, mono: true },
          {
            label: "Route",
            value: params.via_node_id ?? "Direct",
            mono: Boolean(params.via_node_id),
          },
        ]
      : [],
  icon: "node",
  busyLabel: "Working",
  assurance:
    "NyxID verifies the exact service and routing target here. The assistant receives only the safe service reference.",
  resource: (completion) => ({
    userService: { userServiceId: completedId(completion) },
  }),
  wiring: "dialog",
  journey: (params) =>
    params.variant === "service_route" ? "service_route" : null,
};

const serviceRotateCredentialDescriptor: ActionDescriptor = {
  title: () => "Rotate service credential",
  body: () =>
    "NyxID will replace the stored credential for this connected service inside the browser journey.",
  cta: () => "Rotate credential",
  risk: "credential_access",
  normalize: normalizeServiceRotateCredential,
  summary: (params) =>
    params.variant === "service_rotate_credential"
      ? [{ label: "Service", value: params.user_service_id, mono: true }]
      : [],
  icon: "shield",
  busyLabel: "Working",
  assurance:
    "You enter the replacement only inside NyxID. The assistant receives only the verified service reference.",
  resource: (completion) => ({
    userService: { userServiceId: completedId(completion) },
  }),
  wiring: "dialog",
  journey: (params) =>
    params.variant === "service_rotate_credential"
      ? "service_rotate_credential"
      : null,
};

const endpointUpdateDescriptor: ActionDescriptor = {
  title: () => "Update endpoint",
  body: () =>
    "NyxID will update this endpoint's label, target URL, or OpenAPI specification URL.",
  cta: () => "Update endpoint",
  risk: "credential_access",
  normalize: normalizeEndpointUpdate,
  summary: (params) =>
    params.variant === "endpoint_update"
      ? [
          { label: "Endpoint", value: params.endpoint_id, mono: true },
          ...(params.label ? [{ label: "Label", value: params.label }] : []),
          ...(params.endpoint_url
            ? [{ label: "Target URL", value: params.endpoint_url, mono: true }]
            : []),
          ...(params.openapi_spec_url
            ? [
                {
                  label: "OpenAPI spec",
                  value: params.openapi_spec_url,
                  mono: true,
                },
              ]
            : []),
        ]
      : [],
  icon: "globe",
  busyLabel: "Working",
  assurance:
    "NyxID verifies the exact endpoint and applies only these changes. The assistant receives only the safe endpoint reference.",
  resource: (completion) => ({
    endpoint: { endpointId: completedId(completion) },
  }),
  wiring: "dialog",
  journey: (params) =>
    params.variant === "endpoint_update" ? "endpoint_update" : null,
};

const endpointDeleteDescriptor: ActionDescriptor = {
  title: () => "Delete endpoint",
  body: () =>
    "NyxID will permanently delete this endpoint after you confirm the destructive change.",
  cta: () => "Delete endpoint",
  risk: "credential_access",
  normalize: normalizeEndpointDelete,
  summary: (params) =>
    params.variant === "endpoint_delete"
      ? [{ label: "Endpoint", value: params.endpoint_id, mono: true }]
      : [],
  icon: "globe",
  busyLabel: "Working",
  assurance:
    "NyxID confirms and verifies this deletion here. The assistant receives only the deleted endpoint reference.",
  resource: (completion) => ({
    endpoint: { endpointId: completedId(completion) },
  }),
  wiring: "dialog",
  journey: (params) =>
    params.variant === "endpoint_delete" ? "endpoint_delete" : null,
};

const externalKeyRotateDescriptor: ActionDescriptor = {
  title: () => "Rotate external credential",
  body: () =>
    "NyxID will replace this stored external credential inside the browser journey.",
  cta: () => "Rotate external credential",
  risk: "credential_access",
  normalize: normalizeExternalKeyRotate,
  summary: (params) =>
    params.variant === "external_key_rotate"
      ? [{ label: "External key", value: params.external_key_id, mono: true }]
      : [],
  icon: "key",
  busyLabel: "Working",
  assurance:
    "You enter the replacement only inside NyxID. The assistant receives only the verified external-key reference.",
  resource: (completion) => ({
    externalKey: { externalKeyId: completedId(completion) },
  }),
  wiring: "dialog",
  journey: (params) =>
    params.variant === "external_key_rotate" ? "external_key_rotate" : null,
};

const externalKeyDeleteDescriptor: ActionDescriptor = {
  title: () => "Delete external credential",
  body: () =>
    "NyxID will permanently delete this stored external credential after you confirm the destructive change.",
  cta: () => "Delete external credential",
  risk: "credential_access",
  normalize: normalizeExternalKeyDelete,
  summary: (params) =>
    params.variant === "external_key_delete"
      ? [{ label: "External key", value: params.external_key_id, mono: true }]
      : [],
  icon: "key",
  busyLabel: "Working",
  assurance:
    "NyxID confirms and verifies this deletion here. The assistant receives only the deleted external-key reference.",
  resource: (completion) => ({
    externalKey: { externalKeyId: completedId(completion) },
  }),
  wiring: "dialog",
  journey: (params) =>
    params.variant === "external_key_delete" ? "external_key_delete" : null,
};

type DialogResourceKind =
  | "userService"
  | "node"
  | "pendingCredential"
  | "device"
  | "org"
  | "account"
  | "approvalConfig"
  | "grant"
  | "notificationBinding"
  | "serviceAccount"
  | "developerApp"
  | "externalKey";

interface DialogSummaryField {
  readonly label: string;
  readonly key: string;
  readonly mono?: boolean;
}

interface DialogDescriptorConfig {
  readonly variant: Exclude<ActionCardParams["variant"], "unknown">;
  readonly title: string;
  readonly body: string;
  readonly cta: string;
  readonly icon: ActionIcon;
  readonly normalize: (raw: unknown) => ActionCardParams | null;
  readonly resourceKind: DialogResourceKind;
  readonly fields?: readonly DialogSummaryField[];
  readonly assurance?: string;
}

function dialogResource(
  kind: DialogResourceKind,
  completion: unknown,
): ActionResource {
  const id = completedId(completion);
  switch (kind) {
    case "userService":
      return { userService: { userServiceId: id } };
    case "node":
      return { node: { nodeId: id } };
    case "pendingCredential":
      return { pendingCredential: { pendingCredentialId: id } };
    case "device":
      return { device: { deviceId: id } };
    case "org":
      return { org: { orgId: id } };
    case "account":
      return { account: { userId: id } };
    case "approvalConfig":
      return { approvalConfig: { serviceId: id } };
    case "grant":
      return { grant: { grantId: id } };
    case "notificationBinding":
      return { notificationBinding: { bindingId: id } };
    case "serviceAccount":
      return { serviceAccount: { serviceAccountId: id } };
    case "developerApp":
      return { developerApp: { clientId: id } };
    case "externalKey":
      return { externalKey: { externalKeyId: id } };
  }
}

function dialogSummary(
  params: ActionCardParams,
  config: DialogDescriptorConfig,
): readonly SummaryRow[] {
  if (params.variant !== config.variant) return [];
  const values = params as unknown as Readonly<Record<string, unknown>>;
  return (config.fields ?? []).flatMap((field) => {
    const value = values[field.key];
    if (typeof value === "string" && value) {
      return [{ label: field.label, value, mono: field.mono }];
    }
    if (Array.isArray(value)) {
      return value
        .filter(
          (entry): entry is string =>
            typeof entry === "string" && Boolean(entry),
        )
        .map((entry) => ({
          label: field.label,
          value: entry,
          mono: field.mono,
        }));
    }
    return [];
  });
}

function createDialogDescriptor(
  config: DialogDescriptorConfig,
): ActionDescriptor {
  return {
    title: () => config.title,
    body: () => config.body,
    cta: () => config.cta,
    risk: "credential_access",
    normalize: config.normalize,
    summary: (params) => dialogSummary(params, config),
    icon: config.icon,
    busyLabel: "Working",
    assurance:
      config.assurance ??
      "NyxID performs and verifies this exact change in your signed-in browser. The assistant receives only the safe resource reference.",
    resource: (completion) => dialogResource(config.resourceKind, completion),
    wiring: "dialog",
    journey: (params) =>
      params.variant === config.variant ? config.variant : null,
  };
}

const nodeRegisterTokenDescriptor = createDialogDescriptor({
  variant: "node_register_token",
  title: "Create node registration token",
  body: "NyxID will create one registration token and show it only in this browser dialog.",
  cta: "Create registration token",
  icon: "node",
  normalize: normalizeNodeRegisterToken,
  resourceKind: "node",
  fields: [
    { label: "Node name", key: "name" },
    { label: "Organization", key: "target_org_id", mono: true },
  ],
});

const nodeRotateTokenDescriptor = createDialogDescriptor({
  variant: "node_rotate_token",
  title: "Rotate node credentials",
  body: "NyxID will invalidate this node's current credentials and show the replacements once.",
  cta: "Rotate node token",
  icon: "node",
  normalize: normalizeNodeRotateToken,
  resourceKind: "node",
  fields: [{ label: "Node", key: "node_id", mono: true }],
});

const nodeDeleteDescriptor = createDialogDescriptor({
  variant: "node_delete",
  title: "Delete credential node",
  body: "NyxID will deactivate this node and its bindings after explicit destructive confirmation.",
  cta: "Delete node",
  icon: "node",
  normalize: normalizeNodeDelete,
  resourceKind: "node",
  fields: [{ label: "Node", key: "node_id", mono: true }],
});

const nodeTransferDescriptor = createDialogDescriptor({
  variant: "node_transfer",
  title: "Transfer credential node",
  body: "NyxID will move this node and its authority to a different owner after explicit confirmation.",
  cta: "Transfer node",
  icon: "node",
  normalize: normalizeNodeTransfer,
  resourceKind: "node",
  fields: [
    { label: "Node", key: "node_id", mono: true },
    { label: "New owner", key: "new_owner_user_id", mono: true },
  ],
});

const nodeInjectCredentialDescriptor = createDialogDescriptor({
  variant: "node_inject_credential",
  title: "Inject node credential",
  body: "NyxID will queue this credential-injection shape for an online node without exposing a credential to chat.",
  cta: "Inject credential",
  icon: "node",
  normalize: normalizeNodeInjectCredential,
  resourceKind: "pendingCredential",
  fields: [
    { label: "Node", key: "node_id", mono: true },
    { label: "Service", key: "service_slug", mono: true },
    { label: "Injection", key: "injection_method" },
    { label: "Field", key: "field_name", mono: true },
  ],
});

const pendingCredentialPushDescriptor = createDialogDescriptor({
  variant: "pending_credential_push",
  title: "Queue pending credential",
  body: "NyxID will create a pending remote-credential request for this node and verify its safe status projection.",
  cta: "Push credential",
  icon: "node",
  normalize: normalizePendingCredentialPush,
  resourceKind: "pendingCredential",
  fields: [
    { label: "Node", key: "node_id", mono: true },
    { label: "Service", key: "service_slug", mono: true },
    { label: "Injection", key: "injection_method" },
    { label: "Field", key: "field_name", mono: true },
  ],
});

const pendingCredentialCancelDescriptor = createDialogDescriptor({
  variant: "pending_credential_cancel",
  title: "Cancel pending credential",
  body: "NyxID will deactivate this pending credential request after explicit destructive confirmation.",
  cta: "Cancel credential",
  icon: "node",
  normalize: normalizePendingCredentialCancel,
  resourceKind: "pendingCredential",
  fields: [
    { label: "Node", key: "node_id", mono: true },
    { label: "Pending credential", key: "pending_credential_id", mono: true },
  ],
});

const deviceOnboardDescriptor = createDialogDescriptor({
  variant: "device_onboard",
  title: "Onboard headless device",
  body: "NyxID will create a one-time provisioning package and show its QR payload only here.",
  cta: "Onboard device",
  icon: "node",
  normalize: normalizeDeviceOnboard,
  resourceKind: "device",
  fields: [
    { label: "Device", key: "label" },
    { label: "Organization", key: "target_org_id", mono: true },
    { label: "Default service", key: "default_service_ids", mono: true },
  ],
});

const orgCreateDescriptor = createDialogDescriptor({
  variant: "org_create",
  title: "Create organization",
  body: "NyxID will create an organization with the profile values reviewed in the dialog.",
  cta: "Create organization",
  icon: "org",
  normalize: normalizeOrgCreate,
  resourceKind: "org",
  fields: [{ label: "Name", key: "display_name" }],
});

const orgUpdateDescriptor = createDialogDescriptor({
  variant: "org_update",
  title: "Update organization",
  body: "NyxID will apply only the organization profile values reviewed in the dialog.",
  cta: "Update organization",
  icon: "org",
  normalize: normalizeOrgUpdate,
  resourceKind: "org",
  fields: [
    { label: "Organization", key: "org_id", mono: true },
    { label: "Name", key: "display_name" },
  ],
});

const orgDeleteDescriptor = createDialogDescriptor({
  variant: "org_delete",
  title: "Delete organization",
  body: "NyxID will permanently delete this organization after explicit confirmation.",
  cta: "Delete organization",
  icon: "org",
  normalize: normalizeOrgDelete,
  resourceKind: "org",
  fields: [{ label: "Organization", key: "org_id", mono: true }],
});

const orgMemberAddDescriptor = createDialogDescriptor({
  variant: "org_member_add",
  title: "Add organization member",
  body: "NyxID will add this user with the role reviewed in the dialog.",
  cta: "Add member",
  icon: "org",
  normalize: normalizeOrgMemberAdd,
  resourceKind: "org",
  fields: [
    { label: "Organization", key: "org_id", mono: true },
    { label: "User", key: "user_id", mono: true },
    { label: "Role", key: "role" },
  ],
});

const orgMemberRemoveDescriptor = createDialogDescriptor({
  variant: "org_member_remove",
  title: "Remove organization member",
  body: "NyxID will revoke this membership after explicit confirmation.",
  cta: "Remove member",
  icon: "org",
  normalize: normalizeOrgMemberRemove,
  resourceKind: "org",
  fields: [
    { label: "Organization", key: "org_id", mono: true },
    { label: "Member", key: "member_id", mono: true },
  ],
});

const orgMemberUpdateRoleDescriptor = createDialogDescriptor({
  variant: "org_member_update_role",
  title: "Change organization role",
  body: "NyxID will change this member to the role reviewed in the dialog.",
  cta: "Change member role",
  icon: "org",
  normalize: normalizeOrgMemberUpdateRole,
  resourceKind: "org",
  fields: [
    { label: "Member", key: "member_id", mono: true },
    { label: "Role", key: "role" },
  ],
});

const orgInviteDescriptor = createDialogDescriptor({
  variant: "org_invite",
  title: "Create organization invite",
  body: "NyxID will create an invite with the role and lifetime reviewed in the dialog.",
  cta: "Create invite",
  icon: "org",
  normalize: normalizeOrgInvite,
  resourceKind: "org",
  fields: [
    { label: "Organization", key: "org_id", mono: true },
    { label: "Role", key: "role" },
  ],
});

const orgSetPrimaryDescriptor = createDialogDescriptor({
  variant: "org_set_primary",
  title: "Set primary organization",
  body: "NyxID will make this organization the account's primary organization.",
  cta: "Set primary",
  icon: "org",
  normalize: normalizeOrgSetPrimary,
  resourceKind: "org",
  fields: [{ label: "Organization", key: "org_id", mono: true }],
});

const accountProfileUpdateDescriptor = createDialogDescriptor({
  variant: "account_profile_update",
  title: "Update account profile",
  body: "NyxID will apply the profile values shown for review.",
  cta: "Update profile",
  icon: "shield",
  normalize: normalizeAccountProfileUpdate,
  resourceKind: "account",
  fields: [
    { label: "Display name", key: "display_name" },
    { label: "Avatar URL", key: "avatar_url", mono: true },
  ],
});

const accountRevokeConsentDescriptor = createDialogDescriptor({
  variant: "account_revoke_consent",
  title: "Revoke application consent",
  body: "NyxID will revoke this application's consent after explicit confirmation.",
  cta: "Revoke consent",
  icon: "shield",
  normalize: normalizeAccountRevokeConsent,
  resourceKind: "account",
  fields: [{ label: "Client", key: "client_id", mono: true }],
});

const accountDeleteDescriptor = createDialogDescriptor({
  variant: "account_delete",
  title: "Delete account",
  body: "NyxID will require your account email before permanently deleting the account.",
  cta: "Delete account",
  icon: "shield",
  normalize: normalizeAccountDelete,
  resourceKind: "account",
});

const accountMfaSetupDescriptor = createDialogDescriptor({
  variant: "account_mfa_setup",
  title: "Set up multi-factor authentication",
  body: "NyxID will keep setup and recovery material inside the browser journey.",
  cta: "Set up MFA",
  icon: "shield",
  normalize: normalizeAccountMfaSetup,
  resourceKind: "account",
});

const approvalConfigureDescriptor = createDialogDescriptor({
  variant: "approval_configure",
  title: "Configure service approvals",
  body: "NyxID will apply the approval policy reviewed in the dialog.",
  cta: "Configure approvals",
  icon: "shield",
  normalize: normalizeApprovalConfigure,
  resourceKind: "approvalConfig",
  fields: [{ label: "Service", key: "service_id", mono: true }],
});

const approvalEnableDescriptor = createDialogDescriptor({
  variant: "approval_enable",
  title: "Enable service approvals",
  body: "NyxID will require approval for this service.",
  cta: "Enable approvals",
  icon: "shield",
  normalize: normalizeApprovalEnable,
  resourceKind: "approvalConfig",
  fields: [{ label: "Service", key: "service_id", mono: true }],
});

const approvalDisableDescriptor = createDialogDescriptor({
  variant: "approval_disable",
  title: "Disable service approvals",
  body: "NyxID will weaken this service's safety control only after explicit confirmation.",
  cta: "Disable approvals",
  icon: "shield",
  normalize: normalizeApprovalDisable,
  resourceKind: "approvalConfig",
  fields: [{ label: "Service", key: "service_id", mono: true }],
});

const approvalRevokeGrantDescriptor = createDialogDescriptor({
  variant: "approval_revoke_grant",
  title: "Revoke approval grant",
  body: "NyxID will revoke this active grant after explicit confirmation.",
  cta: "Revoke grant",
  icon: "shield",
  normalize: normalizeApprovalRevokeGrant,
  resourceKind: "grant",
  fields: [{ label: "Grant", key: "grant_id", mono: true }],
});

const notificationsUpdateDescriptor = createDialogDescriptor({
  variant: "notifications_update",
  title: "Update notification settings",
  body: "NyxID will load the current settings and let you edit them in the dialog.",
  cta: "Update notifications",
  icon: "bell",
  normalize: normalizeNotificationsUpdate,
  resourceKind: "notificationBinding",
});

const notificationsTelegramLinkDescriptor = createDialogDescriptor({
  variant: "notifications_telegram_link",
  title: "Link Telegram notifications",
  body: "NyxID will create a one-time Telegram linking code and show it only here.",
  cta: "Link Telegram",
  icon: "bell",
  normalize: normalizeNotificationsTelegramLink,
  resourceKind: "notificationBinding",
});

const notificationsTelegramDisconnectDescriptor = createDialogDescriptor({
  variant: "notifications_telegram_disconnect",
  title: "Disconnect Telegram notifications",
  body: "NyxID will disconnect Telegram after explicit confirmation.",
  cta: "Disconnect Telegram",
  icon: "bell",
  normalize: normalizeNotificationsTelegramDisconnect,
  resourceKind: "notificationBinding",
});

const serviceAccountCreateDescriptor = createDialogDescriptor({
  variant: "service_account_create",
  title: "Create service account",
  body: "NyxID will create the service account and show its client secret once.",
  cta: "Create service account",
  icon: "key",
  normalize: normalizeServiceAccountCreate,
  resourceKind: "serviceAccount",
  fields: [{ label: "Name", key: "name" }],
});

const serviceAccountUpdateDescriptor = createDialogDescriptor({
  variant: "service_account_update",
  title: "Update service account",
  body: "NyxID will update only the service-account metadata reviewed in the dialog.",
  cta: "Update service account",
  icon: "key",
  normalize: normalizeServiceAccountUpdate,
  resourceKind: "serviceAccount",
  fields: [{ label: "Service account", key: "service_account_id", mono: true }],
});

const serviceAccountDeleteDescriptor = createDialogDescriptor({
  variant: "service_account_delete",
  title: "Delete service account",
  body: "NyxID will deactivate this service account after explicit confirmation.",
  cta: "Delete service account",
  icon: "key",
  normalize: normalizeServiceAccountDelete,
  resourceKind: "serviceAccount",
  fields: [{ label: "Service account", key: "service_account_id", mono: true }],
});

const serviceAccountRotateSecretDescriptor = createDialogDescriptor({
  variant: "service_account_rotate_secret",
  title: "Rotate service-account secret",
  body: "NyxID will rotate this service account's secret and show it once.",
  cta: "Rotate service-account secret",
  icon: "key",
  normalize: normalizeServiceAccountRotateSecret,
  resourceKind: "serviceAccount",
  fields: [{ label: "Service account", key: "service_account_id", mono: true }],
});

const serviceAccountRevokeTokensDescriptor = createDialogDescriptor({
  variant: "service_account_revoke_tokens",
  title: "Revoke service-account tokens",
  body: "NyxID will invalidate this service account's active tokens after explicit confirmation.",
  cta: "Revoke service-account tokens",
  icon: "key",
  normalize: normalizeServiceAccountRevokeTokens,
  resourceKind: "serviceAccount",
  fields: [{ label: "Service account", key: "service_account_id", mono: true }],
});

const developerAppCreateDescriptor = createDialogDescriptor({
  variant: "developer_app_create",
  title: "Create developer application",
  body: "NyxID will create the OAuth client and show its client secret once.",
  cta: "Create developer app",
  icon: "app",
  normalize: normalizeDeveloperAppCreate,
  resourceKind: "developerApp",
  fields: [
    { label: "Name", key: "name" },
    { label: "Redirect URI", key: "redirect_uris", mono: true },
  ],
});

const developerAppUpdateDescriptor = createDialogDescriptor({
  variant: "developer_app_update",
  title: "Update developer application",
  body: "NyxID will update only the OAuth client values reviewed in the dialog.",
  cta: "Update developer app",
  icon: "app",
  normalize: normalizeDeveloperAppUpdate,
  resourceKind: "developerApp",
  fields: [{ label: "Client", key: "client_id", mono: true }],
});

const developerAppDeleteDescriptor = createDialogDescriptor({
  variant: "developer_app_delete",
  title: "Delete developer application",
  body: "NyxID will deactivate this OAuth client after explicit confirmation.",
  cta: "Delete developer app",
  icon: "app",
  normalize: normalizeDeveloperAppDelete,
  resourceKind: "developerApp",
  fields: [{ label: "Client", key: "client_id", mono: true }],
});

const developerAppRotateSecretDescriptor = createDialogDescriptor({
  variant: "developer_app_rotate_secret",
  title: "Rotate developer-app secret",
  body: "NyxID will rotate this OAuth client secret and show it once.",
  cta: "Rotate developer-app secret",
  icon: "app",
  normalize: normalizeDeveloperAppRotateSecret,
  resourceKind: "developerApp",
  fields: [{ label: "Client", key: "client_id", mono: true }],
});

const externalKeyAddGcpDescriptor = createDialogDescriptor({
  variant: "external_key_add_gcp_service_account",
  title: "Add GCP service account",
  body: "Paste the GCP service-account JSON only inside the NyxID dialog.",
  cta: "Add GCP service account",
  icon: "key",
  normalize: normalizeExternalKeyAddGcp,
  resourceKind: "externalKey",
  fields: [{ label: "Label", key: "label" }],
});

const openClawConnectDescriptor = createDialogDescriptor({
  variant: "openclaw_connect",
  title: "Connect OpenClaw",
  body: "NyxID will collect the OpenClaw bearer credential only inside the browser dialog.",
  cta: "Connect OpenClaw",
  icon: "service",
  normalize: normalizeOpenClawConnect,
  resourceKind: "userService",
  fields: [{ label: "Gateway", key: "gateway_url", mono: true }],
});

const unsupportedDescriptor: ActionDescriptor = {
  title: () => "Unsupported action request",
  body: () =>
    "This assistant requested an action this version of NyxID cannot perform. Decline it to let the assistant continue safely.",
  cta: () => "",
  risk: "unsupported",
  normalize: () => null,
  summary: () => [],
  icon: "shield",
  busyLabel: "Working",
  assurance: "",
  resource: () => {
    throw new Error("Unsupported actions cannot produce a resource.");
  },
  wiring: "deferred",
  journey: () => null,
};

export const ACTION_REGISTRY: Readonly<Record<string, ActionDescriptor>> = {
  "service.connect": serviceConnectDescriptor,
  "service.reauthorize": serviceReauthorizeDescriptor,
  "key.create": keyCreateDescriptor,
  "key.rotate": keyRotateDescriptor,
  "key.update": keyUpdateDescriptor,
  "key.delete": keyDeleteDescriptor,
  "key.extend_scope": keyExtendScopeDescriptor,
  "key.bind_credential": keyBindCredentialDescriptor,
  "service.update": serviceUpdateDescriptor,
  "service.delete": serviceDeleteDescriptor,
  "service.route": serviceRouteDescriptor,
  "service.rotate_credential": serviceRotateCredentialDescriptor,
  "endpoint.update": endpointUpdateDescriptor,
  "endpoint.delete": endpointDeleteDescriptor,
  "external_key.rotate": externalKeyRotateDescriptor,
  "external_key.delete": externalKeyDeleteDescriptor,
  "node.register_token": nodeRegisterTokenDescriptor,
  "node.rotate_token": nodeRotateTokenDescriptor,
  "node.delete": nodeDeleteDescriptor,
  "node.transfer": nodeTransferDescriptor,
  "node.inject_credential": nodeInjectCredentialDescriptor,
  "pending_credential.push": pendingCredentialPushDescriptor,
  "pending_credential.cancel": pendingCredentialCancelDescriptor,
  "device.onboard": deviceOnboardDescriptor,
  "org.create": orgCreateDescriptor,
  "org.update": orgUpdateDescriptor,
  "org.delete": orgDeleteDescriptor,
  "org.member_add": orgMemberAddDescriptor,
  "org.member_remove": orgMemberRemoveDescriptor,
  "org.member_update_role": orgMemberUpdateRoleDescriptor,
  "org.invite": orgInviteDescriptor,
  "org.set_primary": orgSetPrimaryDescriptor,
  "account.profile_update": accountProfileUpdateDescriptor,
  "account.revoke_consent": accountRevokeConsentDescriptor,
  "account.delete": accountDeleteDescriptor,
  "account.mfa_setup": accountMfaSetupDescriptor,
  "approval.configure": approvalConfigureDescriptor,
  "approval.enable": approvalEnableDescriptor,
  "approval.disable": approvalDisableDescriptor,
  "approval.revoke_grant": approvalRevokeGrantDescriptor,
  "notifications.update": notificationsUpdateDescriptor,
  "notifications.telegram_link": notificationsTelegramLinkDescriptor,
  "notifications.telegram_disconnect":
    notificationsTelegramDisconnectDescriptor,
  "service_account.create": serviceAccountCreateDescriptor,
  "service_account.update": serviceAccountUpdateDescriptor,
  "service_account.delete": serviceAccountDeleteDescriptor,
  "service_account.rotate_secret": serviceAccountRotateSecretDescriptor,
  "service_account.revoke_tokens": serviceAccountRevokeTokensDescriptor,
  "developer_app.create": developerAppCreateDescriptor,
  "developer_app.update": developerAppUpdateDescriptor,
  "developer_app.delete": developerAppDeleteDescriptor,
  "developer_app.rotate_secret": developerAppRotateSecretDescriptor,
  "external_key.add_gcp_service_account": externalKeyAddGcpDescriptor,
  "openclaw.connect": openClawConnectDescriptor,
};

export interface ResolvedAction {
  readonly descriptor: ActionDescriptor;
  readonly params: ActionCardParams;
  readonly supported: boolean;
  readonly journey: ActionJourney;
}

export function resolveAssistantAction(
  request: AssistantActionRequest,
): ResolvedAction {
  const descriptor = ACTION_REGISTRY[request.action];
  const params = descriptor?.normalize(request.params) ?? {
    variant: "unknown",
  };
  const journey = descriptor?.journey(params) ?? null;
  const supported =
    request.schemaVersion === ACTION_SCHEMA_VERSION &&
    descriptor !== undefined &&
    journey !== null;
  if (!supported) {
    return {
      descriptor: unsupportedDescriptor,
      params,
      supported: false,
      journey: null,
    };
  }
  return {
    descriptor,
    params,
    supported: true,
    journey,
  };
}

export function descriptorForAction(
  action: string,
  params: ActionCardParams,
  supported: boolean,
): ActionDescriptor {
  if (!supported) return unsupportedDescriptor;
  const descriptor = ACTION_REGISTRY[action];
  if (!descriptor) return unsupportedDescriptor;
  return descriptor.journey(params) ? descriptor : unsupportedDescriptor;
}
