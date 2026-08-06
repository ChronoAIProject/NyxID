import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useConfigureConnectionWebhook,
  useDisableConnectionWebhook,
  useRotateConnectionWebhookSecret,
} from "./use-connection-webhooks";

const { mockDelete, mockPost, mockPut } = vi.hoisted(() => ({
  mockDelete: vi.fn(),
  mockPost: vi.fn(),
  mockPut: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { delete: mockDelete, post: mockPost, put: mockPut },
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
  });
  const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");
  return {
    invalidateQueries,
    Wrapper({ children }: PropsWithChildren) {
      return (
        <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
      );
    },
  };
}

const secretResponse = {
  client_id: "client-1",
  connection_webhook_url: "https://events.example.com/nyxid",
  connection_webhook_enabled: true,
  signing_secret: "nyx_whsec_once",
};

beforeEach(() => vi.clearAllMocks());

describe("connection webhook hooks", () => {
  it("configures, parses, and invalidates the developer app query", async () => {
    mockPut.mockResolvedValue(secretResponse);
    const { Wrapper, invalidateQueries } = createWrapper();
    const { result } = renderHook(() => useConfigureConnectionWebhook(), {
      wrapper: Wrapper,
    });

    await expect(
      result.current.mutateAsync({
        clientId: "client/1",
        url: "https://events.example.com/nyxid",
      }),
    ).resolves.toEqual(secretResponse);
    expect(mockPut).toHaveBeenCalledWith(
      "/developer/oauth-clients/client%2F1/connection-webhook",
      { url: "https://events.example.com/nyxid" },
    );
    await waitFor(() =>
      expect(invalidateQueries).toHaveBeenCalledWith({
        queryKey: ["developer", "oauth-clients"],
      }),
    );
  });

  it("rotates and disables against the exact backend routes", async () => {
    mockPost.mockResolvedValue(secretResponse);
    mockDelete.mockResolvedValue({
      id: "client-1",
      connection_webhook_url: null,
      connection_webhook_enabled: false,
    });
    const rotate = createWrapper();
    const rotated = renderHook(() => useRotateConnectionWebhookSecret(), {
      wrapper: rotate.Wrapper,
    });
    await rotated.result.current.mutateAsync("client-1");
    expect(mockPost).toHaveBeenCalledWith(
      "/developer/oauth-clients/client-1/connection-webhook/rotate-secret",
    );

    const disable = createWrapper();
    const disabled = renderHook(() => useDisableConnectionWebhook(), {
      wrapper: disable.Wrapper,
    });
    await disabled.result.current.mutateAsync("client-1");
    expect(mockDelete).toHaveBeenCalledWith(
      "/developer/oauth-clients/client-1/connection-webhook",
    );
  });
});
