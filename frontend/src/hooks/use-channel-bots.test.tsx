import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import type { CreateChannelBotRequest } from "@/types/channels";
import {
  useChannelBot,
  useChannelBots,
  useCreateChannelBot,
  useDeleteChannelBot,
  useUpdateChannelBot,
  useVerifyChannelBot,
  channelBotsQueryKeys,
} from "./use-channel-bots";

const { mockDelete, mockGet, mockPatch, mockPost } = vi.hoisted(() => ({
  mockDelete: vi.fn(),
  mockGet: vi.fn(),
  mockPatch: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    delete: mockDelete,
    get: mockGet,
    patch: mockPatch,
    post: mockPost,
  },
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });

  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useChannelBots", () => {
  it("keeps all-scope, personal and organization results separate when switching", async () => {
    mockGet.mockImplementation(async (path: string) => ({ bots: [{ id: path }] }));
    const { result, rerender } = renderHook(
      ({ scope }: { scope: string | null }) => useChannelBots(
        scope === "all" ? { scope: "all" } : scope === null ? { scope: "user" } : { orgId: scope },
      ),
      { wrapper: createWrapper(), initialProps: { scope: "all" as string | null } },
    );
    await waitFor(() => expect(result.current.data).toEqual([{ id: "/channel-bots?scope=all" }]));
    rerender({ scope: null });
    await waitFor(() => expect(result.current.data).toEqual([{ id: "/channel-bots?scope=user" }]));
    rerender({ scope: "org-1" });
    await waitFor(() => expect(result.current.data).toEqual([{ id: "/channel-bots?org_id=org-1" }]));
    rerender({ scope: "all" });
    await waitFor(() => expect(result.current.data).toEqual([{ id: "/channel-bots?scope=all" }]));
  });

  it("lists personal bots at the bare endpoint and unwraps `bots`", async () => {
    mockGet.mockResolvedValue({ bots: [{ id: "bot-1" }] });
    const { result } = renderHook(() => useChannelBots(), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/channel-bots");
    expect(result.current.data).toEqual([{ id: "bot-1" }]);
  });

  it("appends an encoded org_id query param when scoped to an org", async () => {
    mockGet.mockResolvedValue({ bots: [] });
    const { result } = renderHook(() => useChannelBots({ orgId: "org/1" }), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/channel-bots?org_id=org%2F1");
  });
});

describe("useChannelBot", () => {
  it("fetches by id and stays idle for an empty id", async () => {
    mockGet.mockResolvedValue({ id: "bot-1" });
    const idle = renderHook(() => useChannelBot(""), {
      wrapper: createWrapper(),
    });
    expect(idle.result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    const active = renderHook(() => useChannelBot("bot-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(active.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/channel-bots/bot-1");
  });
});

describe("channel bot mutations", () => {
  it("useCreateChannelBot POSTs the create payload", async () => {
    mockPost.mockResolvedValue({ id: "bot-1" });
    const { result } = renderHook(() => useCreateChannelBot(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      platform: "telegram",
      label: "support",
    } as unknown as CreateChannelBotRequest);
    expect(mockPost).toHaveBeenCalledWith("/channel-bots", {
      platform: "telegram",
      label: "support",
    });
  });

  it.each([null, "org-1", "all"])(
    "shows a saved bot after webhook setup fails in scope %s",
    async (scope) => {
      const orgId = scope === "all" ? null : scope;
      const client = new QueryClient({
        defaultOptions: {
          queries: { retry: false, staleTime: Infinity },
          mutations: { retry: false },
        },
      });
      client.setQueryData(
        scope === "all"
          ? channelBotsQueryKeys.allScopes()
          : channelBotsQueryKeys.list(orgId),
        [],
      );
      const failed = {
        id: "manager-1",
        credential_source: "telegram_manager",
        status: "failed",
        webhook_registered: false,
      };
      mockGet.mockResolvedValue({ bots: [failed] });
      mockPost.mockRejectedValue(new Error("Webhook registration failed"));
      const { result } = renderHook(
        () => ({
          list: useChannelBots(scope === "all" ? { scope: "all" } : { orgId }),
          create: useCreateChannelBot(),
        }),
        {
          wrapper: ({ children }: PropsWithChildren) => (
            <QueryClientProvider client={client}>{children}</QueryClientProvider>
          ),
        },
      );
      expect(result.current.list.data).toEqual([]);
      expect(mockGet).not.toHaveBeenCalled();

      await expect(
        result.current.create.mutateAsync({
          platform: "telegram",
          label: "Manager",
          bot_token: "test-token",
          ...(orgId ? { target_org_id: orgId } : {}),
        }),
      ).rejects.toThrow("Webhook registration failed");

      await waitFor(() => expect(result.current.list.data).toEqual([failed]));
      expect(mockPost).toHaveBeenCalledTimes(1);
      expect(mockGet).toHaveBeenCalledWith(
        scope === "all"
          ? "/channel-bots?scope=all"
          : orgId ? `/channel-bots?org_id=${orgId}` : "/channel-bots",
      );
      client.clear();
    },
  );

  it("useUpdateChannelBot PATCHes the specific bot with the data", async () => {
    mockPatch.mockResolvedValue({ id: "bot-1" });
    const { result } = renderHook(() => useUpdateChannelBot(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      id: "bot-1",
      data: { verification_token: "vtoken_x" },
    });
    expect(mockPatch).toHaveBeenCalledWith("/channel-bots/bot-1", {
      verification_token: "vtoken_x",
    });
  });

  it("useDeleteChannelBot DELETEs the specific bot", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDeleteChannelBot(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("bot-1");
    expect(mockDelete).toHaveBeenCalledWith("/channel-bots/bot-1");
  });

  it("useVerifyChannelBot POSTs to the verify endpoint", async () => {
    const response = {
      id: "bot-1",
      status: "active",
      webhook_registered: true,
    };
    mockPost.mockResolvedValue(response);
    const { result } = renderHook(() => useVerifyChannelBot(), {
      wrapper: createWrapper(),
    });
    await expect(result.current.mutateAsync("bot-1")).resolves.toEqual(
      response,
    );
    expect(mockPost).toHaveBeenCalledWith("/channel-bots/bot-1/verify");
  });

  it("refreshes manager readiness and the list after failed verification", async () => {
    const client = new QueryClient({
      defaultOptions: {
        queries: { retry: false, staleTime: Infinity },
        mutations: { retry: false },
      },
    });
    const initial = {
      id: "bot-1",
      credential_source: "telegram_manager",
      status: "active",
      error: null,
    };
    const failed = {
      ...initial,
      status: "failed",
      error: "Manager is not ready",
    };
    client.setQueryData(channelBotsQueryKeys.detail("bot-1"), initial);
    client.setQueryData(channelBotsQueryKeys.list(null), [initial]);
    mockGet.mockImplementation(async (path: string) =>
      path === "/channel-bots" ? { bots: [failed] } : failed,
    );
    mockPost.mockRejectedValue(new Error("Manager is not ready"));
    const { result } = renderHook(
      () => ({
        detail: useChannelBot("bot-1"),
        list: useChannelBots(),
        verify: useVerifyChannelBot(),
      }),
      {
        wrapper: ({ children }: PropsWithChildren) => (
          <QueryClientProvider client={client}>{children}</QueryClientProvider>
        ),
      },
    );
    await expect(result.current.verify.mutateAsync("bot-1")).rejects.toThrow(
      "Manager is not ready",
    );
    await waitFor(() => {
      expect(result.current.detail.data).toEqual(failed);
      expect(result.current.list.data).toEqual([failed]);
    });
    client.clear();
  });
});

it("refreshes the current server observation once after a failed check", async () => {
  mockGet.mockResolvedValue({ id: "bot-1", last_verification: null });
  const { result } = renderHook(() => ({ bot: useChannelBot("bot-1"), verify: useVerifyChannelBot() }), { wrapper: createWrapper() });
  await waitFor(() => expect(result.current.bot.isSuccess).toBe(true));
  mockGet.mockResolvedValue({ id: "bot-1", last_verification: { id: "check", status: "failed" } });
  mockPost.mockRejectedValue(new Error("verification failed"));
  await expect(result.current.verify.mutateAsync("bot-1")).rejects.toThrow("verification failed");
  await waitFor(() => expect(result.current.bot.data?.last_verification?.status).toBe("failed"));
  expect(mockGet).toHaveBeenCalledTimes(2);
});
