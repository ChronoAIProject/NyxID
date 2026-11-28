import allowedCardContinuation from "../../../../shared/allowed-card-continuation.json";
import { nyxAgentTransport } from "./nyxagent-transport";
import type { NyxAgentAcknowledgement, NyxAgentConversationAgent } from "@/schemas/assistant-nyxagent";
import { currentCreditsActor, isInsufficientCreditsCode, notifyCreditsDenied } from "@/lib/credits-denial";

export function notifyTurnCredits(
  conversationId: string,
  turnId: string,
  code: string | null,
  actorId: string | null,
): void {
  if (!isInsufficientCreditsCode(code)) return;
  // SSE and the history poll can both deliver one failure; the key dedupes.
  notifyCreditsDenied(
    { key: `assistant:nyxagent:${conversationId}:${turnId}`, payer: "self" },
    actorId,
  );
}

/**
 * A foreground send whose live failure may open the out-of-credits dialog.
 * `agent` is only used when the send starts a new thread.
 */
export function liveSend(
  conversationId: string | undefined,
  text: string,
  onAdopt: (id: string) => void,
  agent?: NyxAgentConversationAgent,
  attachmentIds?: string[],
  attachments?: readonly import("@/schemas/assistant-nyxagent").NyxAgentAttachment[],
  adoptExisting?: boolean,
  acknowledgementId?: string,
  model?: string,
) {
  const actorId = currentCreditsActor();
  const onFailed = (id: string, turnId: string, code: string) =>
    notifyTurnCredits(id, turnId, code, actorId);
  if (attachmentIds?.length || acknowledgementId) {
    return nyxAgentTransport.send(conversationId, text, onAdopt, onFailed, {
      ...(!conversationId && agent ? { agent } : {}),
      attachmentIds,
      attachments,
      adoptExisting,
      acknowledgementId,
      model,
    });
  }
  if (!model && (conversationId || !agent)) {
    return nyxAgentTransport.send(conversationId, text, onAdopt, onFailed);
  }
  return nyxAgentTransport.send(conversationId, text, onAdopt, onFailed, {
    ...(!conversationId && agent ? { agent } : {}),
    ...(model ? { model } : {}),
  });
}

/** The user-visible turn that resumes the assistant after an allowed card. */
export function continuationText(acknowledgement: NyxAgentAcknowledgement): string {
  return `${continuationInstruction(acknowledgement)} ${allowedCardContinuation}`;
}

function continuationInstruction(acknowledgement: NyxAgentAcknowledgement): string {
  switch (acknowledgement.kind) {
    case "service":
      return `Approved: this chat may use ${
        acknowledgement.service_name ?? acknowledgement.service_slug ?? "the service"
      }. Continue.`;
    case "skills":
      // An unrouted skills card attaches nothing and wakes nobody.
      return acknowledgement.decider === "orchestrator"
        ? `Skill proposal reviewed: ${acknowledgement.summary} NyxBot will resolve the exact version and apply the governing mode to attach it (automatic in Full access, otherwise an owner card). Do not request the skill again.`
        : `Skill proposal reviewed: ${acknowledgement.summary} Nothing was attached and nobody else was asked; the user adds the skill from this agent's Skills editor. Do not request the skill again.`;
    case "operations":
      return `Approved: ${acknowledgement.summary} Continue within the updated operation scope.`;
    case "machine":
    case "saved_login":
      return `Approved: ${acknowledgement.summary} Continue within the updated machine and saved-login grants.`;
    case "account":
      return "Approved: account management for this chat. Continue.";
    case "action":
      if (acknowledgement.authored_skill) return `The skill publication was allowed: ${acknowledgement.summary}. Check the attached skills and tell the user which skill/version was attached to which agent. Do not publish it again.`;
      return `Confirmed: ${acknowledgement.summary} (acknowledgement_id ${acknowledgement.id}). Retry it now.`;
  }
}

/** One continuation for every card allowed while a turn was running. */
export function continuationForAll(acknowledgements: readonly NyxAgentAcknowledgement[]): string {
  return acknowledgements.map(continuationText).join("\n");
}

