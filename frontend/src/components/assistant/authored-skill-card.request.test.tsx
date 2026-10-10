import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AuthoredSkillCard } from "./authored-skill-card";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

// Exercises the real request path (assistantJson -> fetch); only fetch is stubbed.
vi.mock("@/stores/auth-store", () => ({ useAuthStore: (select: (s: unknown) => unknown) => select({ user: { id: "owner" } }) }));

const card: NyxAgentAcknowledgement = {
  id: "card", kind: "action", status: "pending", summary: "Review skill draft", decider: "user", decided_by: null,
  reason: null, service_slug: null, service_name: null, tool_name: "nyxid__approve_agent_learning",
  created_at: "2026-10-06T00:00:00Z", decided_at: null, expires_at: "2026-10-07T00:00:00Z",
  authored_skill: { agent_id: "agent", proposal_id: "proposal", revision: 4, skills_revision: 3 },
};
const preview = {
  id: "proposal", agent_id: "agent", agent_name: "Operations", revision: 4, skills_revision: 3, current_skills_revision: 5,
  status: "publication_failed", name: "weekly-review", version: "1.1", state: "active", actions: ["discard", "renew"],
  failure_code: "approval_expired", lease_live: false, files: [{ path: "SKILL.md", content: "# Weekly review" }],
};
const proposalPath = "/api/v1/assistant/nyxagent/agents/agent/learning/proposals/proposal";
let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  globalThis.__nyxidAssistantHttpMock = undefined;
  fetchMock = vi.fn(async (_url: string, init?: RequestInit) => new Response(
    JSON.stringify(init?.method === "POST" ? { status: "ok" } : preview),
    { status: 200, headers: { "Content-Type": "application/json" } },
  ));
  vi.stubGlobal("fetch", fetchMock);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function show() {
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })}>
    <AuthoredSkillCard acknowledgement={card} deciding={false} onDecision={vi.fn()} />
  </QueryClientProvider>);
}

function posted(path: string): RequestInit {
  const call = fetchMock.mock.calls.find(([url, init]) => url === path && (init as RequestInit | undefined)?.method === "POST");
  expect(call).toBeDefined();
  return call![1] as RequestInit;
}

it("renews with a body encoded exactly once", async () => {
  show();
  fireEvent.click(await screen.findByRole("button", { name: "Request a new confirmation" }));
  await waitFor(() => expect(posted(`${proposalPath}/approve`)).toBeDefined());
  const body = posted(`${proposalPath}/approve`).body;
  expect(body).toBe(JSON.stringify({ revision: 4, agent_skills_revision: 5, renewal_of: "card" }));
  expect(JSON.parse(body as string)).toEqual({ revision: 4, agent_skills_revision: 5, renewal_of: "card" });
  expect(await screen.findByText("A new confirmation was posted in this conversation.")).toBeVisible();
});

it("discards with a body encoded exactly once", async () => {
  show();
  fireEvent.click(await screen.findByRole("button", { name: "Discard" }));
  await waitFor(() => expect(posted(`${proposalPath}/reject`)).toBeDefined());
  const body = posted(`${proposalPath}/reject`).body;
  expect(body).toBe(JSON.stringify({ revision: 4, reason: "rejected" }));
  expect(JSON.parse(body as string)).toEqual({ revision: 4, reason: "rejected" });
});
