import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AuthoredSkillScopePreview } from "./authored-skill-scope-preview";
import { NyxAgentAcknowledgementCard } from "./nyxagent-acknowledgement-card";
import type { NyxAgentAcknowledgement, NyxAgentAccessMode } from "@/schemas/assistant-nyxagent";
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
function show(value = card, decide = vi.fn().mockResolvedValue(undefined), accessMode?: NyxAgentAccessMode) {
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })}>
    <NyxAgentAcknowledgementCard acknowledgement={value} accessMode={accessMode} deciding={false} onDecision={decide} />
  </QueryClientProvider>);
  return decide;
}
beforeEach(() => { mocks.json.mockReset().mockResolvedValue(preview); });
afterEach(cleanup);
it.each(["autopilot", "nyxmode", "ask_every_time"] as const)("keeps an expired authored card free of confirmation controls only in %s", async (accessMode) => {
  mocks.json.mockResolvedValue({ ...preview, actions: ["renew", "discard"] });
  const decide = show({ ...card, status: "expired" }, undefined, accessMode);
  await waitFor(() => expect(mocks.json).toHaveBeenCalled());
  if (accessMode === "autopilot") {
    expect(screen.getByRole("status")).toHaveTextContent("Expired · Review skill draft");
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Review skill draft" })).not.toBeInTheDocument();
  } else {
    expect(await screen.findByRole("button", { name: "Request a new confirmation" })).toBeVisible();
    expect(screen.getByText("Verify every date")).toBeVisible();
  }
  expect(decide).not.toHaveBeenCalled();
});
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
  if (state === "changed") await screen.findByText(/This confirmation no longer matches the draft/);
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
  expect(await screen.findByText(status === "rejected" ? /Skill draft discarded/ : /Skill draft is no longer available/)).toBeVisible();
  expect(screen.queryByText(/Publication could not finish/)).not.toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});
it("reports that the drafting agent was asked for a corrected package after a refusal", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publication_failed", actions: ["discard"], failure_code: "ornn_validation_failed" });
  show({ ...card, status: "used" });
  expect(await screen.findByText(/The agent that drafted it was asked for a corrected draft/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Discard" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Retry publication" })).not.toBeInTheDocument();
});
it("shows unavailable format details only when the backend marks them unavailable", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publication_failed", actions: ["discard"], failure_code: "ornn_validation_failed", failure_details_unavailable: true });
  show({ ...card, status: "used" });
  expect(await screen.findByText("Format details unavailable.")).toBeVisible();
});
it("does not call base or registry failures format-detail failures", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "publication_failed", actions: ["discard"], failure_code: "base_changed", failure_details_unavailable: false });
  show({ ...card, status: "used" });
  await screen.findByText(/The source skill changed/);
  expect(screen.queryByText("Format details unavailable.")).not.toBeInTheDocument();
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
  expect(await screen.findByText(/Skill weekly-review 1.0 published and attached to Operations/)).toBeVisible();
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

it("shows plain omissions and observed coverage and requires reduced-scope consent", async () => {
  mocks.json.mockResolvedValue({ ...preview, scope: { status: "reduced", omissions: ["<script>untrusted omission</script>"] }, coverage: [
    { id: "upload", name: "source.txt", reads: 5, percent: 100, lower_bound: false, expired: false, since_reset_percent: 20 },
    { id: "partial", name: "partial.txt", reads: 513, percent: 12, lower_bound: true, expired: false },
    { id: "unread", name: "unread.txt", reads: 0, percent: 0, lower_bound: false, expired: false },
    { id: "expired", name: "expired.txt", reads: 0, percent: 0, lower_bound: false, expired: true },
  ] });
  const decide = show();
  const button = await screen.findByRole("button", { name: "Allow and publish" });
  expect(button).toBeDisabled();
  expect(screen.getByRole("checkbox", { name: /I accept this reduced scope/ })).toHaveAttribute("data-state", "unchecked");
  expect(screen.getByText("<script>untrusted omission</script>")).toBeVisible();
  expect(document.querySelector("script")).toBeNull();
  expect(screen.getByText(/source.txt: read completely; read 20% since/)).toBeVisible();
  expect(screen.getByText("partial.txt: at least 12%")).toBeVisible();
  expect(screen.getByText("unread.txt: not read")).toBeVisible();
  expect(screen.getByText("expired.txt: expired")).toBeVisible();
  fireEvent.click(screen.getByRole("checkbox"));
  expect(button).toBeEnabled();
  fireEvent.click(button);
  await waitFor(() => expect(decide).toHaveBeenCalledWith("allow", true));
});

it.each([
  ["publish", "Allow and publish"],
  ["retry", "Retry publication"],
])("blocks %s when scope is unavailable", async (action, label) => {
  mocks.json.mockResolvedValue({ ...preview, scope_unavailable: true, scope: null, actions: [action] });
  const decide = show();
  expect(await screen.findByText(/Scope unavailable/)).toBeVisible();
  const button = screen.getByRole("button", { name: label });
  expect(button).toBeDisabled();
  fireEvent.click(button);
  expect(decide).not.toHaveBeenCalled();
  expect(screen.queryByText(/Scope not declared/)).toBeNull();
});

it("explains complete scope and bounded coverage in plain language", async () => {
  mocks.json.mockResolvedValue({ ...preview, scope: { status: "complete", omissions: [] }, coverage_omitted: 11, coverage: [{ id: "upload", name: "source.txt", reads: 0, percent: 0, lower_bound: false, expired: false }] });
  show();
  expect(await screen.findByText("The drafting agent declares this draft covers everything requested. The read coverage shown here is what NyxID observed.")).toBeVisible();
  expect(screen.getByText("Observed reads in the drafting conversation")).toBeVisible();
  expect(screen.getByText("11 earlier uploads are not shown.")).toBeVisible();
});

it("keeps a draft available when an upload's legacy coverage is unknown", async () => {
  mocks.json.mockResolvedValue({ ...preview, coverage: [{ id: "upload", name: "legacy.txt", reads: 1, percent: 0, lower_bound: false, expired: false, unknown: true, since_reset_percent: null }] });
  show();
  expect(await screen.findByText("legacy.txt: read coverage unavailable")).toBeVisible();
  expect(screen.getByRole("button", { name: "Allow and publish" })).toBeEnabled();
  expect(screen.queryByText(/legacy.txt: not read/)).toBeNull();
});

it("hides leftover rules when the corrected draft has no current failure", async () => {
  mocks.json.mockResolvedValue({ ...preview, failure_code: null, failure_rules: ["allowed_root_items"], failure_details_unavailable: true });
  show();
  await screen.findByText("Verify every date");
  expect(screen.queryByText("Format details unavailable.")).not.toBeInTheDocument();
  expect(screen.queryByRole("list")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Allow and publish" })).toBeEnabled();
});

it("accepts backend nulls for scope, failure stage and optional coverage timestamps", () => {
  const parsed = authoredSkillPreviewSchema.parse({
    ...preview,
    failure_code: null,
    failure_stage: null,
    scope: null,
    coverage: [{ id: "upload", name: "source.txt", expired: false, reads: 1, percent: 50, lower_bound: false, since_reset_percent: null }],
  });
  expect(parsed.scope).toBeNull();
  expect(parsed.failure_stage).toBeNull();
  expect(parsed.coverage[0]?.since_reset_percent).toBeNull();
});

it.each([undefined, null])("shows no scope section for a scope-less draft (%s)", async (scope) => {
  mocks.json.mockResolvedValue({ ...preview, scope });
  show();
  expect(await screen.findByText("Attach to Operations")).toBeVisible();
  expect(screen.queryByText(/Scope not declared|declares this draft covers/)).toBeNull();
  expect(screen.queryByLabelText("Declared omissions")).toBeNull();
  expect(screen.queryByRole("checkbox")).toBeNull();
});

it.each([
  ["NyxBot drafting for a specialist", "nyxbot-thread", "Operations"],
  ["specialist self-drafting", "specialist-thread", "Research"],
])("attributes scope and reads to the drafting conversation: %s", async (_case, conversationId, targetName) => {
  mocks.json.mockResolvedValue({
    ...preview,
    agent_name: targetName,
    scope: { status: "complete", omissions: [] },
    coverage: [{ id: "source", name: "source.txt", reads: 1, percent: 100, lower_bound: false, expired: false }],
  });
  show({ ...card, id: `${conversationId}-card` });
  expect(await screen.findByText(`Attach to ${targetName}`)).toBeVisible();
  expect(screen.getByText(/The drafting agent declares this draft covers everything requested/)).toBeVisible();
  expect(screen.getByText("Observed reads in the drafting conversation")).toBeVisible();
  expect(screen.queryByText(`${targetName} says this draft covers everything requested.`)).toBeNull();
  expect(screen.queryByText(`What ${targetName} read`)).toBeNull();
});

it("shows Full-access omissions from the linked preview without storing them in the receipt", async () => {
  const summary = "Skill weekly-review 1.0 published and attached to Operations; Reduced scope: 1 declared omission(s); separate consent bypassed by Full access";
  const omission = "SOURCE_SENTINEL_ALPHA historical imports are omitted";
  const link = "/assistant?agent=agent&proposal=proposal";
  mocks.json.mockResolvedValue({
    ...preview,
    status: "pinned",
    scope: { status: "reduced", omissions: [omission] },
  });
  const decide = show({
    ...card,
    authored_skill: null,
    status: "allowed",
    decided_by: "access_mode",
    summary,
    link,
  }, undefined, "autopilot");
  const receipt = screen.getByRole("status");
  expect(receipt).toHaveTextContent("Allowed automatically · Full access");
  expect(receipt).toHaveTextContent("1 declared omission(s)");
  expect(receipt).toHaveTextContent("separate consent bypassed by Full access");
  expect(receipt).not.toHaveTextContent(omission);
  expect(screen.getByRole("link", { name: "Inspect or discard proposal" })).toHaveAttribute("href", link);
  // The receipt link opens this read-only view in the proposal panel.
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })}>
      <AuthoredSkillScopePreview agentId="agent" proposalId="proposal" />
    </QueryClientProvider>,
  );
  expect(await screen.findByRole("list", { name: "Declared omissions" })).toHaveTextContent(omission);
  expect(mocks.json).toHaveBeenCalledWith("/assistant/nyxagent/agents/agent/learning/proposals/proposal");
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(decide).not.toHaveBeenCalled();
});
