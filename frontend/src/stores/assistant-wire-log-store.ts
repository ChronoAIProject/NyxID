import { create } from "zustand";
import {
  createJSONStorage,
  persist,
  type StateStorage,
} from "zustand/middleware";
import {
  assistantUpstreamEnvelopeHeaderDecoderSchema,
  assistantWireLogPersistedSchema,
  type AssistantUpstreamEnvelope,
  type AssistantWireCapture,
  type AssistantWireCaptureOutcome,
  type AssistantWireLine,
  type AssistantWireLogExchange,
  type PersistedAssistantWireLogExchange,
} from "@/schemas/assistant-wire-log";

export const ASSISTANT_WIRE_LOG_STORAGE_KEY = "nyxid.assistant.wirelog.v1";
const MAX_ENTRIES = 100;
const MAX_PERSISTED_BYTES = 2 * 1024 * 1024;
const MAX_CAPTURE_BYTES = 4 * 1024 * 1024;
const textEncoder = new TextEncoder();

interface AssistantWireLogState {
  readonly captureEnabled: boolean;
  readonly showResponses: boolean;
  readonly entries: readonly AssistantWireLogExchange[];
  readonly totalBytes: number;
  readonly captureBytes: number;
  readonly setCaptureEnabled: (enabled: boolean) => void;
  readonly setShowResponses: (enabled: boolean) => void;
  readonly recordExchange: (
    envelopes: readonly AssistantUpstreamEnvelope[],
    kind: AssistantWireLogExchange["kind"],
    status: number,
    droppedEchoCount?: number,
  ) => string | null;
  readonly attachWireLines: (
    exchangeId: string,
    lines: readonly AssistantWireLine[],
    receivedBytes: number,
    truncated?: boolean,
  ) => void;
  readonly attachResponseBody: (
    exchangeId: string,
    text: string,
    receivedBytes: number,
    truncated: boolean,
  ) => void;
  readonly finalizeCapture: (
    exchangeId: string,
    outcome: AssistantWireCaptureOutcome,
  ) => void;
  readonly clear: () => void;
  readonly reset: () => void;
}

interface PersistedWireLogState {
  readonly captureEnabled: boolean;
  readonly showResponses: boolean;
  readonly entries: readonly PersistedAssistantWireLogExchange[];
}

const EMPTY_PERSISTED_WIRE_LOG: PersistedWireLogState = {
  captureEnabled: false,
  showResponses: true,
  entries: [],
};

const EMPTY_WIRE_LOG = {
  ...EMPTY_PERSISTED_WIRE_LOG,
  entries: [] as readonly AssistantWireLogExchange[],
  totalBytes: 0,
  captureBytes: 0,
} as const;

function persistedExchange(
  exchange: AssistantWireLogExchange,
): PersistedAssistantWireLogExchange {
  return {
    id: exchange.id,
    ts: exchange.ts,
    kind: exchange.kind,
    status: exchange.status,
    upstreamEchoes: exchange.upstreamEchoes,
    ...(exchange.droppedEchoCount === undefined
      ? {}
      : { droppedEchoCount: exchange.droppedEchoCount }),
  };
}

function serializedExchangeBytes(exchange: AssistantWireLogExchange): number {
  return textEncoder.encode(JSON.stringify(persistedExchange(exchange)))
    .byteLength;
}

function persistedBytes(entries: readonly AssistantWireLogExchange[]): number {
  return entries.reduce(
    (total, exchange) => total + serializedExchangeBytes(exchange),
    0,
  );
}

function captureSize(capture: AssistantWireCapture | undefined): number {
  if (!capture || capture.state === "evicted") return 0;
  const sseBytes = capture.sse?.retainedBytes ?? 0;
  const bodyBytes = capture.body
    ? textEncoder.encode(capture.body.text).byteLength
    : 0;
  return sseBytes + bodyBytes;
}

function totalCaptureBytes(
  entries: readonly AssistantWireLogExchange[],
): number {
  return entries.reduce(
    (total, exchange) => total + captureSize(exchange.capture),
    0,
  );
}

function boundedPersistedEntries(
  entries: readonly AssistantWireLogExchange[],
): Pick<AssistantWireLogState, "entries" | "totalBytes" | "captureBytes"> {
  let bounded = entries.slice(Math.max(0, entries.length - MAX_ENTRIES));
  let boundedBytes = persistedBytes(bounded);
  while (bounded.length > 0 && boundedBytes > MAX_PERSISTED_BYTES) {
    bounded = bounded.slice(1);
    boundedBytes = persistedBytes(bounded);
  }
  return {
    entries: bounded,
    totalBytes: boundedBytes,
    captureBytes: totalCaptureBytes(bounded),
  };
}

function enforceCaptureBudget(
  entries: readonly AssistantWireLogExchange[],
): Pick<AssistantWireLogState, "entries" | "captureBytes"> {
  let captureBytes = totalCaptureBytes(entries);
  if (captureBytes <= MAX_CAPTURE_BYTES) return { entries, captureBytes };

  const bounded = [...entries];
  for (let index = 0; index < bounded.length; index += 1) {
    const exchange = bounded[index];
    if (!exchange || captureSize(exchange.capture) === 0) continue;
    captureBytes -= captureSize(exchange.capture);
    bounded[index] = { ...exchange, capture: { state: "evicted" } };
    if (captureBytes <= MAX_CAPTURE_BYTES) break;
  }
  return { entries: bounded, captureBytes: Math.max(0, captureBytes) };
}

function lineBytes(lines: readonly AssistantWireLine[]): number {
  return lines.reduce(
    (total, line) =>
      total + textEncoder.encode(`${line.text}${line.ending}`).byteLength,
    0,
  );
}

function updateCapture(
  entries: readonly AssistantWireLogExchange[],
  exchangeId: string,
  update: (
    capture: Exclude<AssistantWireCapture, { state: "evicted" }>,
  ) => Exclude<AssistantWireCapture, { state: "evicted" }> | null,
): readonly AssistantWireLogExchange[] | null {
  const index = entries.findIndex((entry) => entry.id === exchangeId);
  if (index < 0) return null;
  const exchange = entries[index];
  if (!exchange?.capture || exchange.capture.state === "evicted") return null;
  const capture = update(exchange.capture);
  if (!capture) return null;
  const next = [...entries];
  next[index] = { ...exchange, capture };
  return next;
}

function isQuotaExceeded(error: unknown): boolean {
  return (
    error instanceof DOMException &&
    (error.name === "QuotaExceededError" ||
      error.name === "NS_ERROR_DOM_QUOTA_REACHED")
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

function removeInvalidPersistedState(): void {
  if (typeof localStorage !== "undefined") {
    localStorage.removeItem(ASSISTANT_WIRE_LOG_STORAGE_KEY);
  }
}

function migrateWireLog(
  persisted: unknown,
  version: number,
): PersistedWireLogState {
  if (version < 2) {
    removeInvalidPersistedState();
    return EMPTY_PERSISTED_WIRE_LOG;
  }
  const parsed = assistantWireLogPersistedSchema.safeParse(persisted);
  if (!parsed.success) {
    removeInvalidPersistedState();
    return EMPTY_PERSISTED_WIRE_LOG;
  }
  return parsed.data;
}

// Backend echoes persist across restarts; delivered bodies stay session-only.
export const useAssistantWireLogStore = create<AssistantWireLogState>()(
  persist(
    (set, get) => ({
      ...EMPTY_WIRE_LOG,
      setCaptureEnabled: (captureEnabled) => set({ captureEnabled }),
      setShowResponses: (showResponses) => set({ showResponses }),
      recordExchange: (envelopes, kind, status, droppedEchoCount = 0) => {
        if (!get().captureEnabled || envelopes.length === 0) return null;
        const id = crypto.randomUUID();
        const exchange: AssistantWireLogExchange = {
          id,
          ts: Date.now(),
          kind,
          status,
          upstreamEchoes: [...envelopes],
          ...(droppedEchoCount > 0 ? { droppedEchoCount } : {}),
          capture: { state: "open" },
        };
        set((state) => boundedPersistedEntries([...state.entries, exchange]));
        return id;
      },
      attachWireLines: (
        exchangeId,
        lines,
        receivedBytes,
        truncated = false,
      ) => {
        set((state) => {
          const entries = updateCapture(
            state.entries,
            exchangeId,
            (capture) => {
              const previous = capture.sse ?? {
                lines: [],
                bytes: 0,
                retainedBytes: 0,
                truncated: false,
              };
              return {
                ...capture,
                sse: {
                  lines: [...previous.lines, ...lines],
                  bytes: previous.bytes + Math.max(0, receivedBytes),
                  retainedBytes: previous.retainedBytes + lineBytes(lines),
                  truncated: previous.truncated || truncated,
                },
              };
            },
          );
          if (!entries) return state;
          return enforceCaptureBudget(entries);
        });
      },
      attachResponseBody: (exchangeId, text, receivedBytes, truncated) => {
        set((state) => {
          const entries = updateCapture(
            state.entries,
            exchangeId,
            (capture) => ({
              ...capture,
              body: {
                text,
                bytes: Math.max(0, receivedBytes),
                truncated,
              },
            }),
          );
          if (!entries) return state;
          return enforceCaptureBudget(entries);
        });
      },
      finalizeCapture: (exchangeId, outcome) => {
        set((state) => {
          const entries = updateCapture(
            state.entries,
            exchangeId,
            (capture) => ({ ...capture, state: "settled", outcome }),
          );
          return entries ? { entries } : state;
        });
      },
      clear: () => set({ entries: [], totalBytes: 0, captureBytes: 0 }),
      reset: () => {
        set(EMPTY_WIRE_LOG);
        removeInvalidPersistedState();
      },
    }),
    {
      name: ASSISTANT_WIRE_LOG_STORAGE_KEY,
      version: 2,
      storage: createJSONStorage(() => quotaSafeStorage),
      migrate: migrateWireLog,
      partialize: ({ captureEnabled, showResponses, entries }) => ({
        captureEnabled,
        showResponses,
        entries: entries.map(persistedExchange),
      }),
      merge: (persisted, current) => {
        const parsed = assistantWireLogPersistedSchema.safeParse(persisted);
        if (!parsed.success) {
          removeInvalidPersistedState();
          return current;
        }
        const entries = parsed.data.entries.map((entry) => ({ ...entry }));
        return {
          ...current,
          ...parsed.data,
          entries,
          totalBytes: persistedBytes(entries),
          captureBytes: 0,
        };
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
  kind: AssistantWireLogExchange["kind"],
  status: number,
): string | null {
  if (!value) return null;
  try {
    const parsed = assistantUpstreamEnvelopeHeaderDecoderSchema.safeParse(
      JSON.parse(decodeBase64Utf8(value)),
    );
    if (!parsed.success) return null;
    return useAssistantWireLogStore
      .getState()
      .recordExchange(
        parsed.data.echoes,
        kind,
        status,
        parsed.data.droppedEchoCount,
      );
  } catch {
    // A malformed debug header must never affect the assistant request.
    return null;
  }
}
