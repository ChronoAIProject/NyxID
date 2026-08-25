import { AGUIEventType, parseCustomEvent } from "@/lib/assistant/agui-types";
import type { ChatActorProjection } from "@/lib/assistant/chat-actor-state";
import {
  patchChatMessage,
  stringField,
} from "@/lib/assistant/chat-session-state";
import type { ChatMessage, ChatSessionState } from "@/lib/assistant/chat-types";

export class ReaderStoppedError extends Error {
  constructor() {
    super("Assistant observation stopped.");
    this.name = "ReaderStoppedError";
  }
}

export class ChatProgressTimeoutError extends Error {
  constructor() {
    super("The assistant stopped making progress.");
    this.name = "ChatProgressTimeoutError";
  }
}

export class ChatStartTimeoutError extends Error {
  constructor() {
    super("The assistant did not start replying in time. Try again.");
    this.name = "ChatStartTimeoutError";
  }
}

export function chatErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function isKeepaliveEvent(event: { readonly type: string }): boolean {
  return (
    event.type === AGUIEventType.CUSTOM &&
    parseCustomEvent(event as never).name === "aevatar.nyxid_chat.keepalive"
  );
}

export function isRunStoppedEvent(event: { readonly type: string }): boolean {
  return event.type === "RUN_STOPPED";
}

export function updateSessionMessage(
  session: ChatSessionState,
  messageId: string,
  patch: Partial<ChatMessage>,
): ChatSessionState {
  return {
    ...session,
    messages: patchChatMessage(session.messages, messageId, patch),
  };
}

export function currentActorTurnId(
  projection: ChatActorProjection | null,
): string {
  return stringField(
    projection?.activeTurn ?? projection?.latestTurn,
    "turnId",
  );
}
