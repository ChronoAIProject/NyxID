import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { useKey } from "./use-keys";

const { get } = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api-client")>()),
  api: { get },
}));

function harness() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  function Wrapper({ children }: PropsWithChildren) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  }
  return { client, Wrapper };
}

beforeEach(() => {
  get.mockReset();
  useAuthStore.setState({ user: { id: "owner" } as User });
});

it.each([
  new TypeError("Failed to fetch"),
  new ApiError(503, { error: "unavailable", error_code: -1, message: "Unavailable" }),
])("retains verified detail after a transient refresh error: %s", async (error) => {
  const saved = { id: "k1", label: "Saved" };
  get.mockResolvedValue(saved);
  const { Wrapper } = harness();
  const { result } = renderHook(() => useKey("k1"), { wrapper: Wrapper });
  await waitFor(() => expect(result.current.data).toEqual(saved));
  get.mockRejectedValue(error);
  await act(async () => { await result.current.refetch(); });
  await waitFor(() => expect(result.current.isError).toBe(true));
  expect(result.current.data).toEqual(saved);
});

it.each([400, 401, 403, 404])("discards rejected cached details after HTTP %s, including later network failures", async (status) => {
  const saved = { id: "k1", label: "Saved" };
  get.mockResolvedValue(saved);
  const { client, Wrapper } = harness();
  const { result } = renderHook(() => useKey("k1"), { wrapper: Wrapper });
  await waitFor(() => expect(result.current.data).toEqual(saved));
  get.mockRejectedValue(new ApiError(status, { error: "rejected", error_code: -1, message: "Rejected" }));
  await act(async () => { await result.current.refetch(); });
  await waitFor(() => expect(result.current.isError).toBe(true));
  expect(result.current.data).toBeUndefined();
  expect(client.getQueryData(["keys", "k1", "owner"])).not.toEqual(saved);
  get.mockRejectedValue(new TypeError("Failed to fetch"));
  await act(async () => { await result.current.refetch(); });
  expect(result.current.data).toBeUndefined();
  get.mockResolvedValue({ ...saved, label: "Verified again" });
  await act(async () => { await result.current.refetch(); });
  await waitFor(() => expect(result.current.data?.label).toBe("Verified again"));
});

it("does not expose the previous identity's cached detail on an account switch", async () => {
  get.mockResolvedValue({ id: "k1", label: "Owner's service" });
  const { Wrapper } = harness();
  const { result } = renderHook(() => useKey("k1"), { wrapper: Wrapper });
  await waitFor(() => expect(result.current.data).toBeDefined());
  get.mockRejectedValue(new TypeError("Failed to fetch"));
  act(() => useAuthStore.setState({ user: { id: "another-owner" } as User }));
  await waitFor(() => expect(result.current.isError).toBe(true));
  expect(result.current.data).toBeUndefined();
});
