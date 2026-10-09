import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NyxAgentAcknowledgementCard } from "./nyxagent-acknowledgement-card";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";
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
});
it("denies through the same card", async () => {
  const decide = show();
  await screen.findByText("Verify every date");
  fireEvent.click(screen.getByRole("button", { name: "Deny" }));
  await waitFor(() => expect(decide).toHaveBeenCalledExactlyOnceWith("deny"));
});
it.each(["changed", "unavailable"])("blocks approval when content is %s", async (state) => {
  if (state === "changed") mocks.json.mockResolvedValue({ ...preview, revision: 1 });
  else mocks.json.mockRejectedValue(new Error("Unavailable"));
  show();
  await screen.findByRole("alert");
  expect(screen.getByRole("button", { name: "Allow and publish" })).toBeDisabled();
});
it.each(["publication_failed", "publishing"])("offers recovery from %s without a new card", async (status) => {
  mocks.json.mockResolvedValue({ ...preview, status });
  const decide = show({ ...card, status: "used" });
  await screen.findByText("Verify every date");
  fireEvent.click(screen.getByRole("button", { name: "Retry publication" }));
  await waitFor(() => expect(decide).toHaveBeenCalledExactlyOnceWith("allow"));
});
it.each(["denied", "expired"] as const)("never loads content or publishes a %s draft", (status) => {
  const decide = show({ ...card, status });
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(mocks.json).not.toHaveBeenCalled();
  expect(decide).not.toHaveBeenCalled();
});
it("shows the settled publication result", async () => {
  mocks.json.mockResolvedValue({ ...preview, status: "pinned", files: [] });
  show({ ...card, status: "used" });
  expect(await screen.findByText("Skill published and attached")).toBeVisible();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});
