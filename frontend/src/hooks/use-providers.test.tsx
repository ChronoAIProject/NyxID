import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import type { TelegramLoginData } from "@/types/api";
import { useConnectTelegramWidget } from "./use-providers";

const { mockPost } = vi.hoisted(() => ({
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
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
