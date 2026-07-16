import type { ApprovalRequestItem } from "@/types/approvals";

export function isToolApproval(request: ApprovalRequestItem): boolean {
  return request.tool_name != null;
}

export function operationIdentityLabel(
  request: ApprovalRequestItem,
): string | null {
  const method = request.http_method?.trim();
  const resource = request.resource?.trim();
  if (method && resource) return `${method} ${resource}`;
  if (resource) return resource;
  if (method) return method;
  return request.verb ?? null;
}

const HUMANIZED_VERBS: Record<string, string> = {
  GET: "Read",
  POST: "Create",
  PUT: "Update",
  PATCH: "Update",
  DELETE: "Revoke",
};

const HUMANIZED_RESOURCES: Record<string, string> = {
  users: "user list",
  keys: "an API key",
  api_keys: "an API key",
  apikeys: "an API key",
  sessions: "a session",
};

function isOpaqueIdSegment(segment: string): boolean {
  return (
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
      segment,
    ) ||
    (/^[0-9a-f]+$/i.test(segment) && segment.length >= 16) ||
    /^[0-9]+$/.test(segment)
  );
}

export function humanizeOperation(
  method: string | null | undefined,
  resource: string | null | undefined,
): string | null {
  const m = method?.trim();
  const verb = m ? (HUMANIZED_VERBS[m.toUpperCase()] ?? null) : null;

  const segments =
    resource && resource.trim().length > 0
      ? resource
          .split("/")
          .map((s) => s.trim())
          .filter(Boolean)
      : [];
  if (segments.length > 0) {
    for (let i = segments.length - 1; i >= 0; i -= 1) {
      const curated = HUMANIZED_RESOURCES[segments[i]!.toLowerCase()];
      if (curated) {
        return verb ? `${verb} ${curated}` : curated;
      }
    }
    let noun: string | undefined;
    for (let i = segments.length - 1; i >= 0; i -= 1) {
      const seg = segments[i]!;
      if (!isOpaqueIdSegment(seg)) {
        noun = seg;
        break;
      }
    }
    const resolvedNoun: string = noun ?? segments[segments.length - 1]!;
    return verb ? `${verb} ${resolvedNoun}` : resolvedNoun;
  }

  return verb;
}

export function primaryActionLabel(request: ApprovalRequestItem): string {
  if (isToolApproval(request)) {
    return request.tool_arguments ?? "Tool execution approval";
  }
  return (
    request.action_description ||
    request.operation_summary ||
    humanizeOperation(request.http_method, request.resource) ||
    operationIdentityLabel(request) ||
    "Proxy request"
  );
}

export function shouldShowRawIdentityLine(
  request: ApprovalRequestItem,
): boolean {
  return !request.action_description && operationIdentityLabel(request) != null;
}
