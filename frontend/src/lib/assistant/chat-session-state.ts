import type {
  ChatMessage,
  ChatSessionState,
  StoredChatMessage,
} from "@/lib/assistant/chat-types";
import type { RuntimeEventAccumulator } from "@/lib/assistant/runtime-event-semantics";

export function createClientId(): string {
  return (
    globalThis.crypto?.randomUUID?.() ??
    `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
  );
}

export function createDraftChatSession(): ChatSessionState {
  return {
    clientId: createClientId(),
    expectedTurnCount: 0,
    messages: [],
    status: "draft",
    title: "New chat",
  };
}

export function hydrateStoredMessages(
  messages: readonly StoredChatMessage[],
): ChatMessage[] {
  return messages.map((message) => ({
    authorId: message.authorId,
    authorName: message.authorName,
    content: message.content,
    error: message.error || undefined,
    id: message.id,
    role: message.role,
    status:
      message.error?.trim() || message.status === "error"
        ? "error"
        : "complete",
    thinking: message.thinking || undefined,
    timestamp: message.timestamp,
    turnId: message.turnId,
  }));
}

export function resolveStoredConversationStatus(
  messages: readonly StoredChatMessage[],
): ChatSessionState["status"] {
  const terminal =
    [...messages].reverse().find((message) => message.role === "assistant") ??
    messages.at(-1);
  return terminal?.status === "error" || Boolean(terminal?.error?.trim())
    ? "error"
    : "completed_text";
}

export function createChatMessage(
  role: ChatMessage["role"],
  content: string,
  status: ChatMessage["status"] = "complete",
): ChatMessage {
  return {
    content,
    id: createClientId(),
    role,
    status,
    timestamp: Date.now(),
  };
}

export function buildAssistantMessagePatch(
  accumulator: RuntimeEventAccumulator,
  status: ChatMessage["status"],
): Partial<ChatMessage> {
  return {
    content: accumulator.finalOutput || accumulator.assistantText,
    error: accumulator.errorText || undefined,
    events: [...accumulator.events],
    pendingApproval: accumulator.pendingApproval
      ? { ...accumulator.pendingApproval }
      : undefined,
    pendingRunIntervention: accumulator.pendingRunIntervention
      ? { ...accumulator.pendingRunIntervention }
      : undefined,
    status,
    steps: accumulator.steps.map((step) => ({ ...step })),
    thinking: accumulator.thinking,
    toolCalls: accumulator.toolCalls.map((toolCall) => ({ ...toolCall })),
  };
}

export function patchChatMessage(
  messages: readonly ChatMessage[],
  messageId: string,
  patch: Partial<ChatMessage>,
): ChatMessage[] {
  return messages.map((message) =>
    message.id === messageId ? { ...message, ...patch } : message,
  );
}

export function trimChatTitle(value: string): string {
  const normalized = value.trim().replace(/\s+/g, " ");
  if (!normalized) return "New chat";
  return normalized.length > 60 ? `${normalized.slice(0, 57)}...` : normalized;
}

export function stringField(
  record: Record<string, unknown> | null | undefined,
  key: string,
): string {
  const value = record?.[key];
  return typeof value === "string" ? value : "";
}
