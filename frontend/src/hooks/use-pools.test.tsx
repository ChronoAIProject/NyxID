import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useServicePools,
  usePoolCandidates,
  usePoolHealth,
  useResetPoolHealth,
  useUpdateServicePool,
} from "./use-pools";

const api = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  put: vi.fn(),
  delete: vi.fn(),
}));
vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api-client")>()),
  api,
  apiClient: api.get,
}));

function wrapperFactory({ staleTime = 0, gcTime = 0 } = {}) {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime, staleTime },
      mutations: { retry: false },
    },
  });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

beforeEach(() => vi.resetAllMocks());

it("finishes pending pagination then checks every loaded page against the latest selection", async () => {
  let release: (() => void) | undefined;
  let pageSignal: AbortSignal | undefined;
  api.get.mockImplementation(
    async (path: string, { signal }: { signal: AbortSignal }) => {
      const query = new URL(path, "https://nyxid.invalid").searchParams;
      const peer = query.get("peer_ids");
      if (query.has("after") && peer === "") {
        pageSignal = signal;
        await new Promise<void>((resolve) => {
          release = resolve;
        });
      }
      return {
        candidates: [
          {
            user_service_id: query.has("after") ? "second" : "first",
            eligible: peer !== "selected",
          },
        ],
        next_cursor: query.has("after") ? null : "100",
        has_more: !query.has("after"),
      };
    },
  );
  const { result, rerender } = renderHook(
    ({ peers }) => usePoolCandidates({ peerIds: peers }),
    { wrapper: wrapperFactory(), initialProps: { peers: [] as string[] } },
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  let pending: Promise<unknown>;
  act(() => {
    pending = result.current.fetchNextPage();
  });
  await waitFor(() => expect(release).toBeDefined());
  rerender({ peers: ["selected"] });
  expect(result.current.isCheckingCompatibility).toBe(true);
  expect(pageSignal?.aborted).toBe(false);
  await act(async () => {
    release!();
    await pending;
  });
  await waitFor(() =>
    expect(result.current.isCheckingCompatibility).toBe(false),
  );
  expect(result.current.data?.pages).toHaveLength(2);
  expect(
    result.current.data?.pages
      .flatMap((page) => page.candidates)
      .every((row) => !row.eligible),
  ).toBe(true);
  expect(api.get).toHaveBeenCalledTimes(4);
});

it("marks cached rows busy until the current selection has been checked and stops after an error", async () => {
  let release: (() => void) | undefined;
  api.get.mockImplementation(async (path: string) => {
    const query = new URL(path, "https://nyxid.invalid").searchParams;
    if (!query.has("search") && query.get("peer_ids") === "selected") {
      await new Promise<void>((resolve) => {
        release = resolve;
      });
      throw new Error("Temporary inventory failure");
    }
    return {
      candidates: [{ user_service_id: "first", eligible: true }],
      has_more: false,
      next_cursor: null,
    };
  });
  const { result, rerender } = renderHook(
    ({ search, peers }) => usePoolCandidates({ search, peerIds: peers }),
    {
      wrapper: wrapperFactory({ staleTime: 60000, gcTime: 300000 }),
      initialProps: { search: "", peers: [] as string[] },
    },
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  rerender({ search: "backup", peers: ["selected"] });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  rerender({ search: "", peers: ["selected"] });
  expect(result.current.isCheckingCompatibility).toBe(true);
  await waitFor(() => expect(release).toBeDefined());
  await act(async () => {
    release!();
  });
  await waitFor(() => expect(result.current.isError).toBe(true));
  expect(result.current.isCheckingCompatibility).toBe(true);
  expect(api.get).toHaveBeenCalledTimes(3);
  api.get.mockResolvedValue({
    candidates: [{ user_service_id: "first", eligible: false }],
    has_more: false,
    next_cursor: null,
  });
  await act(() => result.current.refetch());
  await waitFor(() =>
    expect(result.current.isCheckingCompatibility).toBe(false),
  );
  expect(result.current.data?.pages[0]?.candidates[0]?.eligible).toBe(false);
});

describe("pool management requests", () => {
  it("sends configuration and members in one PUT and preserves revision and explicit clears", async () => {
    api.put.mockResolvedValue({ id: "pool-id", config_revision: 8 });
    const { result } = renderHook(() => useUpdateServicePool(), {
      wrapper: wrapperFactory(),
    });
    const body = {
      name: "Changed together",
      strategy: "priority" as const,
      member_contract: "same_api" as const,
      expected_revision: 7,
      description: null,
      failover: null,
      members: [
        {
          user_service_id: "member-id",
          enabled: true,
          weight: 1,
          priority: 2,
          model: null,
          same_api_compatible: true,
        },
      ],
    };
    await act(() => result.current.mutateAsync({ poolId: "pool-id", ...body }));
    expect(api.put).toHaveBeenCalledTimes(1);
    expect(api.put).toHaveBeenCalledWith("/service-pools/pool-id", body);
    expect(api.get).not.toHaveBeenCalled();
    expect(api.post).not.toHaveBeenCalled();
    expect(api.delete).not.toHaveBeenCalled();
  });

  it("surfaces a revision conflict without retrying or issuing a partial member write", async () => {
    const conflict = new Error(
      "Pool configuration changed; reload before saving",
    );
    api.put.mockRejectedValue(conflict);
    const { result } = renderHook(() => useUpdateServicePool(), {
      wrapper: wrapperFactory(),
    });
    await act(async () => {
      await expect(
        result.current.mutateAsync({
          poolId: "pool-id",
          expected_revision: 7,
          name: "Old draft",
          members: [],
        }),
      ).rejects.toBe(conflict);
    });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.error).toBe(conflict);
    expect(api.put).toHaveBeenCalledTimes(1);
    expect(api.post).not.toHaveBeenCalled();
    expect(api.delete).not.toHaveBeenCalled();
  });

  it("retains owner and operation on subsequent candidate pages and starts a fresh page for another owner", async () => {
    api.get.mockImplementation(async (path: string) => {
      const query = new URL(path, "https://nyxid.invalid").searchParams;
      const after = query.get("after");
      return {
        candidates: [
          { user_service_id: `${query.get("org_id")}:${after ?? "first"}` },
        ],
        next_cursor: after ? null : "100",
        has_more: !after,
      };
    });
    const { result, rerender } = renderHook(
      ({ owner }) =>
        usePoolCandidates({
          orgId: owner,
          contract: "same_api",
          strategy: "priority",
          declaredPeerIds: ["member-id"],
          method: "GET",
          path: "items/detail",
          search: "research & tools",
          peerIds: ["member-id"],
        }),
      { wrapper: wrapperFactory(), initialProps: { owner: "org-one" } },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.pages).toHaveLength(1);
    await act(() => result.current.fetchNextPage());
    await waitFor(() => expect(result.current.data?.pages).toHaveLength(2));
    const second = new URL(api.get.mock.calls[1]![0], "https://nyxid.invalid");
    expect(second.pathname).toBe("/service-pools/candidates");
    expect(Object.fromEntries(second.searchParams)).toMatchObject({
      org_id: "org-one",
      member_contract: "same_api",
      strategy: "priority",
      declared_peer_ids: "member-id",
      method: "GET",
      path: "items/detail",
      search: "research & tools",
      peer_ids: "member-id",
      after: "100",
    });
    expect(result.current.hasNextPage).toBe(false);
    rerender({ owner: "org-two" });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.pages).toHaveLength(1);
    expect(result.current.data?.pages[0]?.candidates[0]?.user_service_id).toBe(
      "org-two:first",
    );
    const last = new URL(
      api.get.mock.calls.at(-1)![0],
      "https://nyxid.invalid",
    );
    expect(last.searchParams.get("org_id")).toBe("org-two");
    expect(last.searchParams.has("after")).toBe(false);
  });

  it("explicitly browses inventory and fetches selected draft IDs independently of search", async () => {
    api.get.mockResolvedValue({
      candidates: [],
      operation_checked: false,
      method: null,
      path: null,
      next_cursor: null,
      has_more: false,
    });
    const { result } = renderHook(
      () =>
        usePoolCandidates({
          poolId: "pool-id",
          checkOperation: false,
          selectedOnly: true,
          peerIds: ["one", "two"],
          declaredPeerIds: ["one"],
        }),
      { wrapper: wrapperFactory() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const request = new URL(api.get.mock.calls[0]![0], "https://nyxid.invalid");
    expect(Object.fromEntries(request.searchParams)).toMatchObject({
      check_operation: "false",
      selected_only: "true",
      peer_ids: "one,two",
      declared_peer_ids: "one",
    });
    expect(request.searchParams.has("method")).toBe(false);
    expect(request.searchParams.has("path")).toBe(false);
    expect(result.current.data?.pages[0]).toMatchObject({
      operation_checked: false,
      method: null,
      path: null,
    });
  });

  it("resets the selected member and refreshes the matching operation health", async () => {
    const cooled = {
      user_service_id: "member-id",
      eligible: false,
      reason: "cooldown",
      consecutive_failures: 2,
    };
    const ready = {
      ...cooled,
      eligible: true,
      reason: null,
      consecutive_failures: 0,
    };
    api.get
      .mockResolvedValueOnce({
        candidates: [cooled],
        has_more: false,
        next_cursor: null,
      })
      .mockResolvedValue({
        candidates: [ready],
        has_more: false,
        next_cursor: null,
      });
    api.post.mockResolvedValue({ reset: true });
    const { result } = renderHook(
      () => ({
        health: usePoolHealth({
          poolId: "pool-id",
          contract: "ai_chat",
          checkOperation: true,
          method: "POST",
          path: "chat/completions",
        }),
        reset: useResetPoolHealth(),
      }),
      { wrapper: wrapperFactory() },
    );
    await waitFor(() => expect(result.current.health.isSuccess).toBe(true));
    expect(result.current.health.data?.candidates[0]?.reason).toBe("cooldown");
    await act(() =>
      result.current.reset.mutateAsync({
        poolId: "pool-id",
        userServiceId: "member-id",
      }),
    );
    expect(api.post).toHaveBeenCalledWith(
      "/service-pools/pool-id/health/reset",
      {
        user_service_id: "member-id",
      },
    );
    await waitFor(() =>
      expect(result.current.health.data?.candidates[0]?.eligible).toBe(true),
    );
    expect(api.get).toHaveBeenCalledTimes(2);
    const healthUrl = new URL(
      api.get.mock.calls[1]![0],
      "https://nyxid.invalid",
    );
    expect(healthUrl.pathname).toBe("/service-pools/pool-id/health");
    expect(healthUrl.searchParams.get("path")).toBe("chat/completions");
  });
});

it("keeps every loaded inventory page while draft compatibility refreshes", async () => {
  let release: (() => void) | undefined;
  api.get.mockImplementation(async (path: string) => {
    const query = new URL(path, "https://nyxid.invalid").searchParams;
    const peers = query.get("peer_ids");
    if (peers === "selected" && !query.has("after"))
      await new Promise<void>((resolve) => {
        release = resolve;
      });
    return {
      candidates: [
        {
          user_service_id: query.has("after") ? "second" : "first",
          reason:
            peers === "selected" ? "compatibility_declaration_required" : null,
        },
      ],
      next_cursor: query.has("after") ? null : "100",
      has_more: !query.has("after"),
    };
  });
  const { result, rerender } = renderHook(
    ({ peers }) => usePoolCandidates({ peerIds: peers, checkOperation: false }),
    {
      wrapper: wrapperFactory(),
      initialProps: { peers: [] as string[] },
    },
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(result.current.data?.pages).toHaveLength(1);
  await act(() => result.current.fetchNextPage());
  await waitFor(() => expect(result.current.data?.pages).toHaveLength(2));
  rerender({ peers: ["selected"] });
  await waitFor(() => expect(release).toBeDefined());
  expect(result.current.data?.pages).toHaveLength(2);
  expect(result.current.isLoading).toBe(false);
  await act(async () => {
    release!();
  });
  await waitFor(() => expect(result.current.isFetching).toBe(false));
  expect(result.current.data?.pages).toHaveLength(2);
  expect(result.current.data?.pages[1]?.candidates[0]?.reason).toBe(
    "compatibility_declaration_required",
  );
});

it("refreshes the list after a revision conflict so reopening uses the latest pool", async () => {
  const { ApiError } = await import("@/lib/api-client");
  api.get
    .mockResolvedValueOnce({ pools: [{ id: "pool", config_revision: 1 }] })
    .mockResolvedValue({ pools: [{ id: "pool", config_revision: 2 }] });
  api.put.mockRejectedValue(
    new ApiError(409, {
      error: "conflict",
      error_code: 1009,
      message: "Changed elsewhere",
    }),
  );
  const { result } = renderHook(
    () => ({ pools: useServicePools(), update: useUpdateServicePool() }),
    {
      wrapper: wrapperFactory(),
    },
  );
  await waitFor(() =>
    expect(result.current.pools.data?.[0]?.config_revision).toBe(1),
  );
  await act(async () => {
    await expect(
      result.current.update.mutateAsync({
        poolId: "pool",
        name: "stale",
        expected_revision: 1,
      }),
    ).rejects.toMatchObject({ status: 409 });
  });
  await waitFor(() =>
    expect(result.current.pools.data?.[0]?.config_revision).toBe(2),
  );
  expect(api.put).toHaveBeenCalledTimes(1);
});

it.each(["peerIds", "declaredPeerIds"] as const)(
  "rechecks a cached search after %s changes under production cache settings",
  async (field) => {
    api.get.mockImplementation(async (path: string) => {
      const query = new URL(path, "https://nyxid.invalid").searchParams;
      const changed =
        query.get(field === "peerIds" ? "peer_ids" : "declared_peer_ids") ===
        "B";
      return {
        candidates: [{ user_service_id: "candidate", eligible: !changed }],
        has_more: false,
        next_cursor: null,
      };
    });
    const { result, rerender } = renderHook(
      ({ search, ids }) => usePoolCandidates({ search, [field]: ids }),
      {
        wrapper: wrapperFactory({ staleTime: 60000, gcTime: 300000 }),
        initialProps: { search: "", ids: [] as string[] },
      },
    );
    await waitFor(() =>
      expect(result.current.data?.pages[0]?.candidates[0]?.eligible).toBe(true),
    );
    rerender({ search: "backup", ids: [] });
    await waitFor(() => expect(api.get).toHaveBeenCalledTimes(2));
    await waitFor(() =>
      expect(result.current.data?.pages[0]?.candidates[0]?.eligible).toBe(true),
    );
    rerender({ search: "backup", ids: ["B"] });
    await waitFor(() =>
      expect(result.current.data?.pages[0]?.candidates[0]?.eligible).toBe(
        false,
      ),
    );
    rerender({ search: "", ids: ["B"] });
    await waitFor(() =>
      expect(result.current.data?.pages[0]?.candidates[0]?.eligible).toBe(
        false,
      ),
    );
    expect(api.get).toHaveBeenCalledTimes(4);
    const request = new URL(
      api.get.mock.calls.at(-1)![0],
      "https://nyxid.invalid",
    );
    expect(request.searchParams.get("search")).toBeNull();
    expect(
      request.searchParams.get(
        field === "peerIds" ? "peer_ids" : "declared_peer_ids",
      ),
    ).toBe("B");
  },
);
