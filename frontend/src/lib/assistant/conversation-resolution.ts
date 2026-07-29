import type { Conversation } from "@/types/assistant";

export function resolveAssistantConversationId({
  explicitConversationId,
  boundConversationId,
  entryScreen,
  conversationsResolved,
  conversations,
}: {
  readonly explicitConversationId?: string;
  readonly boundConversationId?: string;
  readonly entryScreen: string | null;
  readonly conversationsResolved: boolean;
  readonly conversations: readonly Conversation[];
}): string | undefined {
  if (
    explicitConversationId &&
    (!conversationsResolved ||
      conversations.some(
        (conversation) => conversation.id === explicitConversationId,
      ))
  ) {
    return explicitConversationId;
  }
  if (
    boundConversationId &&
    conversations.some(
      (conversation) => conversation.id === boundConversationId,
    )
  ) {
    return boundConversationId;
  }
  if (entryScreen !== null) return undefined;
  return conversations[0]?.id;
}
