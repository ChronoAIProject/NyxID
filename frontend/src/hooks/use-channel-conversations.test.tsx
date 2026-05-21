import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import type {
  CreateChannelConversationRequest,
  CreateDeviceConversationRequest,
  UpdateChannelConversationRequest,
} from "@/types/channels";
import {
  useChannelConversations,
  useCreateChannelConversation,
  useCreateDeviceConversation,
  useDeleteChannelConversation,
  useUpdateChannelConversation,
} from "./use-channel-conversations";

const { mockDelete, mockGet, mockPost, mockPut } = vi.hoisted(() => ({
  mockDelete: vi.fn(),
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockPut: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    delete: mockDelete,
    get: mockGet,
    post: mockPost,
    put: mockPut,
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

describe("useChannelConversations query building", () => {
  it("hits the bare endpoint with no filters and unwraps `conversations`", async () => {
    mockGet.mockResolvedValue({ conversations: [{ id: "c1" }] });
    const { result } = renderHook(() => useChannelConversations(), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/channel-conversations");
    expect(result.current.data).toEqual([{ id: "c1" }]);
  });

  it("encodes and joins bot_id then org_id with &", async () => {
    mockGet.mockResolvedValue({ conversations: [] });
    const { result } = renderHook(
      () => useChannelConversations({ botId: "bot/1", orgId: "org 1" }),
      { wrapper: createWrapper() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/channel-conversations?bot_id=bot%2F1&org_id=org%201",
    );
  });

  it("emits only bot_id when org is absent", async () => {
    mockGet.mockResolvedValue({ conversations: [] });
    const { result } = renderHook(
      () => useChannelConversations({ botId: "bot-1" }),
      { wrapper: createWrapper() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/channel-conversations?bot_id=bot-1",
    );
  });
});

describe("channel conversation mutations", () => {
  it("useCreateChannelConversation POSTs the payload as-is", async () => {
    mockPost.mockResolvedValue({ id: "c1" });
    const { result } = renderHook(() => useCreateChannelConversation(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      bot_id: "bot-1",
    } as unknown as CreateChannelConversationRequest);
    expect(mockPost).toHaveBeenCalledWith("/channel-conversations", {
      bot_id: "bot-1",
    });
  });

  it("useCreateDeviceConversation injects platform:'device' into the body", async () => {
    mockPost.mockResolvedValue({ id: "c1" });
    const { result } = renderHook(() => useCreateDeviceConversation(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      agent_api_key_id: "key-1",
    } as unknown as Omit<CreateDeviceConversationRequest, "platform">);
    expect(mockPost).toHaveBeenCalledWith("/channel-conversations", {
      platform: "device",
      agent_api_key_id: "key-1",
    });
  });

  it("useUpdateChannelConversation strips `id` from the PUT body", async () => {
    mockPut.mockResolvedValue({ id: "c1" });
    const { result } = renderHook(() => useUpdateChannelConversation(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      id: "c1",
      agent_api_key_id: "key-2",
    } as unknown as { id: string } & UpdateChannelConversationRequest);
    expect(mockPut).toHaveBeenCalledWith("/channel-conversations/c1", {
      agent_api_key_id: "key-2",
    });
  });

  it("useDeleteChannelConversation DELETEs the specific conversation", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDeleteChannelConversation(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync("c1");
    expect(mockDelete).toHaveBeenCalledWith("/channel-conversations/c1");
  });
});
