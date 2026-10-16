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
      return "Confirm this action";
  }
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

export function continuationText(row: AssistantAcknowledgement): string {
  switch (row.kind) {
    case "service":
      return `Approved: this chat may use ${row.service_name ?? row.service_slug ?? "the service"}. Continue.`;
    case "skills":
      return `Skill proposal reviewed: ${row.summary} NyxBot will resolve the exact version and ask you to confirm attaching it on a separate card. Do not request the skill again.`;
    case "operations":
      return `Approved: ${row.summary} Continue within the updated operation scope.`;
    case "account":
      return "Approved: account management for this chat. Continue.";
    case "action":
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
    case "model_not_configured":
    case "model_unavailable":
      return "The selected assistant model is unavailable.";
    case "session_busy":
    case "capacity_exceeded":
      return "The assistant is busy. Try again shortly.";
    default:
      return "The assistant could not complete this turn. Review completed actions before retrying.";
  }
}
