import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import { createElement, type PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelHostedConnectLink,
  compactCompleteConnectLinkInput,
  connectLinkStorageKey,
  useCompleteConnectLink,
} from "@/hooks/use-connect-links";
import { api } from "@/lib/api-client";

vi.mock("@/lib/api-client", () => ({
  api: { post: vi.fn() },
}));

const post = vi.mocked(api.post);

beforeEach(() => {
  post.mockReset();
});

describe("connect link OAuth storage", () => {
  it("namespaces the token by link id", () => {
    expect(connectLinkStorageKey("link-123")).toBe(
      "nyxid:connect-link:link-123",
    );
  });

  it("omits empty optional secrets without dropping a device polling state", () => {
    expect(
      compactCompleteConnectLinkInput({
        credential: "secret",
        endpoint_url: "",
        oauth_client_id: "",
        oauth_client_secret: "",
        device_state: "device-state",
      }),
    ).toEqual({ credential: "secret", device_state: "device-state" });
  });

  it("cancels a hosted request with the raw token in the request body", async () => {
    post.mockResolvedValue({
      id: "65dd8fe8-9ee8-4c89-af1e-b283a17bcf37",
      status: "cancelled",
      service_name: "GitHub",
      service_slug: "github",
      expires_at: "2026-08-05T10:15:00Z",
      callback_url:
        "desktop-app://connect/return?status=cancelled&connect_link_id=65dd8fe8-9ee8-4c89-af1e-b283a17bcf37",
    });

    await expect(cancelHostedConnectLink("nyx_clk_secret")).resolves.toMatchObject({
      status: "cancelled",
    });
    expect(post).toHaveBeenCalledWith("/connect-links/cancel", {
      token: "nyx_clk_secret",
    });
  });
});

describe("connect link completion input", () => {
  it.each([false, true])("preserves use_platform_key=%s", (use_platform_key) => {
    expect(compactCompleteConnectLinkInput({ use_platform_key })).toEqual({
      use_platform_key,
    });
  });

  it("omits whitespace-only strings and undefined fields without modifying retained values", () => {
    expect(
      compactCompleteConnectLinkInput({
        use_platform_key: undefined,
        credential: "  FAKE_TEST_ONLY  ",
        endpoint_url: " \t\n ",
        oauth_client_id: undefined,
        oauth_client_secret: "",
        device_state: "  test-device-state  ",
      }),
    ).toStrictEqual({
      credential: "  FAKE_TEST_ONLY  ",
      device_state: "  test-device-state  ",
    });
  });

  it("preserves absent and empty input without inventing a platform-key choice", () => {
    expect(compactCompleteConnectLinkInput(undefined)).toBeUndefined();
    expect(compactCompleteConnectLinkInput({})).toEqual({});
  });
});

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return createElement(QueryClientProvider, { client: queryClient }, children);
  };
}

describe("connect link completion mutation", () => {
  it.each([false, true])(
    "posts the real filtered input with use_platform_key=%s",
    async (use_platform_key) => {
      const response = {
        id: "65dd8fe8-9ee8-4c89-af1e-b283a17bcf37",
        status: "completed",
        service_slug: "llm-openai",
      };
      post.mockResolvedValue(response);
      const { result } = renderHook(() => useCompleteConnectLink(), {
        wrapper: createWrapper(),
      });

      await act(async () => {
        await expect(
          result.current.mutateAsync({
            token: "FAKE_CONNECT_TOKEN_TEST_ONLY",
            values: {
              use_platform_key,
              credential: use_platform_key ? "" : "FAKE_TEST_ONLY",
              endpoint_url: " \t ",
              oauth_client_id: undefined,
            },
          }),
        ).resolves.toEqual(response);
      });

      expect(post).toHaveBeenCalledExactlyOnceWith("/connect-links/complete", {
        token: "FAKE_CONNECT_TOKEN_TEST_ONLY",
        use_platform_key,
        ...(use_platform_key ? {} : { credential: "FAKE_TEST_ONLY" }),
      });
    },
  );

  it.each([false, true])(
    "propagates API errors with use_platform_key=%s",
    async (use_platform_key) => {
      const apiError = new Error("Connection request expired");
      post.mockRejectedValue(apiError);
      const { result } = renderHook(() => useCompleteConnectLink(), {
        wrapper: createWrapper(),
      });

      await expect(
        result.current.mutateAsync({
          token: "FAKE_CONNECT_TOKEN_TEST_ONLY",
          values: { use_platform_key },
        }),
      ).rejects.toBe(apiError);

      expect(post).toHaveBeenCalledExactlyOnceWith("/connect-links/complete", {
        token: "FAKE_CONNECT_TOKEN_TEST_ONLY",
        use_platform_key,
      });
    },
  );
});
