import { create } from "zustand";
import {
  createJSONStorage,
  persist,
  type StateStorage,
} from "zustand/middleware";
import {
  assistantUpstreamEnvelopeListSchema,
  assistantWireLogPersistedSchema,
  type AssistantUpstreamEnvelope,
  type AssistantWireLogEntry,
} from "@/schemas/assistant-wire-log";

export const ASSISTANT_WIRE_LOG_STORAGE_KEY =
  "nyxid.assistant.wirelog.v1";
const MAX_ENTRIES = 100;
const MAX_SERIALIZED_BYTES = 2 * 1024 * 1024;

interface AssistantWireLogState {
  readonly captureEnabled: boolean;
  readonly entries: readonly AssistantWireLogEntry[];
  readonly setCaptureEnabled: (enabled: boolean) => void;
  readonly record: (
    envelope: AssistantUpstreamEnvelope,
    kind: AssistantWireLogEntry["kind"],
    status: number,
  ) => void;
  readonly clear: () => void;
  readonly reset: () => void;
}

const EMPTY_WIRE_LOG = {
  captureEnabled: false,
  entries: [] as readonly AssistantWireLogEntry[],
} as const;

function serializedSize(
  captureEnabled: boolean,
  entries: readonly AssistantWireLogEntry[],
): number {
  return new TextEncoder().encode(JSON.stringify({ captureEnabled, entries }))
    .byteLength;
}

function boundedEntries(
  captureEnabled: boolean,
  entries: readonly AssistantWireLogEntry[],
): readonly AssistantWireLogEntry[] {
  const bounded = entries.slice(-MAX_ENTRIES);
  while (
    bounded.length > 0 &&
    serializedSize(captureEnabled, bounded) > MAX_SERIALIZED_BYTES
  ) {
    bounded.shift();
  }
  return bounded;
}

function isQuotaExceeded(error: unknown): boolean {
  return (
    error instanceof DOMException &&
    (error.name === "QuotaExceededError" || error.name === "NS_ERROR_DOM_QUOTA_REACHED")
  );
}

const quotaSafeStorage: StateStorage = {
  getItem: (name) => localStorage.getItem(name),
  removeItem: (name) => localStorage.removeItem(name),
  setItem: (name, value) => {
    try {
      localStorage.setItem(name, value);
      return;
    } catch (error) {
      if (!isQuotaExceeded(error)) return;
    }

    try {
      const persisted = JSON.parse(value) as {
        state?: { entries?: unknown[] };
      };
      const entries = persisted.state?.entries;
      if (!Array.isArray(entries) || entries.length === 0) return;
      entries.shift();
      localStorage.setItem(name, JSON.stringify(persisted));
    } catch {
      // Persistence is best-effort; the in-memory ring remains available.
    }
  },
};

// Payloads belong in localStorage: unlike cookies it survives browser restarts
// without a 4KB cap or attaching captured prompts to every HTTP request.
export const useAssistantWireLogStore = create<AssistantWireLogState>()(
  persist(
    (set) => ({
      ...EMPTY_WIRE_LOG,
      setCaptureEnabled: (captureEnabled) => set({ captureEnabled }),
      record: (envelope, kind, status) => {
        set((state) => {
          const entry: AssistantWireLogEntry = {
            ...envelope,
            id: crypto.randomUUID(),
            ts: Date.now(),
            kind,
            status,
          };
          return {
            entries: boundedEntries(state.captureEnabled, [
              ...state.entries,
              entry,
            ]),
          };
        });
      },
      clear: () => set({ entries: [] }),
      reset: () => {
        set(EMPTY_WIRE_LOG);
        if (typeof localStorage !== "undefined") {
          localStorage.removeItem(ASSISTANT_WIRE_LOG_STORAGE_KEY);
        }
      },
    }),
    {
      name: ASSISTANT_WIRE_LOG_STORAGE_KEY,
      version: 1,
      storage: createJSONStorage(() => quotaSafeStorage),
      partialize: ({ captureEnabled, entries }) => ({
        captureEnabled,
        entries,
      }),
      merge: (persisted, current) => {
        const parsed = assistantWireLogPersistedSchema.safeParse(persisted);
        if (parsed.success) return { ...current, ...parsed.data };
        if (typeof localStorage !== "undefined") {
          localStorage.removeItem(ASSISTANT_WIRE_LOG_STORAGE_KEY);
        }
        return current;
      },
    },
  ),
);

function decodeBase64Utf8(value: string): string {
  const binary = atob(value);
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

export function captureAssistantWireLogHeader(
  value: string | null | undefined,
  kind: AssistantWireLogEntry["kind"],
  status: number,
): void {
  if (!value) return;
  try {
    const parsed = assistantUpstreamEnvelopeListSchema.safeParse(
      JSON.parse(decodeBase64Utf8(value)),
    );
    if (!parsed.success) return;
    for (const envelope of parsed.data) {
      useAssistantWireLogStore.getState().record(envelope, kind, status);
    }
  } catch {
    // A malformed debug header must never affect the assistant request.
  }
}
