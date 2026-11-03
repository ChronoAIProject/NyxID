import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { transitionAssistantIdentity } from "./identity";
import {
  cmaProxyPath,
  listCmaConnections,
  listActiveCmaSessions,
  resolveCmaRoutes,
  watchCmaSession,
} from "./cma-session-api";
import { cmaFrame, cmaHead, cmaSession } from "./cma-session.test-fixtures";
import type { CmaSessionState } from "@/schemas/cma-session";

const encoder = new TextEncoder();
let tail: ReadableStreamDefaultController<Uint8Array>;
let cancel: ReturnType<typeof vi.fn<() => void>>;
let requests: { url: string; config?: RequestInit }[];
let tailCount: number;
let streamResponse: (config: RequestInit) => Response;

function openStream(config: RequestInit) {
  const abort = () => tail.error(new DOMException("Aborted", "AbortError"));
  return new Response(
    new ReadableStream<Uint8Array>({
      start(controller) {
        tail = controller;
        config.signal?.addEventListener("abort", abort, { once: true });
      },
      cancel() {
        config.signal?.removeEventListener("abort", abort);
        cancel();
      },
    }),
    { headers: { "content-type": "text/event-stream; charset=utf-8" } },
  );
}

beforeEach(() => {
  transitionAssistantIdentity("owner");
  cancel = vi.fn();
  requests = [];
  tailCount = 0;
  streamResponse = openStream;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string, config: RequestInit) => {
      const url = String(input);
      requests.push({ url, config });
      if (url.includes("/events/head")) return Response.json(cmaHead);
      if (url.includes("/events?")) {
        tailCount++;
        return streamResponse(config);
      }
      return Response.json({
        id: cmaSession.agentId,
        title: "Review the pull request",
        status: "working",
      });
    }),
  );
});

afterEach(() => {
  transitionAssistantIdentity(null);
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("read-only CMA session observation", () => {
  it("retains normal NyxID sign-out handling for a discovery 401", async () => {
    const clearUser = vi
      .spyOn(useAuthStore.getState(), "setUser")
      .mockImplementation(() => undefined);
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        Response.json(
          {
            error: "unauthorized",
            message: "Expired session",
            error_code: 1001,
          },
          { status: 401 },
        ),
      ),
    );
    await expect(
      listCmaConnections("owner", new AbortController().signal),
    ).rejects.toBeDefined();
    expect(clearUser).toHaveBeenCalledWith(null);
  });
  it("replays active work and decodes UTF-8 split in the middle of a character", async () => {
    const controller = new AbortController();
    const states: CmaSessionState[] = [];
    const watching = watchCmaSession(
      "owner",
      cmaSession,
      controller.signal,
      (state) => {
        states.push(state);
        if (state.status === "completed") controller.abort();
      },
    );
    await vi.waitFor(() => expect(tailCount).toBe(1));
    expect(requests.at(-1)?.url).toContain(
      "after_cursor=cursor-10&follow=true",
    );
    const bytes = encoder.encode(
      cmaFrame(11, {
        type: "tool_started",
        call_id: "a",
        title: "Inspect café",
        status: "in_progress",
      }),
    );
    const split = bytes.indexOf(0xc3) + 1;
    tail.enqueue(bytes.slice(0, split));
    tail.enqueue(bytes.slice(split));
    tail.enqueue(
      encoder.encode(
        cmaFrame(12, {
          type: "tool_updated",
          call_id: "a",
          status: "completed",
        }),
      ),
    );
    tail.enqueue(
      encoder.encode(
        cmaFrame(13, { type: "turn_completed", turn_sequence: 10 }),
      ),
    );
    await watching;
    expect(states.at(-1)).toMatchObject({
      connection: "live",
      status: "completed",
      actions: [{ title: "Inspect café", status: "completed" }],
    });
    expect(
      requests.every(
        ({ config }) =>
          config?.method === "GET" && config.credentials === "include",
      ),
    ).toBe(true);
    expect(requests.at(-1)?.config?.signal?.aborted).toBe(true);
  });

  it("reconnects after the last fully received cursor, acknowledging withheld and unknown events", async () => {
    vi.useFakeTimers();
    const controller = new AbortController();
    const states: CmaSessionState[] = [];
    const watching = watchCmaSession(
      "owner",
      cmaSession,
      controller.signal,
      (state) => states.push(state),
    );
    await vi.waitFor(() => expect(tailCount).toBe(1));
    tail.enqueue(
      encoder.encode(
        cmaFrame(11, {
          type: "tool_started",
          call_id: "a",
          title: "Checking",
          status: "in_progress",
        }),
      ),
    );
    tail.enqueue(
      encoder.encode(
        cmaFrame(12, null) + cmaFrame(13, { type: "future_event" }),
      ),
    );
    tail.enqueue(
      encoder.encode(
        cmaFrame(14, {
          type: "tool_updated",
          call_id: "a",
          status: "completed",
        }).slice(0, -2),
      ),
    );
    tail.close();
    await vi.waitFor(() =>
      expect(states.at(-1)?.connection).toBe("reconnecting"),
    );
    await vi.advanceTimersByTimeAsync(1000);
    expect(tailCount).toBe(2);
    expect(requests.at(-1)?.url).toContain(
      "after_cursor=cursor-13&follow=true",
    );
    tail.enqueue(
      encoder.encode(
        cmaFrame(11, {
          type: "tool_started",
          call_id: "duplicate",
          title: "Duplicate",
          status: "in_progress",
        }) +
          cmaFrame(14, {
            type: "tool_updated",
            call_id: "a",
            status: "completed",
          }),
      ),
    );
    await vi.waitFor(() =>
      expect(states.at(-1)?.actions[0]?.status).toBe("completed"),
    );
    expect(states.at(-1)?.actions).toHaveLength(1);
    expect(
      requests.filter(({ url }) => url.includes("/events/head")),
    ).toHaveLength(1);
    controller.abort();
    await watching;
  });

  it.each([
    ["agent_id", "agt_someone_else"],
    ["incarnation", "incarnation-2"],
    ["cursor", "wrong-cursor"],
  ])(
    "rejects a mismatched %s without applying the event or retrying",
    async (field, value) => {
      const states: CmaSessionState[] = [];
      const controller = new AbortController();
      const watching = watchCmaSession(
        "owner",
        cmaSession,
        controller.signal,
        (state) => states.push(state),
      );
      await vi.waitFor(() => expect(tailCount).toBe(1));
      const data = JSON.parse(cmaFrame(11, null).split("data: ")[1]!) as Record<
        string,
        unknown
      >;
      tail.enqueue(
        encoder.encode(
          `event: cma.agent.event\nid: cursor-11\ndata: ${JSON.stringify({ ...data, [field]: value })}\n\n`,
        ),
      );
      await watching;
      expect(states.at(-1)).toMatchObject({
        connection: "unavailable",
        actions: [],
      });
      expect(tailCount).toBe(1);
    },
  );

  it("preserves NyxID authentication on a downstream 401 and stops automatic retries", async () => {
    const clearUser = vi.spyOn(useAuthStore.getState(), "setUser");
    streamResponse = () =>
      Response.json(
        {
          error: "unauthorized",
          message: "Private upstream detail",
          error_code: 42,
        },
        { status: 401 },
      );
    const states: CmaSessionState[] = [];
    await watchCmaSession(
      "owner",
      cmaSession,
      new AbortController().signal,
      (state) => states.push(state),
    );
    expect(clearUser).not.toHaveBeenCalled();
    expect(states.at(-1)).toMatchObject({
      connection: "unavailable",
      error: expect.stringContaining("Check your connection"),
    });
    expect(JSON.stringify(states)).not.toContain("Private upstream detail");
    expect(tailCount).toBe(1);
    clearUser.mockRestore();
  });

  it("ignores late events after account changes", async () => {
    const states: CmaSessionState[] = [];
    const watching = watchCmaSession(
      "owner",
      cmaSession,
      new AbortController().signal,
      (state) => states.push(state),
    );
    await vi.waitFor(() => expect(tailCount).toBe(1));
    const count = states.length;
    transitionAssistantIdentity("other-owner");
    tail.enqueue(
      encoder.encode(
        cmaFrame(11, { type: "title_set", title: "Old account data" }),
      ),
    );
    await watching;
    expect(states).toHaveLength(count);
    expect(requests.at(-1)?.config?.signal?.aborted).toBe(true);
  });

  it("uses the current head for an idle session instead of replaying historical work", async () => {
    const originalFetch = globalThis.fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: string, config: RequestInit) =>
        String(input).includes("/events/head")
          ? Promise.resolve(Response.json({ ...cmaHead, active_turn: null }))
          : originalFetch(input, config),
      ),
    );
    const controller = new AbortController();
    const watching = watchCmaSession(
      "owner",
      cmaSession,
      controller.signal,
      () => undefined,
    );
    await vi.waitFor(() => expect(tailCount).toBe(1));
    expect(requests.at(-1)?.url).toContain("after_cursor=cursor-15");
    controller.abort();
    await watching;
  });
});

it("uses advertised CMA routes while keeping requests on NyxID's proxy", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () =>
      Response.json({
        paths: Object.fromEntries(
          Object.values(cmaSession.routes).map((path) => [path, { get: {} }]),
        ),
      }),
    ),
  );
  expect(
    await resolveCmaRoutes(
      "owner",
      cmaSession.connection.id,
      new AbortController().signal,
    ),
  ).toEqual(cmaSession.routes);
  expect(
    cmaProxyPath(
      cmaSession.connection.id,
      cmaSession.routes.agent,
      cmaSession.agentId,
    ),
  ).toBe(`/proxy/${cmaSession.connection.id}/api/v1/agents/agt_observed`);
  expect(() =>
    cmaProxyPath(cmaSession.connection.id, "//external.test/agents"),
  ).toThrow();
});

it("discovers only accessible CMA connections across catalog and custom pages", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string) => {
      const page = new URL(String(input), "http://localhost").searchParams.get(
        "page",
      );
      return Response.json({
        services:
          page === "1"
            ? [
                cmaSession.connection,
                {
                  ...cmaSession.connection,
                  id: "32ec5c70-b8bb-4133-8824-031a1dc63945",
                  name: "GitHub",
                  slug: "github",
                },
                {
                  ...cmaSession.connection,
                  id: "42ec5c70-b8bb-4133-8824-031a1dc63945",
                  connected: false,
                },
              ]
            : [],
        custom_services:
          page === "2"
            ? [
                {
                  ...cmaSession.connection,
                  id: "52ec5c70-b8bb-4133-8824-031a1dc63945",
                  slug: "cma_staging",
                  connected: false,
                  requires_connection: false,
                },
              ]
            : [],
        page: Number(page),
        per_page: 100,
        total: 101,
      });
    }),
  );
  expect(
    (await listCmaConnections("owner", new AbortController().signal)).map(
      (connection) => connection.id,
    ),
  ).toEqual([cmaSession.connection.id, "52ec5c70-b8bb-4133-8824-031a1dc63945"]);
});

it("finds active sessions on later pages, excludes idle and terminal sessions, and deduplicates", async () => {
  const seen: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string) => {
      const url = String(input);
      seen.push(url);
      return Response.json(
        url.includes("cursor=next")
          ? {
              items: [
                {
                  id: "agt_waiting",
                  status: "waiting_approval",
                  title: "Approval",
                },
                { id: "agt_paused", status: "paused" },
                {
                  id: cmaSession.agentId,
                  status: "working",
                  owner_title: "Owner title",
                },
              ],
              next: null,
            }
          : {
              items: [
                { id: cmaSession.agentId, status: "working" },
                { id: "agt_starting", status: "starting" },
                ...[
                  "ready",
                  "failed",
                  "stopped",
                  "deleted",
                  "completed",
                  "future",
                ].map((status) => ({ id: `agt_${status}`, status })),
              ],
              next: "next",
            },
      );
    }),
  );
  const active = await listActiveCmaSessions(
    "owner",
    cmaSession.connection,
    cmaSession.routes,
    new AbortController().signal,
  );
  expect(active.map((session) => session.agentId)).toEqual([
    cmaSession.agentId,
    "agt_starting",
    "agt_waiting",
    "agt_paused",
  ]);
  expect(active[0]).toMatchObject({ ...cmaSession, title: "Owner title" });
  expect(seen).toHaveLength(2);
  expect(seen[1]).toContain("cursor=next");
});

it("does not spin when CMA repeats a list cursor", async () => {
  const fetch = vi.fn(async () => Response.json({ items: [], next: "same" }));
  vi.stubGlobal("fetch", fetch);
  await expect(
    listActiveCmaSessions(
      "owner",
      cmaSession.connection,
      cmaSession.routes,
      new AbortController().signal,
    ),
  ).rejects.toThrow("repeated");
  expect(fetch).toHaveBeenCalledTimes(2);
});

it("rejects a discovery response that arrives after an account change", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      transitionAssistantIdentity("other");
      return Response.json({
        items: [{ id: cmaSession.agentId, status: "working" }],
      });
    }),
  );
  await expect(
    listActiveCmaSessions(
      "owner",
      cmaSession.connection,
      cmaSession.routes,
      new AbortController().signal,
    ),
  ).rejects.toMatchObject({ name: "AbortError" });
});
