import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { NyxAgentAcknowledgementCard } from "./nyxagent-acknowledgement-card";
import { DecisionStack } from "./decision-stack";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

const mocks = vi.hoisted(() => ({ json: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantJson: mocks.json }));
vi.mock("@/stores/auth-store", () => ({ useAuthStore: (select: (state: unknown) => unknown) => select({ user: { id: "owner" } }) }));
vi.mock("@tanstack/react-router", async (original) => ({
  ...await original<typeof import("@tanstack/react-router")>(),
  Link: ({ children }: { children: ReactNode }) => <a href="/assistant?details=skills">{children}</a>,
}));
afterEach(cleanup);

it("requires full review and explicit reduced-scope acceptance before publishing a pinned draft", async () => {
  mocks.json.mockResolvedValue({
    id: "proposal", agent_id: "agent", agent_name: "NyxBot", revision: 0, skills_revision: 0, current_skills_revision: 0,
    name: "hello-world", version: "1.0", status: "pending", state: "active", actions: ["deny", "publish"],
    scope: { status: "reduced", omissions: ["Historical imports are excluded"] },
    files: [{ path: "SKILL.md", content: Array.from({ length: 40 }, (_, index) => `Draft line ${index + 1}`).join("\n") }],
  });
  const card: NyxAgentAcknowledgement = {
    id: "card", kind: "action", status: "pending", summary: "Review skill", decider: "user", decided_by: null, reason: null,
    service_slug: null, service_name: null, tool_name: "draft_agent_skill", created_at: "2026-11-26T00:00:00Z",
    decided_at: null, expires_at: "2099-01-01T00:00:00Z",
    authored_skill: { agent_id: "agent", proposal_id: "proposal", revision: 0, skills_revision: 0 },
  };
  const decide = vi.fn().mockResolvedValue(undefined);
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })}>
    <DecisionStack items={[{ id: card.id, content: <NyxAgentAcknowledgementCard acknowledgement={card} deciding={false} onDecision={decide} /> }]} onEmptyFocus={vi.fn()} />
  </QueryClientProvider>);
  const review = await screen.findByRole("button", { name: "Review full draft" });
  await waitFor(() => expect(review).toBeEnabled());
  fireEvent.click(review);
  const dialog = await screen.findByRole("dialog");
  expect(within(dialog).getByText(/Draft line 40/)).toBeVisible();
  const allow = within(dialog).getByRole("button", { name: "Allow and publish" });
  expect(allow).toBeDisabled();
  fireEvent.click(within(dialog).getByRole("checkbox"));
  expect(allow).toBeEnabled();
  fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
  expect(screen.queryByRole("button", { name: "Allow and publish" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Review full draft" }));
  expect(within(screen.getByRole("dialog")).getByRole("checkbox")).toBeChecked();
  expect(decide).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Allow and publish" }));
  expect(decide).toHaveBeenCalledWith("allow", true);
});
