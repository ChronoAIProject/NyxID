import type { ComponentProps } from "react";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentSkills } from "./agent-skills";
import { useAuthStore } from "@/stores/auth-store";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type { User } from "@/types/api";

// The real hooks, schemas and request path (assistantJson -> fetch -> ApiError);
// only fetch, routing and the signed-in user are stubbed.
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, search, ...props }: ComponentProps<"a"> & { to: string; search?: Record<string, string> }) =>
    <a href={to + (search ? `?${new URLSearchParams(search)}` : "")} {...props} />,
}));

const api = "/api/v1/assistant/nyxagent/agents/agent";
const pin = { source: "ornn", skill_id: "skill-id", name: "research", version: "1.0.0", sha256: "a".repeat(64), dependencies: [] };
const authored = {
  source: "authored", id: "draft-1", agent_id: "agent", status: "pending", revision: 0, config_revision: 0,
  agent_skills_revision: 4, evidence_count: 0, body_bytes: 120, created_at: "2026-10-01T00:00:00Z",
  updated_at: "2026-10-01T00:00:00Z", card_conversation_id: "home-thread",
  draft: {
    schema_version: 1, kind: "improve", name: "research", description: "Research guide", skill_md: "# Research",
    files: [], rationale: "Owner edit", safety_notes: "None",
    base_skill: { source: "ornn", skill_id: "skill-id", name: "research", version: "1.0.0", sha256: "a".repeat(64) },
  },
};
type Reply = { status: number; body: unknown };
let replies: Record<string, Reply[]>;
let proposals: unknown[];
let fetchMock: ReturnType<typeof vi.fn>;

const json = ({ status, body }: Reply) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
const failure = (status: number, error: string, message: string): Reply =>
  ({ status, body: { error, error_code: 1000, message } });

function signIn(features: string[]) {
  useAuthStore.setState({ user: { id: "owner", capabilities: { enabled_features: features } } as unknown as User });
}

beforeEach(() => {
  globalThis.__nyxidAssistantHttpMock = undefined;
  signIn([FEATURE_FLAG.AGENT_LEARNING]);
  replies = {};
  proposals = [];
  fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
    const key = `${init?.method ?? "GET"} ${url}`;
    const queued = replies[key]?.shift();
    if (queued) return json(queued);
    if (key === `GET ${api}/skills`) {
      return json({ status: 200, body: { agent_id: "agent", revision: 4, skills: [pin], metadata: { "skill-id": { description: "Research guide", size_bytes: 1000 } } } });
    }
    if (key === `GET ${api}/skills/history`) return json({ status: 200, body: { items: [] } });
    if (key === `POST ${api}/skills/skill-id/improve`) {
      proposals = [authored];
      return json({ status: 201, body: { proposal_id: "draft-1", acknowledgement_id: "card-1" } });
    }
    if (key === `GET ${api}/learning/proposals`) return json({ status: 200, body: { proposals } });
    if (key === `PUT ${api}/learning/proposals/draft-1`) return json({ status: 200, body: { status: "updated" } });
    if (url.includes("/skills/catalog")) return json({ status: 200, body: { page: 1, total_pages: 1, items: [] } });
    return json({ status: 404, body: { error: "not_found", error_code: 1004, message: "Not found" } });
  });
  vi.stubGlobal("fetch", fetchMock);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  useAuthStore.setState({ user: null });
});

function show() {
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false } } })}>
    <AgentSkills agentId="agent" />
  </QueryClientProvider>);
}

function calls(method: string, url: string) {
  return fetchMock.mock.calls.filter(([u, init]) => u === url && ((init as RequestInit | undefined)?.method ?? "GET") === method);
}

it("tests a pin only on request and shows what the agent reads", async () => {
  const user = userEvent.setup();
  replies[`GET ${api}/skills/skill-id/preview`] = [{
    status: 200,
    body: { skill_id: "skill-id", name: "research", version: "1.0.0", sha256: "a".repeat(64), disabled: false, description: "Research guide", size_bytes: 1000, readable: true, excerpt: "# Research\nStep one" },
  }];
  show();
  await screen.findByText("Research guide");
  expect(calls("GET", `${api}/skills/skill-id/preview`)).toHaveLength(0);
  await user.click(screen.getByRole("button", { name: "Test research" }));
  expect(await screen.findByText("Test passed: the agent reads research 1.0.0 (1,000 bytes).")).toBeVisible();
  expect(screen.getByLabelText("research SKILL.md excerpt")).toHaveTextContent("# Research Step one");
  expect(calls("GET", `${api}/skills/skill-id/preview`)).toHaveLength(1);
});

it("reports a disabled pin and a failed test", async () => {
  const user = userEvent.setup();
  replies[`GET ${api}/skills/skill-id/preview`] = [
    { status: 200, body: { skill_id: "skill-id", name: "research", version: "1.0.0", sha256: "a".repeat(64), disabled: true, description: "Research guide", size_bytes: 1000, readable: false, excerpt: "" } },
    failure(409, "conflict", "Ornn now serves different content for this pinned version; reattach the skill"),
  ];
  show();
  await user.click(await screen.findByRole("button", { name: "Test research" }));
  expect(await screen.findByText("research 1.0.0 is disabled, so the agent does not read it. Enable it to use it again.")).toBeVisible();
  // A second click on the same pin runs the test again.
  await user.click(screen.getByRole("button", { name: "Test research" }));
  expect(await screen.findByText("Test failed: Ornn now serves different content for this pinned version; reattach the skill")).toBeVisible();
  expect(calls("GET", `${api}/skills/skill-id/preview`)).toHaveLength(2);
});

it("labels each history event and shows per-pin disabled state", async () => {
  replies[`GET ${api}/skills/history`] = [{
    status: 200,
    body: { items: [
      { id: "h2", event_type: "assistant_skills_updated", created_at: "2026-10-02T00:00:00Z", revision: 5, proposal_id: null, skills: [{ skill_id: "skill-id", name: "research", version: "1.0.0", disabled: true }] },
      { id: "h1", event_type: "assistant_learning_proposal_edited", created_at: "2026-10-01T00:00:00Z", revision: null, proposal_id: "draft-1", skills: [] },
    ] },
  }];
  show();
  const history = await screen.findByRole("list", { name: "Skill history" });
  const rows = within(history).getAllByRole("listitem");
  expect(rows).toHaveLength(2);
  expect(rows[0]).toHaveTextContent("Attached skills changed");
  expect(rows[0]).toHaveTextContent("Revision 5");
  expect(rows[0]).toHaveTextContent("research 1.0.0 (disabled)");
  expect(rows[1]).toHaveTextContent("Skill draft edited");
  expect(rows[1]).not.toHaveTextContent("Revision");
});

it("edits an attached skill through an authored improve draft", async () => {
  const user = userEvent.setup();
  show();
  await user.click(await screen.findByRole("button", { name: "Edit research" }));
  await waitFor(() => expect(calls("POST", `${api}/skills/skill-id/improve`)).toHaveLength(1));
  expect(calls("POST", `${api}/skills/skill-id/improve`)[0]![1]).toMatchObject({ body: "{}" });
  const form = await screen.findByRole("form", { name: "Edit skill draft" });
  expect(screen.getByText(/publish the draft from its confirmation card in NyxBot's home thread/)).toBeVisible();
  expect(screen.getByRole("link", { name: "Open the confirmation card" })).toHaveAttribute("href", "/assistant?c=home-thread");
  const markdown = within(form).getByRole("textbox", { name: "Skill draft" });
  await user.type(markdown, "\nStep two");
  await user.click(within(form).getByRole("button", { name: "Save draft" }));
  await waitFor(() => expect(calls("PUT", `${api}/learning/proposals/draft-1`)).toHaveLength(1));
  expect(JSON.parse(String((calls("PUT", `${api}/learning/proposals/draft-1`)[0]![1] as RequestInit).body))).toEqual({
    revision: 0,
    draft: { ...authored.draft, skill_md: "# Research\nStep two" },
  });
});

it("shows why an attached skill cannot be edited", async () => {
  const user = userEvent.setup();
  replies[`POST ${api}/skills/skill-id/improve`] = [failure(409, "conflict", "skill_files_unsupported: this skill has files that cannot be edited here")];
  show();
  await user.click(await screen.findByRole("button", { name: "Edit research" }));
  expect(await screen.findByText("skill_files_unsupported: this skill has files that cannot be edited here")).toBeVisible();
  expect(screen.queryByRole("form", { name: "Edit skill draft" })).not.toBeInTheDocument();
});

it("hides Edit while agent learning is off", async () => {
  signIn([]);
  show();
  await screen.findByText("Research guide");
  expect(screen.queryByRole("button", { name: "Edit research" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Test research" })).toBeVisible();
});

it("surfaces a refused pin change", async () => {
  const user = userEvent.setup();
  signIn([FEATURE_FLAG.SKILL_PIN_DISABLE]);
  replies[`PUT ${api}/skills`] = [failure(403, "forbidden", "Disabling attached skills is not enabled")];
  show();
  await user.click(await screen.findByRole("switch", { name: "Use research" }));
  expect(await screen.findByText("Disabling attached skills is not enabled")).toBeVisible();
  expect(JSON.parse(String((calls("PUT", `${api}/skills`)[0]![1] as RequestInit).body))).toEqual({
    expected_revision: 4,
    skills: [{ ...pin, disabled: true }],
  });
});
