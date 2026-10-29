import { z } from "zod";
import { conversationIdSchema } from "../../features/assistant/assistantTypes";
import {
  activeVoiceSessionSchema,
  voiceControlRequestSchema,
  voiceIdentitySchema,
  voiceOptionsSchema,
  voicePreferencesSchema,
  voiceRequestSchema,
  voiceSessionSchema,
  voiceSettingsSchema,
  voiceStartRequestSchema,
  type VoiceControlRequest,
  type VoicePreferences,
  type VoiceStartRequest,
} from "../../features/assistant/voice/voiceContracts";
import type { RequestOptions } from "./authenticatedTransport";

const ROOT = "/assistant/nyxagent";
const localId = z.string().uuid();
type JsonRequest = (path: string, options?: RequestOptions) => Promise<unknown>;

/** Only trusted app configuration supplies this origin, never a scanned URL. */
export function trustedVoiceOrigin(value: string): string {
  const invalid = () => new Error("Voice requires a trusted frontend origin.");
  if (
    typeof value !== "string" ||
    !/^https?:\/\/[^/?#\\@]+\/?$/iu.test(value) ||
    value.trim() !== value
  )
    throw invalid();
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw invalid();
  }
  if (
    !["https:", "http:"].includes(url.protocol) ||
    !url.hostname ||
    url.username ||
    url.password ||
    url.pathname !== "/" ||
    url.search ||
    url.hash ||
    /[\s\\]/u.test(value)
  )
    throw invalid();
  return url.origin;
}

export function assistantVoiceSessionsPath(conversationId: string): string {
  return `${ROOT}/conversations/${conversationIdSchema.parse(conversationId)}/voice-sessions`;
}

export function assistantVoiceStreamPath(
  conversationId: string,
  sessionId: string,
): string {
  return `${assistantVoiceSessionsPath(conversationId)}/${localId.parse(sessionId)}/stream`;
}

export function assistantDictationPath(conversationId: string): string {
  return `${ROOT}/conversations/${conversationIdSchema.parse(conversationId)}/dictation`;
}

/** The human-session HTTP implementation is injected; no tokens enter media. */
export function createAssistantVoiceApi(
  request: JsonRequest,
  trustedOrigin: string,
) {
  const origin = trustedVoiceOrigin(trustedOrigin);
  const sameId = <T extends { id: string }>(expected: string, result: T): T => {
    if (result.id !== expected)
      throw new Error(
        "The server returned a different voice resource. Please reload.",
      );
    return result;
  };
  const requestPath = (conversationId: string, requestId: string) =>
    `${ROOT}/conversations/${conversationIdSchema.parse(conversationId)}/voice-requests/${localId.parse(requestId)}`;

  async function identity(signal?: AbortSignal) {
    return voiceIdentitySchema.parse(await request("/users/me", { signal }));
  }
  return {
    identity,
    async features(signal?: AbortSignal) {
      return (await identity(signal)).capabilities?.enabled_features ?? [];
    },
    async settings(signal?: AbortSignal) {
      return voiceSettingsSchema.parse(
        await request(`${ROOT}/settings`, { signal }),
      );
    },
    async savePreferences(preferences: VoicePreferences, signal?: AbortSignal) {
      const voice = voicePreferencesSchema.parse(preferences);
      return voiceSettingsSchema.parse(
        await request(`${ROOT}/settings`, {
          method: "PUT",
          body: { voice },
          signal,
        }),
      );
    },
    async options(conversationId: string, signal?: AbortSignal) {
      const id = conversationIdSchema.parse(conversationId);
      return voiceOptionsSchema.parse(
        await request(`${ROOT}/voice/options?conversation_id=${id}`, {
          signal,
        }),
      );
    },
    async dictationOptions(conversationId: string, signal?: AbortSignal) {
      const id = conversationIdSchema.parse(conversationId);
      return voiceOptionsSchema.parse(
        await request(`${ROOT}/dictation/options?conversation_id=${id}`, {
          signal,
        }),
      );
    },
    async start(
      conversationId: string,
      input: VoiceStartRequest,
      signal?: AbortSignal,
    ): Promise<unknown> {
      const path = assistantVoiceSessionsPath(conversationId);
      const body = voiceStartRequestSchema.parse(input);
      // The controller validates this response after preserving any valid owned
      // session. Parsing here would discard that identity if the media answer is
      // malformed, preventing durable End after a failed or cancelled start.
      return request(path, {
        method: "POST",
        headers: { Origin: origin },
        body,
        signal,
      });
    },
    async active(conversationId: string, signal?: AbortSignal) {
      return activeVoiceSessionSchema.parse(
        await request(`${assistantVoiceSessionsPath(conversationId)}/active`, {
          signal,
        }),
      );
    },
    async status(
      conversationId: string,
      sessionId: string,
      signal?: AbortSignal,
    ) {
      const path = `${assistantVoiceSessionsPath(conversationId)}/${localId.parse(sessionId)}`;
      return sameId(
        sessionId,
        voiceSessionSchema.parse(await request(path, { signal })),
      );
    },
    async control(
      conversationId: string,
      sessionId: string,
      input: VoiceControlRequest,
      signal?: AbortSignal,
    ) {
      const path = `${assistantVoiceSessionsPath(conversationId)}/${localId.parse(sessionId)}/control`;
      const body = voiceControlRequestSchema.parse(input);
      return sameId(
        sessionId,
        voiceSessionSchema.parse(
          await request(path, {
            method: "POST",
            headers: { Origin: origin },
            body,
            signal,
          }),
        ),
      );
    },
    async request(
      conversationId: string,
      requestId: string,
      signal?: AbortSignal,
    ) {
      return sameId(
        requestId,
        voiceRequestSchema.parse(
          await request(requestPath(conversationId, requestId), { signal }),
        ),
      );
    },
    async cancelRequest(
      conversationId: string,
      requestId: string,
      signal?: AbortSignal,
    ): Promise<void> {
      await request(`${requestPath(conversationId, requestId)}/cancel`, {
        method: "POST",
        body: {},
        signal,
      });
    },
  };
}

export type AssistantVoiceApi = ReturnType<typeof createAssistantVoiceApi>;
