import {
  ACTION_SCHEMA_VERSION,
  type ActionCardParams,
  type AssistantActionRequest,
} from "@/schemas/assistant-actions";

export type ActionRisk = "credential_access" | "unsupported";
export type ActionJourney = "catalog_service" | "custom_service" | null;

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
  return label || "service";
}

export function actionServiceLabel(params: ActionCardParams): string {
  if (params.variant === "catalog") {
    return humanizeServiceSlug(params.service_slug);
  }
  if (params.variant === "custom")
    return params.name.trim() || "custom service";
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

function safeEndpointUrl(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return "";
    if (parsed.username || parsed.password) return "";
    return parsed.toString();
  } catch {
    // Leave an invalid/missing field blank so the user can complete it in the
    // wizard instead of rejecting the whole custom-service journey.
    return "";
  }
}

function normalizeParams(request: AssistantActionRequest): ActionCardParams {
  const catalog = request.params.catalogService;
  const custom = request.params.customService;
  if (catalog && !custom) {
    return {
      variant: "catalog",
      service_slug: catalog.serviceSlug.trim(),
      requested_scopes: catalog.requestedScopes.map((scope) => scope.trim()),
      via_node_id: nullableId(catalog.viaNodeId),
      target_org_id: nullableId(catalog.targetOrgId),
    };
  }
  if (custom && !catalog) {
    return {
      variant: "custom",
      name: custom.name.trim(),
      endpoint_url: safeEndpointUrl(custom.endpointUrl),
      auth_method: custom.authMethod.trim(),
      auth_key_name: custom.authKeyName.trim(),
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
