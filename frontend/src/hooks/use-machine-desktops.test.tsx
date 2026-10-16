import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { useMachineDesktops } from "./use-machines";

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

it("polls a conversation's desktops only while a turn runs or a desktop is open", async () => {
  vi.useFakeTimers();
  let desktops: unknown[] = [];
  const get = vi.spyOn(api, "get").mockImplementation(() => Promise.resolve(desktops));
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const { rerender } = renderHook(
    ({ active }: { active: boolean }) => useMachineDesktops("nyxa-thread", active),
    { wrapper, initialProps: { active: false } },
  );
  await act(() => vi.advanceTimersByTimeAsync(0));
  expect(get).toHaveBeenCalledTimes(1);
  // An idle thread without desktops polls nothing.
  await act(() => vi.advanceTimersByTimeAsync(16_000));
  expect(get).toHaveBeenCalledTimes(1);
  // A running turn may open one.
  rerender({ active: true });
  desktops = [{ node_id: "n", session_id: "s", conversation_id: "nyxa-thread", status: "open", reason: null }];
  await act(() => vi.advanceTimersByTimeAsync(5_000));
  expect(get).toHaveBeenCalledTimes(2);
  // An open desktop keeps its status fresh after the turn ends.
  rerender({ active: false });
  await act(() => vi.advanceTimersByTimeAsync(0));
  expect(get).toHaveBeenCalledTimes(3);
  await act(() => vi.advanceTimersByTimeAsync(5_000));
  expect(get).toHaveBeenCalledTimes(4);
  // Once it closes, polling stops.
  desktops = [];
  await act(() => vi.advanceTimersByTimeAsync(5_000));
  expect(get).toHaveBeenCalledTimes(5);
  await act(() => vi.advanceTimersByTimeAsync(16_000));
  expect(get).toHaveBeenCalledTimes(5);
  client.clear();
});
