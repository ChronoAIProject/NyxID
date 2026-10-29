import type {
  ApprovalDecisionChannel,
  ApprovalMode,
  ChallengeDetail,
  ChallengeStatus,
} from "./types";

export type BackendApprovalRequestItem = {
  id: string;
  service_name: string;
  service_slug: string;
  requester_type: string;
  requester_label?: string | null;
  operation_summary: string;
  action_description?: string | null;
  http_method?: string | null;
  resource?: string | null;
  verb?: string | null;
  tool_name?: string | null;
  tool_call_id?: string | null;
  tool_arguments?: string | null;
  is_destructive?: boolean | null;
  approval_mode: ApprovalMode;
  status: string;
  created_at: string;
  expires_at?: string;
  decided_at?: string | null;
  decision_channel?: string | null;
  from_org_policy?: boolean;
  org_id?: string | null;
  org_name?: string | null;
};

export function mapChallengeStatus(status: string): ChallengeStatus {
  if (status === "approved") return "APPROVED";
  if (status === "rejected") return "DENIED";
  if (status === "expired") return "EXPIRED";
  return "PENDING";
}

/** Match the Web workspace: the push decision path is the mobile app. */
function mapDecisionChannel(
  channel: string | null | undefined,
): ApprovalDecisionChannel | null {
  if (channel === "push") return "mobile";
  if (channel === "web" || channel === "telegram" || channel === "mobile") {
    return channel;
  }
  return null;
}

/** Display the server's operation evidence; HTTP verbs alone do not describe
 * an MCP tool's authority or destructiveness. Never invent request lifetime. */
export function mapBackendRequestToChallenge(
  item: BackendApprovalRequestItem,
): ChallengeDetail {
  const summary = item.action_description ?? item.operation_summary;
  const parsed = summary
    .replace(/^proxy:/i, "")
    .trim()
    .match(/^([A-Z]+)\s+(.+)$/);
  const action =
    item.tool_name?.trim() ||
    item.http_method?.trim() ||
    item.verb?.trim() ||
    parsed?.[1] ||
    "Request";
  const resource =
    item.resource?.trim() || parsed?.[2] || summary || "Unspecified resource";
  const risk =
    item.is_destructive === true || item.verb === "destructive"
      ? "high"
      : item.verb === "write"
        ? "medium"
        : item.verb === "read"
          ? "low"
          : "unknown";
  const fromOrgPolicy = item.from_org_policy === true;
  return {
    id: item.id,
    title: item.service_name.trim() || "Unknown Service",
    action,
    resource,
    approval_mode: item.approval_mode,
    risk_level: risk,
    status: mapChallengeStatus(item.status),
    created_at: item.created_at,
    expires_at: item.expires_at ?? "",
    summary,
    decided_at: item.decided_at ?? null,
    decision_channel: mapDecisionChannel(item.decision_channel),
    request_context: {
      client: item.requester_type || "Unknown",
      requester:
        item.requester_label?.trim() ||
        item.requester_type?.trim() ||
        "Unspecified",
    },
    service_slug: item.service_slug,
    tool_name: item.tool_name,
    tool_call_id: item.tool_call_id,
    tool_arguments: item.tool_arguments,
    is_destructive: item.is_destructive,
    verb: item.verb,
    from_org_policy: fromOrgPolicy,
    org_id: fromOrgPolicy ? (item.org_id ?? null) : undefined,
    org_name: fromOrgPolicy ? (item.org_name ?? null) : undefined,
  };
}
