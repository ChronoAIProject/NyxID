import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  PLATFORM_OPERATION_QUERY_KEY,
  type PlatformOperationList,
} from "@/schemas/platform-ops";
import {
  discoveryPricing,
  perCallPricing,
} from "@/schemas/__fixtures__/platform-ops-builders";
import {
  usePlatformOperationDiscovery,
  usePlatformOperations,
  useUpdatePlatformOperation,
} from "./use-platform-ops";

const { mockGet, mockPut } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPut: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, put: mockPut },
}));

const speakOperation = {
  operation_id: "00000000-0000-4000-8000-000000000001",
  op: "speak" as const,
  enabled: false,
  vendor_service_slug: "platform-elevenlabs",
  config: {
    type: "speak" as const,
    allowed_voice_ids: ["voice-a"],
    max_chars: 1_000,
    model_id: "eleven_multilingual_v2",
    max_calls_per_user_per_day: 50,
  },
  updated_at: "2026-08-25T09:00:00Z",
  updated_by: "admin-1",
  vendor_service_id: "platform-elevenlabs-id",
  pricing: perCallPricing("0.25"),
};

const speakAdminOperation = {
  operation_id: speakOperation.operation_id,
  catalog_service_id: "platform-elevenlabs-id",
  provider_slug: "platform-elevenlabs",
  provider_name: "ElevenLabs",
  operation_name: "Speak",
  enabled: false,
  kind: {
    type: "constrained",
    op: "speak",
    config: {
      type: "speak",
      allowed_voice_ids: ["voice-a"],
      model_id: "eleven_multilingual_v2",
      max_calls_per_user_per_day: 50,
    },
  },
  limits: {
    per_request: { type: "speak", max_chars: 1_000 },
    per_user_per_day: 50,
  },
  pricing: speakOperation.pricing,
  created_at: "2026-08-25T09:00:00Z",
  created_by: "admin-1",
  updated_at: "2026-08-25T09:00:00Z",
  updated_by: "admin-1",
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

  it("loads and parses the admin operation list", async () => {
    mockGet.mockResolvedValue({ operations: [speakAdminOperation] });
    const { Wrapper } = createHarness();
    const { result } = renderHook(() => usePlatformOperations(), {
      wrapper: Wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(mockGet).toHaveBeenCalledWith("/admin/platform-ops");
    expect(result.current.data?.operations).toEqual([speakOperation]);
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

  it("rejects an invalid response instead of exposing untyped data", async () => {
    mockGet.mockResolvedValue({ operations: [{ op: "caller_defined" }] });
    const { Wrapper } = createHarness();
    const { result } = renderHook(() => usePlatformOperations(), {
      wrapper: Wrapper,
    });

    await waitFor(() => expect(result.current.isError).toBe(true));
  });

  it("puts the typed payload and replaces the matching cached row", async () => {
    const updated = {
      ...speakOperation,
      enabled: true,
      config: { ...speakOperation.config, max_chars: 2_000 },
      updated_at: "2026-08-25T10:00:00Z",
      updated_by: "admin-1",
    };
    mockPut.mockResolvedValue({
      ...speakAdminOperation,
      enabled: true,
      limits: {
        ...speakAdminOperation.limits,
        per_request: { type: "speak", max_chars: 2_000 },
      },
      updated_at: updated.updated_at,
      updated_by: updated.updated_by,
    });
    const { queryClient, Wrapper } = createHarness();
    queryClient.setQueryData<PlatformOperationList>(
      PLATFORM_OPERATION_QUERY_KEY,
      { operations: [speakOperation] },
    );
    const { result } = renderHook(() => useUpdatePlatformOperation(), {
      wrapper: Wrapper,
    });

    await act(async () => {
      await result.current.mutateAsync({
        op: "speak",
        data: {
          enabled: true,
          vendor_service_slug: "platform-elevenlabs",
          config: { ...speakOperation.config, max_chars: 2_000 },
        },
      });
    });

    expect(mockPut).toHaveBeenCalledWith(
      `/admin/platform-ops/${speakOperation.operation_id}`,
      {
        enabled: true,
        kind: {
          kind: "constrained",
          op: "speak",
          config: {
            type: "speak",
            allowed_voice_ids: ["voice-a"],
            model_id: "eleven_multilingual_v2",
            max_calls_per_user_per_day: 50,
          },
        },
        limits: {
          per_request: { type: "speak", max_chars: 2_000 },
          per_user_per_day: 50,
        },
        billing: {
          metric: "requests",
          price_per_unit: "0.25",
          secondary: null,
          base_fee_per_call: null,
        },
      },
    );
    expect(
      queryClient.getQueryData<PlatformOperationList>(
        PLATFORM_OPERATION_QUERY_KEY,
      )?.operations[0],
    ).toEqual(updated);
  });
});
