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

const unattached = {
  ...legacyProposal, status: "published_unpinned", failure_code: "pin_conflict",
  published_skill_id: "skill-9", published_version: "1.0.1",
};

function respondWith(listed: Record<string, unknown>, approvals: unknown[] = []) {
  fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
    const json = (body: unknown) => new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } });
    if (init?.method === "POST") return json(approvals.shift());
    if (url === `${base}/proposals`) return json({ proposals: [listed] });
    return json({ enabled: true });
  });
}

function show() {
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })}>
    <AgentLearningReview agentId="agent" />
  </QueryClientProvider>);
}

it("explains the two steps first, then finishes an unattached publication through approve", async () => {
  respondWith(unattached, [
    { status: "confirmation_required", acknowledgement: { acknowledgement_id: "card-9" } },
    { status: "pinned" },
  ]);
  show();
  expect(await screen.findByText("Published version: 1.0.1")).toBeVisible();
  expect(screen.getByText(/Publishing takes two steps: this button first asks for an owner confirmation card/)).toBeVisible();
  // A publication that reached Ornn cannot be rejected or published again.
  expect(screen.queryByRole("button", { name: "Reject" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Publish & request confirmation" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Finish publishing" }));
  await waitFor(() => expect(posted(`${base}/proposals/proposal/approve`).body).toBe(JSON.stringify({})));
  fireEvent.click(await screen.findByRole("button", { name: "Confirm finish publishing" }));
  expect(screen.getByText(/Step 2 of 2: Confirming this exact card publishes the private skill/)).toBeVisible();
  await waitFor(() => expect(fetchMock.mock.calls.filter(([url, init]) => url === `${base}/proposals/proposal/approve` && (init as RequestInit).method === "POST")).toHaveLength(2));
  const confirm = fetchMock.mock.calls.filter(([url]) => url === `${base}/proposals/proposal/approve`)[1]![1] as RequestInit;
  expect(confirm.body).toBe(JSON.stringify({ acknowledgement_id: "card-9" }));
});

it("offers Finish publishing for an interrupted publishing row", async () => {
  respondWith({ ...legacyProposal, status: "publishing", failure_code: null });
  show();
  expect(await screen.findByRole("button", { name: "Finish publishing" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Reject" })).not.toBeInTheDocument();
});

it("offers no recovery for a settled unattached publication", async () => {
  respondWith({ ...unattached, failure_code: "base_changed" });
  show();
  expect(await screen.findByText(/did not attach it because the source skill changed/)).toBeVisible();
  expect(screen.queryByRole("button", { name: "Finish publishing" })).not.toBeInTheDocument();
  expect(screen.queryByText(/Publishing takes two steps/)).not.toBeInTheDocument();
});
