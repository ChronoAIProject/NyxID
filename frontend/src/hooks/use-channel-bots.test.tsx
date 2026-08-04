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
    mockPost.mockResolvedValue(undefined);
    const { result } = renderHook(() => useVerifyChannelBot(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("bot-1");
    expect(mockPost).toHaveBeenCalledWith("/channel-bots/bot-1/verify");
  });
});
