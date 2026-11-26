import { createElement, type ReactNode } from "react";
import { act, cleanup, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { useChatDecisions } from "./use-chat-decisions";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

const mocks = vi.hoisted(() => ({ shared: vi.fn() }));
vi.mock("./use-shared-agents", () => ({ useSharedConsents: mocks.shared }));
afterEach(() => { cleanup(); vi.useRealTimers(); });
function wrapper({ children }: { children: ReactNode }) {
  return createElement(QueryClientProvider, { client: new QueryClient({ defaultOptions: { queries: { retry: false } } }) }, children);
}

it("removes a request at its expiry without a history poll", () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-11-26T00:00:00Z"));
  mocks.shared.mockReturnValue({ data: undefined });
  const card: NyxAgentAcknowledgement = {
    id: "card", kind: "action", status: "pending", decider: "user", decided_by: null, summary: "Write",
    reason: null, service_slug: null, service_name: null, tool_name: "write", created_at: new Date().toISOString(),
    decided_at: null, expires_at: new Date(Date.now() + 5000).toISOString(),
  };
  const { result } = renderHook(() => useChatDecisions({ acknowledgements: [card], approvals: [], conversationId: "chat" }), { wrapper });
  expect(result.current.decisions).toHaveLength(1);
  act(() => { vi.advanceTimersByTime(5000); });
  expect(result.current.decisions).toHaveLength(0);
});

it("counts only this conversation's shared consents and clears the stack when the thread is disabled", () => {
  const consent = { id: "mine", conversation_id: "chat", decision: "pending", expires_at: null };
  mocks.shared.mockReturnValue({ data: { consents: [consent, { ...consent, id: "another-chat", conversation_id: "other" }] } });
  const { result, rerender } = renderHook(({ disabled }) => useChatDecisions({ acknowledgements: [], approvals: [], sharedAgentId: "shared", conversationId: "chat", disabled }), { wrapper, initialProps: { disabled: false } });
  expect(result.current.decisions.map((row) => row.id)).toEqual(["consent:mine"]);
  rerender({ disabled: true });
  expect(result.current.decisions).toEqual([]);
});
