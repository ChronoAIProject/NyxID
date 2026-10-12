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
  const draft = {
    schema_version: 1, kind: "new", name: "updated", description: "Updated", skill_md: "# Updated",
    files: [], base_skill: null, rationale: "Reusable", safety_notes: "None",
  };
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

it("posts the draft to the proposal's validate route and parses the result", async () => {
  assistantJson.mockResolvedValueOnce({ valid: true, bytes: 87 });
  const { result } = renderHook(() => useAgentLearningReview("agent", false), { wrapper });
  const draft = {
    schema_version: 1, kind: "new", name: "checked", description: "Checked", skill_md: "# Checked",
    files: [], base_skill: null, rationale: "Reusable", safety_notes: "None",
  };
  await expect(result.current.validate.mutateAsync({ proposalId: "proposal", draft })).resolves.toEqual({ valid: true, bytes: 87 });
  expect(assistantJson).toHaveBeenCalledWith(
    "/assistant/nyxagent/agents/agent/learning/proposals/proposal/validate",
    { method: "POST", body: { draft } },
  );
});
