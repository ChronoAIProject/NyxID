import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import type { TelegramLoginData } from "@/types/api";
import {
  useConnectTelegramWidget,
  useDisconnectProvider,
  useMyProviderTokens,
} from "./use-providers";

const { mockDelete, mockGet, mockPost } = vi.hoisted(() => ({
  mockDelete: vi.fn(),
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    delete: mockDelete,
    get: mockGet,
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

describe("useConnectTelegramWidget", () => {
  beforeEach(() => {
    mockDelete.mockReset();
    mockGet.mockReset();
    mockPost.mockReset();
  });

  it("coerces Telegram widget numeric fields before posting", async () => {
    mockPost.mockResolvedValue({
      status: "connected",
      message: "Telegram identity verified and stored",
    });

    const { result } = renderHook(() => useConnectTelegramWidget(), {
      wrapper: createWrapper(),
    });

    await result.current.mutateAsync({
      providerId: "provider-telegram",
      data: {
        id: "12345",
        first_name: "Nyx",
        auth_date: "1742518400",
        hash: "a".repeat(64),
      } as unknown as TelegramLoginData,
    });

    expect(mockPost).toHaveBeenCalledWith(
      "/providers/provider-telegram/connect/telegram/callback",
      {
        id: 12345,
        first_name: "Nyx",
        auth_date: 1742518400,
        hash: "a".repeat(64),
      },
    );
  });

  it("rejects malformed Telegram widget payloads before posting", async () => {
    const { result } = renderHook(() => useConnectTelegramWidget(), {
      wrapper: createWrapper(),
    });

    await expect(
      result.current.mutateAsync({
        providerId: "provider-telegram",
        data: {
          id: 12345,
          first_name: "Nyx",
          auth_date: 1742518400,
          hash: "deadbeef",
        } as TelegramLoginData,
      }),
    ).rejects.toThrow("Invalid Telegram login hash");

    expect(mockPost).not.toHaveBeenCalled();
  });
});

describe("provider token scope hooks", () => {
  beforeEach(() => {
    mockDelete.mockReset();
    mockGet.mockReset();
    mockPost.mockReset();
  });

  it("appends target_org_id when listing provider tokens for an org", async () => {
    mockGet.mockResolvedValue({ tokens: [] });

    const { result } = renderHook(
      () => useMyProviderTokens({ targetOrgId: "org-1" }),
      { wrapper: createWrapper() },
    );

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });
    expect(mockGet).toHaveBeenCalledWith(
      "/providers/my-tokens?target_org_id=org-1",
    );
  });

  it("appends target_org_id when disconnecting an org provider token", async () => {
    mockDelete.mockResolvedValue({
      status: "disconnected",
      message: "Provider disconnected",
    });

    const { result } = renderHook(() => useDisconnectProvider(), {
      wrapper: createWrapper(),
    });

    await result.current.mutateAsync({
      providerId: "provider-1",
      targetOrgId: "org-1",
    });

    expect(mockDelete).toHaveBeenCalledWith(
      "/providers/provider-1/disconnect?target_org_id=org-1",
    );
  });
});
