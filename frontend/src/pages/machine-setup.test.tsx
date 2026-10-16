import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { MachinePairPage, MachineSetupPage } from "./machine-setup";
import userEvent from "@testing-library/user-event";
import { useAuthStore } from "@/stores/auth-store";
import {
  remoteMachineFixture,
  remoteMachinePlatformFixture,
} from "@/test/remote-machine-fixture";
import { machineSetupCommand } from "@/schemas/machines";
const mock = vi.hoisted(() => ({
  issue: vi.fn(),
  create: vi.fn(),
  preview: vi.fn(),
  decide: vi.fn(),
  navigate: vi.fn(),
  quote: vi.fn(),
  createRemote: vi.fn(),
  setup: undefined as unknown,
  search: {} as Record<string, string>,
}));
vi.mock("@tanstack/react-router", () => ({
  useSearch: () => mock.search,
  useNavigate: () => mock.navigate,
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));
vi.mock("@/components/shared/org-scope-select", () => ({
  OrgScopeSelect: () => <span>Personal account</span>,
}));
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({
    data: {
      node_ws_url: "wss://nyxid.example/api/v1/nodes/ws",
      version: "0.39.0",
    },
  }),
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({ data: { agents: [] } }),
}));
vi.mock("@/hooks/use-billing", () => ({
  useBillingWallet: () => ({
    data: { owner_id: "owner", available_credits: 12 },
  }),
}));
vi.mock("@/hooks/use-machines", () => ({
  useRemoteMachineEntitlement: () => ({
    data: {
      eligible: true,
      free_slot_available: true,
      platform: remoteMachinePlatformFixture,
    },
    refetch: vi.fn(),
  }),
  useRemoteMachineQuote: () => ({
    mutate: mock.quote,
    data: {
      free_slot_requested: true,
      profile: remoteMachineFixture.profile,
      platform: remoteMachinePlatformFixture,
      valid_until: "2099-01-01T00:00:00Z",
      policy: {
        policy_version: "remote-machine-mvp",
        data_policy_version: "remote-machine-data",
      },
    },
  }),
  useCreateRemoteMachine: () => ({ mutateAsync: mock.createRemote }),
  useVerifiedUpdaterImage: () => ({
    data: {
      version: "0.39.0",
      image:
        "ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater@sha256:" +
        "ab".repeat(32),
    },
  }),
  useMachineSetup: () => ({ data: mock.setup }),
  useCreateMachineSetup: () => ({ mutateAsync: mock.create }),
  issueMachineSetup: mock.issue,
  useMachinePairPreview: () => ({ mutateAsync: mock.preview }),
  useMachinePairDecision: () => ({ mutateAsync: mock.decide }),
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  mock.setup = undefined;
  mock.search = {};
  sessionStorage.clear();
});
const choices = {
  name: "coding",
  where: "docker" as const,
  capabilities: ["shell" as const, "files" as const],
  grant_to: null,
};
it("shows one reviewed command only on the private setup page and removes it when connected", async () => {
  const client = new QueryClient();
  mock.search = { setup: "intent" };
  mock.setup = { id: "intent", choices, status: "review" };
  const token = `nyx_nreg_${"a".repeat(64)}`;
  mock.issue.mockResolvedValue({ token });
  const view = render(
    <QueryClientProvider client={client}>
      <MachineSetupPage />
    </QueryClientProvider>,
  );
  expect(screen.getByText(/prompt injection is possible/)).toBeInTheDocument();
  await waitFor(() =>
    expect(screen.getByLabelText("Machine name")).toHaveValue("coding"),
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Create my setup command" }),
  );
  await waitFor(() =>
    expect(mock.issue).toHaveBeenCalledWith("intent", {
      ...choices,
      automatic_updates: true,
    }),
  );
  await waitFor(() =>
    expect(screen.getByText(/docker run -d/)).toHaveTextContent(token),
  );
  expect(JSON.stringify(client.getQueryCache().getAll())).not.toContain(token);
  expect(screen.getByText(/docker run -d/)).toHaveTextContent(
    "nyxid-node-machine:0.39.0",
  );
  expect(screen.getByText(/docker run -d/)).not.toHaveTextContent(":latest");
  expect(screen.getByText(/Do not paste it into chat/)).toBeInTheDocument();
  mock.setup = {
    id: "intent",
    choices,
    status: "connected",
    machine: { shell: true, files: true },
  };
  view.rerender(
    <QueryClientProvider client={client}>
      <MachineSetupPage />
    </QueryClientProvider>,
  );
  await waitFor(() =>
    expect(screen.queryByText(/docker run -d/)).not.toBeInTheDocument(),
  );
  expect(screen.getByRole("status")).toHaveTextContent("Connected");
});
it("pairing shows machine details and requires explicit recognition before approval", async () => {
  mock.search = { code: "ABCD-2345" };
  mock.preview.mockResolvedValue({
    hostname: "work-vm",
    os: "linux",
    ip: "203.0.113.7",
    choices,
  });
  render(<MachinePairPage />);
  fireEvent.click(screen.getByRole("button", { name: "Review" }));
  await screen.findByText("work-vm");
  expect(screen.getByText("203.0.113.7")).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Approve pairing" }),
  ).toBeDisabled();
  fireEvent.click(
    screen.getByRole("checkbox", { name: /I started this setup/ }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Approve pairing" }));
  await waitFor(() =>
    expect(mock.decide).toHaveBeenCalledWith({
      code: "ABCD-2345",
      approve: true,
      automatic_updates: true,
    }),
  );
});
it("quotes setup credentials and URLs, installs the CLI if missing and preserves chosen capabilities", () => {
  const token = `nyx_nreg_${"b".repeat(64)}`;
  const host = machineSetupCommand(
    { ...choices, where: "vm" },
    token,
    "wss://example.test/'literal",
  );
  expect(host).toContain("command -v nyxid");
  expect(host).toContain('PATH="$HOME/.local/bin:$PATH"');
  expect(host).toContain("--shell --files");
  expect(host).not.toContain("--computer");
  expect(host).toContain("'\\''literal'");
  expect(() =>
    machineSetupCommand(choices, "not-a-token", "wss://example.test"),
  ).toThrow();
  for (const version of [undefined, "latest", "1.2.3; malicious"]) {
    expect(() =>
      machineSetupCommand(choices, token, "wss://example.test", version),
    ).toThrow();
  }
});

it("renders a browser-only setup without broad computer access", () => {
  const token = `nyx_nreg_${"b".repeat(64)}`;
  const command = machineSetupCommand(
    { ...choices, where: "vm", capabilities: ["browser"] },
    token,
    "wss://example.test",
  );
  expect(command).toContain("--browser");
  expect(command).not.toContain("--computer");
  expect(command).not.toContain("--shell");
});

it("renders the actual fourth hosted option with its creation form and no installation command", async () => {
  useAuthStore.setState({
    user: {
      ...useAuthStore.getState().user!,
      id: "owner",
      capabilities: { enabled_features: ["experimental:remote-machines"] },
    },
    isAuthenticated: true,
    isLoading: false,
  });
  mock.search = { where: "hosted" };
  mock.createRemote.mockResolvedValue({
    machine: { ...remoteMachineFixture, name: "my-remote-machine" },
    operation: { id: "operation-1" },
  });
  render(<MachineSetupPage />);
  expect(screen.getAllByRole("tab")).toHaveLength(4);
  expect(screen.getByRole("tab", { name: "NyxID hosted" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(
    screen.getByRole("form", { name: "NyxID hosted machine configuration" }),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Create my setup command" }),
  ).not.toBeInTheDocument();
  expect(screen.queryByText(/Run on your machine/)).not.toBeInTheDocument();
  expect(
    screen.queryByText(/Docker command downloads/),
  ).not.toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I accept the shared environment/ }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Create free remote machine" }),
  );
  await waitFor(() => expect(mock.createRemote).toHaveBeenCalledTimes(1));
  expect(mock.createRemote.mock.calls[0]![0].body).toMatchObject({
    name: "my-remote-machine",
    capabilities: ["shell", "files"],
    specialist_id: null,
  });
  expect(mock.navigate).toHaveBeenCalledWith({
    to: "/assistant/machines",
    search: { machine: "machine-1", operation: "operation-1" },
  });
  expect(mock.issue).not.toHaveBeenCalled();
});
it.each(["This computer", "Remote VM", "Docker"])(
  "preserves the %s command setup on the actual four-option page",
  async (mode) => {
    mock.create.mockResolvedValue({ id: "setup-1" });
    mock.issue.mockResolvedValue({ token: `nyx_nreg_${"a".repeat(64)}` });
    render(<MachineSetupPage />);
    await userEvent.click(screen.getByRole("tab", { name: mode }));
    await userEvent.click(
      screen.getByRole("button", { name: "Create my setup command" }),
    );
    await waitFor(() => expect(mock.issue).toHaveBeenCalledTimes(1));
    expect(mock.issue.mock.calls[0]![1].where).toBe(
      mode === "This computer"
        ? "this_computer"
        : mode === "Docker"
          ? "docker"
          : "vm",
    );
    expect(
      screen.getByText(
        mode === "Docker" ? /docker run -d/ : /command -v nyxid/,
      ),
    ).toBeInTheDocument();
    expect(mock.createRemote).not.toHaveBeenCalled();
  },
);

// useFeature still reads the actual auth store; its inactive org query is a double.
vi.mock("@/hooks/use-orgs", () => ({ useOrg: () => ({ data: undefined }) }));

it.each(["absent", "off", "loading"])(
  "direct hosted URL cannot mount or quote when %s; self-host choices remain",
  (mode) => {
    useAuthStore.setState({
      user: {
        id: "owner",
        ...(mode === "absent"
          ? {}
          : {
              capabilities: {
                enabled_features:
                  mode === "loading" ? ["experimental:remote-machines"] : [],
              },
            }),
      } as never,
      isAuthenticated: true,
      isLoading: mode === "loading",
    });
    mock.search = { where: "hosted" };
    render(<MachineSetupPage />);
    expect(screen.getAllByRole("tab")).toHaveLength(3);
    for (const name of ["This computer", "Remote VM", "Docker"])
      expect(screen.getByRole("tab", { name })).toBeInTheDocument();
    expect(
      screen.queryByRole("tab", { name: "NyxID hosted" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("form", {
        name: "NyxID hosted machine configuration",
      }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Create my setup command" }),
    ).toBeInTheDocument();
    expect(mock.quote).not.toHaveBeenCalled();
    expect(mock.createRemote).not.toHaveBeenCalled();
  },
);
