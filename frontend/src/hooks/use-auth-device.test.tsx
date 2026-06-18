import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useApproveAuthDevice,
  usePreviewAuthDevice,
} from "./use-auth-device";

const { mockPost } = vi.hoisted(() => ({ mockPost: vi.fn() }));

vi.mock("@/lib/api-client", () => ({
  api: { post: mockPost },
}));

function wrapper({ children }: PropsWithChildren) {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("usePreviewAuthDevice", () => {
  it("is idle on mount and only fires when mutateAsync is called", async () => {
    const { result } = renderHook(() => usePreviewAuthDevice(), { wrapper });
    expect(mockPost).not.toHaveBeenCalled();
    expect(result.current.isPending).toBe(false);
    expect(result.current.data).toBeUndefined();
  });

  it("posts the user_code to /auth/device/preview and parses the response", async () => {
    mockPost.mockResolvedValue({
      client_label: "kitchen-rpi",
      client_user_agent: "nyxid-cli/0.7.1",
      initiated_at: "2026-06-18T10:00:00Z",
      expires_at: "2026-06-18T10:10:00Z",
      status: "pending",
    });
    const { result } = renderHook(() => usePreviewAuthDevice(), { wrapper });

    const response = await result.current.mutateAsync("ABCDEFGH");

    expect(mockPost).toHaveBeenCalledWith("/auth/device/preview", {
      user_code: "ABCDEFGH",
    });
    expect(response.status).toBe("pending");
    expect(response.client_label).toBe("kitchen-rpi");
  });

  it("rejects responses that don't match the preview schema", async () => {
    mockPost.mockResolvedValue({
      client_label: "ok",
      client_user_agent: "ok",
      initiated_at: "not-a-datetime",
      expires_at: "2026-06-18T10:10:00Z",
      status: "pending",
    });
    const { result } = renderHook(() => usePreviewAuthDevice(), { wrapper });

    await expect(result.current.mutateAsync("ABCDEFGH")).rejects.toThrow();
  });

  it("rejects status values outside the documented enum", async () => {
    mockPost.mockResolvedValue({
      client_label: null,
      client_user_agent: null,
      initiated_at: "2026-06-18T10:00:00Z",
      expires_at: "2026-06-18T10:10:00Z",
      status: "weird-state",
    });
    const { result } = renderHook(() => usePreviewAuthDevice(), { wrapper });

    await expect(result.current.mutateAsync("ABCDEFGH")).rejects.toThrow();
  });

  it("can be re-fired with reset() between calls", async () => {
    mockPost
      .mockResolvedValueOnce({
        client_label: "device-1",
        client_user_agent: null,
        initiated_at: "2026-06-18T10:00:00Z",
        expires_at: "2026-06-18T10:10:00Z",
        status: "pending",
      })
      .mockResolvedValueOnce({
        client_label: "device-2",
        client_user_agent: null,
        initiated_at: "2026-06-18T10:05:00Z",
        expires_at: "2026-06-18T10:15:00Z",
        status: "pending",
      });
    const { result } = renderHook(() => usePreviewAuthDevice(), { wrapper });

    await result.current.mutateAsync("AAAA1111");
    await waitFor(() => {
      expect(result.current.data?.client_label).toBe("device-1");
    });

    result.current.reset();
    await waitFor(() => {
      expect(result.current.data).toBeUndefined();
    });

    await result.current.mutateAsync("BBBB2222");
    await waitFor(() => {
      expect(result.current.data?.client_label).toBe("device-2");
    });

    expect(mockPost).toHaveBeenCalledTimes(2);
  });
});

describe("useApproveAuthDevice", () => {
  it("is idle on mount and only fires when mutateAsync is called", () => {
    const { result } = renderHook(() => useApproveAuthDevice(), { wrapper });
    expect(mockPost).not.toHaveBeenCalled();
    expect(result.current.isPending).toBe(false);
  });

  it("normalizes the user_code (strips dashes, uppercases) before posting", async () => {
    mockPost.mockResolvedValue({ ok: true });
    const { result } = renderHook(() => useApproveAuthDevice(), { wrapper });

    await result.current.mutateAsync("abcd-efgh");

    expect(mockPost).toHaveBeenCalledWith("/auth/device/approve", {
      user_code: "ABCDEFGH",
    });
  });

  it("rejects responses that aren't { ok: true }", async () => {
    mockPost.mockResolvedValue({ ok: false });
    const { result } = renderHook(() => useApproveAuthDevice(), { wrapper });

    await expect(result.current.mutateAsync("ABCDEFGH")).rejects.toThrow();
  });
});
