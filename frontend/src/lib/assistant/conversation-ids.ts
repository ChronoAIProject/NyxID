export const LEGACY_CONVERSATION_PREFIX = "chatc-";
export const TYPED_CONVERSATION_PREFIX = "nyxid-chat-";
export const DIRECT_CONVERSATION_PREFIX = "direct-";

export function isLegacyConversationId(id: string): boolean {
  return id.startsWith(LEGACY_CONVERSATION_PREFIX);
}

export function isTypedConversationId(id: string): boolean {
  return id.startsWith(TYPED_CONVERSATION_PREFIX);
}

export function isDirectConversationId(id: string): boolean {
  return id.startsWith(DIRECT_CONVERSATION_PREFIX);
}

export function assistantChatSurface({
  directEnabled,
  drafting,
  selectedConversationId,
}: {
  readonly directEnabled: boolean;
  readonly drafting: boolean;
  readonly selectedConversationId?: string;
}): "actor" | "direct" {
  return directEnabled &&
    (drafting ||
      !selectedConversationId ||
      isDirectConversationId(selectedConversationId))
    ? "direct"
    : "actor";
}
