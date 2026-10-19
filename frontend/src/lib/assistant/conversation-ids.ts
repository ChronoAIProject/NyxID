export const LEGACY_CONVERSATION_PREFIX = "chatc-";
export const TYPED_CONVERSATION_PREFIX = "nyxid-chat-";
export const DIRECT_CONVERSATION_PREFIX = "direct-";
export const NYXAGENT_CONVERSATION_PREFIX = "nyxa-";

export function isNyxAgentConversationId(id: string): boolean {
  return /^nyxa-[a-f0-9]{32}$/.test(id);
}

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
  nyxagentEnabled = false,
  directEnabled,
  drafting,
  selectedConversationId,
}: {
  readonly nyxagentEnabled?: boolean;
  readonly directEnabled: boolean;
  readonly drafting: boolean;
  readonly selectedConversationId?: string;
}): "actor" | "direct" | "nyxagent" {
  if (!drafting && selectedConversationId && isNyxAgentConversationId(selectedConversationId)) return "nyxagent";
  if (nyxagentEnabled && (drafting || !selectedConversationId)) return "nyxagent";
  return directEnabled &&
    (drafting ||
      !selectedConversationId ||
      isDirectConversationId(selectedConversationId))
    ? "direct"
    : "actor";
}
