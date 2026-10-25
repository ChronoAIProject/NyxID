import {
  cleanup,
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { RemoteMachineAgentAccess } from "./remote-machine-agent-access";
import { useAuthStore } from "@/stores/auth-store";
import type { MachineAccess } from "@/schemas/machine-access";
import type { NodeInfo } from "@/types/nodes";
import { machineAccessFixture, machineContextReviewFixture } from "@/test/machine-access-fixture";
import { FEATURE_FLAG } from "@/lib/feature-flags";

const state = vi.hoisted(() => ({
  setGrants: vi.fn(),
  setCapabilities: vi.fn(),
  requestContext: vi.fn(),
  refetch: vi.fn(),
  feature: true,
  contexts: true,
  canMaintain: true,
  assigned: false,
  machine: undefined as MachineAccess | undefined,
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({
    data: {
      agents: [
        {
          id: "agent-1",
          name: "Researcher",
          owner_id: "owner",
          owner_kind: "person",
          kind: "specialist",
          status: "idle",
          can_maintain: state.canMaintain,
          machines: state.assigned
            ? ["other-machine", "machine-1"]
            : ["other-machine"],
          services: ["my-service"],
          account_read: true,
          logins: ["existing-login"],
        },
      ],
    },
  }),
  useSetNyxBotAgentGrants: () => ({ mutateAsync: state.setGrants }),
}));
vi.mock("@/hooks/use-machine-access", () => ({
  useMachineAccess: () => ({
    refetch: state.refetch,
    data: [state.machine],
  }),
  useSetMachineAccess: () => ({ mutateAsync: state.setCapabilities }),
  usePrepareMachineContext: () => ({ mutateAsync: state.requestContext }),
  useCommitMachineContext: () => ({ mutateAsync: vi.fn() }),
  useMachineContextReview: () => ({ refetch: vi.fn() }),
}));
vi.mock("@/hooks/use-machines", () => ({
  useMachineSettings: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: (flag: string) => flag === FEATURE_FLAG.MACHINE_CONTEXTS ? state.contexts : state.feature,
}));
const auth = useAuthStore.getState();
it("opens the selected Specialist configuration as membership only without a capability write", () => {
  state.assigned = true;
  render(<RemoteMachineAgentAccess machineId="machine-1" initialAgentId="agent-1" />);
  expect(screen.getByText("No tools granted. Membership alone does not authorize tools.")).toBeVisible();
  expect(screen.getByText(/all tools and saved logins off, regardless of editing flags/)).toBeVisible();
  expect(state.setCapabilities).not.toHaveBeenCalled();
  expect(state.setGrants).not.toHaveBeenCalled();
});
beforeEach(() => {
  useAuthStore.setState({ user: { ...auth.user!, id: "owner" } });
  state.feature = true;
  state.contexts = true;
  state.canMaintain = true;
  state.assigned = false;
  state.machine = {
    ...machineAccessFixture,
    node_id: "machine-1",
    name: "Hosted machine",
    revision: 8,
    can_edit: true,
    protocol_v2: true,
    legacy: false,
    mode: "shared_legacy",
    capabilities: {
      shell: false,
      files: false,
      browser: false,
      computer: false,
      developer_browser: false,
    },
    ceiling: {
      shell: true,
      files: true,
      browser: true,
      computer: true,
      developer_browser: true,
    },
    saved_login_ids: null,
  };
  state.setGrants.mockResolvedValue({});
  state.setCapabilities.mockResolvedValue({});
  state.requestContext.mockResolvedValue(machineContextReviewFixture);
  state.refetch.mockResolvedValue({ error: null });
});
afterEach(() => {
  cleanup();
  useAuthStore.setState(auth);
  vi.resetAllMocks();
});
async function show() {
  const view = render(<RemoteMachineAgentAccess machineId="machine-1" />);
  await userEvent.click(
    screen.getByRole("combobox", { name: "Agent for machine access" }),
  );
  await userEvent.click(screen.getByRole("option", { name: "Researcher" }));
  return view;
}

it("renders Node machine_confirm as read-only detail data", async () => {
  const node = { id: "node-1", machine_confirm: "changes" } as NodeInfo;
  render(<RemoteMachineAgentAccess machineId="machine-1" node={node} />);
  expect(screen.getByText("Confirm changes")).toBeInTheDocument();
  expect(
    screen.getByText(/does not save confirmation settings/),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Save confirmation policy" }),
  ).not.toBeInTheDocument();
});

it("updates specialist membership through existing grants while preserving services and other machines", async () => {
  await show();
  const form = screen.getByRole("form", {
    name: "Specialist machine membership",
  });
  await userEvent.click(within(form).getByRole("checkbox"));
  await userEvent.click(
    within(form).getByRole("button", { name: "Save assignment" }),
  );
  await waitFor(() =>
    expect(state.setGrants).toHaveBeenCalledWith({
      id: "agent-1",
      services: ["my-service"],
      account_read: true,
      logins: ["existing-login"],
      machines: ["other-machine", "machine-1"],
    }),
  );
  expect(state.setGrants.mock.calls[0]![0]).toHaveProperty("logins", [
    "existing-login",
  ]);
});

it("keeps a successful assignment when the separate capability save fails", async () => {
  const view = await show();
  const membership = screen.getByRole("form", {
    name: "Specialist machine membership",
  });
  await userEvent.click(within(membership).getByRole("checkbox"));
  await userEvent.click(
    within(membership).getByRole("button", { name: "Save assignment" }),
  );
  await waitFor(() => expect(state.setGrants).toHaveBeenCalled());
  // The real grants mutation invalidates and refreshes the Agent list.
  state.assigned = true;
  view.rerender(<RemoteMachineAgentAccess machineId="machine-1" />);
  expect(
    await screen.findByText(/Machine assignment saved/),
  ).toBeInTheDocument();
  state.setCapabilities.mockRejectedValueOnce(new Error("capability failed"));
  const capabilities = screen.getByRole("form", { name: /capabilities$/ });
  await userEvent.click(
    within(capabilities).getByRole("checkbox", { name: "Shell commands" }),
  );
  await userEvent.click(
    within(capabilities).getByRole("button", { name: /^Save capabilities$/ }),
  );
  expect(
    await screen.findByText(
      /Assignment succeeded, but capability changes failed/,
    ),
  ).toBeInTheDocument();
  expect(
    within(
      screen.getByRole("form", { name: "Specialist machine membership" }),
    ).getByRole("checkbox"),
  ).toBeChecked();
  expect(state.setGrants).toHaveBeenCalledTimes(1);
  state.assigned = false;
  view.rerender(<RemoteMachineAgentAccess machineId="machine-1" />);
  await waitFor(() =>
    expect(
      within(
        screen.getByRole("form", { name: "Specialist machine membership" }),
      ).getByRole("checkbox"),
    ).not.toBeChecked(),
  );
  expect(
    screen.queryByText(/Assignment succeeded, but capability changes failed/),
  ).not.toBeInTheDocument();
});

it("refreshes authoritative membership without keeping another Machine's feedback", async () => {
  const view = await show();
  state.assigned = true;
  view.rerender(<RemoteMachineAgentAccess machineId="machine-1" />);
  await waitFor(() =>
    expect(
      within(
        screen.getByRole("form", { name: "Specialist machine membership" }),
      ).getByRole("checkbox"),
    ).toBeChecked(),
  );
  view.rerender(<RemoteMachineAgentAccess machineId="different-machine" />);
  expect(
    within(
      screen.getByRole("form", { name: "Specialist machine membership" }),
    ).getByRole("checkbox"),
  ).not.toBeChecked();
  expect(
    screen.queryByText(/Machine assignment saved/),
  ).not.toBeInTheDocument();
});

it("uses the observed revision and live capability ceiling for shell/file edits", async () => {
  state.machine!.ceiling.browser = false;
  state.machine!.ceiling.computer = false;
  state.machine!.ceiling.developer_browser = false;
  await show();
  const form = screen.getByRole("form", { name: /capabilities$/ });
  expect(within(form).getAllByRole("checkbox")).toHaveLength(5);
  expect(
    within(form).getByRole("checkbox", { name: /Secure browser/ }),
  ).toBeDisabled();
  expect(
    within(form).getByRole("checkbox", { name: /Full computer control/ }),
  ).toBeDisabled();
  expect(
    within(form).getByRole("checkbox", { name: /Developer browser/ }),
  ).toBeDisabled();
  await userEvent.click(
    within(form).getByRole("checkbox", { name: "Shell commands" }),
  );
  await userEvent.click(
    within(form).getByRole("button", { name: /^Save (agent )?capabilities$/ }),
  );
  await waitFor(() =>
    expect(state.setCapabilities).toHaveBeenCalledWith({
      node: "machine-1",
      selection: {
        expected_revision: 8,
        capabilities: {
          shell: true,
          files: false,
          browser: false,
          computer: false,
          developer_browser: false,
        },
        saved_login_ids: null,
      },
    }),
  );
});

it("does not enable editing when the existing capability feature is unavailable", async () => {
  state.feature = false;
  await show();
  expect(
    screen.getByRole("button", { name: /^Save (agent )?capabilities$/ }),
  ).toBeDisabled();
  expect(
    screen.getByText(/Capability editing is not enabled for this account/),
  ).toBeInTheDocument();
  expect(state.setCapabilities).not.toHaveBeenCalled();
});

it.each([
  [false, false], [false, true], [true, false], [true, true],
])("keeps capability editing %s independent from context opt-in %s", async (editing, contexts) => {
  state.feature = editing;
  state.contexts = contexts;
  state.machine!.separated = { available: true, reason: null, landlock_abi: 6 };
  await show();
  const shell = screen.getByRole("checkbox", { name: "Shell commands" });
  if (editing) expect(shell).toBeEnabled();
  else expect(shell).toBeDisabled();
  const request = screen.getByRole("button", { name: "Separate workspace and browser for this agent" });
  if (contexts) {
    expect(request).toBeEnabled();
    await userEvent.click(request);
    await waitFor(() => expect(state.requestContext).toHaveBeenCalledWith({
      node: "machine-1",
      body: { mode: "separated", policy_version: "machine-context-mode-v1", acknowledge_separation: true },
    }));
  } else {
    expect(request).toBeDisabled();
    expect(screen.getByText(/Context setup is not enabled for this account/)).toBeVisible();
  }
  expect(state.setCapabilities).not.toHaveBeenCalled();
});

it("shows membership and all-off tools separately and updates membership from the Agent read", async () => {
  const view = await show();
  expect(screen.getByText("Not assigned")).toBeVisible();
  expect(screen.getByText(/No tools granted/)).toBeVisible();
  expect(screen.queryByText(/permissions need configuration/i)).not.toBeInTheDocument();
  state.assigned = true;
  view.rerender(<RemoteMachineAgentAccess machineId="machine-1" />);
  expect(screen.getByText("Assigned")).toBeVisible();
  expect(screen.getByText(/No tools granted/)).toBeVisible();
  expect(state.setCapabilities).not.toHaveBeenCalled();
});

it("keeps capability and mode requests read-only without Agent maintenance rights", async () => {
  state.canMaintain = false;
  state.machine!.mode = "selection_required";
  state.machine!.separated = { available: true, reason: null, landlock_abi: 6 };
  await show();
  for (const checkbox of screen.getAllByRole("checkbox")) expect(checkbox).toBeDisabled();
  expect(screen.getByRole("button", { name: "Separate workspace and browser for this agent" })).toBeDisabled();
  expect(screen.getByText(/This Agent is read-only/)).toBeVisible();
  expect(state.requestContext).not.toHaveBeenCalled();
  await act(async () => {
    fireEvent.submit(screen.getByRole("form", { name: "Specialist machine membership" }));
    fireEvent.submit(screen.getByRole("form", { name: /capabilities$/ }));
  });
  expect(state.setGrants).not.toHaveBeenCalled();
  expect(state.setCapabilities).not.toHaveBeenCalled();
});

it("shows an offline connection without treating saved permissions as runtime readiness", async () => {
  render(<RemoteMachineAgentAccess machineId="machine-1" node={{ id: "machine-1", is_connected: false } as NodeInfo} />);
  expect(screen.getByText(/Machine is offline/)).toBeVisible();
  expect(screen.getByText(/Saved permissions do not confirm runtime availability/)).toBeVisible();
});

it("preserves existing Browser, Computer, developer browser and saved-login grants when Commands changes", async () => {
  state.machine!.capabilities = {
    shell: false,
    files: true,
    browser: true,
    computer: true,
    developer_browser: true,
  };
  state.machine!.saved_login_ids = ["existing-login"];
  await show();
  const form = screen.getByRole("form", { name: /capabilities$/ });
  await userEvent.click(
    within(form).getByRole("checkbox", { name: "Shell commands" }),
  );
  await userEvent.click(
    within(form).getByRole("button", { name: /^Save (agent )?capabilities$/ }),
  );
  await waitFor(() =>
    expect(state.setCapabilities).toHaveBeenCalledWith({
      node: "machine-1",
      selection: {
        expected_revision: 8,
        capabilities: {
          shell: true,
          files: true,
          browser: true,
          computer: true,
          developer_browser: true,
        },
        saved_login_ids: ["existing-login"],
      },
    }),
  );
});

it("lets the owner explicitly grant Browser then Computer within the Node ceiling", async () => {
  await show();
  const form = screen.getByRole("form", { name: /capabilities$/ });
  expect(
    within(form).getByRole("checkbox", { name: "Full computer control" }),
  ).toBeDisabled();
  await userEvent.click(
    within(form).getByRole("checkbox", { name: "Secure browser" }),
  );
  await userEvent.click(
    within(form).getByRole("checkbox", { name: "Full computer control" }),
  );
  await userEvent.click(
    within(form).getByRole("button", { name: /^Save (agent )?capabilities$/ }),
  );
  await waitFor(() =>
    expect(state.setCapabilities).toHaveBeenCalledWith({
      node: "machine-1",
      selection: {
        expected_revision: 8,
        capabilities: {
          shell: false,
          files: false,
          browser: true,
          computer: true,
          developer_browser: false,
        },
        saved_login_ids: null,
      },
    }),
  );
});

it("removes Computer explicitly without revoking Browser or developer browser", async () => {
  state.machine!.capabilities = {
    shell: true,
    files: true,
    browser: true,
    computer: true,
    developer_browser: true,
  };
  await show();
  const form = screen.getByRole("form", { name: /capabilities$/ });
  await userEvent.click(
    within(form).getByRole("checkbox", { name: "Full computer control" }),
  );
  await userEvent.click(
    within(form).getByRole("button", { name: /^Save (agent )?capabilities$/ }),
  );
  await waitFor(() =>
    expect(state.setCapabilities).toHaveBeenCalledWith({
      node: "machine-1",
      selection: {
        expected_revision: 8,
        capabilities: {
          shell: true,
          files: true,
          browser: true,
          computer: false,
          developer_browser: true,
        },
        saved_login_ids: null,
      },
    }),
  );
});

it("removes only Browser-dependent capability grants when Browser is unchecked", async () => {
  state.machine!.capabilities = {
    shell: true,
    files: true,
    browser: true,
    computer: true,
    developer_browser: true,
  };
  state.machine!.saved_login_ids = ["existing-login"];
  await show();
  const form = screen.getByRole("form", { name: /capabilities$/ });
  expect(
    screen.getByText(
      /Turning off Browser also removes Computer and Developer browser/,
    ),
  ).toBeInTheDocument();
  await userEvent.click(
    within(form).getByRole("checkbox", { name: "Secure browser" }),
  );
  expect(
    within(form).getByRole("checkbox", { name: "Full computer control" }),
  ).not.toBeChecked();
  expect(
    within(form).getByRole("checkbox", { name: "Developer browser" }),
  ).not.toBeChecked();
  await userEvent.click(
    within(form).getByRole("button", { name: /^Save (agent )?capabilities$/ }),
  );
  await waitFor(() =>
    expect(state.setCapabilities).toHaveBeenCalledWith({
      node: "machine-1",
      selection: {
        expected_revision: 8,
        capabilities: {
          shell: true,
          files: true,
          browser: false,
          computer: false,
          developer_browser: false,
        },
        saved_login_ids: ["existing-login"],
      },
    }),
  );
});

it.each(["can_edit", "protocol_v2"] as const)(
  "keeps the existing %s write gate",
  async (gate) => {
    state.machine![gate] = false;
    state.machine!.capabilities.browser = true;
    await show();
    const form = screen.getByRole("form", { name: /capabilities$/ });
    expect(
      within(form).getByRole("checkbox", { name: "Secure browser" }),
    ).toBeChecked();
    for (const checkbox of within(form).getAllByRole("checkbox"))
      expect(checkbox).toBeDisabled();
    expect(
      within(form).getByRole("button", {
        name: /^Save (agent )?capabilities$/,
      }),
    ).toBeDisabled();
    expect(state.setCapabilities).not.toHaveBeenCalled();
  },
);

it("retains shared-workspace and external-client scope disclosures for full Machine grants", async () => {
  await show();
  expect(
    screen.getByText(
      /Workspace and browser sessions are shared with other agents/,
    ),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/External OAuth and Agent Key clients/),
  ).toHaveTextContent(/shell\/file tools/);
  expect(
    screen.queryByText(
      /Browser and saved-login access are outside this hosted profile/,
    ),
  ).not.toBeInTheDocument();
});
