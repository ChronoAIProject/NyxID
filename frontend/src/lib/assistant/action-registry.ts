import {
  ACTION_SCHEMA_VERSION,
  ACTION_SERVICE_SLUG_PATTERN,
  keyCreateActionParamsSchema,
  keyRotateActionParamsSchema,
  serviceConnectActionParamsSchema,
  serviceReauthorizeActionParamsSchema,
  type ActionCardParams,
  type ActionResource,
  type AssistantActionRequest,
} from "@/schemas/assistant-actions";

export type ActionRisk = "credential_access" | "unsupported";
export type ActionJourney = ActionCardParams["variant"] | null;
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
  readonly resource: (id: string) => ActionResource;
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
  resource: (id) => ({ userService: { userServiceId: id } }),
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
  resource: (id) => ({ userService: { userServiceId: id } }),
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
  resource: (id) => ({ key: { keyId: id } }),
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
  resource: (id) => ({ key: { keyId: id } }),
  wiring: "dialog",
  journey: (params) => (params.variant === "key_rotate" ? "key_rotate" : null),
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
