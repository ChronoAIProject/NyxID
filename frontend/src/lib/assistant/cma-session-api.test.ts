import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { transitionAssistantIdentity } from "./identity";
import {
  cmaProxyPath,
  listCmaConnections,
  listActiveCmaSessions,
  resolveCmaRoutes,
  watchCmaSession,
  discoverCmaSessions,
  listPinnedCmaSessions,
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
  expect(seen).toHaveLength(8);
  expect(
    new Set(
      seen.map((url) =>
        new URL(url, "https://nyx.example").searchParams.get("status"),
      ),
    ),
  ).toEqual(new Set(["starting", "working", "waiting_approval", "paused"]));
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

it("reads only bounded server origin receipts and preserves supplied routing/callback metadata", async () => {
  const { listCmaSessionOrigins } = await import("./cma-session-api");
  const source = {
    service_id: cmaSession.connection.id,
    agent_id: cmaSession.agentId,
    conversation_id: "nyxa-original",
    specialist_id: "original-specialist",
    turn_id: "original-turn",
    cma_url: "https://example.com:8443/bot",
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string) => {
      const url = new URL(String(input), "https://nyx.example");
      expect(url.searchParams.get("conversation")).toBe(source.conversation_id);
      expect(url.searchParams.get("pins")).toBe(
        `${source.service_id}:${source.agent_id}`,
      );
      return Response.json({ sessions: [source, source], next_cursor: null });
    }),
  );
  expect(
    await listCmaSessionOrigins(
      "owner",
      new AbortController().signal,
      source.conversation_id,
      [`${source.service_id}:${source.agent_id}`],
    ),
  ).toEqual([source]);
});

it("performs no discovery without source or pins and rejects repeated receipt pagination", async () => {
  const { listCmaSessionOrigins } = await import("./cma-session-api");
  const fetcher = vi.fn(async () =>
    Response.json({ sessions: [], next_cursor: cmaSession.connection.id }),
  );
  vi.stubGlobal("fetch", fetcher);
  expect(
    await listCmaSessionOrigins("owner", new AbortController().signal),
  ).toEqual([]);
  expect(fetcher).not.toHaveBeenCalled();
  await expect(
    listCmaSessionOrigins("owner", new AbortController().signal, "source"),
  ).rejects.toThrow("repeated an origin cursor");
  expect(fetcher).toHaveBeenCalledTimes(2);
});

it("cancels a stale reader after switching away and back to the same account", async () => {
  const states: CmaSessionState[] = [];
  const watching = watchCmaSession(
    "owner",
    cmaSession,
    new AbortController().signal,
    (state) => states.push(state),
  );
  await vi.waitFor(() => expect(tailCount).toBe(1));
  const before = states.length;
  transitionAssistantIdentity("other");
  transitionAssistantIdentity("owner");
  tail.enqueue(
    encoder.encode(
      cmaFrame(11, { type: "title_changed", title: "stale account data" }),
    ),
  );
  await watching;
  expect(states).toHaveLength(before);
  expect(
    requests.find((request) => request.url.includes("/events?"))?.config?.signal
      ?.aborted,
  ).toBe(true);
});

it("joins beyond fifty unlinked active candidates without scanning terminal history and resolves pins independently", async () => {
  const connection = cmaSession.connection;
  const origin = {
    service_id: connection.id,
    agent_id: cmaSession.agentId,
    conversation_id: "source",
    specialist_id: "specialist",
    turn_id: "turn",
    cma_url: null,
    connection_name: "My CMA",
    connection_slug: "cma",
  };
  const oldOrigins = Array.from({ length: 150 }, (_, i) => ({
    ...origin,
    agent_id: `agt_terminal_${i}`,
  }));
  const paths: string[] = [];
  const joined: string[][] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string) => {
      const url = new URL(String(input), "https://nyx.example");
      paths.push(url.pathname + url.search);
      if (url.pathname.endsWith("cma-sessions")) {
        if (url.searchParams.has("discover"))
          return Response.json({
            connections: [connection],
            sessions: [],
            next_cursor: null,
          });
        const keys = (
          url.searchParams.get("agents") ??
          url.searchParams.get("pins") ??
          ""
        ).split(",");
        joined.push(keys);
        return Response.json({
          sessions: [...oldOrigins, origin].filter((o) =>
            keys.includes(`${o.service_id}:${o.agent_id}`),
          ),
          next_cursor: null,
        });
      }
      if (url.pathname.endsWith("openapi.json"))
        return Response.json({
          paths: Object.fromEntries(
            Object.values(cmaSession.routes).map((path) => [path, { get: {} }]),
          ),
        });
      if (url.pathname.endsWith("/agents")) {
        expect(["starting", "working", "waiting_approval", "paused"]).toContain(
          url.searchParams.get("status"),
        );
        if (url.searchParams.get("status") !== "working")
          return Response.json({ items: [], next: null });
        return Response.json(
          url.searchParams.get("cursor") === "active-page-2"
            ? {
                items: [
                  {
                    id: origin.agent_id,
                    status: "working",
                    title: "Linked work",
                  },
                ],
                next: null,
              }
            : {
                items: Array.from({ length: 50 }, (_, i) => ({
                  id: `agt_external_${i}`,
                  status: "working",
                })),
                next: "active-page-2",
              },
        );
      }
      throw new Error(`Unexpected request ${url}`);
    }),
  );
  const found = await discoverCmaSessions(
    "owner",
    new AbortController().signal,
    "source",
  );
  expect(found).toHaveLength(1);
  expect(found[0]?.origin).toEqual(origin);
  expect(joined.map((batch) => batch.length)).toEqual([50, 1]);
  expect(paths.some((path) => path.includes("agt_terminal_"))).toBe(false);
  expect(paths.some((path) => /\/agents\/agt_external/.test(path))).toBe(false);
  paths.length = 0;
  const pinned = await listPinnedCmaSessions(
    "owner",
    new AbortController().signal,
    [`${connection.id}:${origin.agent_id}`],
  );
  expect(pinned[0]?.origin).toEqual(origin);
  expect(pinned[0]?.connection.name).toBe("My CMA");
  expect(paths.some((path) => /\/agents\?/.test(path))).toBe(false);
});
it("follows connection pagination and rejects a cyclic active cursor", async () => {
  let connectionPages = 0;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string) => {
      const url = new URL(String(input), "https://nyx.example");
      if (url.pathname.endsWith("cma-sessions")) {
        connectionPages++;
        return Response.json({
          sessions: [],
          connections: connectionPages === 1 ? [] : [cmaSession.connection],
          next_cursor: connectionPages === 1 ? cmaSession.connection.id : null,
        });
      }
      if (url.pathname.endsWith("openapi.json"))
        return new Response(null, { status: 404 });
      const cursor = url.searchParams.get("cursor");
      return Response.json({ items: [], next: cursor === "a" ? "b" : "a" });
    }),
  );
  // A malformed connection is unavailable, without rejecting healthy peers.
  expect(
    await discoverCmaSessions("owner", new AbortController().signal, "source"),
  ).toEqual([]);
  expect(connectionPages).toBe(2);
});

const badConnection = {
  ...cmaSession.connection,
  id: "00000000-0000-4000-8000-000000000001",
  name: "Other CMA",
};
for (const mode of ["source-routes", "source-list", "pins"] as const) {
  it.each([403, 503, "network"] as const)(
    `isolates ${mode} connection failure %s and retains only its authorized transient state`,
    async (failure) => {
      let failing = false;
      let withdraw = false;
      const origin = (id: string) => ({
        service_id: id,
        agent_id: cmaSession.agentId,
        conversation_id: "source",
        specialist_id: "specialist",
        turn_id: "turn",
        cma_url: null,
      });
      const pins = [badConnection, cmaSession.connection].map(
        (connection) => `${connection.id}:${cmaSession.agentId}`,
      );
      const routeReads: string[] = [];
      vi.stubGlobal(
        "fetch",
        vi.fn(async (input: string) => {
          const url = new URL(String(input), "https://nyx.example");
          if (url.pathname.endsWith("cma-sessions")) {
            if (url.searchParams.has("discover"))
              return Response.json({
                sessions: [],
                connections: [badConnection, cmaSession.connection],
                next_cursor: null,
              });
            const requested = (url.searchParams.get("agents") ?? "").split(",");
            return Response.json({
              sessions: [badConnection, cmaSession.connection]
                .filter(
                  (connection) =>
                    !withdraw || connection.id !== badConnection.id,
                )
                .map((connection) => origin(connection.id))
                .filter((receipt) =>
                  requested.includes(
                    `${receipt.service_id}:${receipt.agent_id}`,
                  ),
                ),
              next_cursor: null,
            });
          }
          const bad = url.pathname.includes(badConnection.id);
          const routes = url.pathname.endsWith("openapi.json");
          if (routes) routeReads.push(url.pathname);
          if (failing && bad && (mode === "source-list" ? !routes : routes)) {
            if (failure === "network") throw new TypeError("offline");
            return Response.json(
              { error: "unavailable", message: "Unavailable", error_code: 1 },
              { status: failure },
            );
          }
          if (routes)
            return Response.json({
              paths: Object.fromEntries(
                Object.values(cmaSession.routes).map((path) => [
                  path,
                  { get: {} },
                ]),
              ),
            });
          return Response.json({
            items:
              url.searchParams.get("status") === "working"
                ? [
                    {
                      id: cmaSession.agentId,
                      status: "working",
                      title: failing ? "Updated healthy work" : "Original work",
                    },
                  ]
                : [],
            next: null,
          });
        }),
      );
      const read = (
        previous: Parameters<typeof discoverCmaSessions>[4] = [],
      ) =>
        mode === "pins"
          ? listPinnedCmaSessions(
              "owner",
              new AbortController().signal,
              pins,
              undefined,
              previous,
            )
          : discoverCmaSessions(
              "owner",
              new AbortController().signal,
              "source",
              undefined,
              previous,
            );
      const previous = await read();
      expect(previous).toHaveLength(2);
      failing = true;
      const refreshed = await read(previous);
      expect(
        refreshed.some(
          (session) => session.connection.id === cmaSession.connection.id,
        ),
      ).toBe(true);
      expect(
        refreshed.some((session) => session.connection.id === badConnection.id),
      ).toBe(failure !== 403);
      if (mode !== "pins")
        expect(
          refreshed.find(
            (session) => session.connection.id === cmaSession.connection.id,
          )?.title,
        ).toBe("Updated healthy work");
      expect(
        routeReads.some((path) => path.includes(cmaSession.connection.id)),
      ).toBe(true);
      withdraw = true;
      expect(
        (await read(refreshed)).map((session) => session.connection.id),
      ).toEqual([cmaSession.connection.id]);
    },
  );
}

it("preserves completed peers and denials at the collection deadline, then reaches a late healthy connection", async () => {
  // Fake only timeout signals: native AbortSignal.timeout uses a host timer.
  vi.useFakeTimers();
  vi.spyOn(AbortSignal, "timeout").mockImplementation((ms) => {
    const controller = new AbortController();
    setTimeout(
      () => controller.abort(new DOMException("Timed out", "TimeoutError")),
      ms,
    );
    return controller.signal;
  });
  const connections = Array.from({ length: 13 }, (_, i) => ({
    ...cmaSession.connection,
    id: `00000000-0000-4000-8000-${String(i + 1).padStart(12, "0")}`,
  }));
  const origin = (id: string) => ({
    service_id: id,
    agent_id: cmaSession.agentId,
    conversation_id: "source",
    specialist_id: "specialist",
    turn_id: "turn",
    cma_url: null,
  });
  const previous = [connections[0]!, connections[1]!].map((connection) => ({
    ...cmaSession,
    connection,
    origin: origin(connection.id),
  }));
  const routeReads: string[] = [];
  const getRoutes = async (id: string, signal: AbortSignal) => {
    routeReads.push(id);
    if (id === connections[1]!.id)
      throw new (await import("@/lib/api-client")).ApiError(403, {
        error: "forbidden",
        message: "denied",
        error_code: 1,
      });
    if ([connections[0]!.id, connections.at(-1)!.id].includes(id))
      return cmaSession.routes;
    return new Promise<typeof cmaSession.routes>((_, reject) => {
      signal.addEventListener("abort", () => reject(signal.reason), {
        once: true,
      });
    });
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string) => {
      const url = new URL(String(input), "https://nyx.example");
      if (url.searchParams.has("discover"))
        return Response.json({ sessions: [], connections, next_cursor: null });
      if (url.pathname.endsWith("cma-sessions")) {
        const keys = url.searchParams.get("agents")!.split(",");
        return Response.json({
          sessions: connections
            .map((c) => origin(c.id))
            .filter((o) => keys.includes(`${o.service_id}:${o.agent_id}`)),
          next_cursor: null,
        });
      }
      return Response.json({
        items:
          url.searchParams.get("status") === "working"
            ? [
                {
                  id: cmaSession.agentId,
                  status: "working",
                  title: "Updated healthy work",
                },
              ]
            : [],
        next: null,
      });
    }),
  );
  const progress = {};
  const pending = discoverCmaSessions(
    "owner",
    new AbortController().signal,
    "source",
    getRoutes,
    previous,
    progress,
  );
  await vi.advanceTimersByTimeAsync(25_001);
  const first = await pending;
  expect(first.map((s) => s.connection.id)).toEqual([connections[0]!.id]);
  expect(first[0]?.title).toBe("Updated healthy work");
  expect(routeReads).not.toContain(connections.at(-1)!.id);
  const next = discoverCmaSessions(
    "owner",
    new AbortController().signal,
    "source",
    getRoutes,
    first,
    progress,
  );
  await vi.advanceTimersByTimeAsync(25_001);
  expect((await next).map((s) => s.connection.id)).toContain(
    connections.at(-1)!.id,
  );
  expect(routeReads.filter((id) => id === connections[1]!.id)).toHaveLength(1);
});

it("cancels a partially completed discovery on caller abort rather than publishing partial account data", async () => {
  const controller = new AbortController();
  vi.stubGlobal(
    "fetch",
    vi.fn(async () =>
      Response.json({
        sessions: [],
        connections: [cmaSession.connection],
        next_cursor: null,
      }),
    ),
  );
  const pending = discoverCmaSessions(
    "owner",
    controller.signal,
    "source",
    async (_id, signal) =>
      new Promise((_, reject) => {
        signal.addEventListener("abort", () => reject(signal.reason), {
          once: true,
        });
      }),
  );
  await vi.waitFor(() => expect(vi.mocked(fetch)).toHaveBeenCalled());
  controller.abort();
  await expect(pending).rejects.toMatchObject({ name: "AbortError" });
});

it.each([0, 12_000])(
  "budgets pin route work across slow peers after a %ims receipt read without hiding a late healthy pin or retaining a denied pin",
  async (receiptDelay) => {
    vi.useFakeTimers();
    vi.spyOn(AbortSignal, "timeout").mockImplementation((ms) => {
      const controller = new AbortController();
      setTimeout(
        () => controller.abort(new DOMException("Timed out", "TimeoutError")),
        ms,
      );
      return controller.signal;
    });
    const connections = Array.from({ length: 11 }, (_, i) => ({
      ...cmaSession.connection,
      id: `00000000-0000-4000-8000-${String(i + 1).padStart(12, "0")}`,
    }));
    const origins = connections.map((c) => ({
      service_id: c.id,
      agent_id: cmaSession.agentId,
      conversation_id: "source",
      specialist_id: "specialist",
      turn_id: "turn",
      cma_url: null,
    }));
    const previous = connections.slice(0, 2).map((connection, i) => ({
      ...cmaSession,
      connection,
      origin: origins[i]!,
    }));
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        if (receiptDelay)
          await new Promise((resolve) => setTimeout(resolve, receiptDelay));
        return Response.json({ sessions: origins, next_cursor: null });
      }),
    );
    const pending = listPinnedCmaSessions(
      "owner",
      new AbortController().signal,
      origins.map((o) => `${o.service_id}:${o.agent_id}`),
      async (id, signal) => {
        if (id === connections[1]!.id)
          throw new (await import("@/lib/api-client")).ApiError(403, {
            error: "forbidden",
            message: "denied",
            error_code: 1,
          });
        if (id === connections.at(-1)!.id) return cmaSession.routes;
        return new Promise((_, reject) => {
          signal.addEventListener("abort", () => reject(signal.reason), {
            once: true,
          });
        });
      },
      previous,
    );
    await vi.advanceTimersByTimeAsync(24_001);
    expect((await pending).map((s) => s.connection.id)).toEqual([
      connections[0]!.id,
      connections.at(-1)!.id,
    ]);
  },
);

it("propagates a backend receipt 401 instead of treating it as a local connection denial", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string) => {
      const url = new URL(String(input), "https://nyx.example");
      if (url.searchParams.has("discover"))
        return Response.json({
          sessions: [],
          connections: [cmaSession.connection],
          next_cursor: null,
        });
      if (
        url.pathname.endsWith("cma-sessions") ||
        url.pathname.endsWith("/users/me")
      )
        return Response.json(
          {
            error: "unauthorized",
            message: "Expired NyxID session",
            error_code: 1001,
          },
          { status: 401 },
        );
      return Response.json({
        items: [{ id: cmaSession.agentId, status: "working" }],
        next: null,
      });
    }),
  );
  const clearUser = vi
    .spyOn(useAuthStore.getState(), "setUser")
    .mockImplementation(() => undefined);
  await expect(
    discoverCmaSessions(
      "owner",
      new AbortController().signal,
      "source",
      async () => cmaSession.routes,
    ),
  ).rejects.toMatchObject({ status: 401 });
  expect(clearUser).toHaveBeenCalledWith(null);
});

it.each([403, 503, "network"] as const)(
  "preserves newly receipt-verified partial inventory only on retryable later status failure %s",
  async (failure) => {
    const origin = {
      service_id: cmaSession.connection.id,
      agent_id: cmaSession.agentId,
      conversation_id: "source",
      specialist_id: "specialist",
      turn_id: "turn",
      cma_url: null,
    };
    let failing = true;
    const statuses: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: string) => {
        const url = new URL(String(input), "https://nyx.example");
        if (url.searchParams.has("discover"))
          return Response.json({
            sessions: [],
            connections: [cmaSession.connection],
            next_cursor: null,
          });
        if (url.pathname.endsWith("cma-sessions"))
          return Response.json({ sessions: [origin], next_cursor: null });
        const status = url.searchParams.get("status")!;
        statuses.push(status);
        if (status === "waiting_approval" && failing) {
          if (failure === "network") throw new TypeError("offline");
          return Response.json(
            { error: "unavailable", message: "unavailable", error_code: 1 },
            { status: failure },
          );
        }
        return Response.json({
          items:
            failing && status === "working"
              ? [
                  {
                    id: cmaSession.agentId,
                    status: "working",
                    title: "New verified work",
                  },
                ]
              : [],
          next: null,
        });
      }),
    );
    const sessions = await discoverCmaSessions(
      "owner",
      new AbortController().signal,
      "source",
      async () => cmaSession.routes,
    );
    expect(sessions).toHaveLength(failure === 403 ? 0 : 1);
    if (failure !== 403) expect(sessions[0]?.origin).toEqual(origin);
    expect(statuses).toEqual(["starting", "working", "waiting_approval"]);
    failing = false;
    // A completed inventory with no active work withdraws even a retained partial row.
    expect(
      await discoverCmaSessions(
        "owner",
        new AbortController().signal,
        "source",
        async () => cmaSession.routes,
        sessions,
      ),
    ).toEqual([]);
  },
);
