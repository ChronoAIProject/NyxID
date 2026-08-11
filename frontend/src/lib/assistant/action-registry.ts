import {
  ACTION_SCHEMA_VERSION,
  ACTION_SERVICE_SLUG_PATTERN,
  keyCreateActionParamsSchema,
  serviceConnectActionParamsSchema,
  type ActionCardParams,
  type AssistantActionRequest,
} from "@/schemas/assistant-actions";

export type ActionRisk = "credential_access" | "unsupported";
export type ActionJourney =
  | "catalog_service"
  | "custom_service"
  | "key_create"
  | null;

export interface ActionDescriptor {
  readonly title: (params: ActionCardParams) => string;
  readonly body: (params: ActionCardParams) => string;
  readonly cta: (params: ActionCardParams) => string;
  readonly risk: ActionRisk;
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

const serviceConnectDescriptor: ActionDescriptor = {
  title: (params) => `Connect ${actionServiceLabel(params)}`,
  body: (params) =>
    `NyxID will broker access to ${actionServiceLabel(params)} for this assistant request. Your credential stays in NyxID and is never shared with the model.`,
  cta: (params) => `Connect ${actionServiceLabel(params)}`,
  risk: "credential_access",
  journey: (params) => {
    if (params.variant === "catalog") return "catalog_service";
    if (params.variant === "custom") return "custom_service";
    return null;
  },
};

const keyCreateDescriptor: ActionDescriptor = {
  title: () => "Create API key",
  body: (params) =>
    params.variant === "key_create"
      ? `NyxID will create ${clampServiceLabel(params.name) || "an API key"} with proxy access limited to the listed services.`
      : "NyxID will create a least-scope API key.",
  cta: () => "Create key",
  risk: "credential_access",
  journey: (params) => (params.variant === "key_create" ? "key_create" : null),
};

const unsupportedDescriptor: ActionDescriptor = {
  title: () => "Unsupported action request",
  body: () =>
    "This assistant requested an action this version of NyxID cannot perform. Decline it to let the assistant continue safely.",
  cta: () => "",
  risk: "unsupported",
  journey: () => null,
};

export const ACTION_REGISTRY: Readonly<Record<string, ActionDescriptor>> = {
  "service.connect": serviceConnectDescriptor,
  "key.create": keyCreateDescriptor,
};

export interface ResolvedAction {
  readonly descriptor: ActionDescriptor;
  readonly params: ActionCardParams;
  readonly supported: boolean;
  readonly journey: ActionJourney;
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

function normalizeParams(request: AssistantActionRequest): ActionCardParams {
  if (request.action === "key.create") {
    const parsed = keyCreateActionParamsSchema.safeParse(request.params);
    if (!parsed.success) return { variant: "unknown" };
    return {
      variant: "key_create",
      name: parsed.data.name,
      platform: parsed.data.platform,
      allowed_service_ids: parsed.data.allowedServiceIds,
    };
  }

  const connected = serviceConnectActionParamsSchema.safeParse(request.params);
  if (!connected.success) return { variant: "unknown" };
  const catalog = connected.data.catalogService;
  const custom = connected.data.customService;
  if (catalog && !custom) {
    const serviceSlug = safeCatalogServiceSlug(catalog.serviceSlug);
    if (!serviceSlug) return { variant: "unknown" };
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
    if (!endpointUrl || authKeyName === null) return { variant: "unknown" };
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
  return { variant: "unknown" };
}

export function resolveAssistantAction(
  request: AssistantActionRequest,
): ResolvedAction {
  const params = normalizeParams(request);
  const descriptor = ACTION_REGISTRY[request.action];
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
