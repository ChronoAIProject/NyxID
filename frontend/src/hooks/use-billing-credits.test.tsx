import {
  focusManager,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import type { UserCreditSummary } from "@/schemas/billing-credits";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  useUserCreditSummary,
  useAdminCreditSummary,
  useAddCredits,
  useAdminCreditGrants,
  useAdminCreditSchedules,
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

function userSummary(): UserCreditSummary {
  return {
    owner_id: "owner-1",
    available_micros: "95250001",
    wallet_available_micros: "85000000",
    issued_available_micros: "10250001",
    issued_reserved_micros: "5000000",
    pending_activation_micros: "0",
    restricted_micros: "10250001",
    restricted_grant_count: 1,
    generated_at: "2026-10-17T00:00:00Z",
    wallet: {
      plan_kind: "prepaid",
      collection_state: "good",
      balance_credits: "100",
      reserved_credits: "10",
      pending_lago_debits: "5",
      pending_topup_expiry_credits: "0",
      available_credits: "85",
      spendable_credits: "85",
      overdraft_cap_credits: "0",
      has_payment_instrument: false,
      suspended: false,
      balance_synced_at: "2026-10-17T00:00:00Z",
    },
    grants: [
      {
        id: "grant-1",
        remaining_micros: "15250001",
        reserved_micros: "5000000",
        available_micros: "10250001",
        scope: {
          all_services: false,
          service_ids: ["service-1"],
          service_slugs: ["llm-one"],
        },
        expires_at: "2026-11-01T00:00:00Z",
      },
    ],
    allowances: [
      {
        id: "allowance-1",
        service_id: "service-1",
        service_slug: "llm-one",
        metric: "tokens",
        quantity: 1000,
        recurrence: "daily",
        period_start: "2026-10-17T00:00:00Z",
        period_end: "2026-10-18T00:00:00Z",
        consumed_quantity: 250,
        reserved_quantity: 50,
        remaining_quantity: 700,
      },
    ],
  };
}

beforeEach(() => vi.clearAllMocks());
afterEach(() => {
  vi.useRealTimers();
  focusManager.setFocused(undefined);
});

describe("billing credit hooks", () => {
  it("refreshes both summaries and visible history every 30s, pauses in background, and keeps refresh failures stale", async () => {
    vi.useFakeTimers();
    focusManager.setFocused(true);
    const summary = {
      owner_id: "owner-1",
      available_micros: "10",
      wallet_available_micros: "0",
      issued_available_micros: "10",
      issued_reserved_micros: "0",
      pending_activation_micros: "0",
      restricted_micros: "0",
      restricted_grant_count: 0,
      generated_at: "2026-10-17T00:00:00Z",
      wallet: null,
      grants: [],
      allowances: [],
    };
    const admin = {
      outstanding_micros: "10",
      available_micros: "10",
      reserved_micros: "0",
      pending_activation_micros: "0",
      expiring_within_30_days_micros: "0",
      restricted_micros: "0",
      expired_awaiting_cleanup_micros: "0",
      active_grant_count: 1,
      pending_activation_count: 0,
      recipient_count: 1,
      generated_at: summary.generated_at,
    };
    mockGet.mockImplementation(async (path: string) =>
      path === "/billing/credits"
        ? summary
        : path === "/admin/credits/summary"
          ? admin
          : { grants: [], page: 1, per_page: 50, total: 0 },
    );
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const wrapper = ({ children }: PropsWithChildren) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
    const hooks = renderHook(
      () => ({
        user: useUserCreditSummary(),
        admin: useAdminCreditSummary(),
        history: useAdminCreditGrants(),
      }),
      { wrapper },
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });
    expect(hooks.result.current.user.data?.available_micros).toBe("10");
    expect(hooks.result.current.admin.data?.outstanding_micros).toBe("10");
    expect(mockGet).toHaveBeenCalledTimes(3);
    summary.available_micros = "0";
    admin.outstanding_micros = "0";
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(mockGet).toHaveBeenCalledTimes(6);
    expect(hooks.result.current.user.data?.available_micros).toBe("0");
    expect(hooks.result.current.admin.data?.outstanding_micros).toBe("0");
    focusManager.setFocused(false);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(mockGet).toHaveBeenCalledTimes(6);
    mockGet.mockRejectedValue(new Error("Refresh offline"));
    await act(async () => {
      focusManager.setFocused(true);
      await vi.advanceTimersByTimeAsync(50);
    });
    expect(hooks.result.current.user.isError).toBe(true);
    expect(hooks.result.current.user.data?.available_micros).toBe("0");
    expect(hooks.result.current.admin.isError).toBe(true);
    expect(hooks.result.current.admin.data?.outstanding_micros).toBe("0");
    hooks.unmount();
    client.clear();
  });

  it("does not send an invalid recurring expiry to the server", async () => {
    const { result } = renderHook(() => useAddCredits(), {
      wrapper: wrapperFactory(),
    });
    result.current.mutate({
      amount_credits: 10,
      recurrence: "monthly",
      target_kind: "all_users",
      target_user_ids: [],
      all_services: true,
      service_refs: [],
      expires_at: "",
      expiry: { kind: "after_days", days: 0 },
      reason: "",
    });

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(mockPost).not.toHaveBeenCalled();
  });

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

  it("loads and validates the personal combined summary without provisioning a wallet", async () => {
    const response = userSummary();
    mockGet.mockResolvedValue(response);
    const summary = renderHook(() => useUserCreditSummary(), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(summary.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/billing/credits");
    expect(summary.result.current.data).toEqual(response);
    expect(summary.result.current.data?.available_micros).toBe("95250001");
    expect(summary.result.current.data?.wallet?.available_credits).toBe("85");
    expect(summary.result.current.data?.grants[0]?.available_micros).toBe(
      "10250001",
    );
    expect(summary.result.current.data?.allowances[0]?.remaining_quantity).toBe(
      700,
    );
    expect(mockPost).not.toHaveBeenCalled();
  });

  it("keeps personal and encoded organization summary paths and caches separate", async () => {
    const personalResponse = userSummary();
    const orgResponse: UserCreditSummary = {
      ...userSummary(),
      owner_id: "org/one",
      available_micros: "195250001",
      wallet_available_micros: "185000000",
      wallet: {
        ...personalResponse.wallet!,
        balance_credits: "200",
        available_credits: "185",
        spendable_credits: "185",
      },
    };
    mockGet.mockImplementation(async (path: string) => {
      if (path === "/billing/credits") return personalResponse;
      if (path === "/billing/credits?owner_id=org%2Fone") return orgResponse;
      throw new Error(`Unexpected summary path: ${path}`);
    });
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const wrapper = ({ children }: PropsWithChildren) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
    const summary = renderHook(
      () => ({
        personal: useUserCreditSummary(),
        org: useUserCreditSummary("org/one"),
      }),
      { wrapper },
    );

    await waitFor(() =>
      expect(summary.result.current.personal.isSuccess).toBe(true),
    );
    await waitFor(() =>
      expect(summary.result.current.org.isSuccess).toBe(true),
    );
    expect(mockGet).toHaveBeenCalledWith("/billing/credits");
    expect(mockGet).toHaveBeenCalledWith("/billing/credits?owner_id=org%2Fone");
    expect(summary.result.current.personal.data).toEqual(personalResponse);
    expect(summary.result.current.org.data).toEqual(orgResponse);
    expect(
      client.getQueryData(["billing", "credits", "summary", "personal"]),
    ).toEqual(personalResponse);
    expect(
      client.getQueryData(["billing", "credits", "summary", "org/one"]),
    ).toEqual(orgResponse);
    expect(mockPost).not.toHaveBeenCalled();
    summary.unmount();
    client.clear();
  });

  it("rejects legacy numeric wallet fields in the string-valued summary contract", async () => {
    const response = userSummary();
    mockGet.mockResolvedValue({
      ...response,
      wallet: { ...response.wallet, balance_credits: 100 },
    });
    const summary = renderHook(() => useUserCreditSummary(), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(summary.result.current.isError).toBe(true));
    expect(summary.result.current.data).toBeUndefined();
    expect(mockGet).toHaveBeenCalledWith("/billing/credits");
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

describe("allocation submissions", () => {
  const value = {
    amount_credits: 10_000, recurrence: "one_time" as const,
    target_kind: "selected_users" as const, target_user_ids: ["owner-1"],
    all_services: true, service_refs: [], expires_at: "",
    expiry: { kind: "never" as const }, reason: "Test allocation",
    service_limits: [{ service_refs: ["twitter"], amount_credits: 500, label: "Twitter" }, { service_refs: ["openai", "anthropic"], amount_credits: 300, label: "LLM" }],
  };
  const response = { batch_id: "batch", created_count: 1, activated_count: 0, pending_activation_count: 1,
    recipients: [{ recipient_user_id: "owner-1", recipient_billing_enabled: true, activation_state: "pending_activation" }] };

  it("preserves a request UUID across ambiguous retry, sends groups inside one total, and refreshes affected summaries", async () => {
    mockPost.mockRejectedValueOnce(new Error("Connection lost after send")).mockResolvedValue(response);
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const wrapper = ({ children }: PropsWithChildren) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
    const hook = renderHook(() => useAddCredits(), { wrapper });
    await act(async () => { await expect(hook.result.current.mutateAsync(value)).rejects.toThrow("Connection lost"); });
    const first = mockPost.mock.calls[0]?.[1];
    expect(first).toMatchObject({ amount_credits: 10_000, service_limits: value.service_limits, expires_at: null });
    expect(first.request_id).toMatch(/^[\da-f-]{36}$/);
    await act(async () => { await hook.result.current.mutateAsync(value); });
    expect(mockPost.mock.calls[1]?.[1].request_id).toBe(first.request_id);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["admin", "credits"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["billing", "credits"] });
    await act(async () => { await hook.result.current.mutateAsync(value); });
    expect(mockPost.mock.calls[2]?.[1].request_id).not.toBe(first.request_id);
    hook.unmount(); client.clear();
  });

  it("regenerates identity for edited settings and explicit new allocations, preserving identity for a recurring retry", async () => {
    mockPost.mockRejectedValue(new Error("Offline"));
    const hook = renderHook(() => useAddCredits(), { wrapper: wrapperFactory() });
    const recurring = { ...value, recurrence: "monthly" as const };
    await act(async () => { await expect(hook.result.current.mutateAsync(recurring)).rejects.toThrow(); });
    expect(mockPost.mock.calls[0]?.[0]).toBe("/admin/credits/schedules");
    const first = mockPost.mock.calls[0]?.[1].request_id;
    await act(async () => { await expect(hook.result.current.mutateAsync(recurring)).rejects.toThrow(); });
    expect(mockPost.mock.calls[1]?.[1].request_id).toBe(first);
    await act(async () => { await expect(hook.result.current.mutateAsync({ ...recurring, amount_credits: 9000 })).rejects.toThrow(); });
    expect(mockPost.mock.calls[2]?.[1].request_id).not.toBe(first);
    const edited = mockPost.mock.calls[2]?.[1].request_id;
    act(() => hook.result.current.reset());
    await act(async () => { await expect(hook.result.current.mutateAsync({ ...recurring, amount_credits: 9000 })).rejects.toThrow(); });
    expect(mockPost.mock.calls[3]?.[1].request_id).not.toBe(edited);
    hook.unmount();
  });

  it("rejects overlapping or contradictory limits before sending any request", async () => {
    const hook = renderHook(() => useAddCredits(), { wrapper: wrapperFactory() });
    await act(async () => { await expect(hook.result.current.mutateAsync({ ...value, service_limits: [value.service_limits[0]!, value.service_limits[0]!] })).rejects.toThrow(); });
    await act(async () => { await expect(hook.result.current.mutateAsync({ ...value, all_services: false, service_refs: ["twitter"] })).rejects.toThrow(); });
    expect(mockPost).not.toHaveBeenCalled();
    hook.unmount();
  });
});
