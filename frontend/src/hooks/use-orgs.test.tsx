import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useCreateOrg,
  useDeleteOrg,
  useOrg,
  useOrgs,
  useRedeemInvite,
  useSetPrimaryOrg,
  useUpdateOrg,
} from "./use-orgs";

const { mockGet, mockPost, mockPatch, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockPatch: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, patch: mockPatch, delete: mockDelete },
}));

function wrapperFactory() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("org queries", () => {
  it("useOrgs unwraps the `orgs` array", async () => {
    mockGet.mockResolvedValue({ orgs: [{ id: "org-1" }] });
    const { result } = renderHook(() => useOrgs(), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/orgs");
    expect(result.current.data).toEqual([{ id: "org-1" }]);
  });

  it("useOrg fetches a single org and is disabled for an empty id", async () => {
    mockGet.mockResolvedValue({ id: "org-1" });
    const { result, rerender } = renderHook(({ id }) => useOrg(id), {
      wrapper: wrapperFactory(),
      initialProps: { id: "" },
    });
    expect(result.current.fetchStatus).toBe("idle");
    expect(mockGet).not.toHaveBeenCalled();

    rerender({ id: "org-1" });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/orgs/org-1");
  });
});

describe("useCreateOrg vs useUpdateOrg empty-string handling", () => {
  it("CREATE strips empty strings (so an empty contact_email never reaches validation)", async () => {
    mockPost.mockResolvedValue({ id: "org-1" });
    const { result } = renderHook(() => useCreateOrg(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({
      display_name: "Acme",
      contact_email: "",
      avatar_url: undefined,
    });
    expect(mockPost).toHaveBeenCalledWith("/orgs", { display_name: "Acme" });
  });

  it("UPDATE preserves empty strings (so avatar_url:'' can clear the field)", async () => {
    mockPatch.mockResolvedValue({ id: "org-1" });
    const { result } = renderHook(() => useUpdateOrg(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({
      orgId: "org-1",
      body: { avatar_url: "", contact_email: undefined },
    });
    expect(mockPatch).toHaveBeenCalledWith("/orgs/org-1", { avatar_url: "" });
  });
});

describe("other org mutations", () => {
  it("useDeleteOrg deletes /orgs/{id}", async () => {
    mockDelete.mockResolvedValue(undefined);
    const { result } = renderHook(() => useDeleteOrg(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync("org-1");
    expect(mockDelete).toHaveBeenCalledWith("/orgs/org-1");
  });

  it("useSetPrimaryOrg PATCHes /users/me/primary-org", async () => {
    mockPatch.mockResolvedValue({ primary_org_id: "org-1" });
    const { result } = renderHook(() => useSetPrimaryOrg(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ primary_org_id: "org-1" });
    expect(mockPatch).toHaveBeenCalledWith("/users/me/primary-org", {
      primary_org_id: "org-1",
    });
  });

  it("useRedeemInvite POSTs to the join nonce endpoint", async () => {
    mockPost.mockResolvedValue({ org_id: "org-1", role: "member" });
    const { result } = renderHook(() => useRedeemInvite(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync("ORGINV-ABC-123");
    expect(mockPost).toHaveBeenCalledWith("/orgs/join/ORGINV-ABC-123");
  });
});
