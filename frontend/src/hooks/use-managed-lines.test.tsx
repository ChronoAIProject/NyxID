import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import {
  useManagedEligibility,
  useManagedLineActions,
} from "./use-managed-lines";
import { useAdminManagedLineActions } from "./use-admin-managed-lines";
import type { ManagedLine } from "@/schemas/managed-lines";

const mock = vi.hoisted(() => {
  class ApiError extends Error {
    readonly status: number;
    constructor(status: number) {
      super(`HTTP ${status}`);
      this.status = status;
    }
  }
  return { get: vi.fn(), apiClient: vi.fn(), ApiError };
});
vi.mock("@/lib/api-client", () => ({
  api: { get: mock.get },
  apiClient: mock.apiClient,
  ApiError: mock.ApiError,
}));

const ids = {
  line: "0b8f4b44-3d73-4d0c-8f59-5d1c4f7b2a61",
  claim: "8f8a6e93-1f0c-4f3e-9d77-2a8b9b0b6c3a",
  request: "a7c0f5d2-5b8e-4d3a-8c5f-0e1d2c3b4a59",
  binding: "c3d2e1f0-9a8b-4c7d-8e6f-5a4b3c2d1e0f",
};
function wrapper({ children }: PropsWithChildren) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
const line = {
  id: ids.line,
  phone_number: "+6560000000",
  country: "SG",
  state: "claimed",
  voice: true,
  sms: true,
  standard_voice_options: [{ voice: "Telnyx.Ultra.Callie", language: "zh-CN" }],
  claim: {
    id: ids.claim,
    generation: 2,
    state: "claimed",
    suspension: null,
    auto_renew: true,
    standard_voice: null,
    covered_until: null,
    renewal_quote: null,
    voice_binding_id: ids.binding,
    voice_binding_revision: 7,
  },
} as ManagedLine;
const receipt = {
  request_id: ids.request,
  kind: "release",
  line_id: ids.line,
  claim_id: ids.claim,
  period_id: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  useAuthStore.getState().setUser({
    id: "owner",
    email: "owner@example.com",
    display_name: "Owner",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: true,
    is_active: true,
    created_at: "2026-11-01T00:00:00Z",
  });
});

it("treats missing eligibility as not approved instead of an error", async () => {
  mock.get.mockRejectedValue(new mock.ApiError(404));
  const { result } = renderHook(() => useManagedEligibility(), { wrapper });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(result.current.data).toBeNull();
  expect(mock.get).toHaveBeenCalledWith("/managed-lines/eligibility", expect.anything());
});

it("replays lifecycle writes with the caller's request ID and uses owner-only binding routes", async () => {
  mock.apiClient.mockResolvedValue(receipt);
  const { result } = renderHook(() => useManagedLineActions(), { wrapper });
  const release = { request_id: ids.request, line_id: ids.line, claim_id: ids.claim };
  await act(() => result.current.release.mutateAsync(release));
  await act(() => result.current.release.mutateAsync(release));
  expect(mock.apiClient).toHaveBeenNthCalledWith(1, "/managed-lines/releases", expect.objectContaining({ method: "POST", body: release }));
  expect(mock.apiClient.mock.calls[1]?.[1]).toMatchObject({ body: release });

  mock.apiClient.mockResolvedValue(undefined);
  await act(() => result.current.confirmations.mutateAsync({ line, enabled: true }));
  expect(mock.apiClient).toHaveBeenLastCalledWith(
    `/call-adapters/bindings/${ids.binding}/caller-confirmations`,
    expect.objectContaining({
      method: "PUT",
      body: { enabled: true, sms_binding_id: null, voice_revision: 7 },
    }),
  );
  await act(() =>
    result.current.setVoice.mutateAsync({ line, option: line.standard_voice_options[0]! }),
  );
  expect(mock.apiClient).toHaveBeenLastCalledWith(
    `/managed-lines/${ids.line}/standard-voice`,
    expect.objectContaining({
      method: "PUT",
      body: { claim_id: ids.claim, binding_revision: 7, option: line.standard_voice_options[0] },
    }),
  );
  await expect(
    result.current.confirmations.mutateAsync({
      line: { ...line, claim: { ...line.claim!, voice_binding_revision: null } },
      enabled: false,
    }),
  ).rejects.toThrow();
});

it("sends quote kinds verbatim and parses the quote", async () => {
  mock.apiClient.mockResolvedValue({
    id: ids.request,
    line_id: ids.line,
    kind: { kind: "resume", claim_id: ids.claim, generation: 2 },
    rental: { metric: "phone_number_days", credits_per_unit: "1", revision: "r" },
    usage: [],
    standard_voice_options: [],
    terms: { managed_line: "v1", elevenlabs_eua: null },
    accept_before: "2026-11-20T00:00:00Z",
    period_days: 30,
    paid_period_refundable: false,
  });
  const { result } = renderHook(() => useManagedLineActions(), { wrapper });
  const kind = { kind: "resume" as const, claim_id: ids.claim, generation: 2 };
  let parsed: unknown;
  await act(async () => {
    parsed = await result.current.quote.mutateAsync({ lineId: ids.line, kind });
  });
  expect(parsed).toMatchObject({ kind });
  expect(mock.apiClient).toHaveBeenCalledWith(
    `/managed-lines/${ids.line}/quotes`,
    expect.objectContaining({ method: "POST", body: { kind } }),
  );
});

it("maps admin operations, rotation and evidence resolution to their routes", async () => {
  const { result } = renderHook(() => useAdminManagedLineActions(), { wrapper });
  mock.apiClient.mockResolvedValue({
    line_id: ids.line,
    claim_id: ids.claim,
    revision: 5,
    state: "suspended",
    suspension: "admin",
    live_calls: 1,
    termination_pending: true,
  });
  for (const action of ["suspend", "resume", "retire"] as const) {
    let outcome: unknown;
    await act(async () => {
      outcome = await result.current[action].mutateAsync({ lineId: ids.line, body: { revision: 4 } });
    });
    expect(outcome).toMatchObject({ termination_pending: true });
    expect(mock.apiClient).toHaveBeenLastCalledWith(
      `/admin/managed-lines/${ids.line}/${action}`,
      expect.objectContaining({ method: "POST", body: { revision: 4 } }),
    );
  }
  mock.apiClient.mockResolvedValue(undefined);
  await act(() =>
    result.current.rotate.mutateAsync({ accountId: ids.claim, revision: 3, credential: "new-key" }),
  );
  expect(mock.apiClient).toHaveBeenLastCalledWith(
    `/admin/managed-comms/accounts/${ids.claim}/rotate`,
    expect.objectContaining({ body: { expected_revision: 3, credential: "new-key" } }),
  );
  await act(() =>
    result.current.resolveEvidence.mutateAsync({ sessionId: ids.request, seconds: 61, reference: "cdr-1" }),
  );
  expect(mock.apiClient).toHaveBeenLastCalledWith(
    `/admin/managed-lines/calls/${ids.request}/resolve-evidence`,
    expect.objectContaining({ body: { provider_seconds: 61, provider_reference: "cdr-1" } }),
  );
  await act(() => result.current.premiumSwitch.mutateAsync(false));
  expect(mock.apiClient).toHaveBeenLastCalledWith(
    "/admin/managed-lines/premium-switch",
    expect.objectContaining({ method: "PUT", body: { enabled: false } }),
  );
});
