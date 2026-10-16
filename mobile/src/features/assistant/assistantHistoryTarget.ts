import { acknowledgementSchema, conversationIdSchema } from "./assistantTypes";

export type AssistantHistoryTarget = {
  conversationId: string;
  acknowledgementId?: string;
};

export function selectAssistantHistoryTarget(
  current: AssistantHistoryTarget | undefined,
  conversationId?: string,
  acknowledgementId?: string,
): AssistantHistoryTarget | undefined {
  if (!conversationId) return undefined;
  if (
    acknowledgementId === undefined &&
    current?.conversationId === conversationId
  ) {
    return current;
  }
  return acknowledgementId === undefined
    ? { conversationId }
    : { conversationId, acknowledgementId };
}

export function acknowledgementForConversation(
  target: AssistantHistoryTarget | undefined,
  conversationId: string,
): string | undefined {
  return target?.conversationId === conversationId
    ? target.acknowledgementId
    : undefined;
}

export function assistantHistoryRequestPath(
  id: string,
  beforeSeq?: number,
  acknowledgementId?: string,
): string {
  const conversationId = conversationIdSchema.parse(id);
  const query = new URLSearchParams({ limit: "50" });
  if (beforeSeq) query.set("before_seq", String(beforeSeq));
  if (acknowledgementId !== undefined) {
    query.set("card", acknowledgementSchema.shape.id.parse(acknowledgementId));
  }
  return `/assistant/nyxagent/conversations/${conversationId}?${query}`;
}
