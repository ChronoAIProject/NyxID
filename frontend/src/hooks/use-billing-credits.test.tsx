import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useActiveCreditGrants,
  useAdminCreditGrants,
  useAdminCreditSchedules,
  useCreateCreditSchedule,
  useCreateAllowance,
  useCreateAllowanceBundle,
  useReplaceAllowanceBundle,
  useCurrentAllowances,
  useIssueCreditGrant,
  useUpdateCreditSchedule,
} from "./use-billing-credits";

const { mockGet, mockPost, mockPatch, mockPut } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockPatch: vi.fn(),
  mockPut: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, patch: mockPatch, put: mockPut },
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

  it("normalizes all-service grant payloads and empty target lists", async () => {
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
      target_user_ids: [],
      target_org_ids: [],
      target_group_ids: [],
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
      target_org_ids: [],
      target_group_ids: [],
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
      target_org_ids: [],
      target_group_ids: [],
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
      target_org_ids: [],
      target_group_ids: [],
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
      target_org_ids: [],
      target_group_ids: [],
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

it.each(["org_members", "groups"] as const)(
  "sends %s targets through grant, schedule and allowance hooks",
  async (kind) => {
    const targets = {
      target_kind: kind,
      target_user_ids: [],
      target_org_ids: kind === "org_members" ? ["org"] : [],
      target_group_ids: kind === "groups" ? ["group"] : [],
    };
    const commonResponse = {
      ...targets,
      id: "benefit",
      amount_credits: 10,
      amount_micros: 10_000_000,
      recurrence: "monthly",
      expiry: { kind: "never" },
      scope: { all_services: true, service_ids: [], service_slugs: [] },
      created_by: "admin",
      created_at: "2026-09-01T00:00:00Z",
      updated_at: "2026-09-01T00:00:00Z",
      is_active: true,
      skipped_periods: 0,
      batch_id: "batch",
      created_count: 1,
      activated_count: 1,
      pending_activation_count: 0,
      recipients: [],
      service_id: "service",
      service_slug: "service",
      metric: "requests",
      quantity: 100,
    };
    mockPost.mockResolvedValue(commonResponse);
    const grant = renderHook(() => useIssueCreditGrant(), {
      wrapper: wrapperFactory(),
    });
    const schedule = renderHook(() => useCreateCreditSchedule(), {
      wrapper: wrapperFactory(),
    });
    const allowance = renderHook(() => useCreateAllowance(), {
      wrapper: wrapperFactory(),
    });
    const grantForm = {
      ...targets,
      amount_credits: 10,
      all_services: true,
      service_refs: [],
      expires_at: "",
      reason: "",
    };
    await grant.result.current.mutateAsync(grantForm);
    await schedule.result.current.mutateAsync({
      ...grantForm,
      recurrence: "monthly",
      expiry: { kind: "never" },
    });
    await allowance.result.current.mutateAsync({
      ...targets,
      service_ref: "service",
      quantity: 100,
      recurrence: "monthly",
    });
    for (const path of ["grants", "schedules", "allowances"]) {
      expect(mockPost).toHaveBeenCalledWith(
        `/admin/credits/${path}`,
        expect.objectContaining(targets),
      );
    }
  },
);

it("normalizes stale target lists at both bundle hook boundaries", async () => {
  const body = {
    service_ref: "service",
    target_kind: "all_users" as const,
    target_user_ids: ["stale-user"],
    target_org_ids: ["stale-org"],
    target_group_ids: ["stale-group"],
    units: [
      { metric: "tokens" as const, quantity: 10, recurrence: "daily" as const },
    ],
  };
  const response = { bundle_id: "bundle", allowances: [] };
  mockPost.mockResolvedValue(response);
  mockPut.mockResolvedValue(response);
  const create = renderHook(() => useCreateAllowanceBundle(), {
    wrapper: wrapperFactory(),
  });
  const replace = renderHook(() => useReplaceAllowanceBundle(), {
    wrapper: wrapperFactory(),
  });
  create.result.current.mutate(body);
  replace.result.current.mutate({ id: "bundle", body });
  await waitFor(() => expect(create.result.current.isSuccess).toBe(true));
  await waitFor(() => expect(replace.result.current.isSuccess).toBe(true));
  const normalized = {
    ...body,
    target_user_ids: [],
    target_org_ids: [],
    target_group_ids: [],
  };
  expect(mockPost).toHaveBeenCalledWith(
    "/admin/credits/allowances",
    normalized,
  );
  expect(mockPut).toHaveBeenCalledWith(
    "/admin/credits/allowances/bundles/bundle",
    normalized,
  );
});
