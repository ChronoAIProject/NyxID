import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useActiveCreditGrants,
  useAdminCreditGrants,
  useCurrentAllowances,
  useIssueCreditGrant,
} from "./use-billing-credits";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
}));

function wrapperFactory() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

beforeEach(() => vi.clearAllMocks());

describe("billing credit hooks", () => {
  it("loads a specific admin grant page without a silent fixed cutoff", async () => {
    mockGet.mockResolvedValue({ grants: [], page: 2, per_page: 50, total: 75 });
    const grants = renderHook(() => useAdminCreditGrants(2, 50), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(grants.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/admin/credits/grants?page=2&per_page=50",
    );
  });

  it("normalizes all-owner and all-service grant payloads", async () => {
    mockPost.mockResolvedValue({
      batch_id: "batch-1",
      created_count: 3,
      activated_count: 3,
      pending_activation_count: 0,
      recipients: [
        {
          recipient_user_id: "user-1",
          recipient_billing_enabled: false,
          activation_state: "active",
        },
      ],
    });
    const { result } = renderHook(() => useIssueCreditGrant(), {
      wrapper: wrapperFactory(),
    });

    result.current.mutate({
      amount_credits: 100,
      target_kind: "all_users",
      target_user_ids: ["ignored-user"],
      all_services: true,
      service_refs: ["ignored-service"],
      expires_at: "",
      reason: "",
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockPost).toHaveBeenCalledWith("/admin/credits/grants", {
      amount_credits: 100,
      target_kind: "all_users",
      target_user_ids: [],
      all_services: true,
      service_refs: [],
      expires_at: null,
      reason: null,
    });
  });

  it("loads user benefit surfaces from billing-only endpoints", async () => {
    mockGet.mockImplementation((path: string) =>
      Promise.resolve(
        path === "/billing/grants"
          ? { grants: [], page: 1, per_page: 0, total: 0 }
          : { allowances: [] },
      ),
    );
    const grants = renderHook(() => useActiveCreditGrants(), {
      wrapper: wrapperFactory(),
    });
    const allowances = renderHook(() => useCurrentAllowances(), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(grants.result.current.isSuccess).toBe(true));
    await waitFor(() => expect(allowances.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/billing/grants");
    expect(mockGet).toHaveBeenCalledWith("/billing/allowances");
  });

  it("scopes organization benefit reads without sharing personal cache paths", async () => {
    mockGet.mockImplementation((path: string) =>
      Promise.resolve(
        path.includes("grants")
          ? { grants: [], page: 1, per_page: 0, total: 0 }
          : { allowances: [] },
      ),
    );
    const grants = renderHook(() => useActiveCreditGrants("org/one"), {
      wrapper: wrapperFactory(),
    });
    const allowances = renderHook(() => useCurrentAllowances("org/one"), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(grants.result.current.isSuccess).toBe(true));
    await waitFor(() => expect(allowances.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/billing/grants?owner_id=org%2Fone");
    expect(mockGet).toHaveBeenCalledWith(
      "/billing/allowances?owner_id=org%2Fone",
    );
  });
});
