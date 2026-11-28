import { confirmChanges } from "@/test-helpers/confirm-changes";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ShareAgentDialog, SharedConsentCards, SharedAgentProfile } from "./shared-agent-controls";
import { assistantAgentSchema } from "@/schemas/assistant-nyxagent";
import { agentOwnerSections } from "@/lib/assistant/nyxbot-labels";

const mocks = vi.hoisted(() => ({ enabled: true, save: vi.fn(), decide: vi.fn(), orgs: vi.fn(), audience: [] as { subject_kind: string; subject_id: string; role: string; name?: string; slug?: string }[] }));
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: mocks.orgs }));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => mocks.enabled }));
vi.mock("@/hooks/use-shared-agents", () => ({
  useAgentSharing: () => ({
    data: {
      role: "owner",
      version: 0,
      revision: 0,
      owner_id: "owner",
      enabled: false,
      audience: mocks.audience,
      shared_access: {},
      services: [{ id: "calendar", name: "Calendar" }],
    },
  }),
  useSaveAgentSharing: () => ({ mutateAsync: mocks.save }),
  useSharedConsents: () => ({
    data: {
      consents: [
        {
          id: "consent",
          conversation_id: "my-chat",
          decision: "pending",
          connection_id: "my-account",
          catalog_service_id: "calendar",
          operation: "POST /events",
          scope: "exact_operation",
          expires_at: null,
        },
      ],
    },
  }),
  useDecideSharedConsent: () => ({ mutate: mocks.decide }),
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({ useUpdateNyxBotAgent: () => ({ mutate: vi.fn() }) }));
const agent = assistantAgentSchema.parse({ id: "agent", name: "Researcher", kind: "specialist", status: "idle", created_at: "2026-10-26T00:00:00Z", last_active_at: "2026-10-26T00:00:00Z" });
afterEach(() => { cleanup(); mocks.enabled = true; mocks.audience = []; vi.clearAllMocks(); });

it("starts withheld and saves an explicit audience only after edits", async () => {
  const user = userEvent.setup();
  render(<QueryClientProvider client={new QueryClient()}><ShareAgentDialog agent={agent} /></QueryClientProvider>);
  await user.click(screen.getByRole("button", { name: "Share agent" }));
  expect(screen.getByRole("combobox", { name: "Calendar" })).toHaveTextContent("Withheld");
  expect(
    screen.queryByRole("region", { name: "Unsaved changes" }),
  ).not.toBeInTheDocument();
  await user.type(screen.getByLabelText("Person email"), "person@example.com");
  await user.click(screen.getByRole("button", { name: "Add to audience" }));
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
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
  expect(screen.getByLabelText("Tone and style")).toBeInTheDocument();
  expect(screen.queryByLabelText("Model")).not.toBeInTheDocument();
  expect(screen.queryByText("Grants")).not.toBeInTheDocument();
  expect(agentOwnerSections([agent, manager]).map((s) => s.label)).toEqual(["Personal", "Shared with me"]);
});

it("refreshes a manager's pristine profile fields while retaining drafts until Discard", async () => {
  const user = userEvent.setup();
  const manager = { ...agent, share_role: "manager" as const, persona: "Original tone", description: "Original role" };
  const view = render(<SharedAgentProfile agent={manager} />);
  const persona = screen.getByLabelText("Tone and style");
  await user.clear(persona);
  await user.type(persona, "Draft tone");
  view.rerender(<SharedAgentProfile agent={{ ...manager, persona: "Remote tone", description: "Remote role" }} />);
  await waitFor(() => expect(screen.getByLabelText("Instructions")).toHaveValue("Remote role"));
  expect(persona).toHaveValue("Draft tone");
  expect(screen.getByRole("button", { name: "Save profile" })).toBeEnabled();
  await user.click(screen.getByRole("button", { name: "Discard" }));
  expect(persona).toHaveValue("Remote tone");
  expect(
    screen.queryByRole("button", { name: "Save profile" }),
  ).not.toBeInTheDocument();
});

const orgId = "12345678-1234-4123-8123-123456789012";
beforeEach(() => mocks.orgs.mockReturnValue({ data: [{ id: orgId, display_name: "Design team", slug: "design", your_role: "viewer" }], isPending: false, error: null }));

it("selects an organization by name, sends its UUID, and retains saved organizations outside the list", async () => {
  const missingId = "22345678-1234-4123-8123-123456789012";
  mocks.audience = [{ subject_kind: "org", subject_id: missingId, role: "user", name: "Archived team", slug: "archived" }];
  const user = userEvent.setup();
  render(<QueryClientProvider client={new QueryClient()}><ShareAgentDialog agent={agent} /></QueryClientProvider>);
  await user.click(screen.getByRole("button", { name: "Share agent" }));
  expect(screen.getByText(/Archived team · archived/)).toBeVisible();
  await user.click(screen.getByRole("combobox", { name: "Audience type" }));
  await user.click(screen.getByRole("option", { name: "Organization" }));
  expect(screen.queryByLabelText("Organization ID")).toBeNull();
  await user.click(screen.getByRole("combobox", { name: "Organization" }));
  await user.click(screen.getByRole("option", { name: "Design team · design" }));
  await user.click(screen.getByRole("button", { name: "Add to audience" }));
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() => expect(mocks.save).toHaveBeenCalledWith(expect.objectContaining({ audience: [
    { subject_kind: "org", subject_id: missingId, role: "user" },
    { subject_kind: "org", subject_id: orgId, role: "user" },
  ] })));
});

it.each(["loading", "empty", "unavailable"])("keeps the audience while organizations are %s", async (state) => {
  mocks.orgs.mockReturnValue({ data: state === "empty" ? [] : undefined, isPending: state === "loading", error: state === "unavailable" ? new Error("Offline") : null, refetch: vi.fn() });
  mocks.audience = [{ subject_kind: "org", subject_id: orgId, role: "user", name: "Saved team" }];
  render(<QueryClientProvider client={new QueryClient()}><ShareAgentDialog agent={agent} /></QueryClientProvider>);
  await userEvent.click(screen.getByRole("button", { name: "Share agent" }));
  await userEvent.click(screen.getByRole("combobox", { name: "Audience type" }));
  await userEvent.click(screen.getByRole("option", { name: "Organization" }));
  expect(screen.getByText(/Saved team/)).toBeVisible();
  expect(screen.getByRole("combobox", { name: "Organization" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Add to audience" })).toBeDisabled();
  expect(screen.getByText(state === "loading" ? "Loading organizations…" : state === "empty" ? "No organizations available" : "Could not load organizations. Existing audience entries are kept.")).toBeVisible();
});
