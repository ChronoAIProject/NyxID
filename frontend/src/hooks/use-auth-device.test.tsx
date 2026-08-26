import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  useApproveAuthDevice,
  useDenyAuthDevice,
  usePreviewAuthDevice,
  useWebAuthDeviceLogin,
} from "./use-auth-device";

const { mockPost, mockCheckAuth } = vi.hoisted(() => ({
  mockPost: vi.fn(),
  mockCheckAuth: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { post: mockPost },
}));

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { checkAuth: typeof mockCheckAuth }) => unknown) =>
    selector({ checkAuth: mockCheckAuth }),
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
  mockCheckAuth.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
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
      client_ip: "203.0.113.10",
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
      client_ip: "203.0.113.10",
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
      client_ip: null,
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
        client_ip: "203.0.113.11",
        initiated_at: "2026-06-18T10:00:00Z",
        expires_at: "2026-06-18T10:10:00Z",
        status: "pending",
      })
      .mockResolvedValueOnce({
        client_label: "device-2",
        client_user_agent: null,
        client_ip: "203.0.113.12",
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

describe("useDenyAuthDevice", () => {
  it("is idle on mount and only fires when mutateAsync is called", () => {
    const { result } = renderHook(() => useDenyAuthDevice(), { wrapper });
    expect(mockPost).not.toHaveBeenCalled();
    expect(result.current.isPending).toBe(false);
  });

  it("normalizes the user_code before posting to the deny endpoint", async () => {
    mockPost.mockResolvedValue({ ok: true });
    const { result } = renderHook(() => useDenyAuthDevice(), { wrapper });

    await result.current.mutateAsync("abcd-efgh");

    expect(mockPost).toHaveBeenCalledWith("/auth/device/deny", {
      user_code: "ABCDEFGH",
    });
  });
});

const requestResponse = {
  device_code: "nyx_adc_test",
  user_code: "ABCD-EFGH",
  verification_uri: "https://id.example/login/device",
  verification_uri_complete:
    "https://id.example/login/device?user_code=ABCD-EFGH",
  expires_in: 60,
  interval: 5,
};

describe("useWebAuthDeviceLogin", () => {
  it("does not request on mount and starts only after explicit activation", async () => {
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    expect(mockPost).not.toHaveBeenCalled();

    mockPost.mockResolvedValueOnce(requestResponse);
    await act(async () => {
      result.current.start();
      await Promise.resolve();
    });

    await waitFor(() => expect(result.current.phase).toBe("pending"));
    expect(mockPost).toHaveBeenCalledWith(
      "/auth/device/request",
      expect.objectContaining({
        client_label: expect.stringMatching(/ on /),
        client_user_agent: expect.any(String),
        client_form_factor: expect.stringMatching(
          /^(desktop|mobile|tablet|unknown)$/,
        ),
      }),
    );
  });

  it("polls at the server interval and adds five seconds after slow_down", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    mockPost.mockResolvedValueOnce(requestResponse);
    const { result } = renderHook(() => useWebAuthDeviceLogin());

    await act(async () => {
      result.current.start();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.phase).toBe("pending");
    expect(mockPost).toHaveBeenCalledTimes(1);

    const slowDown = {
      errorCode: 11203,
      errorResponse: {
        error: "auth_device_slow_down",
        error_code: 11203,
        message: "Slow down",
      },
    };
    mockPost.mockRejectedValueOnce(slowDown).mockResolvedValueOnce({ ok: true });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(4_999);
    });
    expect(mockPost).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(mockPost).toHaveBeenCalledTimes(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(9_999);
    });
    expect(mockPost).toHaveBeenCalledTimes(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(mockPost).toHaveBeenCalledTimes(3);
    expect(result.current.phase).toBe("success");
    expect(mockCheckAuth).toHaveBeenCalledTimes(1);
  });

  it("treats rate limiting as backoff and keeps polling", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    mockPost.mockResolvedValueOnce(requestResponse);
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => {
      result.current.start();
      await Promise.resolve();
      await Promise.resolve();
    });

    const rateLimited = {
      errorCode: 11206,
      errorResponse: {
        error: "auth_device_rate_limited",
        error_code: 11206,
        message: "Rate limited",
      },
    };
    mockPost.mockRejectedValueOnce(rateLimited).mockResolvedValueOnce({ ok: true });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(mockPost).toHaveBeenCalledTimes(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(9_999);
    });
    expect(mockPost).toHaveBeenCalledTimes(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(mockPost).toHaveBeenCalledTimes(3);
    expect(result.current.phase).toBe("success");
  });

  it("recovers from one transient poll failure", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    mockPost.mockResolvedValueOnce(requestResponse);
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => {
      result.current.start();
      await Promise.resolve();
      await Promise.resolve();
    });

    mockPost
      .mockRejectedValueOnce(new Error("temporary network failure"))
      .mockResolvedValueOnce({ ok: true });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(mockPost).toHaveBeenCalledTimes(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(mockPost).toHaveBeenCalledTimes(3);
    expect(result.current.phase).toBe("success");
  });

  it("enters error after the consecutive transient-failure budget", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    mockPost.mockResolvedValueOnce(requestResponse);
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => {
      result.current.start();
      await Promise.resolve();
      await Promise.resolve();
    });

    mockPost
      .mockRejectedValueOnce(new Error("network failure 1"))
      .mockRejectedValueOnce(new Error("network failure 2"))
      .mockRejectedValueOnce(new Error("network failure 3"))
      .mockRejectedValueOnce(new Error("network failure 4"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(result.current.phase).toBe("pending");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(result.current.phase).toBe("pending");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(result.current.phase).toBe("error");
    expect(mockPost).toHaveBeenCalledTimes(5);
  });

  it("stops polling on denied and supports explicit regeneration", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    mockPost.mockResolvedValueOnce(requestResponse);
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => {
      result.current.start();
      await Promise.resolve();
      await Promise.resolve();
    });
    const denied = {
      errorCode: 11204,
      errorResponse: {
        error: "auth_device_access_denied",
        error_code: 11204,
        message: "Denied",
      },
    };
    mockPost.mockRejectedValueOnce(denied);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(result.current.phase).toBe("denied");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(mockPost).toHaveBeenCalledTimes(2);

    mockPost.mockResolvedValueOnce({ ...requestResponse, device_code: "nyx_adc_new" });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(750);
      result.current.generateNew();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mockPost).toHaveBeenCalledTimes(3);
    expect(result.current.phase).toBe("pending");
  });
});
