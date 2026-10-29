import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { ShareAgentDialog, SharedConsentCards, SharedAgentProfile } from "./shared-agent-controls";
import { assistantAgentSchema } from "@/schemas/assistant-nyxagent";
import { agentOwnerSections } from "@/lib/assistant/nyxbot-labels";

const mocks = vi.hoisted(() => ({ enabled: true, save: vi.fn(), decide: vi.fn() }));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => mocks.enabled }));
vi.mock("@/hooks/use-shared-agents", () => ({
  useAgentSharing: () => ({ data: { role: "owner", version: 0, revision: 0, owner_id: "owner", enabled: false, audience: [], shared_access: {}, services: [{ id: "calendar", name: "Calendar" }] } }),
  useSaveAgentSharing: () => ({ mutate: mocks.save }),
  useSharedConsents: () => ({ data: { consents: [{ id: "consent", conversation_id: "my-chat", decision: "pending", connection_id: "my-account", catalog_service_id: "calendar", operation: "POST /events", scope: "exact_operation", expires_at: null }] } }),
  useDecideSharedConsent: () => ({ mutate: mocks.decide }),
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({ useUpdateNyxBotAgent: () => ({ mutate: vi.fn() }) }));
const agent = assistantAgentSchema.parse({ id: "agent", name: "Researcher", kind: "specialist", status: "idle", created_at: "2026-10-26T00:00:00Z", last_active_at: "2026-10-26T00:00:00Z" });
afterEach(() => { cleanup(); mocks.enabled = true; vi.clearAllMocks(); });

it("starts withheld and saves an explicit audience only after edits", async () => {
  const user = userEvent.setup();
  render(<QueryClientProvider client={new QueryClient()}><ShareAgentDialog agent={agent} /></QueryClientProvider>);
  await user.click(screen.getByRole("button", { name: "Share agent" }));
  expect(screen.getByRole("combobox", { name: "Calendar" })).toHaveTextContent("Withheld");
  expect(screen.getByRole("button", { name: "Save sharing" })).toBeDisabled();
  await user.type(screen.getByLabelText("Person email"), "person@example.com");
  await user.click(screen.getByRole("button", { name: "Add to audience" }));
  await user.click(screen.getByRole("button", { name: "Save sharing" }));
  await waitFor(() => expect(mocks.save).toHaveBeenCalledWith(expect.objectContaining({ expected_version: 0, shared_access: {}, audience: [{ subject_kind: "person", email: "person@example.com", role: "user" }] })));
});

it("hides publishing when the flag is off or the agent is organization owned", () => {
  mocks.enabled = false;
  const view = render(<ShareAgentDialog agent={agent} />);
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  mocks.enabled = true;
  view.rerender(<ShareAgentDialog agent={{ ...agent, owner_kind: "org" }} />);
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

it("keeps consent out of other conversations and sends the explicit choice", async () => {
  const view = render(<SharedConsentCards agentId="agent" conversationId="someone-else" />);
  expect(screen.queryByText("my-account")).not.toBeInTheDocument();
  view.rerender(<SharedConsentCards agentId="agent" conversationId="my-chat" />);
  await userEvent.click(screen.getByRole("button", { name: "Always" }));
  expect(mocks.decide).toHaveBeenCalledWith({ consent: "consent", decision: "always" });
});

it("limits manager controls and puts recipients in Shared with me", () => {
  const manager = { ...agent, share_role: "manager" as const };
  render(<SharedAgentProfile agent={manager} />);
  expect(screen.getByLabelText("Persona")).toBeInTheDocument();
  expect(screen.queryByLabelText("Model")).not.toBeInTheDocument();
  expect(screen.queryByText("Grants")).not.toBeInTheDocument();
  expect(agentOwnerSections([agent, manager]).map((s) => s.label)).toEqual(["Personal", "Shared with me"]);
});
