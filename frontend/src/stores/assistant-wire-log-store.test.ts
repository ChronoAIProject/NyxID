import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AssistantUpstreamEnvelope } from "@/schemas/assistant-wire-log";
import {
  ASSISTANT_WIRE_LOG_STORAGE_KEY,
  useAssistantWireLogStore,
} from "./assistant-wire-log-store";

function envelope(index: number): AssistantUpstreamEnvelope {
  return {
    method: "POST",
    path: "api/chat",
    commandType: "text",
    body: { type: "text", prompt: `prompt-${String(index)}` },
    headers: {
      accept: "text/event-stream",
      "content-type": "application/json",
      "idempotency-key": `request-${String(index)}`,
    },
    identity: {
      mode: "jwt",
      forward_access_token: false,
      inject_delegation_token: true,
      bridge_minted: false,
    },
    truncated: false,
  };
}

function resetStore() {
  useAssistantWireLogStore.setState({ captureEnabled: false, entries: [] });
}

describe("useAssistantWireLogStore", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    localStorage.clear();
    resetStore();
  });

  it("evicts the oldest entries from the 100-entry ring", () => {
    const store = useAssistantWireLogStore.getState();
    for (let index = 0; index < 105; index += 1) {
      store.record(envelope(index), "header", 200);
    }

    const entries = useAssistantWireLogStore.getState().entries;
    expect(entries).toHaveLength(100);
    expect(entries[0]?.body).toMatchObject({ prompt: "prompt-5" });
    expect(entries.at(-1)?.body).toMatchObject({ prompt: "prompt-104" });
  });

  it("evicts oldest payloads to stay below the two-megabyte budget", () => {
    const first = envelope(1);
    const second = envelope(2);
    useAssistantWireLogStore.getState().record(
      { ...first, body: { prompt: "a".repeat(1_200_000) } },
      "header",
      200,
    );
    useAssistantWireLogStore.getState().record(
      { ...second, body: { prompt: "b".repeat(1_200_000) } },
      "header",
      200,
    );

    const entries = useAssistantWireLogStore.getState().entries;
    expect(entries).toHaveLength(1);
    expect((entries[0]?.body as { prompt: string }).prompt.startsWith("b")).toBe(
      true,
    );
  });

  it("round-trips validated entries through persistence", async () => {
    useAssistantWireLogStore.getState().record(envelope(1), "sse", 201);
    const persisted = localStorage.getItem(ASSISTANT_WIRE_LOG_STORAGE_KEY);
    resetStore();
    localStorage.setItem(ASSISTANT_WIRE_LOG_STORAGE_KEY, persisted!);

    await useAssistantWireLogStore.persist.rehydrate();

    expect(useAssistantWireLogStore.getState().entries).toHaveLength(1);
    expect(useAssistantWireLogStore.getState().entries[0]).toMatchObject({
      kind: "sse",
      status: 201,
      body: { prompt: "prompt-1" },
    });
  });

  it("drops corrupt persisted state during hydration", async () => {
    localStorage.setItem(
      ASSISTANT_WIRE_LOG_STORAGE_KEY,
      JSON.stringify({
        state: { captureEnabled: "yes", entries: [{ body: "private" }] },
        version: 1,
      }),
    );

    await useAssistantWireLogStore.persist.rehydrate();

    expect(useAssistantWireLogStore.getState()).toMatchObject({
      captureEnabled: false,
      entries: [],
    });
    expect(localStorage.getItem(ASSISTANT_WIRE_LOG_STORAGE_KEY)).toBeNull();
  });

  it("persists the capture toggle", async () => {
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    const persisted = localStorage.getItem(ASSISTANT_WIRE_LOG_STORAGE_KEY);
    resetStore();
    localStorage.setItem(ASSISTANT_WIRE_LOG_STORAGE_KEY, persisted!);

    await useAssistantWireLogStore.persist.rehydrate();

    expect(useAssistantWireLogStore.getState().captureEnabled).toBe(true);
  });

  it("drops the oldest persisted entry and retries once after quota exhaustion", () => {
    useAssistantWireLogStore.getState().record(envelope(1), "header", 200);
    useAssistantWireLogStore.getState().record(envelope(2), "header", 200);
    const writes: string[] = [];
    vi.stubGlobal("localStorage", {
      getItem: vi.fn(() => null),
      removeItem: vi.fn(),
      setItem: vi.fn((_name: string, value: string) => {
        writes.push(value);
        if (writes.length === 1) {
          throw new DOMException("Storage quota exceeded", "QuotaExceededError");
        }
      }),
    });

    useAssistantWireLogStore.getState().record(envelope(3), "header", 200);

    expect(writes).toHaveLength(2);
    const retried = JSON.parse(writes[1]!) as {
      state: { entries: Array<{ body: { prompt: string } }> };
    };
    expect(retried.state.entries.map((entry) => entry.body.prompt)).toEqual([
      "prompt-2",
      "prompt-3",
    ]);
  });
});
