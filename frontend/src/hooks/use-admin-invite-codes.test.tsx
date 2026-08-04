import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useAdminInviteCodes,
  useCreateInviteCode,
  useDeactivateInviteCode,
  useUpdateInviteCode,
} from "./use-admin-invite-codes";

const { mockGet, mockPost, mockPatch, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockPatch: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    get: mockGet,
    post: mockPost,
    patch: mockPatch,
    delete: mockDelete,
  },
}));

function wrapperFactory() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  const Wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return { Wrapper, queryClient };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useAdminInviteCodes", () => {
  it("GETs the invite-codes collection and returns the raw list response", async () => {
    mockGet.mockResolvedValue({ invite_codes: [{ id: "ic1" }] });
    const { Wrapper } = wrapperFactory();
    const { result } = renderHook(() => useAdminInviteCodes(), {
      wrapper: Wrapper,
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/admin/invite-codes");
    expect(result.current.data).toEqual({ invite_codes: [{ id: "ic1" }] });
  });
});

describe("useCreateInviteCode", () => {
  it("POSTs the create body to the collection", async () => {
    mockPost.mockResolvedValue({ id: "ic1", code: "INV-ABC" });
    const { Wrapper } = wrapperFactory();
    const { result } = renderHook(() => useCreateInviteCode(), {
      wrapper: Wrapper,
    });
    await result.current.mutateAsync({ note: "for Ann" } as never);
    expect(mockPost).toHaveBeenCalledWith("/admin/invite-codes", {
      note: "for Ann",
    });
  });
});

describe("useUpdateInviteCode", () => {
  it("PATCHes the code by id with an abort signal and splices the response into the cached list", async () => {
    const updated = { id: "ic1", note: "edited" };
    mockPatch.mockResolvedValue(updated);
    const { Wrapper, queryClient } = wrapperFactory();
    queryClient.setQueryData(["admin", "invite-codes"], {
      invite_codes: [
        { id: "ic1", note: "old" },
        { id: "ic2", note: "keep" },
      ],
    });

    const { result } = renderHook(() => useUpdateInviteCode(), {
      wrapper: Wrapper,
    });
    await result.current.mutateAsync({ id: "ic1", body: { note: "edited" } });

    const [url, body, options] = mockPatch.mock.calls[0]!;
    expect(url).toBe("/admin/invite-codes/ic1");
    expect(body).toEqual({ note: "edited" });
    expect((options as { signal: unknown }).signal).toBeInstanceOf(AbortSignal);

    // onSuccess replaces only the matching record in place.
    expect(
      queryClient.getQueryData(["admin", "invite-codes"]),
    ).toEqual({
      invite_codes: [updated, { id: "ic2", note: "keep" }],
    });
  });
});

describe("useDeactivateInviteCode", () => {
  it("DELETEs the code by id", async () => {
    mockDelete.mockResolvedValue({ message: "deactivated" });
    const { Wrapper } = wrapperFactory();
    const { result } = renderHook(() => useDeactivateInviteCode(), {
      wrapper: Wrapper,
    });
    await result.current.mutateAsync("ic1");
    expect(mockDelete).toHaveBeenCalledWith("/admin/invite-codes/ic1");
  });
});
