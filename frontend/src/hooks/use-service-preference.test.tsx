import {
  MutationCache,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import {
  useServicePreference,
  useSaveServiceGroupOrder,
} from "./use-service-preference";
import { ApiError } from "@/lib/api-client";
const { get, put, transport } = vi.hoisted(() => ({
  get: vi.fn(),
  put: vi.fn(),
  transport: vi.fn(),
}));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<object>()),
  api: { get, put },
  apiClient: transport,
}));
const empty = { groups: [], version: 0, updated_at: null };
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
  await waitFor(() => expect(result.current.data).toBe("unavailable"));
});
it("query identity changes use an independent cache and late mutations are rejected", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  get.mockResolvedValue(empty);
  const { result } = renderHook(
    () => ({ query: useServicePreference(), save: useSaveServiceGroupOrder() }),
    { wrapper: wrapper(client) },
  );
  await waitFor(() => expect(result.current.query.isSuccess).toBe(true));
  const oldMutation = result.current.save.save;
  act(() => useAuthStore.setState({ user: { id: "two" } as User }));
  await waitFor(() =>
    expect(client.getQueryData(["service-preference", "two"])).toEqual(empty),
  );
  await expect(
    oldMutation("catalog:aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa", {
      ordered: [],
      expected_version: 0,
    }),
  ).rejects.toThrow("Account changed");
  expect(put).not.toHaveBeenCalled();
});

it.each(["save", "release"] as const)(
  "captures actor in %s variables across a deferred onMutate",
  async (method) => {
    let resume!: () => void;
    const gate = new Promise<void>((resolve) => {
      resume = resolve;
    });
    let pause = true;
    const client = new QueryClient({
      mutationCache: new MutationCache({
        onMutate: () => (pause ? gate : undefined),
      }),
      defaultOptions: { queries: { retry: false } },
    });
    put.mockResolvedValue(empty);
    transport.mockResolvedValue(empty);
    const { result } = renderHook(() => useSaveServiceGroupOrder(), {
      wrapper: wrapper(client),
    });
    let pending!: Promise<unknown>;
    act(() => {
      pending =
        method === "save"
          ? result.current.save(
              "catalog:aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa",
              { ordered: [], expected_version: 0 },
            )
          : result.current.release(0);
    });
    // Attach rejection before the gate resumes to avoid an unhandled promise.
    const rejected = expect(pending).rejects.toThrow("Account changed");
    act(() => useAuthStore.setState({ user: { id: "two" } as User }));
    pause = false;
    await act(async () => {
      resume();
      await rejected;
    });
    expect(put).not.toHaveBeenCalled();
    expect(transport).not.toHaveBeenCalled();
    expect(result.current.isPending).toBe(false);
    await act(async () => {
      if (method === "save")
        await result.current.save(
          "catalog:aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa",
          { ordered: [], expected_version: 0 },
        );
      else await result.current.release(0);
    });
    expect(method === "save" ? put : transport).toHaveBeenCalledTimes(1);
  },
);

it("late A success marks only A stale without reading through B authentication", async () => {
  const a = { groups: [], version: 1, updated_at: null };
  const b = { groups: [], version: 7, updated_at: null };
  get.mockImplementation(async () =>
    useAuthStore.getState().user?.id === "one" ? a : b,
  );
  let finish!: (value: unknown) => void;
  put.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const invalidate = vi.spyOn(client, "invalidateQueries");
  const { result } = renderHook(
    () => ({
      query: useServicePreference(),
      mutation: useSaveServiceGroupOrder(),
    }),
    { wrapper: wrapper(client) },
  );
  await waitFor(() => expect(result.current.query.data).toEqual(a));
  let request!: Promise<unknown>;
  act(() => {
    request = result.current.mutation.save(
      "catalog:aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa",
      { ordered: [], expected_version: 1 },
    );
  });
  await waitFor(() => expect(put).toHaveBeenCalledTimes(1));
  act(() => useAuthStore.setState({ user: { id: "two" } as User }));
  await waitFor(() => expect(result.current.query.data).toEqual(b));
  const reads = get.mock.calls.length;
  await act(async () => {
    finish(a);
    await request;
  });
  expect(get).toHaveBeenCalledTimes(reads);
  expect(invalidate).toHaveBeenCalledWith({
    queryKey: ["service-preference", "one"],
    refetchType: "none",
  });
  expect(
    invalidate.mock.calls.some(
      ([options]) => options?.queryKey?.[0] === "keys",
    ),
  ).toBe(false);
  expect(client.getQueryData(["service-preference", "one"])).toEqual(a);
  expect(client.getQueryData(["service-preference", "two"])).toEqual(b);
});
