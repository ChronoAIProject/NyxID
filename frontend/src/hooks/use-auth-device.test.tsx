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
import { useLoginApproval } from "./use-login-approval";

const { mockPost, mockCheckAuth, mockApprovalApi } = vi.hoisted(() => ({
  mockPost: vi.fn(),
  mockCheckAuth: vi.fn(),
  mockApprovalApi: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { post: mockPost },
  apiClient: mockApprovalApi,
  ApiError: class extends Error {},
}));

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: Object.assign(
    (selector: (state: { checkAuth: typeof mockCheckAuth }) => unknown) =>
      selector({ checkAuth: mockCheckAuth }),
    { getState: () => ({ checkAuth: mockCheckAuth }) },
  ),
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
  mockPost.mockReset();
  mockCheckAuth.mockReset().mockResolvedValue(true);
  mockApprovalApi.mockReset();
  sessionStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
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
  function resume() {
    document.dispatchEvent(new Event("visibilitychange"));
    window.dispatchEvent(new Event("pageshow"));
  }

  it.each(["resume first", "countdown first"])("collects approval after QR expiry within the server delivery window (%s)", async (order) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    let finish!: (value: unknown) => void;
    mockPost.mockResolvedValueOnce(requestResponse).mockImplementationOnce(() =>
      new Promise((resolve) => { finish = resolve; }));
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    // The app approved at +55s: the server permits collection until +115s.
    // Resume at +90s without allowing background timers to run.
    vi.setSystemTime(new Date("2026-08-20T10:01:30Z"));
    if (order === "countdown first") {
      await act(async () => { await vi.advanceTimersByTimeAsync(1_000); });
      expect(result.current.request).toBeNull();
    }
    await act(async () => { resume(); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(2);
    expect(mockPost).toHaveBeenLastCalledWith("/auth/device/poll-web", { device_code: requestResponse.device_code });
    expect(result.current.phase).toBe("pending");
    expect(result.current.remainingSeconds).toBe(0);
    expect(result.current.request).toBeNull();
    await act(async () => { finish({ ok: true }); });
    expect(result.current.phase).toBe("success");
    expect(mockCheckAuth).toHaveBeenCalledOnce();
  });

  it.each([11200, 11201, 11204])("keeps server response %s terminal after QR expiry", async (code) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    mockPost.mockResolvedValueOnce(requestResponse).mockRejectedValueOnce({ errorCode: code });
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    vi.setSystemTime(new Date("2026-08-20T10:01:30Z"));
    await act(async () => { window.dispatchEvent(new Event("pageshow")); });
    expect(result.current.phase).toBe(code === 11204 ? "denied" : "expired");
    expect(result.current.error?.code).toBe(code);
    expect(result.current.request).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(120_000); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(2);
    expect(mockCheckAuth).not.toHaveBeenCalled();
  });

  it("does not clear an in-flight collection when the QR countdown reaches zero", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    let finish!: (value: unknown) => void;
    mockPost.mockResolvedValueOnce({ ...requestResponse, interval: 59 })
      .mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(59_000); });
    await act(async () => { await vi.advanceTimersByTimeAsync(31_000); resume(); });
    expect(result.current.phase).toBe("pending");
    expect(result.current.request).toBeNull();
    expect(result.current.remainingSeconds).toBe(0);
    expect(mockPost).toHaveBeenCalledTimes(2);
    await act(async () => { finish({ ok: true }); });
    expect(result.current.phase).toBe("success");
    expect(mockCheckAuth).toHaveBeenCalledOnce();
  });

  it("bounds even a persistently pending server to original QR expiry plus 60 seconds", async () => {
    vi.useFakeTimers();
    const startedAt = new Date("2026-08-20T10:00:00Z");
    vi.setSystemTime(startedAt);
    const pollTimes: number[] = [];
    mockPost.mockImplementation(async () => {
      pollTimes.push(Date.now() - startedAt.getTime());
      throw { errorCode: 11202 };
    }).mockResolvedValueOnce(requestResponse);
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(119_999); });
    expect(result.current.phase).toBe("pending");
    expect(result.current.request).toBeNull();
    expect(pollTimes.some((time) => time > 60_000)).toBe(true);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(result.current.phase).toBe("expired");
    expect(pollTimes.every((time) => time < 120_000)).toBe(true);
    const calls = mockPost.mock.calls.length;
    await act(async () => { await vi.advanceTimersByTimeAsync(300_000); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(calls);
  });

  it.each([120_000, 121_000])("does not start a collection on resume at +%sms", async (elapsed) => {
    vi.useFakeTimers();
    const startedAt = new Date("2026-08-20T10:00:00Z");
    vi.setSystemTime(startedAt);
    mockPost.mockResolvedValueOnce(requestResponse);
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    vi.setSystemTime(startedAt.getTime() + elapsed);
    await act(async () => { window.dispatchEvent(new Event("pageshow")); });
    expect(result.current.phase).toBe("expired");
    expect(result.current.request).toBeNull();
    expect(mockPost).toHaveBeenCalledTimes(1);
  });

  it("adopts cookie delivery from a final poll whose response crosses the poll-start cutoff", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    let finish!: (value: unknown) => void;
    mockPost.mockResolvedValueOnce({ ...requestResponse, interval: 119 })
      .mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(119_000); });
    expect(mockPost).toHaveBeenCalledTimes(2);
    expect(result.current.phase).toBe("pending");
    // The server accepted this one-time delivery at +119s; its HTTP response
    // arrives at +120.1s. The original +60s QR remains retired throughout.
    await act(async () => { await vi.advanceTimersByTimeAsync(1_100); resume(); });
    expect(result.current.phase).toBe("pending");
    expect(result.current.request).toBeNull();
    expect(result.current.remainingSeconds).toBe(0);
    await act(async () => { finish({ ok: true }); });
    expect(result.current.phase).toBe("success");
    expect(mockCheckAuth).toHaveBeenCalledOnce();
    await act(async () => { await vi.advanceTimersByTimeAsync(20_000); resume(); });
    expect(result.current.phase).toBe("success");
    expect(mockPost).toHaveBeenCalledTimes(2);
  });

  it.each(["countdown", "suspended countdown"])("bounds a hung final response and fences late delivery (%s)", async (clock) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    let finish!: (value: unknown) => void;
    mockPost.mockResolvedValueOnce({ ...requestResponse, interval: 119 })
      .mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(129_999); resume(); });
    expect(result.current.phase).toBe("pending");
    expect(mockPost).toHaveBeenCalledTimes(2);
    if (clock === "countdown") {
      await act(async () => { await vi.advanceTimersByTimeAsync(1); });
      expect(result.current.phase).toBe("expired");
      expect(vi.getTimerCount()).toBe(0);
    } else {
      // No countdown callback runs before the response at the finite deadline.
      vi.setSystemTime(new Date("2026-08-20T10:02:10Z"));
    }
    await act(async () => { finish({ ok: true }); resume(); });
    expect(result.current.phase).toBe("expired");
    expect(result.current.request).toBeNull();
    expect(mockCheckAuth).not.toHaveBeenCalled();
    expect(mockPost).toHaveBeenCalledTimes(2);
  });

  it.each([
    { error: { errorCode: 11202 }, phase: "expired" },
    { error: { errorCode: 11203 }, phase: "expired" },
    { error: { errorCode: 11206 }, phase: "expired" },
    { error: new Error("network failed"), phase: "expired" },
    { error: { errorCode: 11201 }, phase: "expired" },
    { error: { errorCode: 11204 }, phase: "denied" },
  ])("terminates a final read's $error after the cutoff without retrying", async ({ error, phase }) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    let fail!: (error: unknown) => void;
    mockPost.mockResolvedValueOnce({ ...requestResponse, interval: 119 })
      .mockImplementationOnce(() => new Promise((_resolve, reject) => { fail = reject; }));
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(120_100); });
    await act(async () => { fail(error); });
    expect(result.current.phase).toBe(phase);
    expect(result.current.request).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(2);
    expect(mockCheckAuth).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each(["regenerate", "close", "unmount"])("fences the final-response wait on %s", async (action) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    let finishOld!: (value: unknown) => void;
    let finishNew!: (value: unknown) => void;
    mockPost.mockResolvedValueOnce({ ...requestResponse, interval: 119 })
      .mockImplementationOnce(() => new Promise((resolve) => { finishOld = resolve; }))
      .mockResolvedValueOnce({ ...requestResponse, device_code: "nyx_adc_new" })
      .mockImplementationOnce(() => new Promise((resolve) => { finishNew = resolve; }));
    const { result, unmount } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(120_100); });
    if (action === "regenerate") await act(async () => { result.current.generateNew(); });
    else if (action === "close") act(() => result.current.close());
    else unmount();
    // Cross the old response deadline while the replacement has its own read.
    await act(async () => { await vi.advanceTimersByTimeAsync(11_000); });
    await act(async () => { finishOld({ ok: true }); resume(); });
    expect(mockCheckAuth).not.toHaveBeenCalled();
    if (action === "regenerate") {
      expect(result.current.phase).toBe("pending");
      expect(result.current.request?.device_code).toBe("nyx_adc_new");
      expect(result.current.remainingSeconds).toBe(49);
      expect(mockPost).toHaveBeenCalledTimes(4);
      await act(async () => { finishNew({ ok: true }); });
      expect(result.current.phase).toBe("success");
      expect(mockCheckAuth).toHaveBeenCalledOnce();
    } else {
      expect(mockPost).toHaveBeenCalledTimes(2);
      if (action === "close") expect(result.current.phase).toBe("idle");
      expect(vi.getTimerCount()).toBe(0);
    }
  });

  const unverifiedIdentity = {
    id: "22c851d2-a23d-4a81-8f8e-804c3aa6f7c1",
    flow: "device",
    user_code: "WXYZ2345",
    verified: false,
    mfa_required: false,
    keep_signed_in: false,
    expires_at: "2026-08-20T10:10:00Z",
    user: null,
  };
  const verifiedIdentity = {
    ...unverifiedIdentity,
    verified: true,
    user: { id: "approver", email: "approver@example.com", display_name: null },
  };

  it.each([false, true])("adopts request-bound proof returned across the cutoff (keep signed in: %s)", async (keep) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    let finish!: (value: unknown) => void;
    mockApprovalApi.mockResolvedValueOnce({ ...unverifiedIdentity, keep_signed_in: keep })
      .mockResolvedValueOnce({ ...requestResponse, interval: 119 })
      .mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const { result } = renderHook(() => {
      const approval = useLoginApproval("device", "WXYZ2345", "?user_code=WXYZ2345");
      const device = useWebAuthDeviceLogin(approval.appTransport(keep));
      return { approval, device };
    });
    await act(async () => { result.current.device.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(119_000); });
    expect(mockApprovalApi).toHaveBeenLastCalledWith(
      `/auth/approval/${unverifiedIdentity.id}/app/poll`,
      expect.objectContaining({ body: { device_code: requestResponse.device_code } }),
    );
    await act(async () => { await vi.advanceTimersByTimeAsync(1_100); resume(); });
    expect(result.current.device.request).toBeNull();
    expect(result.current.approval.identity?.verified).toBe(false);
    await act(async () => { finish({ ...verifiedIdentity, keep_signed_in: keep }); });
    expect(result.current.device.phase).toBe("success");
    expect(result.current.approval.identity?.verified).toBe(true);
    expect(mockCheckAuth).toHaveBeenCalledTimes(keep ? 1 : 0);
    expect(mockPost).not.toHaveBeenCalled();
    await act(async () => { await vi.advanceTimersByTimeAsync(20_000); resume(); });
    expect(result.current.device.phase).toBe("success");
    expect(mockApprovalApi).toHaveBeenCalledTimes(3);
  });

  it.each(["timeout", "suspended timeout", "regenerate", "close", "unmount"])("fences late request-bound proof on %s", async (action) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    let finishOld!: (value: unknown) => void;
    mockApprovalApi.mockResolvedValueOnce({ ...unverifiedIdentity, keep_signed_in: true })
      .mockResolvedValueOnce({ ...requestResponse, interval: 119 })
      .mockImplementationOnce(() => new Promise((resolve) => { finishOld = resolve; }))
      .mockResolvedValueOnce({ ...requestResponse, device_code: "nyx_adc_new", interval: 119 });
    const { result, unmount } = renderHook(() => {
      const approval = useLoginApproval("device", "WXYZ2345", "?user_code=WXYZ2345");
      const device = useWebAuthDeviceLogin(approval.appTransport(true));
      return { approval, device };
    });
    await act(async () => { result.current.device.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(120_100); });
    if (action === "regenerate") await act(async () => { result.current.device.generateNew(); });
    else if (action === "close") act(() => result.current.device.close());
    else if (action === "unmount") unmount();
    if (action === "suspended timeout") vi.setSystemTime(new Date("2026-08-20T10:02:10Z"));
    else await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    await act(async () => { finishOld({ ...verifiedIdentity, keep_signed_in: true }); resume(); });
    expect(result.current.approval.identity?.verified).toBe(false);
    expect(mockCheckAuth).not.toHaveBeenCalled();
    expect(mockPost).not.toHaveBeenCalled();
    if (action === "regenerate") {
      expect(result.current.device.phase).toBe("pending");
      expect(result.current.device.request?.device_code).toBe("nyx_adc_new");
      expect(result.current.device.remainingSeconds).toBe(50);
    } else if (action === "close") expect(result.current.device.phase).toBe("idle");
    else if (action !== "unmount") expect(result.current.device.phase).toBe("expired");
    expect(mockApprovalApi).toHaveBeenCalledTimes(action === "regenerate" ? 4 : 3);
  });

  it.each([11203, 11206])("collection after QR expiry retains server backoff %s", async (code) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    mockPost.mockResolvedValueOnce({ ...requestResponse, interval: 55 })
      .mockRejectedValueOnce({ errorCode: code }).mockResolvedValueOnce({ ok: true });
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(55_000); });
    await act(async () => { await vi.advanceTimersByTimeAsync(35_000); resume(); resume(); });
    expect(result.current.request).toBeNull();
    expect(result.current.phase).toBe("pending");
    expect(mockPost).toHaveBeenCalledTimes(2);
    await act(async () => { await vi.advanceTimersByTimeAsync(24_999); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(2);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(mockPost).toHaveBeenCalledTimes(3);
    expect(result.current.phase).toBe("success");
  });

  it("a regenerated request remains independent of a late collection response", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    let finishOld!: (value: unknown) => void;
    mockPost.mockResolvedValueOnce(requestResponse)
      .mockImplementationOnce(() => new Promise((resolve) => { finishOld = resolve; }))
      .mockResolvedValueOnce({ ...requestResponse, device_code: "nyx_adc_new" })
      .mockRejectedValue({ errorCode: 11202 });
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    vi.setSystemTime(new Date("2026-08-20T10:01:30Z"));
    await act(async () => { window.dispatchEvent(new Event("pageshow")); });
    expect(result.current.request).toBeNull();
    await act(async () => { result.current.generateNew(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    await act(async () => { finishOld({ ok: true }); });
    expect(result.current.phase).toBe("pending");
    expect(result.current.request?.device_code).toBe("nyx_adc_new");
    expect(result.current.remainingSeconds).toBe(55);
    expect(mockCheckAuth).not.toHaveBeenCalled();
    expect(mockPost).toHaveBeenCalledTimes(4);
  });

  it("re-arms a consumed timer when a backward clock adjustment leaves its wall deadline in the future", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    mockPost.mockResolvedValueOnce(requestResponse).mockResolvedValueOnce({ ok: true });
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    vi.setSystemTime(new Date("2026-08-20T09:59:59Z"));
    // The original timer fires after five elapsed seconds, at wall time +4s.
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(mockPost).toHaveBeenCalledTimes(1);
    expect(result.current.phase).toBe("pending");
    await act(async () => { await vi.advanceTimersByTimeAsync(999); });
    expect(mockPost).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(mockPost).toHaveBeenCalledTimes(2);
    expect(result.current.phase).toBe("success");
  });

  it.each(["visible", "pageshow"])("polls immediately on %s after a background timer's deadline elapsed", async (event) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    mockPost.mockResolvedValueOnce(requestResponse).mockResolvedValueOnce({ ok: true });
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    // Move wall time without running timers, like a suspended browser tab.
    vi.setSystemTime(new Date("2026-08-20T10:00:45Z"));
    await act(async () => {
      if (event === "visible") document.dispatchEvent(new Event("visibilitychange"));
      else window.dispatchEvent(new Event("pageshow"));
    });
    expect(mockPost).toHaveBeenCalledTimes(2);
    expect(result.current.phase).toBe("success");
    expect(mockCheckAuth).toHaveBeenCalledOnce();
  });

  it("ignores hidden visibility events and deduplicates resume events and the timer during an in-flight poll", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    let finish!: (value: unknown) => void;
    mockPost.mockResolvedValueOnce(requestResponse).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    vi.setSystemTime(new Date("2026-08-20T10:00:06Z"));
    await act(async () => { document.dispatchEvent(new Event("visibilitychange")); });
    expect(mockPost).toHaveBeenCalledTimes(1);
    visibility.mockReturnValue("visible");
    await act(async () => { resume(); resume(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(2);
    await act(async () => { finish({ ok: true }); });
    expect(result.current.phase).toBe("success");
  });

  it.each([11202, 11203, 11206])("resume respects the initial interval and server response %s backoff", async (code) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    mockPost.mockResolvedValueOnce(requestResponse).mockRejectedValueOnce({ errorCode: code }).mockResolvedValueOnce({ ok: true });
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); resume(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(4_999); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(mockPost).toHaveBeenCalledTimes(2);
    const delay = code === 11202 ? 5_000 : 10_000;
    await act(async () => { await vi.advanceTimersByTimeAsync(delay - 1); resume(); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(2);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(3);
    expect(result.current.phase).toBe("success");
  });

  it("a regenerated request polls while the old generation is unresolved and ignores its late backoff", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    let failOld!: (error: unknown) => void;
    let failNew!: (error: unknown) => void;
    mockPost.mockResolvedValueOnce(requestResponse)
      .mockImplementationOnce(() => new Promise((_resolve, reject) => { failOld = reject; }))
      .mockResolvedValueOnce({ ...requestResponse, device_code: "nyx_adc_new" })
      .mockImplementationOnce(() => new Promise((_resolve, reject) => { failNew = reject; }))
      .mockResolvedValueOnce({ ok: true });
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    await act(async () => { result.current.generateNew(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(mockPost).toHaveBeenLastCalledWith("/auth/device/poll-web", { device_code: "nyx_adc_new" });
    await act(async () => { failOld({ errorCode: 11203 }); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(4);
    await act(async () => { failNew({ errorCode: 11202 }); });
    await act(async () => { await vi.advanceTimersByTimeAsync(4_999); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(4);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(mockPost).toHaveBeenCalledTimes(5);
    expect(result.current.phase).toBe("success");
  });

  it.each(["close", "unmount", "success"])("removes resume listeners and stops polling after %s", async (action) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    const removeDocument = vi.spyOn(document, "removeEventListener");
    const removeWindow = vi.spyOn(window, "removeEventListener");
    mockPost.mockResolvedValueOnce(requestResponse).mockResolvedValueOnce({ ok: true });
    const { result, unmount } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    if (action === "unmount") unmount();
    else if (action === "close") act(() => result.current.close());
    else await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(removeDocument).toHaveBeenCalledWith("visibilitychange", expect.any(Function));
    expect(removeWindow).toHaveBeenCalledWith("pageshow", expect.any(Function));
    const calls = mockPost.mock.calls.length;
    await act(async () => { resume(); await vi.advanceTimersByTimeAsync(30_000); resume(); });
    expect(mockPost).toHaveBeenCalledTimes(calls);
  });

  it("uses the request-bound app transport without adopting a browser session", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    const transport = { request: vi.fn().mockResolvedValue(requestResponse), poll: vi.fn().mockResolvedValue(undefined) };
    const { result } = renderHook(() => useWebAuthDeviceLogin(transport));
    await act(async () => { result.current.start(); });
    expect(result.current.phase).toBe("pending");
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(transport.request).toHaveBeenCalledOnce();
    expect(transport.poll).toHaveBeenCalledWith(requestResponse.device_code, expect.any(Function));
    expect(result.current.phase).toBe("success");
    expect(mockPost).not.toHaveBeenCalled();
    expect(mockCheckAuth).not.toHaveBeenCalled();
  });
  const restrictedDelivery = {
    ok: false,
    auth_kind: "agent_key",
    login_code: { request_id: "handoff", code: "JKLM-NPQR", expires_at: "2026-08-20T10:00:10Z" },
  };

  it("keeps a restricted handoff collected after QR expiry on its own strict deadline", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    mockPost.mockResolvedValueOnce(requestResponse).mockResolvedValueOnce({
      ...restrictedDelivery,
      login_code: { ...restrictedDelivery.login_code, expires_at: "2026-08-20T10:01:35Z" },
    });
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    vi.setSystemTime(new Date("2026-08-20T10:01:30Z"));
    await act(async () => { window.dispatchEvent(new Event("pageshow")); });
    expect(result.current.phase).toBe("restricted");
    expect(result.current.loginCode?.code).toBe("JKLM-NPQR");
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(result.current.phase).toBe("expired");
    expect(result.current.loginCode).toBeNull();
    expect(mockPost).toHaveBeenCalledTimes(2);
    expect(mockCheckAuth).not.toHaveBeenCalled();
  });

  it("clears the restricted handoff at its own expiry and permits regeneration", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    mockPost.mockResolvedValueOnce(requestResponse).mockResolvedValueOnce(restrictedDelivery);
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(result.current.phase).toBe("restricted");
    expect(result.current.loginCode?.code).toBe("JKLM-NPQR");
    expect(mockCheckAuth).not.toHaveBeenCalled();
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(result.current.phase).toBe("expired");
    expect(result.current.loginCode).toBeNull();
    mockPost.mockResolvedValueOnce({ ...requestResponse, device_code: "nyx_adc_new" });
    await act(async () => { result.current.generateNew(); });
    expect(result.current.phase).toBe("pending");
    expect(result.current.request?.device_code).toBe("nyx_adc_new");
  });

  it.each(["restricted", "account", "error"])("ignores an old %s poll after close and restart", async (outcome) => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    let finish!: (value: unknown) => void;
    let fail!: (error: unknown) => void;
    mockPost.mockResolvedValueOnce(requestResponse).mockImplementationOnce(() =>
      new Promise((resolve, reject) => { finish = resolve; fail = reject; }));
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    act(() => result.current.close());
    mockPost.mockResolvedValueOnce({ ...requestResponse, device_code: "nyx_adc_new" });
    await act(async () => { result.current.start(); });
    await act(async () => {
      if (outcome === "error") fail({ errorCode: 11204 });
      else finish(outcome === "restricted" ? restrictedDelivery : { ok: true });
    });
    expect(result.current.phase).toBe("pending");
    expect(result.current.request?.device_code).toBe("nyx_adc_new");
    expect(result.current.loginCode).toBeNull();
    expect(mockCheckAuth).not.toHaveBeenCalled();
  });

  it("does not finish a replaced request after checkAuth resolves", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    let finish!: (value: boolean) => void;
    mockCheckAuth.mockImplementationOnce(() => new Promise<boolean>((resolve) => { finish = resolve; }));
    mockPost.mockResolvedValueOnce(requestResponse).mockResolvedValueOnce({ ok: true });
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(mockCheckAuth).toHaveBeenCalledOnce();
    act(() => result.current.close());
    mockPost.mockResolvedValueOnce({ ...requestResponse, device_code: "nyx_adc_new" });
    await act(async () => { result.current.start(); });
    await act(async () => { finish(true); });
    expect(result.current.phase).toBe("pending");
    expect(result.current.request?.device_code).toBe("nyx_adc_new");
  });

  it("retries only profile restoration after delivery, beyond the former code expiry", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
    mockPost.mockResolvedValueOnce(requestResponse).mockResolvedValueOnce({ ok: true });
    mockCheckAuth.mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    const { result } = renderHook(() => useWebAuthDeviceLogin());
    await act(async () => { result.current.start(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(result.current.phase).toBe("restore_error");
    expect(result.current.remainingSeconds).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(120_000); });
    expect(result.current.phase).toBe("restore_error");
    expect(mockPost).toHaveBeenCalledTimes(2);
    await act(async () => { result.current.retryProfile(); });
    expect(result.current.phase).toBe("success");
    expect(mockCheckAuth).toHaveBeenCalledTimes(2);
    expect(mockPost).toHaveBeenCalledTimes(2);
  });

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

  it("treats old unverified approval delivery as terminal denial without a step-up prompt and supports regeneration", async () => {
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
    expect(result.current.error?.message).toBe("This login request was already denied.");
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
