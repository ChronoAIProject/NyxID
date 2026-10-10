import type { ComponentProps } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentLearningReview } from "./agent-learning-review";

// Exercises the real hook and request path (assistantJson -> fetch); only
// fetch, the feature flag, routing and the signed-in user are stubbed.
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => true }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, search, ...props }: ComponentProps<"a"> & { to: string; search?: Record<string, string> }) =>
    <a href={to + (search ? `?${new URLSearchParams(search)}` : "")} {...props} />,
}));
vi.mock("@/stores/auth-store", () => ({ useAuthStore: (select: (s: unknown) => unknown) => select({ user: { id: "owner" } }) }));

const base = "/api/v1/assistant/nyxagent/agents/agent/learning";
const legacyProposal = {
  source: "learned", id: "proposal", agent_id: "agent", status: "publication_failed", revision: 2, config_revision: 1,
  agent_skills_revision: 3, evidence_count: 2, body_bytes: 42, created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z", failure_code: "legacy_package_requires_reprepare",
  draft: { schema_version: 1, kind: "improve", name: "Weekly review", description: "A reusable procedure",
    skill_md: "# Review", rationale: "Reusable", safety_notes: "Review before use", files: [] },
};
let fetchMock: ReturnType<typeof vi.fn>;
let rebuilt: boolean;

beforeEach(() => {
  globalThis.__nyxidAssistantHttpMock = undefined;
  rebuilt = false;
  fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
    const json = (body: unknown) => new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } });
    if (init?.method === "POST") {
      rebuilt = true;
      return json({ status: "reprepared", proposal_id: "proposal" });
    }
    if (url === `${base}/proposals`) {
      return json({ proposals: [rebuilt ? { ...legacyProposal, status: "pending", failure_code: null, revision: 3 } : legacyProposal] });
    }
    return json({ enabled: true });
  });
  vi.stubGlobal("fetch", fetchMock);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function posted(path: string): RequestInit {
  const call = fetchMock.mock.calls.find(([url, init]) => url === path && (init as RequestInit | undefined)?.method === "POST");
  expect(call).toBeDefined();
  return call![1] as RequestInit;
}

it("rebuilds a legacy learned package from the panel instead of publishing it", async () => {
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })}>
    <AgentLearningReview agentId="agent" />
  </QueryClientProvider>);
  expect(await screen.findByText(/older NyxID version that can't preserve the skill's tools\/runtimes/)).toBeVisible();
  expect(screen.queryByRole("button", { name: "Publish & request confirmation" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Review updated package" }));
  await waitFor(() => expect(posted(`${base}/proposals/proposal/reprepare`)).toBeDefined());
  expect(posted(`${base}/proposals/proposal/reprepare`).body).toBe(JSON.stringify({}));
  // The rebuilt draft publishes through the ordinary two-step confirmation.
  expect(await screen.findByRole("button", { name: "Publish & request confirmation" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Review updated package" })).not.toBeInTheDocument();
});
