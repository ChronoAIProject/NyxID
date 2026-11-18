import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type { KeyInfo } from "@/types/keys";
import { useKeys } from "./use-keys";
import { useCurrentKeysReader } from "./use-current-keys";

const key = { id: "one", label: "One", is_active: true } as KeyInfo;
afterEach(() => {
  cleanup();
  useAuthStore.setState({ user: null });
  vi.restoreAllMocks();
});

it("drift guard: reads the real useKeys list synchronously with live authority and query status", async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const get = vi.spyOn(api, "get").mockResolvedValue({ keys: [key] });
  useAuthStore.setState({ user: { id: "A" } as User });
  const list = ["keys", "list", "A"];
  client.setQueryData(list, [key]);
  const hook = renderHook(() => ({ keys: useKeys(), read: useCurrentKeysReader() }), {
    wrapper: ({ children }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>,
  });
  await waitFor(() => expect(hook.result.current.keys.isFetching).toBe(false));
  const read = hook.result.current.read;
  expect(hook.result.current.keys.data).toEqual([key]);
  expect(read()).toEqual({ authority: "A", data: [key], isError: false, isFetching: false });
  const changed = [{ ...key, credential_missing: true }];
  act(() => {
    client.setQueryData(list, changed);
    // Assert before TanStack can notify React: this cannot read rendered data.
    expect(hook.result.current.keys.data).toEqual([key]);
    expect(read().data).toEqual(changed);
  });
  await waitFor(() => expect(hook.result.current.keys.data).toEqual(changed));
  expect(hook.result.current.read).toBe(read);
  let fail!: (error: Error) => void;
  get.mockImplementationOnce(() => new Promise((_resolve, reject) => { fail = reject; }));
  let fetch!: ReturnType<typeof hook.result.current.keys.refetch>;
  act(() => {
    fetch = hook.result.current.keys.refetch();
    expect(read().isFetching).toBe(true);
    expect(read().data).toEqual(changed);
  });
  await act(async () => { fail(new Error("keys denied")); await fetch; });
  expect(client.getQueryState(list)?.data).toEqual(changed);
  expect(read()).toEqual({ authority: "A", data: undefined, isError: true, isFetching: false });
  await waitFor(() => expect(hook.result.current.keys.data).toBeUndefined());
  const other = [{ ...key, id: "B-key" }];
  client.setQueryData(["keys", "list", "B"], other);
  get.mockResolvedValue({ keys: other });
  act(() => {
    useAuthStore.setState({ user: { id: "B" } as User });
    expect(read().authority).toBe("B");
    expect(read().data).toEqual(other);
  });
  await waitFor(() => expect(hook.result.current.keys.data).toEqual(other));
  expect(hook.result.current.read).toBe(read);
  act(() => {
    client.clear();
    expect(read()).toEqual({ authority: "B", data: undefined, isError: false, isFetching: false });
  });
  hook.unmount();
  client.clear();
});
