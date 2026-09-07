import type {
  AgentKeyOptions,
  AgentKeySummary,
  NewAgentKey,
} from "../../lib/api/agentKeyLoginSchema";

export function defaultNewAgentKey(now = Date.now()): NewAgentKey {
  return {
    kind: "new",
    name: "CLI Agent",
    scopes: "read proxy",
    allowed_service_ids: [],
    allowed_node_ids: [],
    allow_all_services: false,
    allow_all_nodes: false,
    expires_at: new Date(now + 90 * 86400000).toISOString(),
    platform: "generic",
  };
}

export function newKeySummary(
  input: NewAgentKey,
  options: AgentKeyOptions,
): AgentKeySummary {
  const org = options.orgs.find((item) => item.id === input.target_org_id);
  return {
    ...input,
    id: "",
    key_prefix: "",
    owner_type: org ? "org" : "personal",
    owner_id: org?.id ?? "",
    owner_name: org?.name ?? "Your personal account",
    allowed_services: options.services.filter((item) =>
      input.allowed_service_ids.includes(item.id),
    ),
    allowed_nodes: options.nodes.filter((item) =>
      input.allowed_node_ids.includes(item.id),
    ),
    expires_at: input.expires_at ?? null,
    rate_limit_per_second: input.rate_limit_per_second ?? null,
    rate_limit_burst: input.rate_limit_burst ?? null,
    platform: input.platform ?? null,
    created_now: true,
  };
}

export function permissionRows(
  key: AgentKeySummary,
): { label: string; value: string; warning?: boolean }[] {
  const scopes = new Set(key.scopes.split(/\s+/));
  const effective = [
    "read",
    scopes.has("write") || scopes.has("admin") ? "write" : null,
    scopes.has("proxy") || scopes.has("proxy:*") ? "proxy" : null,
  ]
    .filter(Boolean)
    .join(", ");
  return [
    { label: "Key", value: key.name },
    { label: "Owner", value: `${key.owner_name} (${key.owner_type})` },
    { label: "Prefix", value: key.key_prefix || "Created on approval" },
    { label: "Effective permissions", value: effective },
    { label: "Scopes", value: key.scopes },
    {
      label: "Services",
      value: key.allow_all_services
        ? "All services, including future services"
        : key.allowed_services.map((item) => item.name).join(", ") || "None",
      warning: key.allow_all_services,
    },
    {
      label: "Nodes",
      value: key.allow_all_nodes
        ? "All nodes, including future nodes"
        : key.allowed_nodes.map((item) => item.name).join(", ") || "None",
      warning: key.allow_all_nodes,
    },
    {
      label: "Key expiry",
      value: key.expires_at
        ? new Date(key.expires_at).toLocaleString()
        : "No expiry",
    },
    {
      label: "Rate limit",
      value: `${key.rate_limit_per_second ?? "Default"} requests/s; burst ${key.rate_limit_burst ?? "default"}`,
    },
    { label: "Platform", value: key.platform ?? "Not specified" },
  ];
}

export function issuanceNotice(existing: boolean): string {
  return `A new login credential will be issued for this key. ${existing ? "The key's existing secret and other consumers are unaffected." : "The primary key secret will not be shown."} Revoke it from the key's Login credentials section. Revoking the key invalidates every login credential.`;
}

export function agentKeyLoginError(error: unknown): string {
  if (error instanceof z.ZodError) {
    return error.issues[0]?.message ?? "Check the login details.";
  }
  const code = (error as { errorCode?: number } | null)?.errorCode;
  const messages: Record<number, string> = {
    11900: "This request is no longer available.",
    11901: "This request has expired.",
    11902: "Waiting for approval.",
    11903: "Please wait before trying again.",
    11904: "This request was rejected.",
    11905: "This request was already completed.",
    11906: "Too many attempts. Try again later.",
    11907: "Enter a valid eight-character code.",
    11908: "This key is no longer eligible. Choose another key.",
    11909: "This credential is no longer available.",
  };
  return (
    (code === undefined ? undefined : messages[code]) ??
    (error instanceof Error
      ? error.message
      : "Could not reach NyxID. Try again.")
  );
}
import { z } from "zod";
