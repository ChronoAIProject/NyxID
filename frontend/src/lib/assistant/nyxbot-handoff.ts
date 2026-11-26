import {
  nyxBotHandoffSchema,
  type NyxBotHandoff,
} from "@/schemas/nyxbot-handoff";
import { nyxAgentTransport } from "./nyxagent-transport";
import { handoffReceipt, saveHandoffReceipt } from "./nyxbot-handoff-receipts";
import { isAdmittedSendError, markAdmitted } from "./send-admission";
import type { NyxAgentConversationAgent } from "@/schemas/assistant-nyxagent";

/** Called only by the card's click handler. The server selects the user's NyxBot. */
async function startHandoff(
  proposal: NyxBotHandoff,
  sourceConversationId: string | undefined,
  onAdopt: (id: string) => void,
  receiptKey: string,
  nyxBot: NyxAgentConversationAgent,
): Promise<void> {
  const handoff = nyxBotHandoffSchema.parse(proposal);
  if (!nyxBot || nyxBot.kind !== "nyxbot" || nyxBot.destroyed)
    throw new Error("NyxBot is unavailable. Refresh your chats and try again.");
  const previous = handoffReceipt(receiptKey);
  if (previous?.state === "started") {
    onAdopt(previous.conversationId);
    return;
  }
  if (previous)
    throw markAdmitted(
      new Error(
        "This task may already have started. Check your chats before trying again.",
      ),
    );
  saveHandoffReceipt(receiptKey, { state: "pending" });
  const source = sourceConversationId
    ? `\n\n[Specialist chat](/assistant?c=${encodeURIComponent(sourceConversationId)})`
    : "";
  // Select the owner's NyxBot explicitly and omit model: both the agent's
  // configured profile and ordinary user-turn approval policies remain live.
  try {
    await nyxAgentTransport.send(
      undefined,
      `${handoff.task}${source}`,
      (id) => {
        saveHandoffReceipt(receiptKey, {
          state: "started",
          conversationId: id,
        });
        onAdopt(id);
      },
      undefined,
      { agent: nyxBot, clientDraftKey: `handoff:${receiptKey}` },
    );
  } catch (error) {
    if (handoffReceipt(receiptKey)?.state !== "started") {
      saveHandoffReceipt(
        receiptKey,
        isAdmittedSendError(error) ? { state: "accepted" } : undefined,
      );
    }
    throw error;
  }
}

/** Web Locks serialize the same saved card across tabs; memory state covers older clients. */
export async function startNyxBotHandoff(
  ...args: Parameters<typeof startHandoff>
): Promise<void> {
  if (typeof navigator !== "undefined" && navigator.locks) {
    return navigator.locks.request(`nyxid-handoff-${args[3]}`, () =>
      startHandoff(...args),
    );
  }
  return startHandoff(...args);
}
