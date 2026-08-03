import { beforeEach, describe, expect, it, vi } from "vitest";
import backendLadderFixtures from "./__fixtures__/assistant-upstream-envelope-ladder.json";
import {
  assistantUpstreamEnvelopeHeaderDecoderSchema,
  type AssistantUpstreamEnvelope,
} from "@/schemas/assistant-wire-log";
import {
  ASSISTANT_WIRE_LOG_STORAGE_KEY,
  captureAssistantWireLogHeader,
  useAssistantWireLogStore,
} from "./assistant-wire-log-store";

function envelope(index: number): AssistantUpstreamEnvelope {
  return {
    degraded: false,
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
    response: {
      status: 200,
      headers: {
        "content-type": { value: "text/event-stream", truncated: false },
      },
      sse: true,
    },
    upstreamOutcome: "response",
  };
}

function resetStore(
  captureEnabled = false,
  featureEnabled = captureEnabled,
) {
  useAssistantWireLogStore.setState({
    featureEnabled,
    captureEnabled,
    showResponses: true,
    entries: [],
    totalBytes: 0,
    captureBytes: 0,
  });
}

function encodeHeader(value: unknown): string {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

describe("assistant upstream envelope decoder", () => {
  it("accepts every backend degradation rung through the real union decoder", () => {
    expect(backendLadderFixtures).toHaveLength(6);
    for (const fixture of backendLadderFixtures) {
      const decoded = assistantUpstreamEnvelopeHeaderDecoderSchema.safeParse(
        fixture.header,
      );
      expect(decoded.success, fixture.rung).toBe(true);
    }
  });

  it("normalizes legacy arrays without inventing an upstream outcome", () => {
    const full = envelope(1);
    if (full.degraded) throw new Error("test fixture must be a full echo");
    const legacy = {
      method: full.method,
      path: full.path,
      commandType: full.commandType,
      body: full.body,
      headers: full.headers,
      identity: full.identity,
      truncated: full.truncated,
    };

    const parsed = assistantUpstreamEnvelopeHeaderDecoderSchema.parse([legacy]);

    expect(parsed).toMatchObject({ version: 2, droppedEchoCount: 0 });
    expect(parsed.echoes[0]).toMatchObject({ degraded: false });
    expect(parsed.echoes[0]).not.toHaveProperty("upstreamOutcome");
    expect(parsed.echoes[0]).not.toHaveProperty("response");
  });
});

describe("useAssistantWireLogStore", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    localStorage.clear();
    resetStore(true);
  });

  it("evicts the oldest exchanges from the 100-entry ring", () => {
    const store = useAssistantWireLogStore.getState();
    for (let index = 0; index < 105; index += 1) {
      store.recordExchange([envelope(index)], "header", 200);
    }

    const entries = useAssistantWireLogStore.getState().entries;
    expect(entries).toHaveLength(100);
    expect(entries[0]?.upstreamEchoes[0]).toMatchObject({
      body: { prompt: "prompt-5" },
    });
    expect(entries.at(-1)?.upstreamEchoes[0]).toMatchObject({
      body: { prompt: "prompt-104" },
    });
  });

  it("evicts oldest envelopes to stay below the two-megabyte persisted budget", () => {
    const first = { ...envelope(1), body: { prompt: "a".repeat(1_200_000) } };
    const second = { ...envelope(2), body: { prompt: "b".repeat(1_200_000) } };
    useAssistantWireLogStore.getState().recordExchange([first], "header", 200);
    useAssistantWireLogStore.getState().recordExchange([second], "header", 200);

    const { entries, totalBytes } = useAssistantWireLogStore.getState();
    expect(entries).toHaveLength(1);
    expect(entries[0]?.upstreamEchoes[0]).toMatchObject({
      body: { prompt: expect.stringMatching(/^b/) },
    });
    expect(totalBytes).toBeGreaterThan(1_200_000);
    expect(totalBytes).toBeLessThanOrEqual(2 * 1024 * 1024);
  });

  it("persists only backend envelopes and recomputes byte accounting", async () => {
    const id = useAssistantWireLogStore
      .getState()
      .recordExchange([envelope(1)], "sse", 201);
    expect(id).not.toBeNull();
    useAssistantWireLogStore
      .getState()
      .attachWireLines(id!, [{ text: "data: private", ending: "\n" }], 14);
    useAssistantWireLogStore.getState().finalizeCapture(id!, "complete");
    const persisted = localStorage.getItem(ASSISTANT_WIRE_LOG_STORAGE_KEY);
    expect(persisted).not.toContain("data: private");
    resetStore();
    localStorage.setItem(ASSISTANT_WIRE_LOG_STORAGE_KEY, persisted!);

    await useAssistantWireLogStore.persist.rehydrate();

    const state = useAssistantWireLogStore.getState();
    expect(state.entries).toHaveLength(1);
    expect(state.entries[0]).toMatchObject({ kind: "sse", status: 201 });
    expect(state.entries[0]).not.toHaveProperty("capture");
    expect(state.totalBytes).toBeGreaterThan(0);
    expect(state.captureBytes).toBe(0);
  });

  it("explicitly migrates a real v1 payload to an empty v2 state", async () => {
    resetStore();
    localStorage.setItem(
      ASSISTANT_WIRE_LOG_STORAGE_KEY,
      JSON.stringify({
        state: {
          captureEnabled: true,
          entries: [{ ...envelope(1), body: { prompt: "v1-private" } }],
        },
        version: 1,
      }),
    );

    await useAssistantWireLogStore.persist.rehydrate();

    expect(useAssistantWireLogStore.getState()).toMatchObject({
      captureEnabled: false,
      showResponses: true,
      entries: [],
      totalBytes: 0,
      captureBytes: 0,
    });
    expect(localStorage.getItem(ASSISTANT_WIRE_LOG_STORAGE_KEY)).not.toContain(
      "v1-private",
    );
  });

  it("drops corrupt persisted v2 state during hydration", async () => {
    resetStore();
    localStorage.setItem(
      ASSISTANT_WIRE_LOG_STORAGE_KEY,
      JSON.stringify({
        state: { captureEnabled: "yes", entries: [{ body: "private" }] },
        version: 2,
      }),
    );

    await useAssistantWireLogStore.persist.rehydrate();

    expect(useAssistantWireLogStore.getState()).toMatchObject({
      captureEnabled: false,
      showResponses: true,
      entries: [],
    });
    expect(localStorage.getItem(ASSISTANT_WIRE_LOG_STORAGE_KEY)).toBeNull();
  });

  it("persists the capture and responses toggles", async () => {
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    useAssistantWireLogStore.getState().setShowResponses(false);
    const persisted = localStorage.getItem(ASSISTANT_WIRE_LOG_STORAGE_KEY);
    resetStore();
    localStorage.setItem(ASSISTANT_WIRE_LOG_STORAGE_KEY, persisted!);

    await useAssistantWireLogStore.persist.rehydrate();

    expect(useAssistantWireLogStore.getState()).toMatchObject({
      captureEnabled: true,
      showResponses: false,
    });
  });

  it("never persists the server-driven feature flag", () => {
    useAssistantWireLogStore.getState().setFeatureEnabled(true);

    const persisted = JSON.parse(
      localStorage.getItem(ASSISTANT_WIRE_LOG_STORAGE_KEY)!,
    ) as { state: Record<string, unknown> };

    expect(persisted.state).not.toHaveProperty("featureEnabled");
  });

  it("never creates an exchange without a backend echo", () => {
    expect(
      useAssistantWireLogStore.getState().recordExchange([], "header", 200),
    ).toBeNull();
    expect(useAssistantWireLogStore.getState().entries).toEqual([]);
  });

  it("ignores a backend echo that arrives after capture is disabled", () => {
    useAssistantWireLogStore.getState().setCaptureEnabled(false);

    const id = useAssistantWireLogStore
      .getState()
      .recordExchange([envelope(1)], "header", 200);

    expect(id).toBeNull();
    expect(useAssistantWireLogStore.getState().entries).toEqual([]);
  });

  it("ignores a backend echo while the server-driven feature is disabled", () => {
    useAssistantWireLogStore.getState().setFeatureEnabled(false);

    const id = useAssistantWireLogStore
      .getState()
      .recordExchange([envelope(1)], "header", 200);

    expect(id).toBeNull();
    expect(useAssistantWireLogStore.getState().entries).toEqual([]);
  });

  it("evicts oldest session captures under the separate four-megabyte budget", () => {
    const firstId = useAssistantWireLogStore
      .getState()
      .recordExchange([envelope(1)], "header", 200)!;
    const secondId = useAssistantWireLogStore
      .getState()
      .recordExchange([envelope(2)], "header", 200)!;
    useAssistantWireLogStore
      .getState()
      .attachResponseBody(firstId, "a".repeat(2_200_000), 2_200_000, false);
    useAssistantWireLogStore
      .getState()
      .attachResponseBody(secondId, "b".repeat(2_200_000), 2_200_000, true);

    const state = useAssistantWireLogStore.getState();
    expect(state.entries[0]?.capture).toEqual({ state: "evicted" });
    expect(state.entries[1]?.capture).toMatchObject({
      state: "open",
      body: { truncated: true },
    });
    expect(state.captureBytes).toBe(2_200_000);
    expect(state.totalBytes).toBeGreaterThan(0);

    useAssistantWireLogStore
      .getState()
      .attachWireLines(firstId, [{ text: "ignored", ending: "\n" }], 8, true);
    useAssistantWireLogStore.getState().finalizeCapture(firstId, "complete");
    expect(useAssistantWireLogStore.getState().entries[0]?.capture).toEqual({
      state: "evicted",
    });
  });

  it("records a multi-echo header as one exchange", () => {
    const header = encodeHeader({
      version: 2,
      echoes: [envelope(1), envelope(2)],
      droppedEchoCount: 3,
    });

    const id = captureAssistantWireLogHeader(header, "sse", 202);

    expect(id).not.toBeNull();
    expect(useAssistantWireLogStore.getState().entries).toHaveLength(1);
    expect(useAssistantWireLogStore.getState().entries[0]).toMatchObject({
      id,
      status: 202,
      droppedEchoCount: 3,
      upstreamEchoes: [
        { body: { prompt: "prompt-1" } },
        { body: { prompt: "prompt-2" } },
      ],
    });
  });

  it("drops the oldest persisted exchange and retries once after quota exhaustion", () => {
    useAssistantWireLogStore
      .getState()
      .recordExchange([envelope(1)], "header", 200);
    useAssistantWireLogStore
      .getState()
      .recordExchange([envelope(2)], "header", 200);
    const writes: string[] = [];
    vi.stubGlobal("localStorage", {
      getItem: vi.fn(() => null),
      removeItem: vi.fn(),
      setItem: vi.fn((_name: string, value: string) => {
        writes.push(value);
        if (writes.length === 1) {
          throw new DOMException(
            "Storage quota exceeded",
            "QuotaExceededError",
          );
        }
      }),
    });

    useAssistantWireLogStore
      .getState()
      .recordExchange([envelope(3)], "header", 200);

    expect(writes).toHaveLength(2);
    const retried = JSON.parse(writes[1]!) as {
      state: {
        entries: Array<{
          upstreamEchoes: Array<{ body: { prompt: string } }>;
        }>;
      };
    };
    expect(
      retried.state.entries.map(
        (entry) => entry.upstreamEchoes[0]!.body.prompt,
      ),
    ).toEqual(["prompt-2", "prompt-3"]);
  });
});
