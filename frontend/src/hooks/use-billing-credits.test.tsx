import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useActiveCreditGrants,
  useAdminCreditGrants,
  useAdminCreditSchedules,
  useCreateCreditSchedule,
  useCurrentAllowances,
  useIssueCreditGrant,
  useUpdateCreditSchedule,
} from "./use-billing-credits";

const { mockGet, mockPost, mockPatch } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockPatch: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, patch: mockPatch },
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

  it("loads and normalizes recurring credit schedule requests", async () => {
    mockGet.mockResolvedValueOnce({ schedules: [] });
    const list = renderHook(() => useAdminCreditSchedules(), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(list.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/credits/schedules");

    const schedule = {
      id: "schedule-1",
      amount_credits: 50,
      amount_micros: 50_000_000,
      recurrence: "monthly",
      expiry: { kind: "end_of_period" },
      target_kind: "all_users",
      target_user_ids: [],
      scope: { all_services: true, service_ids: [], service_slugs: [] },
      is_active: true,
      created_by: "admin-1",
      created_at: "2026-08-01T00:00:00Z",
      updated_at: "2026-08-01T00:00:00Z",
      skipped_periods: 0,
    };
    mockPost.mockResolvedValueOnce(schedule);
    const create = renderHook(() => useCreateCreditSchedule(), {
      wrapper: wrapperFactory(),
    });
    create.result.current.mutate({
      amount_credits: 50,
      recurrence: "monthly",
      expiry: { kind: "end_of_period" },
      target_kind: "all_users",
      target_user_ids: [],
      all_services: true,
      service_refs: [],
      reason: "",
    });
    await waitFor(() => expect(create.result.current.isSuccess).toBe(true));
    expect(mockPost).toHaveBeenCalledWith("/admin/credits/schedules", {
      amount_credits: 50,
      recurrence: "monthly",
      expiry: { kind: "end_of_period" },
      target_kind: "all_users",
      target_user_ids: [],
      all_services: true,
      service_refs: [],
      reason: null,
    });

    mockPatch.mockResolvedValueOnce({ ...schedule, is_active: false });
    const update = renderHook(() => useUpdateCreditSchedule(), {
      wrapper: wrapperFactory(),
    });
    update.result.current.mutate({
      id: "schedule/1",
      body: { is_active: false },
    });
    await waitFor(() => expect(update.result.current.isSuccess).toBe(true));
    expect(mockPatch).toHaveBeenCalledWith(
      "/admin/credits/schedules/schedule%2F1",
      { is_active: false },
    );
  });
});
