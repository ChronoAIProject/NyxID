import type { PropsWithChildren } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useCrmRecord } from "./use-crm";
import { CrmPollingContext } from "@/lib/crm/query-policy";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";

const { request } = vi.hoisted(() => ({ request: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantJson: request }));
afterEach(() => vi.useRealTimers());

it("chat queries refetch on mount and CRM invalidation without interval polling", async () => {
  useAuthStore.setState({ user: { id: "person" } as User });
  const record = {
    id: "r",
    owner_id: "person",
    object_key: "company",
    revision: 1,
    values: { name: "Original" },
    links: {},
    overrides: [],
    archived: false,
    created_at: "2026-11-04T00:00:00Z",
    updated_at: "2026-11-04T00:00:00Z",
  };
  request.mockResolvedValue(record);
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, gcTime: Infinity },
    },
  });
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>
      <CrmPollingContext value={false}>{children}</CrmPollingContext>
    </QueryClientProvider>
  );
  const hook = renderHook(() => useCrmRecord("r"), { wrapper });
  await waitFor(() => expect(hook.result.current.isSuccess).toBe(true));
  expect(request).toHaveBeenCalledTimes(1);
  vi.useFakeTimers();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(90_000);
  });
  expect(request).toHaveBeenCalledTimes(1);
  await act(async () => {
    await client.invalidateQueries({ queryKey: ["crm", "person"] });
  });
  expect(request).toHaveBeenCalledTimes(2);
  hook.unmount();
  const again = renderHook(() => useCrmRecord("r"), { wrapper });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1);
  });
  expect(request).toHaveBeenCalledTimes(3);
  again.unmount();
  client.clear();
});
