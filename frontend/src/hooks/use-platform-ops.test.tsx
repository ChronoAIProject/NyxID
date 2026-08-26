import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  PLATFORM_OPERATION_QUERY_KEY,
  type PlatformVendorRequirement,
  type PlatformOperationList,
} from "@/schemas/platform-ops";
import {
  usePlatformOperations,
  usePlatformVendorRequirements,
  useProvisionPlatformVendor,
  useUpdatePlatformOperation,
} from "./use-platform-ops";

const { mockDelete, mockGet, mockPost, mockPut } = vi.hoisted(() => ({
  mockDelete: vi.fn(),
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockPut: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { delete: mockDelete, get: mockGet, post: mockPost, put: mockPut },
}));

const elevenLabsRequirement: PlatformVendorRequirement = {
  id: "template-elevenlabs",
  vendor: "elevenlabs",
  display_name: "ElevenLabs",
  operation: "speak",
  slug: "platform-elevenlabs",
  base_url: "https://api.elevenlabs.io",
  auth_method: "header",
  auth_key_name: "xi-api-key",
  service_category: "internal",
  visibility: "public",
  credential_label: "API key",
  credential_note: "Use a restricted key.",
  capability_summary: "Serves speak.",
  restriction_summary: "Does not expose vendor tools.",
  is_active: true,
  is_seeded: true,
  existing_service: null,
};

const xSearchOperation = {
  op: "x_search" as const,
  enabled: false,
  vendor_service_slug: "platform-x",
  config: { type: "x_search" as const, max_results_cap: 10 },
  updated_at: null,
  updated_by: null,
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
    mockGet.mockResolvedValue({ operations: [xSearchOperation] });
    const { Wrapper } = createHarness();
    const { result } = renderHook(() => usePlatformOperations(), {
      wrapper: Wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(mockGet).toHaveBeenCalledWith("/admin/platform-ops");
    expect(result.current.data?.operations).toEqual([xSearchOperation]);
  });

  it("loads and parses vendor requirements", async () => {
    mockGet.mockResolvedValue({ vendors: [elevenLabsRequirement] });
    const { Wrapper } = createHarness();
    const { result } = renderHook(() => usePlatformVendorRequirements(), {
      wrapper: Wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(mockGet).toHaveBeenCalledWith(
      "/admin/platform-ops/vendor-requirements",
    );
    expect(result.current.data?.vendors).toEqual([elevenLabsRequirement]);
  });

  it("replaces a vendor row in one mutation while preserving its contract", async () => {
    mockDelete.mockResolvedValue(undefined);
    mockPost.mockResolvedValue({ id: "new-service-id" });
    const { Wrapper } = createHarness();
    const { result } = renderHook(() => useProvisionPlatformVendor(), {
      wrapper: Wrapper,
    });

    await act(async () => {
      await result.current.mutateAsync({
        requirement: elevenLabsRequirement,
        data: {
          vendor: "elevenlabs",
          credential: "write-only-key",
          note: "Restricted to TTS",
        },
        replaceServiceId: "old-service-id",
      });
    });

    expect(mockDelete).toHaveBeenCalledWith("/services/old-service-id");
    expect(mockPost).toHaveBeenCalledWith("/services", {
      name: "Platform ElevenLabs",
      slug: "platform-elevenlabs",
      service_type: "http",
      base_url: "https://api.elevenlabs.io",
      auth_method: "header",
      auth_key_name: "xi-api-key",
      credential: "write-only-key",
      service_category: "internal",
      visibility: "public",
      auth_notes: "Restricted to TTS",
    });
    expect(mockDelete.mock.invocationCallOrder[0]).toBeLessThan(
      mockPost.mock.invocationCallOrder[0] ?? 0,
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
      ...xSearchOperation,
      enabled: true,
      config: { type: "x_search" as const, max_results_cap: 20 },
      updated_at: "2026-08-25T10:00:00Z",
      updated_by: "admin-1",
    };
    mockPut.mockResolvedValue(updated);
    const { queryClient, Wrapper } = createHarness();
    queryClient.setQueryData<PlatformOperationList>(
      PLATFORM_OPERATION_QUERY_KEY,
      { operations: [xSearchOperation] },
    );
    const { result } = renderHook(() => useUpdatePlatformOperation(), {
      wrapper: Wrapper,
    });

    await act(async () => {
      await result.current.mutateAsync({
        op: "x_search",
        data: {
          enabled: true,
          vendor_service_slug: "platform-x",
          config: { type: "x_search", max_results_cap: 20 },
        },
      });
    });

    expect(mockPut).toHaveBeenCalledWith("/admin/platform-ops/x_search", {
      enabled: true,
      vendor_service_slug: "platform-x",
      config: { type: "x_search", max_results_cap: 20 },
    });
    expect(
      queryClient.getQueryData<PlatformOperationList>(
        PLATFORM_OPERATION_QUERY_KEY,
      )?.operations[0],
    ).toEqual(updated);
  });
});
