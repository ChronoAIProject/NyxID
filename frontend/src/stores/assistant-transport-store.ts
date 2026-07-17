/**
 * Which aevatar chat API the assistant streams from — a per-browser
 * preference, not account state, so it lives in localStorage alongside the
 * other client-side policy stores.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

export type AssistantTransportMode = "chat" | "completions" | "workflow";

export interface AssistantTransportState {
  /** `chat` = nyxid-chat conversations (AG-UI stream, server-side history);
   * `completions` = OpenAI-compatible /v1/chat/completions (stateless);
   * `workflow` = ad-hoc workflow chat /api/chat (one prompt per run). */
  mode: AssistantTransportMode;
  setMode(mode: AssistantTransportMode): void;
}

export const useAssistantTransportStore = create<AssistantTransportState>()(
  persist(
    (set) => ({
      mode: "chat",
      setMode(mode) {
        set({ mode });
      },
    }),
    {
      name: "nyxid-assistant-transport-mode",
      version: 1,
    },
  ),
);
