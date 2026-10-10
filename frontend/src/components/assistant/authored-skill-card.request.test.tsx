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

function respond(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}
const legacy = { ...preview, status: "publication_failed", failure_code: "legacy_package_requires_reprepare", actions: ["reprepare", "discard"] };

it("rebuilds a legacy package with a body encoded exactly once", async () => {
  fetchMock.mockImplementation(async (_url: string, init?: RequestInit) =>
    respond(200, init?.method === "POST" ? { status: "reprepared", acknowledgement_id: "new-card" } : legacy));
  show();
  expect(await screen.findByText(/older NyxID version that can't preserve the skill's tools\/runtimes/)).toBeVisible();
  expect(screen.queryByRole("button", { name: "Request a new confirmation" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Review updated package" }));
  await waitFor(() => expect(posted(`${proposalPath}/reprepare`)).toBeDefined());
  expect(posted(`${proposalPath}/reprepare`).body).toBe(JSON.stringify({ acknowledgement_id: "card" }));
  expect(await screen.findByText(/package was rebuilt from the source skill/)).toBeVisible();
  // The card re-reads its state after the rebuild.
  await waitFor(() => expect(fetchMock.mock.calls.filter(([, init]) => (init as RequestInit | undefined)?.method !== "POST").length).toBeGreaterThan(1));
});

it("raises the rebuilt package's card from an older card", async () => {
  let rebuilt = false;
  fetchMock.mockImplementation(async (_url: string, init?: RequestInit) => {
    if (init?.method === "POST") {
      rebuilt = true;
      return respond(200, { status: "reprepared", acknowledgement_id: "new-card" });
    }
    return respond(200, { ...preview, state: "changed", actions: rebuilt ? [] : ["show_updated_draft"] });
  });
  show();
  expect(await screen.findByText("This confirmation no longer matches the draft.")).toBeVisible();
  expect(screen.getByText(/rebuilt package for this draft is waiting for its review card/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Show updated draft" }));
  await waitFor(() => expect(posted(`${proposalPath}/reprepare`)).toBeDefined());
  expect(posted(`${proposalPath}/reprepare`).body).toBe(JSON.stringify({ acknowledgement_id: "card" }));
  expect(await screen.findByText(/Review the new confirmation posted in this conversation/)).toBeVisible();
  await waitFor(() => expect(screen.queryByRole("button", { name: "Show updated draft" })).not.toBeInTheDocument());
});

it("keeps the rebuild visible when the server refuses it", async () => {
  fetchMock.mockImplementation(async (_url: string, init?: RequestInit) =>
    init?.method === "POST"
      ? respond(409, { error: "conflict", error_code: 1004, message: "base_skill_changed" })
      : respond(200, legacy));
  show();
  fireEvent.click(await screen.findByRole("button", { name: "Review updated package" }));
  expect(await screen.findByText("The package could not be rebuilt. The current state is shown below.")).toBeVisible();
});

it("points to Show updated draft when the rebuilt package's card was not posted", async () => {
  let rebuilt = false;
  fetchMock.mockImplementation(async (_url: string, init?: RequestInit) => {
    if (init?.method === "POST") {
      rebuilt = true;
      return respond(200, { status: "reprepared", card_pending: true, proposal_id: "proposal" });
    }
    return respond(200, rebuilt ? { ...preview, state: "changed", actions: ["show_updated_draft"] } : legacy);
  });
  show();
  fireEvent.click(await screen.findByRole("button", { name: "Review updated package" }));
  expect(await screen.findByText(/review card was not posted\. Use Show updated draft to post it\./)).toBeVisible();
  expect(await screen.findByRole("button", { name: "Show updated draft" })).toBeVisible();
  expect(screen.queryByText(/could not be rebuilt/)).not.toBeInTheDocument();
});
