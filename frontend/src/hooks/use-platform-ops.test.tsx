import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  adminPricing,
  discoveryPricing,
} from "@/schemas/__fixtures__/platform-ops-builders";
import {
  PLATFORM_OPERATION_QUERY_KEY,
  type AdminPlatformOperationList,
  type AdminPlatformProviderList,
} from "@/schemas/platform-ops";
import {
  PLATFORM_PROVIDER_QUERY_KEY,
  usePlatformOperationDiscovery,
  usePlatformOperations,
  usePlatformProviders,
  usePromotePlatformProvider,
  useSetPlatformCredential,
  useUpdatePlatformOperation,
} from "./use-platform-ops";

const { mockGet, mockPut } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPut: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, put: mockPut },
}));

const endpointOperation = {
  operation_id: "00000000-0000-4000-8000-000000000001",
  catalog_service_id: "00000000-0000-4000-8000-000000000010",
  provider_slug: "platform-openai",
  provider_name: "OpenAI",
  operation_name: "Create response",
  enabled: false,
  kind: {
    type: "endpoint" as const,
    method: "POST",
    path_template: "/v1/responses",
    name: "Create response",
    description: "Create a model response.",
  },
  limits: {
    per_request: { type: "endpoint" as const },
    per_user_per_day: 100,
  },
  pricing: adminPricing({
    billable: true,
    metric: "input_tokens",
    price_per_unit: "0.000002",
    secondary: {
      metric: "output_tokens",
      price_per_unit: "0.000008",
      lago_metric_code: "platform_op_openai_output",
    },
    display:
      "0.000002 credits per input token + 0.000008 credits per output token",
    lago_metric_code: "platform_op_openai_input",
    sync_status: "failed",
    sync_error: "Lago rejected the charge",
  }),
  created_at: "2026-08-25T09:00:00Z",
  created_by: "admin-1",
  updated_at: "2026-08-25T09:00:00Z",
  updated_by: "admin-1",
};

const provider = {
  catalog_service_id: endpointOperation.catalog_service_id,
  catalog_service_slug: "platform-openai",
  catalog_service_name: "OpenAI",
  catalog_service_active: true,
  eligible: true,
  eligibility_reason: null,
  promoted: false,
  promoted_at: null,
  promoted_by: null,
  vendor_terms_accepted_at: null,
  vendor_terms_accepted_by: null,
  credential: {
    configured: false,
    id: null,
    auth_method: null,
    auth_key_name: null,
    created_at: null,
    updated_at: null,
  },
  enabled_operation_count: 0,
};

function createHarness() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  const Wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return { queryClient, Wrapper };
}

describe("platform operation hooks", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("retains endpoint rows from the admin operation list", async () => {
    mockGet.mockResolvedValue({ operations: [endpointOperation] });
    const { Wrapper } = createHarness();
    const { result } = renderHook(() => usePlatformOperations(), {
      wrapper: Wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(mockGet).toHaveBeenCalledWith("/admin/platform-ops");
    expect(result.current.data?.operations).toEqual([endpointOperation]);
    expect(result.current.data?.operations[0]?.kind.type).toBe("endpoint");
  });

  it("loads and parses user-facing operation discovery", async () => {
    mockGet.mockResolvedValue({
      operations: [
        {
          op: "speak",
          display_name: "Speak",
          description: "Converts bounded text to speech.",
          vendor: "elevenlabs",
          catalog_service_slug: "api-elevenlabs",
          credential_source: "platform",
          credential_intent: "auto",
          availability_reason: null,
          fallback_reason: "own_credential_absent",
          own_connection: null,
          pricing: discoveryPricing({
            billable: true,
            price_per_unit: "0.25",
            display: "0.25 credits per call",
          }),
          mcp_tool: "nyx__speak",
        },
      ],
    });
    const { Wrapper } = createHarness();
    const { result } = renderHook(() => usePlatformOperationDiscovery(), {
      wrapper: Wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(mockGet).toHaveBeenCalledWith("/platform-ops");
    expect(result.current.data?.operations[0]?.credential_source).toBe(
      "platform",
    );
  });

  it("rejects an invalid admin response instead of exposing untyped data", async () => {
    mockGet.mockResolvedValue({ operations: [{ operation_id: "not-a-uuid" }] });
    const { Wrapper } = createHarness();
    const { result } = renderHook(() => usePlatformOperations(), {
      wrapper: Wrapper,
    });

    await waitFor(() => expect(result.current.isError).toBe(true));
  });

  it("puts the typed payload by operation UUID and replaces the cached row", async () => {
    const updated = { ...endpointOperation, enabled: true };
    mockPut.mockResolvedValue(updated);
    const { queryClient, Wrapper } = createHarness();
    queryClient.setQueryData<AdminPlatformOperationList>(
      PLATFORM_OPERATION_QUERY_KEY,
      { operations: [endpointOperation] },
    );
    const { result } = renderHook(() => useUpdatePlatformOperation(), {
      wrapper: Wrapper,
    });
    const data = {
      enabled: true,
      kind: {
        kind: "endpoint" as const,
        method: "POST",
        path_template: "/v1/responses",
        name: "Create response",
        description: "Create a model response.",
      },
      limits: {
        per_request: { type: "endpoint" as const },
        per_user_per_day: 100,
      },
      billing: {
        metric: "input_tokens" as const,
        price_per_unit: "0.000002",
        secondary: {
          metric: "output_tokens" as const,
          price_per_unit: "0.000008",
        },
        base_fee_per_call: null,
      },
    };

    await act(async () => {
      await result.current.mutateAsync({
        operationId: endpointOperation.operation_id,
        data,
      });
    });

    expect(mockPut).toHaveBeenCalledWith(
      `/admin/platform-ops/${endpointOperation.operation_id}`,
      data,
    );
    expect(
      queryClient.getQueryData<AdminPlatformOperationList>(
        PLATFORM_OPERATION_QUERY_KEY,
      )?.operations[0],
    ).toEqual(updated);
  });

  it("loads provider lifecycle state without returning credential material", async () => {
    mockGet.mockResolvedValue({ providers: [provider] });
    const { Wrapper } = createHarness();
    const { result } = renderHook(() => usePlatformProviders(), {
      wrapper: Wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(mockGet).toHaveBeenCalledWith("/admin/platform-providers");
    expect(result.current.data?.providers[0]?.credential).toEqual(
      provider.credential,
    );
    expect(JSON.stringify(result.current.data)).not.toContain("vendor-secret");
  });

  it("promotes only through the explicit vendor-terms acceptance payload", async () => {
    mockPut.mockResolvedValue({
      ...provider,
      promoted: true,
      promoted_at: "2026-08-25T10:00:00Z",
      promoted_by: "admin-1",
      vendor_terms_accepted_at: "2026-08-25T10:00:00Z",
      vendor_terms_accepted_by: "admin-1",
    });
    const { Wrapper } = createHarness();
    const { result } = renderHook(() => usePromotePlatformProvider(), {
      wrapper: Wrapper,
    });

    await act(async () => {
      await result.current.mutateAsync(provider.catalog_service_id);
    });

    expect(mockPut).toHaveBeenCalledWith(
      `/admin/platform-providers/${provider.catalog_service_id}`,
      { vendor_terms_accepted: true },
    );
  });

  it("writes a credential once and caches only returned status metadata", async () => {
    const configured = {
      ...provider,
      promoted: true,
      promoted_at: "2026-08-25T10:00:00Z",
      promoted_by: "admin-1",
      vendor_terms_accepted_at: "2026-08-25T10:00:00Z",
      vendor_terms_accepted_by: "admin-1",
      credential: {
        configured: true,
        id: "00000000-0000-4000-8000-000000000020",
        auth_method: "bearer",
        auth_key_name: "Authorization",
        created_at: "2026-08-25T10:00:00Z",
        updated_at: "2026-08-25T10:00:00Z",
      },
    };
    mockPut.mockResolvedValue(configured);
    const { queryClient, Wrapper } = createHarness();
    queryClient.setQueryData<AdminPlatformProviderList>(
      PLATFORM_PROVIDER_QUERY_KEY,
      { providers: [provider] },
    );
    const { result } = renderHook(() => useSetPlatformCredential(), {
      wrapper: Wrapper,
    });

    let response: unknown;
    await act(async () => {
      response = await result.current.mutateAsync({
        providerId: provider.catalog_service_id,
        data: { credential: "vendor-secret" },
      });
    });

    expect(mockPut).toHaveBeenCalledWith(
      `/admin/platform-providers/${provider.catalog_service_id}/credential`,
      { credential: "vendor-secret" },
    );
    expect(JSON.stringify(response)).not.toContain("vendor-secret");
    expect(
      queryClient.getQueryData<AdminPlatformProviderList>(
        PLATFORM_PROVIDER_QUERY_KEY,
      )?.providers[0]?.credential.configured,
    ).toBe(true);
  });
});
