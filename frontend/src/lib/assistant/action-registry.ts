import {
  ACTION_SCHEMA_VERSION,
  ACTION_SERVICE_SLUG_PATTERN,
  keyBindCredentialActionParamsSchema,
  keyCreateActionParamsSchema,
  keyDeleteActionParamsSchema,
  keyExtendScopeActionParamsSchema,
  keyRotateActionParamsSchema,
  keyUpdateActionParamsSchema,
  serviceConnectActionParamsSchema,
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
  "endpoint.update": unsupportedDescriptor,
  "endpoint.delete": unsupportedDescriptor,
  "external_key.rotate": unsupportedDescriptor,
  "external_key.delete": unsupportedDescriptor,
  "node.register_token": unsupportedDescriptor,
  "node.rotate_token": unsupportedDescriptor,
  "node.delete": unsupportedDescriptor,
  "node.transfer": unsupportedDescriptor,
  "node.inject_credential": unsupportedDescriptor,
  "pending_credential.push": unsupportedDescriptor,
  "pending_credential.cancel": unsupportedDescriptor,
  "device.onboard": unsupportedDescriptor,
  "org.create": unsupportedDescriptor,
  "org.update": unsupportedDescriptor,
  "org.delete": unsupportedDescriptor,
  "org.member_add": unsupportedDescriptor,
  "org.member_remove": unsupportedDescriptor,
  "org.member_update_role": unsupportedDescriptor,
  "org.invite": unsupportedDescriptor,
  "org.set_primary": unsupportedDescriptor,
  "account.profile_update": unsupportedDescriptor,
  "account.revoke_consent": unsupportedDescriptor,
  "account.delete": unsupportedDescriptor,
  "account.mfa_setup": unsupportedDescriptor,
  "approval.configure": unsupportedDescriptor,
  "approval.enable": unsupportedDescriptor,
  "approval.disable": unsupportedDescriptor,
  "approval.revoke_grant": unsupportedDescriptor,
  "notifications.update": unsupportedDescriptor,
  "notifications.telegram_link": unsupportedDescriptor,
  "notifications.telegram_disconnect": unsupportedDescriptor,
  "service_account.create": unsupportedDescriptor,
  "service_account.update": unsupportedDescriptor,
  "service_account.delete": unsupportedDescriptor,
  "service_account.rotate_secret": unsupportedDescriptor,
  "service_account.revoke_tokens": unsupportedDescriptor,
  "developer_app.create": unsupportedDescriptor,
  "developer_app.update": unsupportedDescriptor,
  "developer_app.delete": unsupportedDescriptor,
  "developer_app.rotate_secret": unsupportedDescriptor,
  "external_key.add_gcp_service_account": unsupportedDescriptor,
  "openclaw.connect": unsupportedDescriptor,
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
