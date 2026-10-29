import type { AssistantParams } from "./navigationTypes";

let draftSequence = 0;

/** Each compose action starts empty without moving another thread's draft. */
export function newAssistantChatParams(agentId?: string): AssistantParams {
  return {
    home: false,
    agentId,
    conversationId: undefined,
    acknowledgementId: undefined,
    draft: true,
    draftId: `draft-${Date.now()}-${++draftSequence}`,
    openLatest: false,
  };
}

export function assistantHomeParams(): AssistantParams {
  return {
    home: true,
    agentId: undefined,
    conversationId: undefined,
    acknowledgementId: undefined,
    draftId: undefined,
    draft: false,
    openLatest: false,
  };
}
