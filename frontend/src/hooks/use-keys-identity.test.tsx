import {
  QueryClient,
  QueryClientProvider,
  QueryObserver,
} from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { useKeys } from "./use-keys";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { ApiError } from "@/lib/api-client";

const { get } = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<object>()),
  api: { get },
}));
beforeEach(() => {
  vi.clearAllMocks();
  useAuthStore.setState({ user: { id: "A" } as User });
});
it("fences every retry by query actor even when the old observer stays active", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: 1, retryDelay: 0 } },
  });
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  let fail!: (reason: unknown) => void;
  const actors: (string | undefined)[] = [];
  get.mockImplementation(() => {
    const actor = useAuthStore.getState().user?.id;
    actors.push(actor);
    return actor === "A"
      ? new Promise((_resolve, reject) => {
          fail = reject;
        })
      : Promise.resolve({ keys: [{ id: "B-connection" }] });
  });
  const { result, unmount } = renderHook(() => useKeys(), { wrapper });
  await waitFor(() => expect(actors).toEqual(["A"]));
  // Keep the old query active across the account switch to exercise its retry.
  const query = client
    .getQueryCache()
    .find({ queryKey: ["keys", "list", "A"] })!;
  const observer = new QueryObserver(client, {
    ...query.options,
    queryKey: query.queryKey,
  });
  const unsubscribe = observer.subscribe(() => {});
  act(() => useAuthStore.setState({ user: { id: "B" } as User }));
  await waitFor(() =>
    expect(result.current.data).toEqual([{ id: "B-connection" }]),
  );
  await act(async () =>
    fail(
      new ApiError(503, {
        error: "unavailable",
        error_code: 1000,
        message: "read failed",
      }),
    ),
  );
  await waitFor(() => expect(query.state.status).toBe("error"));
  expect(query.state.fetchStatus).toBe("idle");
  expect(query.state.error).toEqual(
    new Error("Account changed before loading connections"),
  );
  expect(actors).toEqual(["A", "B"]);
  expect(client.getQueryData(["keys", "list", "A"])).toBeUndefined();
  expect(client.getQueryData(["keys", "list", "B"])).toEqual([
    { id: "B-connection" },
  ]);
  await act(async () => {
    await result.current.refetch();
  });
  expect(actors).toEqual(["A", "B", "B"]);
  unsubscribe();
  unmount();
  client.clear();
});

it("does not load inventory before an identity exists", () => {
  useAuthStore.setState({ user: null });
  const client = new QueryClient();
  renderHook(() => useKeys(), {
    wrapper: ({ children }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
  expect(get).not.toHaveBeenCalled();
});
