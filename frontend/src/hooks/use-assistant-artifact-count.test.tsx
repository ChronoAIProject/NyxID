import type { PropsWithChildren } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { useAssistantWorkspaceCounts } from "./use-assistant-workspace";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";

const state = vi.hoisted(() => ({ enabled: false, request: vi.fn() }));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => state.enabled,
}));
vi.mock("@/hooks/use-approvals", () => ({
  useApprovalRequests: () => ({ data: { total: 3 } }),
}));
vi.mock("@/lib/assistant/assistant-http", () => ({
  assistantJson: state.request,
}));

it("skips the flag-off count, reads the combined catalog count and refreshes on live invalidation", async () => {
  useAuthStore.setState({ user: { id: "person" } as User });
  state.request.mockResolvedValue({ count: 2, views: 1, objects: 1 });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const hook = renderHook(useAssistantWorkspaceCounts, { wrapper });
  expect(hook.result.current.data).toEqual({
    artifacts: 0,
    pendingApprovals: 3,
  });
  expect(state.request).not.toHaveBeenCalled();
  state.enabled = true;
  hook.rerender();
  await waitFor(() => expect(hook.result.current.data.artifacts).toBe(2));
  expect(state.request).toHaveBeenCalledWith("/assistant/crm/artifacts/count");
  state.request.mockResolvedValue({ count: 3, views: 1, objects: 2 });
  await act(async () => {
    await client.invalidateQueries({ queryKey: ["crm", "person"] });
  });
  await waitFor(() => expect(hook.result.current.data.artifacts).toBe(3));
  hook.unmount();
  client.clear();
});
