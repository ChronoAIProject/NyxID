import {
  createAssistantVoiceApi,
  trustedVoiceOrigin,
} from "./assistantVoiceTransport";
import { requestJson, refreshAccessTokenIfNeeded } from "./http";
import { getAuthEpoch } from "../auth/authEpoch";
import { loadStoredAuthSession } from "../auth/sessionStore";
import { NYXID_API_BASE_URL, NYXID_FRONTEND_URL } from "../env";
import { createVoiceSocketFactory } from "../../features/assistant/voice/voiceSocket";
import { voiceIdentitySchema } from "../../features/assistant/voice/voiceContracts";

export async function assistantVoiceFeatures(signal?: AbortSignal) {
  const identity = voiceIdentitySchema.parse(
    await requestJson("/users/me", { signal }),
  );
  return identity.capabilities?.enabled_features ?? [];
}

/** Bind every request to the account that opened this voice surface. */
export function nativeVoiceEnvironment() {
  const configured = new URL(NYXID_FRONTEND_URL);
  if (configured.username || configured.password)
    throw new Error("Voice requires a trusted NyxID frontend address.");
  const origin = trustedVoiceOrigin(configured.origin);
  const epoch = getAuthEpoch();
  const api = createAssistantVoiceApi((path, options) => {
    if (getAuthEpoch() !== epoch)
      throw Object.assign(new Error("Voice account changed."), {
        name: "AbortError",
      });
    return requestJson(path, options);
  }, origin);
  return {
    api,
    origin,
    epoch,
    createSocket: createVoiceSocketFactory({
      apiBaseUrl: NYXID_API_BASE_URL,
      origin,
      epoch,
      getEpoch: getAuthEpoch,
      refresh: refreshAccessTokenIfNeeded,
      loadSession: loadStoredAuthSession,
      Socket: WebSocket as unknown as Parameters<
        typeof createVoiceSocketFactory
      >[0]["Socket"],
    }),
  };
}
