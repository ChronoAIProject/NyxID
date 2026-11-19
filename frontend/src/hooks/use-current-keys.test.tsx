import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { onlineManager, QueryClient, QueryClientProvider } from "@tanstack/react-query";
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
  onlineManager.setOnline(true);
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
  expect(read()).toEqual({ authority: "A", data: [key], isError: false, isRefreshing: false, fetchStatus: "idle" });
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
    expect(read().isRefreshing).toBe(true);
    expect(read().fetchStatus).toBe("fetching");
    expect(read().data).toEqual(changed);
  });
  await act(async () => { fail(new Error("keys denied")); await fetch; });
  expect(client.getQueryState(list)?.data).toEqual(changed);
  expect(read()).toEqual({ authority: "A", data: undefined, isError: true, isRefreshing: false, fetchStatus: "idle" });
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
    expect(read()).toEqual({ authority: "B", data: undefined, isError: false, isRefreshing: false, fetchStatus: undefined });
  });
  hook.unmount();
  client.clear();
});

it("R1: drift guard reports a paused refresh until the real online manager resumes it to idle", async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const get = vi.spyOn(api, "get").mockResolvedValue({ keys: [key] });
  useAuthStore.setState({ user: { id: "A" } as User });
  const list = ["keys", "list", "A"];
  client.setQueryData(list, [key]);
  const hook = renderHook(() => ({ keys: useKeys(), read: useCurrentKeysReader() }), {
    wrapper: ({ children }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>,
  });
  try {
    await waitFor(() => expect(hook.result.current.keys.isFetching).toBe(false));
    const read = hook.result.current.read;
    let resume!: (value: { keys: KeyInfo[] }) => void;
    get.mockImplementationOnce(() => new Promise((resolve) => { resume = resolve; }));
    act(() => {
      onlineManager.setOnline(false);
      void hook.result.current.keys.refetch();
    });
    expect(client.getQueryState(list)?.fetchStatus).toBe("paused");
    await waitFor(() => expect(hook.result.current.keys.isPaused).toBe(true));
    expect(read()).toMatchObject({ authority: "A", data: [key], isError: false, isRefreshing: true, fetchStatus: "paused" });
    expect(get).toHaveBeenCalledOnce();
    act(() => onlineManager.setOnline(true));
    await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
    expect(read()).toMatchObject({ isRefreshing: true, fetchStatus: "fetching" });
    const changed = [{ ...key, credential_missing: true }];
    await act(async () => resume({ keys: changed }));
    await waitFor(() => expect(hook.result.current.keys.fetchStatus).toBe("idle"));
    expect(read()).toMatchObject({ data: changed, isRefreshing: false, fetchStatus: "idle" });
    expect(hook.result.current.read).toBe(read);
  } finally {
    hook.unmount();
    client.clear();
    onlineManager.setOnline(true);
  }
});
