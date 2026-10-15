import type { AnchorHTMLAttributes, ReactNode } from "react";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { MachinesPage } from "./machines";
import { MachineSummary } from "@/components/shared/machine-summary";
import { useAuthStore } from "@/stores/auth-store";
import type { NodeInfo } from "@/types/nodes";
import { TooltipProvider } from "@/components/ui/tooltip";

const state = vi.hoisted(() => ({
  navigate: vi.fn(),
  tab: undefined as string | undefined,
  machine: undefined as string | undefined,
}));
vi.mock("@tanstack/react-router", () => ({
  useSearch: () => ({ tab: state.tab, machine: state.machine }),
  useNavigate: () => state.navigate,
  Link: ({
    to,
    params,
    search,
    disabled,
    children,
    ...props
  }: AnchorHTMLAttributes<HTMLAnchorElement> & {
    to: string;
    params?: { nodeId: string };
    search?: { tab?: string };
    disabled?: boolean;
    children: ReactNode;
  }) => (
    <a
      {...props}
      aria-disabled={disabled}
      href={
        to.replace("$nodeId", params?.nodeId ?? "") +
        (search?.tab ? `?tab=${search.tab}` : "")
      }
    >
      {children}
    </a>
  ),
}));
vi.mock("@/hooks/use-machines", () => ({
  useRemoteMachines: () => ({ data: remoteMachines, refetch: vi.fn() }),
  useVerifiedUpdaterImage: () => ({
    data: {
      version: "0.41.0",
      image:
        "ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater@sha256:" +
        "ab".repeat(32),
    },
  }),
  useMachineUpdates: () => ({ data: [] }),
}));
vi.mock("@/components/machines/remote-machine-create", () => ({
  RemoteMachineCreate: ({ open }: { open: boolean }) =>
    open ? <p>Review remote machine preflight</p> : null,
}));
vi.mock("@/components/machines/remote-machine-detail", () => ({
  RemoteMachineDetail: ({ machineId }: { machineId: string }) => (
    <p>Hosted details for {machineId}</p>
  ),
}));
vi.mock("@/components/machines/remote-machine-paid-start", () => ({
  RemoteMachinePaidStart: () => null,
}));
const remoteMachines = [
  {
    node_id: "hosted-machine",
    name: "Remote work",
    sandbox_name: "machine-hosted-uuid",
    resource_state: "reconciling",
    billing_state: "pending",
    authorization_state: "revoked",
    runtime_generation: 4,
    profile: { workspace_gib: 20 },
  },
];
vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => ({ data: nodes }) }));
vi.mock("@/hooks/use-orgs", () => ({
  useOrg: () => ({ data: undefined }),
  useOrgs: () => ({
    data: [
      { id: "org-admin", your_role: "admin" },
      { id: "org-member", your_role: "member" },
    ],
  }),
}));
vi.mock("@/components/shared/machine-settings", () => ({
  MachineSettings: ({ node }: { node: NodeInfo }) => (
    <p>Settings form for {node.name}</p>
  ),
}));
vi.mock("@/pages/saved-logins", () => ({
  SavedLoginsPage: () => <h2>Write-only saved logins</h2>,
}));
const base = {
  id: "machine",
  name: "My VM",
  owner: { id: "me", kind: "user", display_name: "Me" },
  status: "online",
  is_connected: true,
  machine: {
    shell: true,
    files: true,
    computer: true,
    computer_ready: true,
    browser_isolated: false,
    commands_isolated: false,
    roots: ["/workspace"],
    os: "linux",
    arch: "arm64",
  },
} as NodeInfo;
const nodes = [
  base,
  {
    ...base,
    id: "org",
    name: "Team VM",
    owner: { id: "org-admin", kind: "org", display_name: "Team" },
    is_connected: false,
  },
  {
    ...base,
    id: "member",
    name: "Not administered",
    owner: { id: "org-member", kind: "org", display_name: "Team" },
  },
  {
    ...base,
    id: "other",
    name: "Someone else's",
    owner: { id: "another", kind: "user", display_name: "Another" },
  },
  { ...base, id: "credential", name: "Credential-only", machine: null },
  {
    ...base,
    id: "hosted-not-listed",
    name: "Hosted node awaiting list",
    metadata: {
      provisioning_source: "remote_machine",
      agent_version: null,
      os: "linux",
      arch: null,
      ip_address: null,
    },
  },
];
const auth = useAuthStore.getState();
afterEach(() => {
  cleanup();
  useAuthStore.setState(auth);
  state.tab = undefined;
  state.machine = undefined;
  vi.clearAllMocks();
});
function show(enabled = true) {
  useAuthStore.setState({
    user: {
      ...auth.user!,
      id: "me",
      capabilities: {
        enabled_features: enabled ? ["experimental:remote-machines"] : [],
      },
    },
    isAuthenticated: true,
    isLoading: false,
  });
  return render(
    <TooltipProvider>
      <MachinesPage />
    </TooltipProvider>,
  );
}

it("lists usable machines with connection and isolation state and opens their settings sheet", async () => {
  show();
  expect(screen.getByText("My VM")).toBeInTheDocument();
  expect(screen.getByText("Team VM")).toBeInTheDocument();
  expect(screen.queryByText("Not administered")).not.toBeInTheDocument();
  expect(screen.queryByText("Someone else's")).not.toBeInTheDocument();
  expect(screen.queryByText("Credential-only")).not.toBeInTheDocument();
  expect(
    screen.queryByText("Hosted node awaiting list"),
  ).not.toBeInTheDocument();
  expect(screen.getAllByText("Not isolated")).toHaveLength(2);
  expect(screen.getByText(/Team · Disconnected/)).toBeInTheDocument();
  expect(
    screen.getAllByRole("link", { name: "Watch / Take over desktop" })[0],
  ).toHaveAttribute("href", "/assistant/machines/machine/desktop");
  await userEvent.click(
    screen.getByRole("button", { name: "Settings for My VM" }),
  );
  expect(
    within(screen.getByRole("dialog")).getByText("Settings form for My VM"),
  ).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Close" }));
  await userEvent.click(screen.getByRole("button", { name: "Add a machine" }));
  expect(state.navigate).toHaveBeenCalledWith({
    to: "/assistant/machines/new",
  });
});

it("opens the saved-login manager from the tab query and keeps tab navigation shareable", async () => {
  state.tab = "logins";
  show();
  expect(
    screen.getByRole("heading", { name: "Write-only saved logins" }),
  ).toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Machines" }));
  expect(state.navigate).toHaveBeenCalledWith({
    to: "/assistant/machines",
    search: { tab: undefined },
  });
});

it("Studio machine details are read-only and link to the assistant", () => {
  render(<MachineSummary node={base} />);
  expect(
    screen.getByRole("link", { name: "Manage in Assistant → Machines" }),
  ).toHaveAttribute("href", "/assistant/machines");
  expect(
    screen.getByText(/stored credentials, signing secret and node token/),
  ).toBeInTheDocument();
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
});

it("opens a guided-update machine link even when the machines page is already mounted", () => {
  const view = show();
  state.machine = "machine";
  view.rerender(
    <TooltipProvider>
      <MachinesPage />
    </TooltipProvider>,
  );
  expect(
    within(screen.getByRole("dialog")).getByText("Settings form for My VM"),
  ).toBeInTheDocument();
  state.machine = "org";
  view.rerender(
    <TooltipProvider>
      <MachinesPage />
    </TooltipProvider>,
  );
  expect(
    within(screen.getByRole("dialog")).getByText("Settings form for Team VM"),
  ).toBeInTheDocument();
});

it("keeps hosted resource, billing and authorization state separate and opens the real Machines detail", async () => {
  show();
  expect(screen.getAllByText("Remote work").length).toBeGreaterThan(0);
  expect(screen.getAllByText("Reconciling").length).toBeGreaterThan(0);
  expect(
    screen.getAllByText(/Billing: Pending · Authorization: Revoked/).length,
  ).toBeGreaterThan(0);
  await userEvent.click(
    screen.getByRole("button", { name: "View Remote work" }),
  );
  expect(
    screen.getByText("Hosted details for hosted-machine"),
  ).toBeInTheDocument();
  expect(state.navigate).toHaveBeenCalledWith({
    to: "/assistant/machines",
    search: { machine: "hosted-machine", operation: undefined },
  });
});

it("resolves a deleted machine deep link after it leaves the active list", () => {
  state.machine = "deleted-machine";
  show();
  expect(
    screen.getByText("Hosted details for deleted-machine"),
  ).toBeInTheDocument();
});

it("navigates the hosted shortcut to the shared setup form", async () => {
  show();
  await userEvent.click(
    screen.getByRole("button", { name: "Create remote machine" }),
  );
  expect(state.navigate).toHaveBeenCalledWith({
    to: "/assistant/machines/new",
    search: { where: "hosted" },
  });
  expect(
    screen.queryByText("Review remote machine preflight"),
  ).not.toBeInTheDocument();
});

it("hides hosted creation when OFF while existing hosted and self-hosted management remains", async () => {
  show(false);
  expect(
    screen.queryByRole("button", { name: "Create remote machine" }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Add a machine" })).toBeEnabled();
  expect(screen.getAllByText("Remote work").length).toBeGreaterThan(0);
  expect(screen.getByText("My VM")).toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("button", { name: "View Remote work" }),
  );
  expect(
    screen.getByText("Hosted details for hosted-machine"),
  ).toBeInTheDocument();
});
