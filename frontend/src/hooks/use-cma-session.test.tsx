import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useActiveCmaSessions, useCmaSessionStream } from "./use-cma-session";
import { ApiError } from "@/lib/api-client";
import { cmaSession } from "@/lib/assistant/cma-session.test-fixtures";
import type { CmaSessionOrigin } from "@/schemas/cma-session";

const api = vi.hoisted(() => ({
  origins: vi.fn(),
  pins: vi.fn(),
  read: vi.fn(),
  routes: vi.fn(),
  watch: vi.fn(),
  initial: vi.fn(),
}));
vi.mock("@/lib/assistant/cma-session-api", () => ({
  discoverCmaSessions: api.origins,
  listPinnedCmaSessions: api.pins,
  isCmaRetryableError: (error: unknown) =>
    error instanceof TypeError ||
    (error instanceof ApiError &&
      (error.status === 429 || error.status >= 500)),
  readCmaAgent: api.read,
  resolveCmaRoutes: api.routes,
  watchCmaSession: api.watch,
  initialCmaSession: api.initial,
}));
let client: QueryClient;
const origin: CmaSessionOrigin = {
  service_id: cmaSession.connection.id,
  agent_id: cmaSession.agentId,
  conversation_id: "source",
  turn_id: "turn-original",
  specialist_id: "specialist-original",
  cma_url: null,
};
const key = `${origin.service_id}:${origin.agent_id}`;
function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
beforeEach(() => {
  vi.resetAllMocks();
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  api.origins.mockResolvedValue([{ ...cmaSession, origin }]);
  api.pins.mockResolvedValue([{ ...cmaSession, origin }]);
  api.routes.mockResolvedValue(cmaSession.routes);
  api.read.mockResolvedValue({
    id: origin.agent_id,
    title: cmaSession.title,
    status: "working",
  });
  api.initial.mockReturnValue({
    connection: "connecting",
    status: "unknown",
    title: cmaSession.title,
    actions: [],
  });
  api.watch.mockResolvedValue(undefined);
});
afterEach(() => client.clear());

it("shows only receipt-linked source work by default, pin follows routes, and unpin restores source visibility", async () => {
  const { result, rerender } = renderHook(
    ({ conversation, pinned }) =>
      useActiveCmaSessions("owner", conversation, pinned),
    {
      wrapper,
      initialProps: {
        conversation: "source" as string | undefined,
        pinned: {} as Record<string, boolean>,
      },
    },
  );
  await waitFor(() => expect(result.current).toHaveLength(1));
  expect(result.current[0]?.origin).toEqual(origin);
  rerender({ conversation: "other", pinned: {} });
  await waitFor(() => expect(result.current).toHaveLength(0));
  rerender({ conversation: "other", pinned: { [key]: true } });
  await waitFor(() => expect(result.current).toHaveLength(1));
  rerender({ conversation: undefined, pinned: { [key]: true } });
  await waitFor(() => expect(result.current).toHaveLength(1));
  rerender({ conversation: undefined, pinned: {} });
  await waitFor(() => expect(result.current).toHaveLength(0));
  rerender({ conversation: "source", pinned: {} });
  await waitFor(() => expect(result.current).toHaveLength(1));
});
it("never probes unlinked inventory or creates work from stale pin metadata", async () => {
  api.origins.mockResolvedValue([]);
  api.pins.mockResolvedValue([]);
  const { result } = renderHook(
    () => useActiveCmaSessions("owner", "source", { [key]: true }),
    { wrapper },
  );
  await waitFor(() => expect(api.origins).toHaveBeenCalled());
  expect(result.current).toEqual([]);
  expect(api.read).not.toHaveBeenCalled();
  expect(api.routes).not.toHaveBeenCalled();
});
it("aborts a stream on removal, account change and unmount, without restarting for refreshed receipt objects", () => {
  const signals: AbortSignal[] = [];
  api.watch.mockImplementation((_owner, _session, signal: AbortSignal) => {
    signals.push(signal);
    return new Promise(() => undefined);
  });
  const { rerender, unmount } = renderHook(
    ({ owner, session }) => useCmaSessionStream(owner, session),
    {
      initialProps: { owner: "owner", session: { ...cmaSession, origin } },
    },
  );
  expect(signals).toHaveLength(1);
  rerender({
    owner: "owner",
    session: {
      ...cmaSession,
      origin: { ...origin, cma_url: "https://example.com" },
    },
  });
  expect(signals).toHaveLength(1);
  rerender({ owner: "other", session: { ...cmaSession, origin } });
  expect(signals[0]?.aborted).toBe(true);
  expect(signals).toHaveLength(2);
  unmount();
  expect(signals[1]?.aborted).toBe(true);
});
it("keeps the initial observation stable while the stream has not published", () => {
  api.initial.mockImplementation(({ title }) => ({
    connection: "connecting",
    status: "unknown",
    title,
    actions: [],
  }));
  const { result, rerender } = renderHook(
    ({ session }) => useCmaSessionStream("owner", session),
    { initialProps: { session: { ...cmaSession, origin } } },
  );
  const initial = result.current.state;
  rerender({ session: { ...cmaSession, origin: { ...origin } } });
  expect(result.current.state).toBe(initial);
  expect(api.watch).toHaveBeenCalledTimes(1);
});
it("clears observations when server authorization or active detail disappears", async () => {
  const { result } = renderHook(
    () => useActiveCmaSessions("owner", "source", {}),
    { wrapper },
  );
  await waitFor(() => expect(result.current).toHaveLength(1));
  api.read.mockResolvedValue({ id: origin.agent_id, status: "ready" });
  await act(() =>
    client.invalidateQueries({
      queryKey: ["cma-session", "owner", origin.service_id, origin.agent_id],
    }),
  );
  await waitFor(() => expect(result.current).toHaveLength(0));
});

it("uses only a validated supplied detail callback and lets null/invalid withdraw an older receipt link", async () => {
  api.read.mockResolvedValue({
    id: origin.agent_id,
    status: "working",
    callback_url: "https://cma.example:8443/bot",
  });
  const { result } = renderHook(
    () => useActiveCmaSessions("owner", "source", {}),
    { wrapper },
  );
  await waitFor(() =>
    expect(result.current[0]?.callbackUrl).toBe("https://cma.example:8443/bot"),
  );
  api.read.mockResolvedValue({
    id: origin.agent_id,
    status: "working",
    callback_url: "https://secret@example.com/bot",
  });
  await act(() =>
    client.invalidateQueries({
      queryKey: ["cma-session", "owner", origin.service_id, origin.agent_id],
    }),
  );
  await waitFor(() => expect(result.current[0]?.callbackUrl).toBeNull());
});

it.each([
  new TypeError("offline"),
  new ApiError(429, { error: "limited", message: "limited", error_code: 1 }),
  new ApiError(503, {
    error: "unavailable",
    message: "unavailable",
    error_code: 1,
  }),
])(
  "retains healthy detail and receipt on transient refresh %s",
  async (error) => {
    const { result } = renderHook(
      () => useActiveCmaSessions("owner", "source", {}),
      { wrapper },
    );
    await waitFor(() => expect(result.current).toHaveLength(1));
    api.read.mockRejectedValue(error);
    await act(() =>
      client.invalidateQueries({
        queryKey: ["cma-session", "owner", origin.service_id, origin.agent_id],
      }),
    );
    expect(result.current).toHaveLength(1);
    api.origins.mockRejectedValue(error);
    await act(() =>
      client.invalidateQueries({
        queryKey: ["cma-session", "owner", "origins"],
      }),
    );
    expect(result.current).toHaveLength(1);
  },
);
it.each([401, 403, 404])(
  "clears a detail authorization failure %s",
  async (status) => {
    const { result } = renderHook(
      () => useActiveCmaSessions("owner", "source", {}),
      { wrapper },
    );
    await waitFor(() => expect(result.current).toHaveLength(1));
    api.read.mockRejectedValue(
      new ApiError(status, {
        error: "unavailable",
        message: "unavailable",
        error_code: 1,
      }),
    );
    await act(() =>
      client.invalidateQueries({
        queryKey: ["cma-session", "owner", origin.service_id, origin.agent_id],
      }),
    );
    await waitFor(() => expect(result.current).toHaveLength(0));
    api.read.mockRejectedValue(new TypeError("offline after revocation"));
    await act(() =>
      client.invalidateQueries({
        queryKey: ["cma-session", "owner", origin.service_id, origin.agent_id],
      }),
    );
    expect(result.current).toHaveLength(0);
  },
);
it("does not use another account or source as placeholder data", async () => {
  const { result, rerender } = renderHook(
    ({ owner, source }) => useActiveCmaSessions(owner, source, {}),
    { wrapper, initialProps: { owner: "owner", source: "source" } },
  );
  await waitFor(() => expect(result.current).toHaveLength(1));
  api.origins.mockImplementation(() => new Promise(() => undefined));
  rerender({ owner: "other", source: "source" });
  expect(result.current).toHaveLength(0);
  rerender({ owner: "owner", source: "another" });
  expect(result.current).toHaveLength(0);
});

it.each([401, 403, 404])(
  "clears a revoked receipt even when detail still has cached success: %s",
  async (status) => {
    const { result } = renderHook(
      () => useActiveCmaSessions("owner", "source", {}),
      { wrapper },
    );
    await waitFor(() => expect(result.current).toHaveLength(1));
    api.origins.mockRejectedValue(
      new ApiError(status, {
        error: "unavailable",
        message: "unavailable",
        error_code: 1,
      }),
    );
    await act(() =>
      client.invalidateQueries({
        queryKey: ["cma-session", "owner", "origins"],
      }),
    );
    await waitFor(() => expect(result.current).toHaveLength(0));
  },
);
it("removes a withdrawn receipt and preserves independent pin discovery during inventory failure", async () => {
  const { result } = renderHook(
    () => useActiveCmaSessions("owner", "source", { [key]: true }),
    { wrapper },
  );
  await waitFor(() => expect(result.current).toHaveLength(1));
  api.origins.mockResolvedValue([]);
  await act(() =>
    client.invalidateQueries({ queryKey: ["cma-session", "owner", "origins"] }),
  );
  expect(result.current).toHaveLength(1);
  api.origins.mockRejectedValue(new TypeError("inventory unavailable"));
  await act(() =>
    client.invalidateQueries({ queryKey: ["cma-session", "owner", "origins"] }),
  );
  expect(result.current).toHaveLength(1);
  api.pins.mockResolvedValue([]);
  await act(() =>
    client.invalidateQueries({ queryKey: ["cma-session", "owner", "pins"] }),
  );
  await waitFor(() => expect(result.current).toHaveLength(0));
});
it("fifty inactive historical pins cannot consume the slot of an active source receipt", async () => {
  const pins = Object.fromEntries(
    Array.from({ length: 50 }, (_, i) => [
      `${origin.service_id}:agt_old_${i}`,
      true,
    ]),
  );
  api.pins.mockResolvedValue(
    Array.from({ length: 50 }, (_, i) => ({
      ...cmaSession,
      agentId: `agt_old_${i}`,
      origin: { ...origin, agent_id: `agt_old_${i}`, conversation_id: "other" },
    })),
  );
  api.read.mockImplementation((_owner, { agentId }) =>
    Promise.resolve({
      id: agentId,
      status: agentId === origin.agent_id ? "working" : "ready",
    }),
  );
  const { result } = renderHook(
    () => useActiveCmaSessions("owner", "source", pins),
    { wrapper },
  );
  await waitFor(() => expect(result.current).toHaveLength(1));
  expect(result.current[0]?.agentId).toBe(origin.agent_id);
});

it("promotes only an explicitly pinned verified identity while its receipt refresh is pending", async () => {
  let finishPin!: (sessions: (typeof cmaSession)[]) => void;
  api.pins.mockImplementation(
    () =>
      new Promise((resolve) => {
        finishPin = resolve;
      }),
  );
  const { result, rerender } = renderHook(
    ({ conversation, pinned }) =>
      useActiveCmaSessions("owner", conversation, pinned),
    {
      wrapper,
      initialProps: {
        conversation: "source" as string | undefined,
        pinned: {} as Record<string, boolean>,
      },
    },
  );
  await waitFor(() => expect(result.current).toHaveLength(1));
  rerender({ conversation: "source", pinned: { [key]: true } });
  rerender({ conversation: undefined, pinned: { [key]: true } });
  expect(result.current).toHaveLength(1);
  expect(result.current[0]?.origin).toEqual(origin);
  await act(async () => finishPin([]));
  await waitFor(() => expect(result.current).toHaveLength(0));
  rerender({ conversation: undefined, pinned: {} });
  expect(result.current).toHaveLength(0);
});

it.each(["another-account", "unseen-identity"])(
  "does not promote %s into a pending pin query",
  async (change) => {
    const { result, rerender } = renderHook(
      ({ owner, conversation, pinned }) =>
        useActiveCmaSessions(owner, conversation, pinned),
      {
        wrapper,
        initialProps: {
          owner: "owner",
          conversation: "source" as string | undefined,
          pinned: {} as Record<string, boolean>,
        },
      },
    );
    await waitFor(() => expect(result.current).toHaveLength(1));
    api.pins.mockImplementation(() => new Promise(() => undefined));
    api.origins.mockImplementation(() => new Promise(() => undefined));
    rerender({
      owner: change === "another-account" ? "other" : "owner",
      conversation: undefined,
      pinned: {
        [change === "unseen-identity"
          ? `${origin.service_id}:agt_unseen`
          : key]: true,
      },
    });
    expect(result.current).toHaveLength(0);
  },
);

it("repins an authorized visible identity without a gap when an older pin query cached no receipts", async () => {
  const { result, rerender } = renderHook(
    ({ conversation, pinned }) =>
      useActiveCmaSessions("owner", conversation, pinned),
    {
      wrapper,
      initialProps: {
        conversation: "source" as string | undefined,
        pinned: {} as Record<string, boolean>,
      },
    },
  );
  await waitFor(() => expect(result.current).toHaveLength(1));
  client.setQueryData(["cma-session", "owner", "pins", [key]], []);
  let finishPin!: (sessions: (typeof cmaSession)[]) => void;
  api.pins.mockImplementation(
    () =>
      new Promise((resolve) => {
        finishPin = resolve;
      }),
  );
  rerender({ conversation: "source", pinned: { [key]: true } });
  rerender({ conversation: undefined, pinned: { [key]: true } });
  expect(result.current).toHaveLength(1);
  await waitFor(() => expect(finishPin).toBeTypeOf("function"));
  await act(async () => finishPin([]));
  await waitFor(() => expect(result.current).toHaveLength(0));
});

it.each(["empty", "error"])(
  "promotes over a cached %s once, then honors withdrawal and does not resurrect on a later transient refresh",
  async (cached) => {
    const { result, rerender } = renderHook(
      ({ conversation, pinned }) =>
        useActiveCmaSessions("owner", conversation, pinned),
      {
        wrapper,
        initialProps: {
          conversation: "source" as string | undefined,
          pinned: {} as Record<string, boolean>,
        },
      },
    );
    await waitFor(() => expect(result.current).toHaveLength(1));
    const pinQueryKey = ["cma-session", "owner", "pins", [key]];
    if (cached === "empty") client.setQueryData(pinQueryKey, []);
    else
      await client
        .fetchQuery({
          queryKey: pinQueryKey,
          queryFn: () =>
            Promise.reject(
              new ApiError(403, {
                error: "forbidden",
                message: "revoked",
                error_code: 1,
              }),
            ),
        })
        .catch(() => undefined);
    let finish!: (sessions: (typeof cmaSession)[]) => void;
    api.pins.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    rerender({ conversation: "source", pinned: { [key]: true } });
    rerender({ conversation: undefined, pinned: { [key]: true } });
    expect(result.current).toHaveLength(1);
    await waitFor(() => expect(finish).toBeTypeOf("function"));
    await act(async () => finish([]));
    await waitFor(() => expect(result.current).toHaveLength(0));
    api.pins.mockImplementation(async (...args) => {
      expect(args[4]).toEqual([]); // Promotion is consumed by the first authoritative refresh.
      throw new TypeError("offline");
    });
    await act(async () => {
      await client.invalidateQueries({ queryKey: pinQueryKey });
    });
    expect(result.current).toHaveLength(0);
    rerender({ conversation: undefined, pinned: {} });
    rerender({ conversation: undefined, pinned: { [key]: true } });
    expect(result.current).toHaveLength(0);
  },
);

it("withdraws an explicit promotion on a new 403 and never promotes it again during ordinary refreshes", async () => {
  const { result, rerender } = renderHook(
    ({ conversation, pinned }) =>
      useActiveCmaSessions("owner", conversation, pinned),
    {
      wrapper,
      initialProps: {
        conversation: "source" as string | undefined,
        pinned: {} as Record<string, boolean>,
      },
    },
  );
  await waitFor(() => expect(result.current).toHaveLength(1));
  let fail!: (error: Error) => void;
  api.pins.mockImplementation(
    () =>
      new Promise((_, reject) => {
        fail = reject;
      }),
  );
  rerender({ conversation: "source", pinned: { [key]: true } });
  rerender({ conversation: undefined, pinned: { [key]: true } });
  expect(result.current).toHaveLength(1);
  await waitFor(() => expect(fail).toBeTypeOf("function"));
  await act(async () =>
    fail(
      new ApiError(403, {
        error: "forbidden",
        message: "revoked",
        error_code: 1,
      }),
    ),
  );
  await waitFor(() => expect(result.current).toHaveLength(0));
  api.pins.mockRejectedValue(new TypeError("offline"));
  await act(async () => {
    await client.invalidateQueries({
      queryKey: ["cma-session", "owner", "pins"],
    });
  });
  expect(result.current).toHaveLength(0);
});

it("does not revive an old successful pin cache after unpin and repin without a currently verified visible source", async () => {
  const { result, rerender } = renderHook(
    ({ pinned }) => useActiveCmaSessions("owner", undefined, pinned),
    {
      wrapper,
      initialProps: { pinned: { [key]: true } as Record<string, boolean> },
    },
  );
  await waitFor(() => expect(result.current).toHaveLength(1));
  rerender({ pinned: {} });
  expect(result.current).toHaveLength(0);
  api.pins.mockImplementation(() => new Promise(() => undefined));
  rerender({ pinned: { [key]: true } });
  expect(result.current).toHaveLength(0);
});

it("keeps an explicitly promoted visible source over cached empty pins through retryable refreshes, then honors successful withdrawal", async () => {
  const pinQueryKey = ["cma-session", "owner", "pins", [key]];
  client.setQueryData(pinQueryKey, []);
  const { result, rerender } = renderHook(
    ({ conversation, pinned }) =>
      useActiveCmaSessions("owner", conversation, pinned),
    {
      wrapper,
      initialProps: {
        conversation: "source" as string | undefined,
        pinned: {} as Record<string, boolean>,
      },
    },
  );
  await waitFor(() => expect(result.current).toHaveLength(1));
  let fail!: (error: Error) => void;
  api.pins.mockImplementation(
    () =>
      new Promise((_, reject) => {
        fail = reject;
      }),
  );
  rerender({ conversation: "source", pinned: { [key]: true } });
  rerender({ conversation: undefined, pinned: { [key]: true } });
  expect(result.current).toHaveLength(1);
  await waitFor(() => expect(fail).toBeTypeOf("function"));
  await act(async () =>
    fail(
      new ApiError(503, {
        error: "unavailable",
        message: "offline",
        error_code: 1,
      }),
    ),
  );
  await waitFor(() =>
    expect(client.getQueryState(pinQueryKey)?.status).toBe("error"),
  );
  expect(result.current).toHaveLength(1);
  api.pins.mockImplementation(async (...args) => {
    expect(args[4]).toHaveLength(1);
    throw new TypeError("still offline");
  });
  await act(async () => {
    await client.invalidateQueries({ queryKey: pinQueryKey });
  });
  expect(result.current).toHaveLength(1);
  api.pins.mockResolvedValue([]);
  await act(async () => {
    await client.invalidateQueries({ queryKey: pinQueryKey });
  });
  await waitFor(() => expect(result.current).toHaveLength(0));
  api.pins.mockRejectedValue(new TypeError("offline after withdrawal"));
  await act(async () => {
    await client.invalidateQueries({ queryKey: pinQueryKey });
  });
  expect(result.current).toHaveLength(0);
});
