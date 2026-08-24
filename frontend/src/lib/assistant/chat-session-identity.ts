import {
  applyRuntimeEvent,
  createRuntimeEventAccumulator,
} from "./runtime-event-semantics";
import type {
  ChatMessage,
  ConversationLlmPreferences,
  ConversationRuntimeIdentity,
  ConversationSessionMeta,
  ConversationSessionSnapshot,
} from "./chat-types";

function trimConversationValue(value?: string | null): string | undefined {
  const normalized = String(value || "").trim();
  return normalized || undefined;
}

function trimOptional(value?: string): string | undefined {
  const normalized = value?.trim();
  return normalized ? normalized : undefined;
}

function hasRuntimeIdentityValue(value: ConversationRuntimeIdentity): boolean {
  return Boolean(value.actorId || value.commandId || value.runId);
}

function hasLlmPreferenceValue(value: ConversationLlmPreferences): boolean {
  return Boolean(value.llmModel || value.llmRoute);
}

function mergeRuntimeIdentity(
  ...values: Array<ConversationRuntimeIdentity | undefined>
): ConversationRuntimeIdentity {
  return values.reduce<ConversationRuntimeIdentity>(
    (current, value) => ({
      actorId: current.actorId || trimOptional(value?.actorId),
      commandId: current.commandId || trimOptional(value?.commandId),
      runId: current.runId || trimOptional(value?.runId),
    }),
    {},
  );
}

export function readConversationRuntimeIdentity(
  meta?: ConversationSessionMeta,
): ConversationRuntimeIdentity {
  return mergeRuntimeIdentity(meta?.session?.runtime, meta);
}

export function readConversationPreferences(
  meta?: ConversationSessionMeta,
): ConversationLlmPreferences {
  return {
    llmModel:
      trimConversationValue(meta?.session?.preferences?.llmModel) ||
      trimConversationValue(meta?.llmModel) ||
      undefined,
    llmRoute:
      trimConversationValue(meta?.session?.preferences?.llmRoute) ||
      trimConversationValue(meta?.llmRoute) ||
      undefined,
  };
}

export function deriveRuntimeIdentityFromMessages(
  messages: readonly Pick<ChatMessage, "events">[],
): ConversationRuntimeIdentity {
  const accumulator = createRuntimeEventAccumulator();

  for (const message of messages) {
    for (const event of message.events ?? []) {
      applyRuntimeEvent(accumulator, event);
    }
  }

  return mergeRuntimeIdentity(accumulator);
}

export function resolveConversationRuntimeIdentity(input?: {
  messages?: readonly Pick<ChatMessage, "events">[];
  meta?: ConversationSessionMeta;
  session?: ConversationRuntimeIdentity;
}): ConversationRuntimeIdentity {
  return mergeRuntimeIdentity(
    input?.session,
    readConversationRuntimeIdentity(input?.meta),
    input?.messages
      ? deriveRuntimeIdentityFromMessages(input.messages)
      : undefined,
  );
}

export function buildConversationSessionSnapshot(
  messages: readonly Pick<ChatMessage, "events">[],
  session: ConversationRuntimeIdentity,
  options?: ConversationLlmPreferences,
): ConversationSessionSnapshot | undefined {
  const runtime = resolveConversationRuntimeIdentity({
    messages,
    session,
  });
  const preferences: ConversationLlmPreferences = {
    llmModel: trimConversationValue(options?.llmModel) || undefined,
    llmRoute: trimConversationValue(options?.llmRoute) || undefined,
  };

  return {
    ...(hasLlmPreferenceValue(preferences) ? { preferences } : {}),
    ...(hasRuntimeIdentityValue(runtime) ? { runtime } : {}),
  };
}
