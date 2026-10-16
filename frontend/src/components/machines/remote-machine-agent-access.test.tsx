import {
  cleanup,
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

const state = vi.hoisted(() => ({
  setGrants: vi.fn(),
  setCapabilities: vi.fn(),
  refetch: vi.fn(),
  feature: true,
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
          can_maintain: true,
          machines: ["other-machine"],
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
  useRequestMachineContext: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-machines", () => ({
  useMachineSettings: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => state.feature,
}));
const auth = useAuthStore.getState();
beforeEach(() => {
  useAuthStore.setState({ user: { ...auth.user!, id: "owner" } });
  state.feature = true;
  state.machine = {
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
});
afterEach(() => {
  cleanup();
  useAuthStore.setState(auth);
  vi.resetAllMocks();
});
async function show() {
  render(<RemoteMachineAgentAccess machineId="machine-1" />);
  await userEvent.click(
    screen.getByRole("combobox", { name: "Agent for machine access" }),
  );
  await userEvent.click(screen.getByRole("option", { name: "Researcher" }));
}

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
      machines: ["other-machine", "machine-1"],
    }),
  );
  expect(state.setGrants.mock.calls[0]![0]).not.toHaveProperty("logins");
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
    screen.getByText(/Capability editing is unavailable/),
  ).toBeInTheDocument();
  expect(state.setCapabilities).not.toHaveBeenCalled();
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
