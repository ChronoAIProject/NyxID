import allowedCardContinuation from "../../../../shared/allowed-card-continuation.json";
import { translateApp } from "../../i18n/appTranslation";
import type {
  AssistantAcknowledgement,
  AssistantHistory,
  AssistantMessage,
} from "./assistantTypes";

/** Keep older loaded pages when polling the latest server snapshot. */
export function mergeAssistantHistory(
  previous: AssistantHistory | undefined,
  page: AssistantHistory,
  olderPage = false,
): AssistantHistory {
  if (!previous || previous.conversation.id !== page.conversation.id)
    return page;
  const messages = new Map(
    previous.messages.map((message) => [message.seq, message]),
  );
  for (const message of page.messages) messages.set(message.seq, message);
  const previousEnd = previous.messages.at(-1)?.seq;
  const pageStart = page.messages[0]?.seq;
  // A long background interval can create more than one latest page. Keep
  // its cursor until the missing middle has been fetched, even if the old
  // history already reached the start of the conversation.
  const newGap =
    previousEnd !== undefined &&
    pageStart !== undefined &&
    pageStart > previousEnd + 1;
  return {
    ...page,
    before_seq: olderPage || newGap ? page.before_seq : previous.before_seq,
    messages: [...messages.values()].sort((a, b) => a.seq - b.seq),
  };
}

export function acknowledgementTitle(row: AssistantAcknowledgement): string {
  switch (row.kind) {
    case "service":
      return (row.decider === "orchestrator" ? translateApp("Allow this agent to use {{service}}?", { service: row.service_name ?? row.service_slug ?? translateApp("this service") }) : translateApp("Allow this chat to use {{service}}?", { service: row.service_name ?? row.service_slug ?? translateApp("this service") }));
    case "account":
      return row.decider === "orchestrator"
        ? translateApp("Allow this agent to read your NyxID account?")
        : translateApp(
            "Allow this chat to manage your NyxID account (keys, channel bots, services, nodes and approval settings)?",
          );
    case "operations":
      return translateApp("Confirm this specialist's operation permissions");
    case "skills":
      return translateApp("Confirm this skill proposal");
    case "action":
    case "machine":
    case "saved_login":
      return translateApp("Confirm this action");
  }
}

/** Match Web's persisted decision labels without implying an action succeeded. */
export function acknowledgementDecisionLabel(
  row: AssistantAcknowledgement,
): string {
  if (row.decided_by === "access_mode")
    return translateApp("Allowed automatically · Full access");
  const status = {
    pending: translateApp("Pending"),
    allowed: translateApp("Allowed"),
    denied: translateApp("Denied"),
    expired: translateApp("Expired"),
    used: translateApp("Used"),
  }[row.status];
  if (!status) return translateApp("Status unavailable");
  if ((row.status !== "allowed" && row.status !== "denied") || !row.decided_by)
    return status;
  const by = row.decided_by === "orchestrator" ? "NyxBot" : translateApp("you");
  const reason = row.reason?.trim();
  return reason ? translateApp("{{status}} by {{by}}: {{reason}}", {status: status,
        by: by,
        reason: reason,
      })
    : translateApp("{{status}} by {{by}}", { status: status, by: by});
}

export function retryInput(
  history: AssistantHistory | undefined,
  reply: AssistantMessage,
): string | undefined {
  if (
    reply.role !== "assistant" ||
    reply.status !== "failed" ||
    reply.error_code === "cancelled" ||
    reply.error_code === "outcome_unknown"
  )
    return undefined;
  if (reply.recovery)
    return reply.recovery.status === "available"
      ? (reply.recovery.text ?? undefined)
      : undefined;
  return history?.messages.find(
    (message) =>
      message.turn_id === reply.turn_id &&
      message.role === "user" &&
      !message.steering,
  )?.text;
}

export function needsClientContinuation(
  row: AssistantAcknowledgement,
): boolean {
  return (
    row.status === "allowed" &&
    row.decider !== "orchestrator" &&
    row.continuation_owner !== "server" &&
    !row.trigger_run_id
  );
}

/** The turn that resumes the assistant after an allowed card; matches the web. */
export function continuationText(row: AssistantAcknowledgement): string {
  return translateApp(`{{continuationInstruction}} ${allowedCardContinuation}`, {
    continuationInstruction: continuationInstruction(row),
  });
}

function continuationInstruction(row: AssistantAcknowledgement): string {
  switch (row.kind) {
    case "service":
      return translateApp("Approved: this chat may use {{service}}. Continue.", {
        service: row.service_name ?? row.service_slug ?? "the service",
      });
    case "skills":
      // An unrouted skills card attaches nothing and wakes nobody.
      return row.decider === "orchestrator"
        ? translateApp(
            "Skill proposal reviewed: {{summary}} NyxBot will resolve the exact version and apply the governing mode to attach it (automatic in Full access, otherwise an owner card). Do not request the skill again.",
            { summary: row.summary},
          )
        : translateApp(
            "Skill proposal reviewed: {{summary}} Nothing was attached and nobody else was asked; the user adds the skill from this agent's Skills editor. Do not request the skill again.",
            { summary: row.summary},
          );
    case "operations":
      return translateApp(
        "Approved: {{summary}} Continue within the updated operation scope.",
        { summary: row.summary},
      );
    case "machine":
    case "saved_login":
      return translateApp(
        "Approved: {{summary}} Continue within the updated machine and saved-login grants.",
        { summary: row.summary},
      );
    case "account":
      return translateApp(
        "Approved: account management for this chat. Continue.",
      );
    case "action":
      if (row.authored_skill)
        return translateApp(
          "The skill publication was allowed: {{summary}}. Check the attached skills and tell the user which skill/version was attached to which agent. Do not publish it again.",
          { summary: row.summary},
        );
      return translateApp(
        "Confirmed: {{summary}} (acknowledgement_id {{id}}). Retry it now.",
        { summary: row.summary, id: row.id},
      );
  }
}

export function storedAssistantError(code: string | null): string {
  switch (code) {
    case "cancelled":
      return translateApp(
        "Stopped. Review completed actions before continuing.",
      );
    case "outcome_unknown":
      return translateApp(
        "An operation may have taken effect. Check its result before trying again.",
      );
    case "insufficient_credits":
      return translateApp("There aren't enough credits to run this turn.");
    case "tool_budget_exhausted":
    case "turn_timeout":
      return translateApp(
        "This task reached its continuation limit. Its context is saved; send guidance to continue.",
      );
    case "continuation_no_progress":
      return translateApp(
        "Paused because the task repeated without progress. Send new guidance to continue.",
      );
    case "model_policy_denied":
      return translateApp(
        "An approval policy denies this model call. Review the model service's Deny rules in Approval settings; for organization policy, ask its administrator.",
      );
    case "model_org_approval_required":
      return translateApp(
        "The organization requires approval for this model service. Ask its administrator to review the policy in Approval settings.",
      );
    case "model_approval_required":
      return translateApp(
        "This model request requires approval that this thread cannot present. Review Approval settings before retrying.",
      );
    case "model_not_configured":
    case "model_unavailable":
      return translateApp("This agent is unavailable.");
    case "session_busy":
    case "capacity_exceeded":
      return translateApp("The assistant is busy. Try again shortly.");
    default:
      return translateApp(
        "The assistant could not complete this turn. Review completed actions before retrying.",
      );
  }
}
