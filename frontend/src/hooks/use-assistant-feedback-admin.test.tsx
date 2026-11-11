import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { useAssistantFeedbackAdmin } from "./use-assistant-feedback-admin";

vi.mock("@/lib/api-client", () => ({ api: { get: vi.fn() } }));
const initialAuth = useAuthStore.getState();
afterEach(() => {
  useAuthStore.setState(initialAuth);
  vi.resetAllMocks();
});

it("does not show the previous period's totals while a new period loads", async () => {
  const client = new QueryClient();
  vi.mocked(api.get).mockResolvedValueOnce({
    counts: [{ day: "2026-01-01", up: 5, down: 1 }],
    recent: [],
  });
  vi.mocked(api.get).mockImplementationOnce(() => new Promise(() => {}));
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const hook = renderHook(
    ({ from }) => useAssistantFeedbackAdmin(new URLSearchParams({ from })),
    { initialProps: { from: "2026-01-01" }, wrapper },
  );
  await waitFor(() => expect(hook.result.current.data?.counts[0]?.up).toBe(5));
  hook.rerender({ from: "2026-02-01" });
  expect(hook.result.current.data).toBeUndefined();
  expect(hook.result.current.isPending).toBe(true);
  hook.unmount();
  client.clear();
});

it("does not reuse another account's feedback after an identity switch", async () => {
  const client = new QueryClient();
  useAuthStore.setState({
    user: { ...initialAuth.user, id: "account-a" } as NonNullable<
      typeof initialAuth.user
    >,
  });
  vi.mocked(api.get).mockResolvedValueOnce({
    counts: [],
    recent: [
      {
        message_id: "private-a",
        vote: "down",
        comment: "Private account A comment",
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
      },
    ],
  });
  vi.mocked(api.get).mockImplementationOnce(() => new Promise(() => {}));
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const hook = renderHook(
    () => useAssistantFeedbackAdmin(new URLSearchParams()),
    { wrapper },
  );
  await waitFor(() =>
    expect(hook.result.current.data?.recent[0]?.message_id).toBe("private-a"),
  );
  act(() =>
    useAuthStore.setState({
      user: { ...useAuthStore.getState().user!, id: "account-b" },
    }),
  );
  expect(hook.result.current.data).toBeUndefined();
  hook.unmount();
  client.clear();
});
