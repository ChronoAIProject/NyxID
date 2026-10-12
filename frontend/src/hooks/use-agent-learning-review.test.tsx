import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { useAgentLearningReview } from "./use-agent-learning-review";

const { assistantJson } = vi.hoisted(() => ({ assistantJson: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantJson }));

function wrapper({ children }: PropsWithChildren) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

beforeEach(() => {
  assistantJson.mockReset().mockResolvedValue({ status: "rejected" });
});

it("rejects the proposal at its current revision", async () => {
  const { result } = renderHook(() => useAgentLearningReview("agent", false), { wrapper });
  await result.current.reject.mutateAsync({ proposalId: "proposal", revision: 3 });
  expect(assistantJson).toHaveBeenCalledWith(
    "/assistant/nyxagent/agents/agent/learning/proposals/proposal/reject",
    { method: "POST", body: { revision: 3 } },
  );
});


it("sends the revision and draft when editing a proposal", async () => {
  const { result } = renderHook(() => useAgentLearningReview("agent", false), { wrapper });
  const draft = { name: "Updated" };
  await result.current.edit.mutateAsync({ proposalId: "proposal", revision: 4, draft });
  expect(assistantJson).toHaveBeenCalledWith(
    "/assistant/nyxagent/agents/agent/learning/proposals/proposal",
    { method: "PUT", body: { revision: 4, draft } },
  );
});

it("sends the enabled consent body", async () => {
  const { result } = renderHook(() => useAgentLearningReview("agent", false), { wrapper });
  await result.current.consent.mutateAsync(true);
  expect(assistantJson).toHaveBeenCalledWith(
    "/assistant/nyxagent/agents/agent/learning/consent",
    { method: "PUT", body: { enabled: true, threshold: 1 } },
  );
});
