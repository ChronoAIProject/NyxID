import { translateApp, appLocale } from "../../i18n/appTranslation";
import { z } from "zod";
import {
  classifyLoginError,
  LoginRequestError,
  loginErrorCopy,
} from "./loginRequestErrors";
import { MOBILE_MFA_STEP_UP_MESSAGE } from "../../lib/api/errorMessages";
import type {
  AgentKeyOptions,
  AgentKeySummary,
  NewAgentKey,
} from "../../lib/api/agentKeyLoginSchema";

export function defaultNewAgentKey(now = Date.now()): NewAgentKey {
  return {
    kind: "new",
    name: translateApp("CLI Agent"),
    scopes: "read proxy",
    allowed_service_ids: [],
    allowed_node_ids: [],
    allow_all_services: false,
    allow_auto_connected_services: false,
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
    owner_name: org?.name ?? translateApp("Your personal account"),
    allowed_services: options.services.filter(
      (item) =>
        input.allowed_service_ids.includes(item.id) ||
        (!!input.allow_auto_connected_services &&
          !!item.auto_connected &&
          item.owner_id === (input.target_org_id || options.personal_owner_id)),
    ),
    allowed_nodes: options.nodes.filter((item) =>
      input.allowed_node_ids.includes(item.id),
    ),
    expires_at: input.never_expires ? null
      : (input.expires_at ?? new Date(Date.now() + 90 * 86400000).toISOString()),
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
    { label: translateApp("Key"), value: key.name },
    { label: translateApp("Owner"), value: `${key.owner_name} (${key.owner_type})` },
    { label: translateApp("Prefix"), value: key.key_prefix || translateApp("Created on approval"),
    },
    { label: translateApp("Effective permissions"), value: effective },
    { label: translateApp("Scopes"), value: key.scopes },
    {
      label: translateApp("Services"),
      value: key.allow_all_services
        ? translateApp("All services, including future services")
        : [
            key.allow_auto_connected_services
              ? translateApp(
                  "All auto-connected platform services (including future additions)",
                )
              : "",
            ...key.allowed_services.map(
              (item) =>
                (item.auto_connected ? translateApp("{{name}} (Platform)", { name: item.name }) : item.name),
            ),
          ]
            .filter(Boolean)
            .join(", ") || translateApp("None"),
      warning: key.allow_all_services,
    },
    {
      label: translateApp("Nodes"),
      value: key.allow_all_nodes
        ? translateApp("All nodes, including future nodes")
        : key.allowed_nodes.map((item) => item.name).join(", ") ||
          translateApp("None"),
      warning: key.allow_all_nodes,
    },
    {
      label: translateApp("Key expiry"),
      value: key.expires_at
        ? new Date(key.expires_at).toLocaleString(appLocale())
        : translateApp("No expiry"),
    },
    {
      label: translateApp("Rate limit"),
      value: translateApp("{{rate}} requests/s; burst {{burst}}", {
        rate: key.rate_limit_per_second ?? "Default",
        burst: key.rate_limit_burst ?? "default",
      }),
    },
    { label: translateApp("Platform"), value: key.platform ?? translateApp("Not specified"),
    },
  ];
}

export function issuanceNotice(existing: boolean): string {
  return (existing ? translateApp("A new login credential will be issued for this key. The key's existing secret and other consumers are unaffected. Revoke it from the key's Login credentials section. Revoking the key invalidates every login credential.") : translateApp("A new login credential will be issued for this key. The primary key secret will not be shown. Revoke it from the key's Login credentials section. Revoking the key invalidates every login credential."));
}

export function agentKeyLoginError(error: unknown): string {
  if (error instanceof z.ZodError) {
    return error.issues[0]?.message ?? translateApp("Check the login details.");
  }
  const code = (error as { errorCode?: number } | null)?.errorCode;
  if (code === 2003) return MOBILE_MFA_STEP_UP_MESSAGE;
  const messages: Record<number, string> = {
    11900: translateApp("This request is no longer available."),
    11901: translateApp("This request has expired."),
    11902: translateApp("Waiting for approval."),
    11903: translateApp("Please wait before trying again."),
    11904: translateApp("This request was rejected."),
    11905: translateApp("This request was already completed."),
    11906: translateApp("Too many attempts. Try again later."),
    11907: translateApp("Enter a valid eight-character code."),
    11908: translateApp("This key is no longer eligible. Choose another key."),
    11909: translateApp("This credential is no longer available."),
  };
  return (
    (code === undefined ? undefined : messages[code]) ??
    (error instanceof Error
      ? error.message
      : translateApp("Could not reach NyxID. Try again."))
  );
}

/** Select resolver copy for request failures and key-specific copy for local actions. */
export function agentKeyActionErrorCopy(failure: unknown, step: string): string {
  const response = failure as { statusCode?: number } | null;
  const kind = classifyLoginError(failure);
  const useRequestCopy =
    step === "code" ||
    failure instanceof LoginRequestError ||
    ["rate_limited", "network"].includes(kind) ||
    (response?.statusCode !== undefined && kind !== "rejected");
  return useRequestCopy ? loginErrorCopy(failure) : agentKeyLoginError(failure);
}
