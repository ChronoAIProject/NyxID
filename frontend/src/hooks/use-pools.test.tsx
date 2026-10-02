import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
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
vi.mock("@/lib/api-client", () => ({ api }));

function wrapperFactory() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
      mutations: { retry: false },
    },
  });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

beforeEach(() => vi.resetAllMocks());

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
