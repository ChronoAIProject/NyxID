import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { machineSelectionSchema, type MachineSelection } from "@/schemas/machine-access";
import { useAuthStore } from "@/stores/auth-store";
import { machineAccessFixture, offCapabilities } from "@/test/machine-access-fixture";
import type { User } from "@/types/api";
import { MachineCapabilities } from "./machine-capabilities";

vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => ({ data: [] }) }));
vi.mock("@/hooks/use-machines", () => ({ useRemoteMachines: () => ({ data: [] }) }));
vi.mock("@/hooks/use-saved-logins", () => ({ useSavedLogins: () => ({ data: [] }) }));

const agents = {
  agents: [],
  limits: {
    skip_destructive_confirmation: false,
    max_live_subagents: 4,
    max_concurrent_subagent_turns: 2,
    max_live_subagents_limit: 4,
    max_concurrent_subagent_turns_limit: 2,
  },
};
const accessPath = "/api/v1/assistant/nyxagent/agents/agent/machines";
const agentsPath = "/api/v1/assistant/nyxagent/agents?include_destroyed=true";
let client: QueryClient;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

// Keep the normal Agent-list query active so the real owner mutation waits
// for both it and machine access when it invalidates the Agent prefix.
function OwnerPage() {
  const query = useNyxBotAgents();
  return <>
    <p>{query.isFetching ? "Refreshing Agent list" : "Agent list current"}</p>
    <MachineCapabilities agentId="agent" />
  </>;
}

beforeEach(() => {
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  useAuthStore.setState({ user: {
    id: "owner",
    email: "owner@example.test",
    display_name: "Owner",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: true,
    is_admin: false,
    is_active: true,
    created_at: "2026-10-24T00:00:00Z",
    capabilities: { enabled_features: [FEATURE_FLAG.MACHINE_CAPABILITIES, FEATURE_FLAG.MACHINE_CONTEXTS] },
  } satisfies User });
});
afterEach(() => {
  cleanup();
  client.clear();
  vi.unstubAllGlobals();
});

it.each(["machine access first", "Agent list first", "newer live policy before Agent list"] as const)(
  "saves consecutive changes and all-off with the current revision: %s",
  async (order) => {
    const agentRefresh = deferred<Response>();
    const accessRefresh = deferred<Response>();
    const submissions: MachineSelection[] = [];
    let current = { ...machineAccessFixture, revocation_pending: false, delivery_pending: false };
    let agentReads = 0;
    let accessReads = 0;
    const transport = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === agentsPath) {
        agentReads += 1;
        return agentReads === 2 ? agentRefresh.promise : Response.json(agents);
      }
      if (url === accessPath) {
        accessReads += 1;
        return accessReads === 2 ? accessRefresh.promise : Response.json([current]);
      }
      if (url === `${accessPath}/node` && init?.method === "PUT") {
        const selection = machineSelectionSchema.parse(JSON.parse(String(init.body)));
        submissions.push(selection);
        if (selection.expected_revision !== current.revision) {
          return Response.json({ error: "conflict", error_code: 0, message: "Reload if access changed." }, { status: 409 });
        }
        current = { ...current, revision: current.revision + 1, capabilities: selection.capabilities, saved_login_ids: selection.saved_login_ids };
        return Response.json([current]);
      }
      throw new Error(`Unexpected request: ${init?.method ?? "GET"} ${url}`);
    });
    vi.stubGlobal("fetch", transport);
    render(<QueryClientProvider client={client}><OwnerPage /></QueryClientProvider>);
    const shell = await screen.findByRole("checkbox", { name: "Shell commands" });
    const files = screen.getByRole("checkbox", { name: "File tools" });
    const save = screen.getByRole("button", { name: "Save capabilities" });
    const form = screen.getByRole("form", { name: "Work machine capabilities" });
    fireEvent.click(shell);
    await waitFor(() => expect(save).toBeEnabled());
    fireEvent.click(save);
    await waitFor(() => {
      expect(agentReads).toBe(2);
      expect(accessReads).toBe(2);
    });
    expect(submissions[0]?.expected_revision).toBe(7);
    expect(save).toBeDisabled();
    if (order !== "Agent list first") {
      await act(async () => { accessRefresh.resolve(Response.json([current])); });
      await screen.findByText(/policy revision 8/);
      expect(screen.getByText("Refreshing Agent list")).toBeVisible();
      if (order === "newer live policy before Agent list") {
        // A subsequent owner update arrives after the PUT's revision-8
        // response, but before that mutation finishes waiting for Agent list.
        current = { ...current, revision: 9, saved_login_ids: ["login-2"] };
        fireEvent.click(screen.getByRole("button", { name: "Refresh machine status" }));
        await screen.findByText(/policy revision 9/);
      }
      await act(async () => { agentRefresh.resolve(Response.json(agents)); });
    } else {
      await act(async () => { agentRefresh.resolve(Response.json(agents)); });
      await screen.findByText("Agent list current");
      expect(screen.getByText(/policy revision 7/)).toBeVisible();
      await act(async () => { accessRefresh.resolve(Response.json([current])); });
    }
    await screen.findByText(/Capabilities saved/);
    const latestRevision = current.revision;
    const latestLogins = current.saved_login_ids;
    expect(screen.getByRole("form", { name: "Work machine capabilities" })).toBe(form);
    expect(shell).toBeChecked();
    expect(files).toBeChecked();
    expect(save).toBeDisabled();

    // Dirty state remains relative to the saved policy, including reverting a
    // draft, then explicitly revoking the last tools without a remount/reload.
    fireEvent.click(shell);
    await waitFor(() => expect(save).toBeEnabled());
    fireEvent.click(shell);
    await waitFor(() => expect(save).toBeDisabled());
    fireEvent.click(shell);
    fireEvent.click(files);
    await waitFor(() => expect(save).toBeEnabled());
    fireEvent.click(save);
    await waitFor(() => expect(submissions).toHaveLength(2));
    expect(submissions[1]).toEqual({ expected_revision: latestRevision, capabilities: offCapabilities, saved_login_ids: latestLogins });
    await screen.findByText(new RegExp(`policy revision ${latestRevision + 1}`));
    await screen.findByText(/Capabilities saved/);
    expect(screen.getByText(/No tools granted\. Membership alone/)).toBeVisible();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(save).toBeDisabled();
    expect(shell).not.toBeChecked();
    expect(files).not.toBeChecked();
    expect(screen.queryByText("Agent authorization applied")).not.toBeInTheDocument();
    expect(screen.queryByText("Environment mode applied")).not.toBeInTheDocument();
  },
);
