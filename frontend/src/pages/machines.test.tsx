import type { AnchorHTMLAttributes, ReactNode } from "react";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { MachinesPage } from "./machines";
import { MachineSummary } from "@/components/shared/machine-summary";
import { useAuthStore } from "@/stores/auth-store";
import type { NodeInfo } from "@/types/nodes";
import type { RemoteMachineOperation } from "@/schemas/machines";
import { remoteMachineOperationFixture } from "@/test/remote-machine-fixture";
import { TooltipProvider } from "@/components/ui/tooltip";
import {
  getRemoteMachineIntent,
  readRemoteMachineIntent,
  acceptRemoteMachineIntent,
  observeAcceptedRemoteMachineIntent,
} from "@/lib/remote-machine-intent";

const state = vi.hoisted(() => ({
  navigate: vi.fn(),
  tab: undefined as string | undefined,
  machine: undefined as string | undefined,
  activity: undefined as string | undefined,
  slotAvailable: false,
  slotError: false,
  operation: undefined as RemoteMachineOperation | undefined,
}));
vi.mock("@tanstack/react-router", () => ({
  useSearch: () => ({
    tab: state.tab,
    machine: state.machine,
    activity: state.activity,
  }),
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
  useRemoteMachineEntitlement: () => ({
    data: { eligible: true, free_slot_available: state.slotAvailable },
    error: state.slotError ? new Error("Unavailable") : undefined,
  }),
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
  useRemoteMachineOperation: () => ({ data: state.operation }),
}));
vi.mock("@/components/machines/remote-machine-create", () => ({
  RemoteMachineCreate: ({ open }: { open: boolean }) =>
    open ? <p>Review remote machine preflight</p> : null,
}));
vi.mock("@/components/machines/remote-machine-detail", () => ({
  RemoteMachineDetail: ({
    machineId,
    onClose,
  }: {
    machineId: string;
    onClose?: () => void;
  }) => (
    <div>
      <p>Hosted details for {machineId}</p>
      <button onClick={onClose}>Back to machines</button>
    </div>
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
  state.activity = undefined;
  sessionStorage.clear();
  state.operation = undefined;
  remoteMachines[0]!.resource_state = "reconciling";
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

it.each([
  ["pause", "stopping", "ready", "Pausing…"],
  ["resume", "resuming", "paused", "Resuming…"],
] as const)("keeps failed %s feedback while list actions follow current resource eligibility", (action, transitional, eligible, label) => {
  Object.assign(remoteMachines[0]!, { resource_state: transitional, active_operation_id: "failed-b", revision: 6 });
  state.operation = { ...remoteMachineOperationFixture, id: "failed-b", machine_id: "hosted-machine", action, state: "failed", terminal_at: "2026-10-29T00:00:00Z" };
  const view = show();
  const card = within(screen.getByRole("article", { name: "Remote work" }));
  expect(card.getByText(/operation needs attention/)).toBeVisible();
  expect(card.getByRole("button", { name: label })).toBeDisabled();
  remoteMachines[0]!.resource_state = eligible;
  view.rerender(<TooltipProvider><MachinesPage /></TooltipProvider>);
  expect(card.getByRole("button", { name: action === "pause" ? "Pause" : "Resume" })).toBeEnabled();
});

it.each([
  ["create", "paused", "ready", /Setting up your machine/],
  ["pause", "ready", "paused", /Pausing your machine/],
  ["resume", "paused", "ready", /Resuming your machine/],
  ["delete", "ready", "deleted", /Deleting your machine/],
] as const)("keeps another client's succeeded %s pending on the list until its resource result", (action, before, final, message) => {
  const intent = getRemoteMachineIntent("me", "hosted-machine:pause", { action: "pause", expected_revision: 4 });
  acceptRemoteMachineIntent("me", "hosted-machine", "pause", intent.idempotencyKey, "accepted-a");
  observeAcceptedRemoteMachineIntent("me", "hosted-machine", "accepted-a", 5);
  Object.assign(remoteMachines[0]!, { resource_state: before, active_operation_id: "other-client-b", revision: 6 });
  state.operation = { ...remoteMachineOperationFixture, id: "other-client-b", machine_id: "hosted-machine", action, state: "succeeded", terminal_at: "2026-10-29T00:00:00Z" };
  const view = show();
  const card = within(screen.getByRole("article", { name: "Remote work" }));
  expect(card.getByRole("button", { name: before === "paused" ? "Resume" : "Pause" })).toBeDisabled();
  expect(card.getByText(message)).toBeVisible();
  view.unmount();
  const reopened = show();
  expect(within(screen.getByRole("article", { name: "Remote work" })).getByText(message)).toBeVisible();
  Object.assign(remoteMachines[0]!, { resource_state: final, revision: 7 });
  reopened.rerender(<TooltipProvider><MachinesPage /></TooltipProvider>);
  expect(screen.queryByText(message)).not.toBeInTheDocument();
});

it.each(["pause", "resume", "delete"] as const)("keeps accepted %s on the list while resource reads lag across close/remount", async (action) => {
  remoteMachines[0]!.resource_state = action === "resume" ? "paused" : "ready";
  Object.assign(remoteMachines[0]!, { active_operation_id: "old-operation", revision: 4 });
  const intent = getRemoteMachineIntent("me", `hosted-machine:${action}`, { action, expected_revision: 4 });
  acceptRemoteMachineIntent("me", "hosted-machine", action, intent.idempotencyKey, "new-operation");
  const text = action === "pause" ? /Pausing your machine/ : action === "resume" ? /Resuming your machine/ : /Deleting your machine/;
  const view = show();
  const card = within(screen.getByRole("article", { name: "Remote work" }));
  expect(card.getByText(text)).toBeVisible();
  await userEvent.click(card.getByRole("button", { name: "Settings for Remote work" }));
  await userEvent.click(screen.getByRole("button", { name: "Back to machines" }));
  expect(card.getByText(text)).toBeVisible();
  view.unmount();
  show();
  expect(within(screen.getByRole("article", { name: "Remote work" })).getByText(text)).toBeVisible();
  act(() => useAuthStore.setState({ user: { ...auth.user!, id: "other" } }));
  expect(screen.queryByText(text)).not.toBeInTheDocument();
});

it("keeps an unresolved lifecycle request on the list after details close and remount without leaking another actor's intent", async () => {
  remoteMachines[0]!.resource_state = "ready";
  const request = getRemoteMachineIntent("me", "hosted-machine:pause", {
    action: "pause",
    expected_revision: 4,
  });
  getRemoteMachineIntent("other", "hosted-machine:delete", {
    action: "delete",
    expected_revision: 9,
  });
  const view = show();
  const card = within(screen.getByRole("article", { name: "Remote work" }));
  expect(
    card.getByText(/previous pause request has an unconfirmed result/),
  ).toBeVisible();
  expect(card.queryByText(/previous delete request/)).not.toBeInTheDocument();
  await userEvent.click(
    card.getByRole("button", { name: "Settings for Remote work" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Back to machines" }),
  );
  expect(
    card.getByText(/previous pause request has an unconfirmed result/),
  ).toBeVisible();
  view.unmount();
  show();
  expect(
    within(screen.getByRole("article", { name: "Remote work" })).getByText(
      /previous pause request has an unconfirmed result/,
    ),
  ).toBeVisible();
  expect(readRemoteMachineIntent("me", "hosted-machine:pause")).toEqual(
    request,
  );
  act(() =>
    useAuthStore.setState({
      user: { ...useAuthStore.getState().user!, id: "actor-without-intents" },
    }),
  );
  expect(
    screen.queryByText(/previous pause request has an unconfirmed result/),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText(/previous delete request has an unconfirmed result/),
  ).not.toBeInTheDocument();
});

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
  expect(screen.getAllByText("reconciling").length).toBeGreaterThan(0);
  expect(
    screen.getByText(/Waiting for platform confirmation/),
  ).toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("button", { name: "Settings for Remote work" }),
  );
  expect(
    screen.getByText("Hosted details for hosted-machine"),
  ).toBeInTheDocument();
  expect(state.navigate).toHaveBeenCalledWith({
    to: "/assistant/machines",
    search: { machine: "hosted-machine", activity: undefined },
  });
});

it("resolves a deleted machine deep link after it leaves the active list", () => {
  state.machine = "deleted-machine";
  show();
  expect(
    screen.getByText("Hosted details for deleted-machine"),
  ).toBeInTheDocument();
});

it("returns from hosted detail to the machine list and clears the deep-link selection", async () => {
  state.machine = "hosted-machine";
  show();
  await userEvent.click(
    screen.getByRole("button", { name: "Back to machines" }),
  );
  expect(state.navigate).toHaveBeenLastCalledWith({
    to: "/assistant/machines",
    search: {},
  });
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
    screen.getByRole("button", { name: "Settings for Remote work" }),
  );
  expect(
    screen.getByText("Hosted details for hosted-machine"),
  ).toBeInTheDocument();
});

vi.mock("@/components/shared/machine-activity", () => ({
  MachineActivity: ({ nodeId }: { nodeId: string }) => (
    <p>Tool history for {nodeId}</p>
  ),
}));
vi.mock("@/components/machines/remote-machine-activity", () => ({
  RemoteMachineActivity: ({
    machineId,
    initialTab,
  }: {
    machineId: string;
    initialTab?: string;
  }) => (
    <p>
      Remote {initialTab ?? "tools"} history for {machineId}
    </p>
  ),
}));
it("uses one sheet for self-hosted tools and Remote activity and preserves event deep links", async () => {
  const view = show();
  await userEvent.click(
    screen.getByRole("button", { name: "Activity for My VM" }),
  );
  expect(screen.getAllByRole("dialog")).toHaveLength(1);
  expect(screen.getByText("Tool history for machine")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Close" }));
  await userEvent.click(
    screen.getByRole("button", { name: "Activity for Remote work" }),
  );
  expect(screen.getAllByRole("dialog")).toHaveLength(1);
  expect(
    screen.getByText("Remote tools history for hosted-machine"),
  ).toBeInTheDocument();
  state.machine = "hosted-machine";
  state.activity = "events";
  view.rerender(
    <TooltipProvider>
      <MachinesPage />
    </TooltipProvider>,
  );
  expect(
    screen.getByText("Remote events history for hosted-machine"),
  ).toBeInTheDocument();
});

it("shows authoritative free-slot availability without claiming availability on a failed read", () => {
  state.slotAvailable = false;
  const view = show();
  expect(
    screen.getByRole("region", { name: "Personal free slot" }),
  ).toHaveTextContent("1 of 1 in use");
  state.slotAvailable = true;
  view.rerender(
    <TooltipProvider>
      <MachinesPage />
    </TooltipProvider>,
  );
  expect(
    screen.getByRole("region", { name: "Personal free slot" }),
  ).toHaveTextContent("Available · 0 of 1");
  state.slotError = true;
  view.rerender(
    <TooltipProvider>
      <MachinesPage />
    </TooltipProvider>,
  );
  expect(
    screen.getByRole("region", { name: "Personal free slot" }),
  ).toHaveTextContent("Availability not confirmed");
  expect(
    screen.getByRole("region", { name: "Personal free slot" }),
  ).not.toHaveTextContent("Available · 0 of 1");
  state.slotAvailable = false;
  state.slotError = false;
});

it("keeps an explicitly unreported safe Remote version unknown instead of reviving stale node metadata", () => {
  const remote = remoteMachines[0]!;
  const hostedNode = {
    ...base,
    id: remote.node_id,
    metadata: {
      agent_version: "stale-version",
      os: null,
      arch: null,
      ip_address: null,
    },
  };
  nodes.push(hostedNode);
  Object.assign(remote, {
    presentation: {
      agent_version: null,
      connected: false,
      desktop_ready: false,
      capabilities: [],
      roots: [],
      os: null,
      arch: null,
    },
  });
  try {
    show();
    const card = within(screen.getByRole("article", { name: remote.name }));
    expect(card.getByText("Version not reported")).toBeInTheDocument();
    expect(card.queryByText("vstale-version")).not.toBeInTheDocument();
  } finally {
    Reflect.deleteProperty(remote, "presentation");
    nodes.splice(nodes.indexOf(hostedNode), 1);
  }
});
