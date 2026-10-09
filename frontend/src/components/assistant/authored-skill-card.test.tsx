import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NyxAgentAcknowledgementCard } from "./nyxagent-acknowledgement-card";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";
import { authoredSkillPreviewSchema } from "@/schemas/agent-skills";
const mocks = vi.hoisted(() => ({ json: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantJson: mocks.json }));
vi.mock("@/stores/auth-store", () => ({ useAuthStore: (select: (s: unknown) => unknown) => select({ user: { id: "owner" } }) }));
const card: NyxAgentAcknowledgement = {
  id: "card", kind: "action", status: "pending", summary: "Review skill draft", decider: "user", decided_by: null,
  reason: null, service_slug: null, service_name: null, tool_name: "nyxid__approve_agent_learning",
  created_at: "2026-10-06T00:00:00Z", decided_at: null, expires_at: "2026-10-07T00:00:00Z",
  authored_skill: { agent_id: "agent", proposal_id: "proposal", revision: 0, skills_revision: 3 },
};
const preview = {
  id: "proposal", agent_id: "agent", agent_name: "Operations", revision: 0, skills_revision: 3, current_skills_revision: 3,
  status: "pending", name: "weekly-review", version: "1.0",
  state: "active", actions: ["deny", "publish"], lease_live: false,
  files: [{ path: "SKILL.md", content: "---\nname: weekly-review\n---\n# Review every Friday" }, { path: "references/checklist.md", content: "Verify every date" }],
};
function show(value = card, decide = vi.fn().mockResolvedValue(undefined)) {
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })}>
    <NyxAgentAcknowledgementCard acknowledgement={value} deciding={false} onDecision={decide} />
  </QueryClientProvider>);
  return decide;
}
beforeEach(() => { mocks.json.mockReset().mockResolvedValue(preview); });
afterEach(cleanup);
it("shows every file and publishes only after one explicit owner click", async () => {
  const decide = show();
  expect(await screen.findByText("Verify every date")).toBeVisible();
  expect(screen.getByText(/# Review every Friday/)).toBeVisible();
  expect(screen.getByText("references/checklist.md")).toBeVisible();
  expect(screen.getByText("Attach to Operations")).toBeVisible();
  expect(decide).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Allow and publish" }));
  await waitFor(() => expect(decide).toHaveBeenCalledExactlyOnceWith("allow"));
  expect(mocks.json).toHaveBeenCalledWith(expect.stringContaining("acknowledgement_id=card"));
});
it("renders and clicks a renewed pending card at the live skills revision", async () => {
  mocks.json.mockResolvedValue({ ...preview, skills_revision: 3, current_skills_revision: 4, actions: ["deny", "publish"] });
  const renewed = { ...card, id: "renewed-card", authored_skill: { ...card.authored_skill!, skills_revision: 4 } };
  const decide = show(renewed);
  fireEvent.click(await screen.findByRole("button", { name: "Allow and publish" }));
  await waitFor(() => expect(decide).toHaveBeenCalledExactlyOnceWith("allow"));
  expect(mocks.json).toHaveBeenCalledWith(expect.stringContaining("acknowledgement_id=renewed-card"));
});
it("dismisses an older pending card after a renewed publication starts", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publishing", actions: ["dismiss_card"] });
  const decide = show();
  fireEvent.click(await screen.findByRole("button", { name: "Dismiss confirmation" }));
  await waitFor(() => expect(decide).toHaveBeenCalledExactlyOnceWith("deny"));
  expect(screen.queryByText("Skill draft discarded")).not.toBeInTheDocument();
});
it("denies a pending card through the acknowledgement decision", async () => {
  const decide = show();
  await screen.findByText("Verify every date");
  fireEvent.click(screen.getByRole("button", { name: "Deny" }));
  await waitFor(() => expect(decide).toHaveBeenCalledExactlyOnceWith("deny"));
  expect(mocks.json).not.toHaveBeenCalledWith(expect.stringContaining("/reject"), expect.anything());
});
it("keeps Deny available after an unrelated skills revision", async () => {
  mocks.json.mockResolvedValue({ ...preview, current_skills_revision: 4, actions: ["deny", "renew"] });
  const decide = show();
  fireEvent.click(await screen.findByRole("button", { name: "Deny" }));
  await waitFor(() => expect(decide).toHaveBeenCalledExactlyOnceWith("deny"));
  expect(screen.queryByRole("button", { name: "Allow and publish" })).not.toBeInTheDocument();
});
it("offers a fresh confirmation for a verified pin race at the live skills revision", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "published_unpinned", actions: ["renew"], current_skills_revision: 4, failure_code: "pin_conflict" });
  show({ ...card, status: "used" });
  fireEvent.click(await screen.findByRole("button", { name: "Request a new confirmation" }));
  await waitFor(() => expect(mocks.json).toHaveBeenCalledWith(expect.stringContaining("/approve"), expect.objectContaining({
    body: expect.objectContaining({ agent_skills_revision: 4 }),
  })));
  expect(screen.queryByRole("button", { name: "Check again" })).not.toBeInTheDocument();
});
it("discards a non-effective used card through the proposal endpoint", async () => {
  mocks.json.mockResolvedValue({ ...preview, actions: ["discard"] });
  show({ ...card, status: "used" });
  await screen.findByText("Verify every date");
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  await waitFor(() => expect(mocks.json).toHaveBeenCalledWith(expect.stringContaining("/reject"), expect.objectContaining({ method: "POST" })));
});
it("retries an allowed card while the proposal is still pending", async () => {
  mocks.json.mockResolvedValue({ ...preview, actions: ["retry", "discard"] });
  const decide = show({ ...card, status: "allowed" });
  fireEvent.click(await screen.findByRole("button", { name: "Retry publication" }));
  await waitFor(() => expect(decide).toHaveBeenCalledExactlyOnceWith("allow"));
});
it.each(["changed", "unavailable"])("blocks approval when content is %s", async (state) => {
  if (state === "changed") mocks.json.mockResolvedValue({ ...preview, state: "changed", actions: [] });
  else mocks.json.mockRejectedValue(new Error("Unavailable"));
  show();
  if (state === "changed") await screen.findByText("This confirmation no longer matches the draft.");
  else await screen.findByRole("alert");
  expect(screen.queryByRole("button", { name: "Allow and publish" })).not.toBeInTheDocument();
});
it.each([["publication_failed", "retry", "Retry publication"], ["publication_failed", "check", "Check again"]])("offers %s recovery with %s", async (status, action, label) => {
  mocks.json.mockResolvedValue({ ...preview, status, actions: [action] });
  const decide = show({ ...card, status: "used" });
  await screen.findByText("Verify every date");
  fireEvent.click(screen.getByRole("button", { name: label }));
  await waitFor(() => expect(decide).toHaveBeenCalledExactlyOnceWith("allow"));
});
it("shows a denied card as dismissed while the proposal remains active", async () => {
  mocks.json.mockResolvedValue({ ...preview, state: "dismissed", actions: [] });
  const decide = show({ ...card, status: "denied" });
  expect(await screen.findByText(/Confirmation dismissed; the publication remains available/)).toBeVisible();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(decide).not.toHaveBeenCalled();
});
it("renews an expired card for the same draft", async () => {
  mocks.json.mockResolvedValue({ ...preview, actions: ["renew", "discard"] });
  show({ ...card, status: "expired" });
  await screen.findByText("Verify every date");
  fireEvent.click(screen.getByRole("button", { name: "Request a new confirmation" }));
  await waitFor(() => expect(mocks.json).toHaveBeenCalledWith(expect.stringContaining("/approve"), expect.objectContaining({ method: "POST", body: expect.objectContaining({ renewal_of: "card" }) })));
  expect(await screen.findByText("A new confirmation was posted in this conversation.")).toBeVisible();
});
it("renews an expired card for read-only reconciliation", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publication_failed", actions: ["renew"], failure_code: "approval_expired" });
  show({ ...card, status: "expired" });
  fireEvent.click(await screen.findByRole("button", { name: "Request a new confirmation" }));
  await waitFor(() => expect(mocks.json).toHaveBeenCalledWith(expect.stringContaining("/approve"), expect.objectContaining({ method: "POST" })));
});
it.each(["rejected", "invalidated"])('renders %s as terminal rather than a failure', async (status) => {
  mocks.json.mockResolvedValue({ ...preview, status, state: status === "rejected" ? "discarded" : "invalidated", actions: [], failure_code: "evidence_unavailable", files: [] });
  show({ ...card, status: "allowed" });
  expect(await screen.findByText(status === "rejected" ? "Skill draft discarded" : "Skill draft is no longer available")).toBeVisible();
  expect(screen.queryByText(/Publication could not finish/)).not.toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});
it("directs a conclusive package refusal to discard and request a revision", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publication_failed", actions: ["discard"], failure_code: "ornn_validation_failed" });
  show({ ...card, status: "used" });
  expect(await screen.findByText(/Deny or discard this draft and ask NyxBot for a revised one/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Discard" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Retry publication" })).not.toBeInTheDocument();
});
it("shows a sanitized unresolved-publication message and prevents discard", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publication_failed", actions: ["check"], failure_code: "publish_uncertain" });
  show({ ...card, status: "used" });
  expect(await screen.findByText(/cannot yet confirm whether Ornn published/)).toBeVisible();
  expect(screen.queryByRole("button", { name: "Discard" })).not.toBeInTheDocument();
});
it("keeps a conflicting uncertain version available for read-only checks", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publication_failed", actions: ["check"], failure_code: "version_conflict" });
  show({ ...card, status: "used" });
  expect(await screen.findByText(/Ask NyxBot for a new draft after this publication is resolved/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Check again" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Discard" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Retry publication" })).not.toBeInTheDocument();
});
it("shows the settled publication result", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "pinned", state: "pinned", actions: [], files: [] });
  show({ ...card, status: "used" });
  expect(await screen.findByText("Skill published and attached")).toBeVisible();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});
it("shows an active publication without offering another check", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publishing", actions: [], lease_live: true });
  show({ ...card, status: "used" });
  expect(await screen.findByText("Publication is in progress.")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Check again" })).not.toBeInTheDocument();
  expect(screen.queryByText(/The last attempt was interrupted/)).not.toBeInTheDocument();
});
it("does not claim an interrupted attempt is still in progress", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publishing", actions: ["check"], lease_live: false });
  show({ ...card, status: "used" });
  expect(await screen.findByText(/The last attempt was interrupted\. NyxID cannot yet confirm whether Ornn published this version; Check again only reads/)).toBeVisible();
  expect(screen.queryByText("Publication is in progress.")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Check again" })).toBeVisible();
});
it("says nothing was sent when an interrupted attempt can still be retried", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publishing", actions: ["retry", "discard"], lease_live: false });
  show({ ...card, status: "used" });
  expect(await screen.findByText("The last attempt was interrupted before anything was sent to Ornn.")).toBeVisible();
  expect(screen.queryByText("Publication is in progress.")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Retry publication" })).toBeVisible();
});
it.each([
  ["an expired approving card", { ...card, status: "used" }, ["discard", "renew"], "Discard"],
  ["an older pending card", card, ["deny", "renew"], "Deny"],
] as const)("says nothing was sent for a never-dispatched interrupted attempt on %s", async (_name, value, actions, button) => {
  mocks.json.mockResolvedValue({ ...preview, status: "publishing", actions, lease_live: false });
  show(value);
  expect(await screen.findByText("The last attempt was interrupted before anything was sent to Ornn.")).toBeVisible();
  expect(screen.queryByText(/cannot yet confirm whether Ornn published/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: button })).toBeVisible();
  expect(screen.getByRole("button", { name: "Request a new confirmation" })).toBeVisible();
});
it("keeps an unavailable draft and a required revision visible without publishing", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publication_failed", actions: [], files: [], failure_code: "draft_unavailable" });
  show({ ...card, status: "used" });
  expect(await screen.findByText(/draft files are unavailable/i)).toBeVisible();
  expect(screen.queryByRole("button", { name: "Retry publication" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Check again" })).not.toBeInTheDocument();
});
it("refetches and retains a failed retry state after the decision request fails", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publication_failed", actions: ["retry"], failure_code: "ornn_write_forbidden" });
  const decide = vi.fn().mockRejectedValue(new Error("refused"));
  show({ ...card, status: "used" }, decide);
  fireEvent.click(await screen.findByRole("button", { name: "Retry publication" }));
  await waitFor(() => expect(decide).toHaveBeenCalledExactlyOnceWith("allow"));
  expect(await screen.findByText(/Publication did not finish/)).toBeVisible();
  expect(screen.getByText(/Ornn update permission or skill write access denied/)).toBeVisible();
  expect(mocks.json.mock.calls.length).toBeGreaterThan(1);
});

it("accepts a preview from a backend without actions or state", () => {
  const legacy = { ...preview } as Record<string, unknown>;
  delete legacy.actions;
  delete legacy.state;
  const parsed = authoredSkillPreviewSchema.parse(legacy);
  expect(parsed.actions).toEqual([]);
  expect(parsed.state).toBe("active");
});

it("explains a computed target_busy block on a live pending card", async () => {
  mocks.json.mockResolvedValue({ ...preview, actions: ["deny"], failure_code: "target_busy" });
  show();
  expect(await screen.findByText(/Another draft is publishing this skill version/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Deny" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Allow and publish" })).not.toBeInTheDocument();
});

it("ignores actions it does not know and keeps the known ones", async () => {
  mocks.json.mockResolvedValue({ ...preview, actions: ["future_action", "deny"] });
  show();
  expect(await screen.findByRole("button", { name: "Deny" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Allow and publish" })).not.toBeInTheDocument();
});

it("offers a write retry after an operator released an uncertain attempt", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publication_failed", actions: ["retry", "discard"], failure_code: "operator_released" });
  const decide = show({ ...card, status: "used" });
  expect(await screen.findByText(/operator confirmed the earlier attempt had no effect/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Retry publication" }));
  await waitFor(() => expect(decide).toHaveBeenCalledExactlyOnceWith("allow"));
});
