import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useAdminBillingConfig,
  useAdminBillingService,
  useResyncAdminBillingService,
  useUpdateAdminBillingConfig,
  useUpdateAdminBillingService,
} from "./use-admin-billing";

const { mockGet, mockPost, mockPut } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockPut: vi.fn(),
}));
vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, put: mockPut },
}));

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

const config = {
  billing_enabled: true,
  lago_configured: true,
  resale_enabled: false,
  default_overdraft_cap_credits: {
    effective: 9,
    source: "global",
    global: 9,
    env: 5,
  },
  max_default_overdraft_cap_credits: 1_000_000_000_000,
  environment: [],
};

const service = {
  id: "svc/1",
  slug: "svc",
  name: "Service",
  is_active: true,
  effective_metric: "requests",
  allowance_metrics: ["requests"],
  effective: {
    fallback_metric: "requests",
    classes: [],
    resale: { configured: false, applied: false, source: "free" },
  },
};

beforeEach(() => vi.clearAllMocks());

describe("admin billing hooks", () => {
  it("reads the config and validates it", async () => {
    mockGet.mockResolvedValue(config);
    const { result } = renderHook(() => useAdminBillingConfig(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/billing/config");
    expect(result.current.data?.default_overdraft_cap_credits.source).toBe(
      "global",
    );
  });

  it("surfaces a contract violation instead of rendering it", async () => {
    mockGet.mockResolvedValue({
      ...config,
      default_overdraft_cap_credits: { effective: 1 },
    });
    const { result } = renderHook(() => useAdminBillingConfig(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.isError).toBe(true));
  });

  it("sends a flat per-field patch to the config endpoint", async () => {
    mockPut.mockResolvedValue(config);
    const { result } = renderHook(() => useUpdateAdminBillingConfig(), {
      wrapper: wrapper(),
    });
    await act(() =>
      result.current.mutateAsync({ default_overdraft_cap_credits: null }),
    );
    expect(mockPut).toHaveBeenCalledWith("/admin/billing/config", {
      default_overdraft_cap_credits: null,
    });
  });

  it("encodes service ids and wraps lane writes in a billing object", async () => {
    mockGet.mockResolvedValue(service);
    const read = renderHook(() => useAdminBillingService("svc/1"), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(read.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/billing/services/svc%2F1");

    mockPut.mockResolvedValue(service);
    const write = renderHook(() => useUpdateAdminBillingService(), {
      wrapper: wrapper(),
    });
    await act(() =>
      write.result.current.mutateAsync({
        serviceId: "svc/1",
        billing: { platform_key_pricing: null },
      }),
    );
    expect(mockPut).toHaveBeenCalledWith("/admin/billing/services/svc%2F1", {
      billing: { platform_key_pricing: null },
    });

    mockPost.mockResolvedValue({ synced: true, service });
    const resync = renderHook(() => useResyncAdminBillingService(), {
      wrapper: wrapper(),
    });
    const outcome = await act(() => resync.result.current.mutateAsync("svc/1"));
    expect(mockPost).toHaveBeenCalledWith(
      "/admin/billing/services/svc%2F1/resync",
      {},
    );
    expect(outcome.synced).toBe(true);
  });

  it("does not fetch a service without an id", () => {
    renderHook(() => useAdminBillingService(""), { wrapper: wrapper() });
    expect(mockGet).not.toHaveBeenCalled();
  });
});
