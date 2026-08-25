import { resolveAssistantAction } from "@/lib/assistant/action-registry";
import { actionOutcomeNote } from "@/lib/assistant/action-notes";
import type { ChatActionSummary } from "@/lib/assistant/chat-actor-state";
import type { ActionCardContentBlock } from "@/types/assistant";

function reportDisposition(summary: ChatActionSummary): string | undefined {
  return [...(summary.reports ?? [])]
    .reverse()
    .map((report) => report.disposition)
    .find((value): value is string => typeof value === "string");
}

function actionStatus(
  summary: ChatActionSummary,
): ActionCardContentBlock["status"] {
  if (summary.conflicted) return "conflicted";
  const disposition = reportDisposition(summary);
  if (disposition === "completed") return "completed";
  if (disposition === "declined") return "declined";
  if (["failed", "cancelled", "expired"].includes(disposition ?? "")) {
    return "failed";
  }
  return summary.supported ? "pending" : "unsupported";
}

export function actionSummaryBlock(
  summary: ChatActionSummary,
  override?: { readonly status?: string; readonly note?: string },
): ActionCardContentBlock {
  const resolved = summary.request
    ? resolveAssistantAction(summary.request)
    : { params: { variant: "unknown" as const }, supported: false };
  const computedStatus = actionStatus(summary);
  const overrideStatus = override?.status as
    | ActionCardContentBlock["status"]
    | undefined;
  const status = overrideStatus ?? computedStatus;
  return {
    type: "action_card",
    block_id: `current-action:${summary.actionRequestId}`,
    action: summary.action,
    action_request_id: summary.actionRequestId,
    origin_turn_id: summary.originTurnId,
    actor_id: summary.actorId || undefined,
    task_id: summary.taskId,
    step_id: summary.stepId,
    params: resolved.params,
    status,
    outcome_note: actionOutcomeNote(
      status,
      reportDisposition(summary),
      override?.note,
    ),
  };
}
