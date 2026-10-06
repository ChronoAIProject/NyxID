import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import {
  useServicePreference,
  useSaveServicePreference,
} from "./use-service-preference";
import { ApiError } from "@/lib/api-client";
const { get, put } = vi.hoisted(() => ({ get: vi.fn(), put: vi.fn() }));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<object>()),
  api: { get, put },
}));
const empty = { ordered: [], version: 0, updated_at: null };
function wrapper(client: QueryClient) {
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}
beforeEach(() => {
  vi.clearAllMocks();
  useAuthStore.setState({ user: { id: "one" } as User });
});
it("a failed read hides cached order and 404 reports unsupported", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  client.setQueryData(["service-preference", "one"], empty);
  get.mockRejectedValue(new TypeError("offline"));
  const { result } = renderHook(() => useServicePreference(), {
    wrapper: wrapper(client),
  });
  await waitFor(() => expect(result.current.isError).toBe(true));
  expect(result.current.data).toBeUndefined();
  get.mockRejectedValue(
    new ApiError(404, {
      error: "not_found",
      error_code: 1003,
      message: "Not found",
    }),
  );
  await act(async () => {
    await result.current.refetch();
  });
  await waitFor(() => expect(result.current.data).toBeNull());
});
it("query identity changes use an independent cache and late mutations are rejected", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  get.mockResolvedValue(empty);
  const { result } = renderHook(
    () => ({ query: useServicePreference(), save: useSaveServicePreference() }),
    { wrapper: wrapper(client) },
  );
  await waitFor(() => expect(result.current.query.isSuccess).toBe(true));
  const oldMutation = result.current.save.mutateAsync;
  act(() => useAuthStore.setState({ user: { id: "two" } as User }));
  await waitFor(() =>
    expect(client.getQueryData(["service-preference", "two"])).toEqual(empty),
  );
  await expect(
    oldMutation({ ordered: [], expected_version: 0 }),
  ).rejects.toThrow("Account changed");
  expect(put).not.toHaveBeenCalled();
});
