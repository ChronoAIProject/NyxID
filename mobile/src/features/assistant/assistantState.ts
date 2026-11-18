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
      return `Allow this ${row.decider === "orchestrator" ? "agent" : "chat"} to use ${row.service_name ?? row.service_slug ?? "this service"}?`;
    case "account":
      return row.decider === "orchestrator"
        ? "Allow this agent to read your NyxID account?"
        : "Allow this chat to manage your NyxID account (keys, channel bots, services, nodes and approval settings)?";
    case "operations":
      return "Confirm this specialist's operation permissions";
    case "skills":
      return "Confirm this skill proposal";
    case "action":
    case "machine":
    case "saved_login":
      return "Confirm this action";
  }
}

/** Match Web's persisted decision labels without implying an action succeeded. */
export function acknowledgementDecisionLabel(
  row: AssistantAcknowledgement,
): string {
  if (row.decided_by === "access_mode")
    return "Allowed automatically · Full access";
  const status = {
    pending: "Pending",
    allowed: "Allowed",
    denied: "Denied",
    expired: "Expired",
    used: "Used",
  }[row.status];
  if (!status) return "Status unavailable";
  if ((row.status !== "allowed" && row.status !== "denied") || !row.decided_by)
    return status;
  const by = row.decided_by === "orchestrator" ? "NyxBot" : "you";
  const reason = row.reason?.trim();
  return reason ? `${status} by ${by}: ${reason}` : `${status} by ${by}`;
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
  return `${continuationInstruction(row)} Then tell the user in one or two plain sentences what you did and the result.`;
}

function continuationInstruction(row: AssistantAcknowledgement): string {
  switch (row.kind) {
    case "service":
      return `Approved: this chat may use ${row.service_name ?? row.service_slug ?? "the service"}. Continue.`;
    case "skills":
      // An unrouted skills card attaches nothing and wakes nobody.
      return row.decider === "orchestrator"
        ? `Skill proposal reviewed: ${row.summary} NyxBot will resolve the exact version and apply the governing mode to attach it (automatic in Full access, otherwise an owner card). Do not request the skill again.`
        : `Skill proposal reviewed: ${row.summary} Nothing was attached and nobody else was asked; the user adds the skill from this agent's Skills editor. Do not request the skill again.`;
    case "operations":
      return `Approved: ${row.summary} Continue within the updated operation scope.`;
    case "machine":
    case "saved_login":
      return `Approved: ${row.summary} Continue within the updated machine and saved-login grants.`;
    case "account":
      return "Approved: account management for this chat. Continue.";
    case "action":
      if (row.authored_skill)
        return `The skill publication was allowed: ${row.summary}. Check the attached skills and tell the user which skill/version was attached to which agent. Do not publish it again.`;
      return `Confirmed: ${row.summary} (acknowledgement_id ${row.id}). Retry it now.`;
  }
}

export function storedAssistantError(code: string | null): string {
  switch (code) {
    case "cancelled":
      return "Stopped. Review completed actions before continuing.";
    case "outcome_unknown":
      return "An operation may have taken effect. Check its result before trying again.";
    case "insufficient_credits":
      return "There aren't enough credits to run this turn.";
    case "tool_budget_exhausted":
    case "turn_timeout":
      return "This task reached its continuation limit. Its context is saved; send guidance to continue.";
    case "continuation_no_progress":
      return "Paused because the task repeated without progress. Send new guidance to continue.";
    case "model_policy_denied":
      return "An approval policy denies this model call. Review the model service's Deny rules in Approval settings; for organization policy, ask its administrator.";
    case "model_org_approval_required":
      return "The organization requires approval for this model service. Ask its administrator to review the policy in Approval settings.";
    case "model_approval_required":
      return "This model request requires approval that this thread cannot present. Review Approval settings before retrying.";
    case "model_not_configured":
    case "model_unavailable":
      return "This agent is unavailable.";
    case "session_busy":
    case "capacity_exceeded":
      return "The assistant is busy. Try again shortly.";
    default:
      return "The assistant could not complete this turn. Review completed actions before retrying.";
  }
}
