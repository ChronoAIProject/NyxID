import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useClearOrgRoleScope,
  useOrgRoleScopes,
  useSetOrgRoleScope,
} from "./use-org-role-scopes";

const { mockGet, mockPut, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPut: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, put: mockPut, delete: mockDelete },
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useOrgRoleScopes", () => {
  it("unwraps the `role_scopes` array from the response", async () => {
    mockGet.mockResolvedValue({
      role_scopes: [{ role: "member", source: "override" }],
    });
    const { result } = renderHook(() => useOrgRoleScopes("org-1"), {
      wrapper: createWrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/orgs/org-1/role-scopes");
    expect(result.current.data).toEqual([
      { role: "member", source: "override" },
    ]);
  });

  it("stays idle and does not fetch for an empty orgId", async () => {
    mockGet.mockResolvedValue({ role_scopes: [] });
    const { result } = renderHook(() => useOrgRoleScopes(""), {
      wrapper: createWrapper(),
    });
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();
  });
});

describe("useSetOrgRoleScope", () => {
  it("PUTs the body to /orgs/{orgId}/role-scopes/{role}", async () => {
    mockPut.mockResolvedValue({ role: "member", source: "override" });
    const { result } = renderHook(() => useSetOrgRoleScope("org-1"), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({
      role: "member",
      body: { allowed_service_ids: ["svc-1"] } as never,
    });
    expect(mockPut).toHaveBeenCalledWith("/orgs/org-1/role-scopes/member", {
      allowed_service_ids: ["svc-1"],
    });
  });
});

describe("useClearOrgRoleScope", () => {
  it("DELETEs /orgs/{orgId}/role-scopes/{role}", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useClearOrgRoleScope("org-1"), {
      wrapper: createWrapper(),
    });
    await result.current.mutateAsync({ role: "viewer" });
    expect(mockDelete).toHaveBeenCalledWith("/orgs/org-1/role-scopes/viewer");
  });
});
