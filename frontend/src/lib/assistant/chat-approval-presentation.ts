import { redactAssistantDisplayText } from "./chat-display-safety";
import type { ApprovalCardContentBlock, ApprovalDecision } from "@/types/assistant";

type JsonRecord = Record<string, unknown>;

function asRecord(value: unknown): JsonRecord | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : null;
}

function stringField(record: JsonRecord, ...keys: string[]): string {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string") return value;
  }
  return "";
}

export function approvalRequestToBlock(
  request: JsonRecord & { readonly approvalRequestId: string },
  stateVersion: number,
  existing?: ApprovalCardContentBlock,
): ApprovalCardContentBlock {
  const presentation = asRecord(request.presentation);
  const presentationBody = [
    stringField(presentation ?? request, "actorLabel"),
    stringField(presentation ?? request, "action"),
    stringField(presentation ?? request, "target")
      ? `on ${stringField(presentation ?? request, "target")}`
      : "",
  ]
    .filter(Boolean)
    .join(" ");
  const toolName = stringField(request, "toolName");
  const body =
    stringField(request, "message", "body") ||
    presentationBody ||
    (toolName
      ? `The assistant wants to run ${redactAssistantDisplayText(toolName)}.`
      : "The assistant is requesting your approval to continue.");
  const grantDuration = request.grantDurationSec;
  return {
    type: "approval_card",
    block_id: existing?.block_id ?? `current-approval:${request.approvalRequestId}`,
    approval_request_id: request.approvalRequestId,
    body: redactAssistantDisplayText(body),
    service_slug: stringField(request, "serviceSlug", "service_slug"),
    agent_key_prefix: stringField(request, "agentKeyPrefix") || "aevatar",
    approval_mode: request.approvalMode === "grant" ? "grant" : "per_request",
    grant_duration_sec:
      typeof grantDuration === "number" ? grantDuration : null,
    expires_at: stringField(request, "expiresAt", "expires_at"),
    decision: existing?.decision ?? null,
    decision_channel: existing?.decision_channel ?? null,
    decision_submission: existing?.decision_submission ?? null,
    ...(stateVersion > 0 ? { state_version: stateVersion } : {}),
  };
}

export function approvalResolutionDecision(
  resolution: JsonRecord,
): ApprovalDecision | null {
  if (typeof resolution.approved === "boolean") {
    return resolution.approved ? "approved" : "denied";
  }
  switch (String(resolution.outcome ?? resolution.decision).toLowerCase()) {
    case "approved":
      return "approved";
    case "denied":
    case "rejected":
      return "denied";
    case "expired":
      return "expired";
    case "cancelled":
    case "canceled":
      return "cancelled";
    default:
      return null;
  }
}
