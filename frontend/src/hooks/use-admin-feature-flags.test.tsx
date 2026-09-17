import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useAdminFeatureFlags,
  useClearAdminFeatureFlag,
  useSetAdminFeatureFlag,
  useUpdateAdminFeatureFlagMetadata,
} from "./use-admin-feature-flags";

const { mockGet, mockPut, mockPatch, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPut: vi.fn(),
  mockPatch: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, put: mockPut, patch: mockPatch, delete: mockDelete },
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

beforeEach(() => vi.clearAllMocks());

describe("admin feature-flag hooks", () => {
  it("lists platform feature flags", async () => {
    mockGet.mockResolvedValue({ flags: [] });
    const { result } = renderHook(() => useAdminFeatureFlags(), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/feature-flags");
  });

  it("sets an encoded flag key", async () => {
    mockPut.mockResolvedValue({});
    const { result } = renderHook(() => useSetAdminFeatureFlag(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      flagKey: "experimental:ai-assistant",
      body: { target_kind: "global", target_key: null, enabled: true },
    });
    expect(mockPut).toHaveBeenCalledWith(
      "/admin/feature-flags/experimental%3Aai-assistant",
      { target_kind: "global", target_key: null, enabled: true },
    );
  });

  it("writes flag metadata to the encoded metadata sub-route", async () => {
    mockPatch.mockResolvedValue({});
    const { result } = renderHook(() => useUpdateAdminFeatureFlagMetadata(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      flagKey: "experimental:ai-assistant",
      body: { description: "What it gates." },
    });
    expect(mockPatch).toHaveBeenCalledWith(
      "/admin/feature-flags/experimental%3Aai-assistant/metadata",
      { description: "What it gates." },
    );
  });

  it("clears a user override with query parameters", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useClearAdminFeatureFlag(), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      flagKey: "example_ui",
      targetKind: "user",
      targetKey: "user-1",
    });
    expect(mockDelete).toHaveBeenCalledWith(
      "/admin/feature-flags/example_ui?target_kind=user&target_key=user-1",
    );
  });
});

it("publishes saved metadata before resolving and ignores a delayed pre-save list read", async () => {
  const before = {
    key: "test",
    description: "Old",
    code_description: "Code",
    custom_description: "Old",
    owner: "Owner",
    metadata_updated_at: null,
    metadata_updated_by: null,
    default_enabled: false,
    global_override: null,
    org_overrides: [],
    user_overrides: [],
  };
  const saved = {
    ...before,
    description: "Saved",
    custom_description: "Saved",
    metadata_updated_by: "admin",
  };
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const key = ["admin", "feature-flags"];
  client.setQueryData(key, { flags: [before] });
  let finishRead!: (value: unknown) => void;
  const read = client
    .fetchQuery({
      queryKey: key,
      queryFn: () =>
        new Promise((resolve) => {
          finishRead = resolve;
        }),
    })
    .catch(() => undefined);
  mockPatch.mockResolvedValue(saved);
  const { result } = renderHook(() => useUpdateAdminFeatureFlagMetadata(), {
    wrapper: ({ children }: PropsWithChildren) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
  await result.current.mutateAsync({
    flagKey: "test",
    body: { description: "Saved" },
  });
  expect(client.getQueryData(key)).toEqual({ flags: [saved] });
  finishRead({ flags: [before] });
  await read;
  expect(client.getQueryData(key)).toEqual({ flags: [saved] });
  client.clear();
});

it("publishes rollout success before refresh and preserves unrelated target labels", async () => {
  const flag = {
    key: "test",
    description: "Code",
    code_description: "Code",
    custom_description: null,
    owner: null,
    metadata_updated_at: null,
    metadata_updated_by: null,
    default_enabled: false,
    global_override: null,
    org_overrides: [],
    user_overrides: [
      {
        user_id: "a",
        user_email: "a@example.com",
        user_display_name: "A",
        enabled: false,
        updated_at: "old",
        updated_by: "a",
      },
    ],
  };
  const client = new QueryClient();
  const key = ["admin", "feature-flags"];
  client.setQueryData(key, { flags: [flag] });
  mockPut.mockResolvedValue({
    enabled: true,
    updated_at: "new",
    updated_by: "b",
  });
  const { result } = renderHook(
    () => ({
      set: useSetAdminFeatureFlag(),
      clear: useClearAdminFeatureFlag(),
    }),
    {
      wrapper: ({ children }: PropsWithChildren) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      ),
    },
  );
  await result.current.set.mutateAsync({
    flagKey: "test",
    body: { target_kind: "user", target_key: "a", enabled: true },
  });
  expect(client.getQueryData(key)).toMatchObject({
    flags: [
      {
        user_overrides: [
          { user_email: "a@example.com", enabled: true, updated_at: "new" },
        ],
      },
    ],
  });
  await result.current.clear.mutateAsync({
    flagKey: "test",
    targetKind: "user",
    targetKey: "a",
  });
  expect(client.getQueryData(key)).toMatchObject({
    flags: [{ user_overrides: [] }],
  });
  client.clear();
});
