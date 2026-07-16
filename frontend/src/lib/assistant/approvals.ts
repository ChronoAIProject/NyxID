import { primaryActionLabel } from "@/lib/approval-labels";
import type { ApprovalRequestItem } from "@/types/approvals";
import type {
  ApprovalCardContentBlock,
  ApprovalDecision,
  ApprovalDecisionChannel,
} from "@/types/assistant";

/**
 * A real `/approvals/requests` row shaped for the assistant approval
 * card. Unlike the mock `ApprovalListEntry` there is no conversation —
 * real approval requests originate from agent/proxy activity, not from
 * assistant chat turns.
 */
export interface AssistantApprovalEntry {
  readonly requestId: string;
  readonly serviceName: string;
  readonly requestedAt: string;
  readonly decidedAt: string | null;
  readonly block: ApprovalCardContentBlock;
}

/** NyxID "rejected" ⇄ assistant-block "denied" (PRD §5.5). */
function toDecision(
  status: ApprovalRequestItem["status"],
): ApprovalDecision | null {
  switch (status) {
    case "pending":
      return null;
    case "approved":
      return "approved";
    case "rejected":
      return "denied";
    case "expired":
      return "expired";
  }
}

/** The decide paths stamp web/telegram/mobile/push; push is the mobile app. */
function toDecisionChannel(
  channel: string | null,
): ApprovalDecisionChannel | null {
  switch (channel) {
    case "web":
      return "web";
    case "telegram":
      return "telegram";
    case "mobile":
    case "push":
      return "mobile";
    default:
      return null;
  }
}

export function toAssistantApprovalEntry(
  request: ApprovalRequestItem,
  grantDurationSec: number | null = null,
): AssistantApprovalEntry {
  return {
    requestId: request.id,
    serviceName: request.service_name,
    requestedAt: request.created_at,
    decidedAt: request.decided_at,
    block: {
      type: "approval_card",
      block_id: request.id,
      approval_request_id: request.id,
      body: primaryActionLabel(request),
      service_slug: request.service_slug,
      agent_key_prefix: request.requester_label ?? request.requester_type,
      approval_mode: request.approval_mode,
      grant_duration_sec:
        request.approval_mode === "grant" ? grantDurationSec : null,
      expires_at: request.expires_at,
      decision: toDecision(request.status),
      decision_channel: toDecisionChannel(request.decision_channel),
    },
  };
}
