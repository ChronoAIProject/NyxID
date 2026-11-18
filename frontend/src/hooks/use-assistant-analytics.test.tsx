import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { useAssistantAnalytics } from "./use-assistant-analytics";
import {
  internalOwnersFormSchema,
  splitOwnerIds,
} from "@/schemas/assistant-real-work";
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  api: { get: vi.fn() },
}));
it("passes threshold in the query and tolerates older replicas without real-work data", async () => {
  const from = "2026-11-01T00:00:00Z",
    to = "2026-11-08T00:00:00Z";
  vi.mocked(api.get).mockResolvedValue({
    from,
    to,
    generated_at: to,
    active_sessions: 0,
    active_accounts: 0,
    running_now: 0,
    outcomes: {
      completed: 0,
      failed: 0,
      stopped: 0,
      with_tools: 0,
      with_tool_errors: 0,
    },
    daily_sessions: [],
    daily_turns: [],
  });
  const client = new QueryClient();
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const { result, rerender } = renderHook(
    ({ threshold }) => useAssistantAnalytics(from, to, true, threshold),
    { wrapper, initialProps: { threshold: 3 } },
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(api.get).toHaveBeenLastCalledWith(
    `/admin/assistant/analytics?${new URLSearchParams({ from, to, threshold: "3" })}`,
  );
  rerender({ threshold: 7 });
  await waitFor(() =>
    expect(api.get).toHaveBeenLastCalledWith(
      `/admin/assistant/analytics?${new URLSearchParams({ from, to, threshold: "7" })}`,
    ),
  );
  expect(result.current.data?.real_work).toBeUndefined();
});
it("validates internal owner IDs without retaining duplicates", () => {
  const id = "11111111-1111-4111-8111-111111111111";
  expect(splitOwnerIds(`${id},\n${id}`)).toEqual([id]);
  expect(
    internalOwnersFormSchema.safeParse({ owners: "invalid-id" }).success,
  ).toBe(false);
  expect(internalOwnersFormSchema.safeParse({ owners: "" }).success).toBe(true);
});
