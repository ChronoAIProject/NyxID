import type { AssistantVoiceApi } from "./AssistantVoiceClient";
import {
  accountActiveVoiceSessionSchema,
  activeVoiceSessionSchema,
  type VoiceSession,
} from "./voiceContracts";

export type ActiveVoiceCall = {
  session: VoiceSession;
  conversationId: string;
};

/** A bounded legacy scan cannot establish the state of older conversations. */
export class VoiceCallDiscoveryIncompleteError extends Error {
  constructor() {
    super("The account call could not be found in recent conversations.");
    this.name = "VoiceCallDiscoveryIncompleteError";
  }
}

/** Preserve the verified chat binding; minimal injected adapters use the current chat. */
export async function discoverActiveVoiceCall(
  api: Pick<AssistantVoiceApi, "active" | "accountActive">,
  conversationId: string,
  signal?: AbortSignal,
): Promise<ActiveVoiceCall | null> {
  if (api.accountActive) {
    const result = accountActiveVoiceSessionSchema.parse(
      await api.accountActive(conversationId, signal),
    );
    return result.session && result.conversation_id
      ? { session: result.session, conversationId: result.conversation_id }
      : null;
  }
  const result = activeVoiceSessionSchema.parse(
    await api.active(conversationId, signal),
  );
  return result.session ? { session: result.session, conversationId } : null;
}
