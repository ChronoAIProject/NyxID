import {
  act,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import type {
  RemoteMachineEvent,
  RemoteMachineOperation,
  RemoteMachineRuntimeBilling,
} from "@/schemas/machines";
import type { NodeInfo } from "@/types/nodes";
import { useAuthStore } from "@/stores/auth-store";
import { RemoteMachineDetail } from "./remote-machine-detail";
import {
  remoteMachineOperationFixture,
  remoteMachinePaidQuoteFixture,
} from "@/test/remote-machine-fixture";

const fixture = vi.hoisted(() => ({
  machine: {
    node_id: "11111111-1111-4111-8111-111111111111",
    policy_version: "remote-machine-mvp" as const,
    data_policy_version: "remote-machine-data" as const,
    profile: {
      profile_version: "remote-machine",
      cpu_millis: 2000,
      memory_mib: 2048,
      workspace_gib: 20,
      ephemeral_gib: null,
    },
    sandbox_namespace: "nyxid-machine",
    sandbox_name: "machine-one",
    sandbox_uid: null,
    pvc_name: null,
    pvc_uid: null,
    pod_uid: null,
    resource_state: "paused",
    billing_state: "free",
    authorization_state: "revoked",
    revision: 5,
    runtime_generation: 2,
    credential_epoch: 3,
    active_operation_id: null,
    cleanup_state: "not_required",
    last_heartbeat_at: null,
    updated_at: "2026-10-14T08:00:00Z",
  },
  events: [] as RemoteMachineEvent[],
  error: undefined as Error | undefined,
  action: vi.fn(),
  refetch: vi.fn(),
  quote: vi.fn(),
  operation: undefined as RemoteMachineOperation | undefined,
  historyOperation: undefined as RemoteMachineOperation | undefined,
  operationRefetch: vi.fn(),
  billing: undefined as RemoteMachineRuntimeBilling | undefined,
  billingError: undefined as Error | undefined,
  billingRead: vi.fn(),
  node: undefined as NodeInfo | undefined,
  nodeError: undefined as Error | undefined,
  loading: false,
  missing: false,
  retryDelete: vi.fn(),
  retryPending: false,
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    params,
    children,
    className,
  }: {
    to: string;
    params?: { nodeId: string };
    className?: string;
    children: React.ReactNode;
  }) => (
    <a href={to.replace("$nodeId", params?.nodeId ?? "")} className={className}>
      {children}
    </a>
  ),
}));
vi.mock("@/hooks/use-machines", () => ({
  useRemoteMachineQuote: () => ({
    mutate: fixture.quote,
    data: undefined,
    error: null,
  }),
  useRemoteMachine: () => ({
    data: fixture.missing ? undefined : fixture.machine,
    error: fixture.error,
    isPending: fixture.loading,
    refetch: fixture.refetch,
  }),
  useRemoteMachineOperation: (id?: string) => ({
    data:
      id === fixture.operation?.id
        ? fixture.operation
        : id === fixture.historyOperation?.id
          ? fixture.historyOperation
          : undefined,
    isPending: false,
    refetch: fixture.operationRefetch,
  }),
  useRemoteMachineRuntimeBilling: (id?: string, runId?: string) => {
    fixture.billingRead(id, runId);
    return {
      data: fixture.billing,
      error: fixture.billingError,
      isPending: false,
    };
  },
  useRemoteMachineEntitlement: () => ({
    data: { policy: remoteMachinePaidQuoteFixture.policy },
  }),
  useRetryRemoteMachineDelete: () => ({
    mutateAsync: fixture.retryDelete,
    isPending: fixture.retryPending,
  }),
  useRemoteMachineEvents: () => ({
    data: { events: fixture.events, next_cursor: null },
    isPending: false,
    refetch: vi.fn(),
  }),
  useRemoteMachineAction: () => ({
    mutateAsync: fixture.action,
    isPending: false,
  }),
}));
vi.mock("@/hooks/use-nodes", () => ({
  useNode: () => ({
    data: fixture.node,
    error: fixture.nodeError,
    isPending: false,
    refetch: vi.fn(),
  }),
}));
vi.mock("@/components/machines/remote-machine-jobs", () => ({
  RemoteMachineJobs: () => <p>Job panel</p>,
}));
vi.mock("@/components/machines/remote-machine-billing", () => ({
  RemoteMachineBilling: () => <p>Billing panel</p>,
}));
vi.mock("@/components/machines/remote-machine-confirmations", () => ({
  RemoteMachineConfirmations: () => <p>Confirmation panel</p>,
}));
vi.mock("@/components/shared/machine-grant-list", () => ({
  MachineGrantList: () => <p>External grant list</p>,
}));
vi.mock("@/components/machines/remote-machine-paid-start", () => ({
  RemoteMachinePaidStart: ({ open }: { open: boolean }) =>
    open ? <p>Paid quote acceptance review</p> : null,
}));
vi.mock("@/components/machines/remote-machine-usage", () => ({
  RemoteMachineUsage: () => (
    <p>Workspace usage is not reported by the fixture.</p>
  ),
}));
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({ data: { mcp_url: "https://local.test/mcp" } }),
}));

beforeEach(() => {
  fixture.retryDelete.mockReset();
  fixture.action.mockReset();
  fixture.refetch.mockReset();
  fixture.operationRefetch.mockReset();
  useAuthStore.setState({
    user: {
      id: "owner",
      capabilities: { enabled_features: ["experimental:remote-machines"] },
    } as never,
    isAuthenticated: true,
    isLoading: false,
  });
  sessionStorage.clear();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  fixture.machine.resource_state = "paused";
  fixture.machine.billing_state = "free";
  fixture.machine.authorization_state = "revoked";
  fixture.machine.revision = 5;
  Reflect.deleteProperty(fixture.machine, "free_slot");
  Reflect.deleteProperty(fixture.machine, "runtime_run_id");
  Reflect.deleteProperty(fixture.machine, "funded_until");
  fixture.events = [];
  fixture.error = undefined;
  fixture.operation = undefined;
  fixture.historyOperation = undefined;
  fixture.billing = undefined;
  fixture.billingError = undefined;
  fixture.node = undefined;
  fixture.nodeError = undefined;
  fixture.loading = false;
  fixture.missing = false;
  fixture.retryPending = false;
  fixture.machine.active_operation_id = null;
  fixture.machine.cleanup_state = "not_required";
  for (const key of [
    "name",
    "capabilities",
    "specialist_id",
    "specialist_assignment_state",
    "specialist_assignment_error",
  ])
    Reflect.deleteProperty(fixture.machine, key);
});

function stoppedDelete() {
  Object.assign(fixture.machine, {
    resource_state: "deleting",
    active_operation_id: "operation-1",
  });
  fixture.operation = {
    ...remoteMachineOperationFixture,
    machine_id: fixture.machine.node_id,
    action: "delete",
    state: "reconciling",
    cleanup_state: "pending",
    retry_count: 15639,
    delete_retry_attempts: 10,
    delete_retry_limit: 10,
    delete_retry_stopped: true,
  };
}

it("offers a manual retry of the same exhausted delete and preserves the cumulative count", async () => {
  stoppedDelete();
  fixture.retryDelete.mockResolvedValueOnce({ operation: fixture.operation });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(
    screen.getByText(/Automatic deletion attempts stopped/),
  ).toBeInTheDocument();
  expect(screen.getByText("15639")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  await waitFor(() => expect(fixture.retryDelete).toHaveBeenCalledTimes(1));
  expect(fixture.retryDelete).toHaveBeenCalledWith(
    expect.objectContaining({
      machineId: fixture.machine.node_id,
      operationId: "operation-1",
      body: { expected_revision: 5, expected_retry_count: 15639 },
      idempotencyKey: expect.any(String),
    }),
  );
  expect(fixture.action).not.toHaveBeenCalled();
});

it("disables the manual delete retry while submitting and shows a failed retry", async () => {
  stoppedDelete();
  fixture.retryPending = true;
  const view = render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} />,
  );
  expect(
    screen.getByRole("button", { name: "Retrying deletion…" }),
  ).toBeDisabled();
  fixture.retryPending = false;
  fixture.retryDelete.mockRejectedValueOnce(
    new Error("Reload the latest operation."),
  );
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  expect(
    await screen.findByText("Reload the latest operation."),
  ).toBeInTheDocument();
});

it("reuses the same manual retry request after an unknown network result", async () => {
  stoppedDelete();
  fixture.retryDelete.mockRejectedValueOnce(new Error("Connection lost"));
  fixture.retryDelete.mockResolvedValueOnce({ operation: fixture.operation });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  expect(await screen.findByText("Connection lost")).toBeInTheDocument();
  const first = fixture.retryDelete.mock.calls[0]?.[0];
  await userEvent.click(
    screen.getByRole("button", { name: "Confirm retry result" }),
  );
  await waitFor(() => expect(fixture.retryDelete).toHaveBeenCalledTimes(2));
  expect(fixture.retryDelete.mock.calls[1]?.[0]).toEqual(first);
});

it("does not offer retry for another Machine or an active automatic round", () => {
  stoppedDelete();
  fixture.operation!.delete_retry_stopped = false;
  const view = render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} />,
  );
  expect(
    screen.queryByRole("button", { name: "Retry deletion" }),
  ).not.toBeInTheDocument();
  fixture.operation!.delete_retry_stopped = true;
  fixture.operation!.machine_id = "other-machine";
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(
    screen.queryByRole("button", { name: "Retry deletion" }),
  ).not.toBeInTheDocument();
});

it("keeps resource, billing, authorization and unknown measurements distinct", () => {
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getByText("Resource state")).toBeInTheDocument();
  expect(screen.getByText("Authorization state")).toBeInTheDocument();
  expect(screen.getByText("Billing state")).toBeInTheDocument();
  expect(screen.getByText("WSS status unknown")).toBeInTheDocument();
  expect(
    screen.getByText(/Workspace usage is not reported/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/roots do not isolate shell commands/),
  ).toBeInTheDocument();
  expect(screen.queryByText("0 GiB used")).not.toBeInTheDocument();
  expect(
    within(
      screen.getByRole("group", { name: "Enabled capability badges" }),
    ).getByText("Capabilities not reported"),
  ).toBeInTheDocument();
});

it("requires reviewed irreversible data loss acknowledgement before deletion", async () => {
  fixture.action.mockResolvedValue({ operation: { id: "op-delete" } });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(
    screen.getByRole("tab", { name: "Settings and rules" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  const dialog = within(screen.getByRole("dialog"));
  const submit = screen.getByRole("button", { name: "Delete permanently" });
  expect(submit).toBeDisabled();
  expect(
    dialog.getByText(/permanently removes the workspace/),
  ).toBeInTheDocument();
  expect(dialog.getByText(/no snapshots or recovery/)).toBeInTheDocument();
  expect(
    dialog.getByText(/Terminal job metadata remains for 30 days/),
  ).toBeInTheDocument();
  expect(
    dialog.getByText(/terminal operation history for 90 days/),
  ).toBeInTheDocument();
  expect(
    dialog.getByText(
      /resource cleanup remains pending until current Kubernetes observations/,
    ),
  ).toBeInTheDocument();
  expect(
    dialog.getByText(/temporary runtime data.*HOME.*\/tmp/),
  ).toBeInTheDocument();
  await userEvent.click(screen.getByRole("checkbox", { name: /I understand/ }));
  await userEvent.click(submit);
  await waitFor(() => expect(fixture.action).toHaveBeenCalled());
  expect(fixture.action.mock.calls[0]?.[0]).toMatchObject({
    machineId: fixture.machine.node_id,
    body: {
      action: "delete",
      expected_revision: 5,
      acknowledge_data_loss: true,
      data_policy_version: "remote-machine-data" as const,
    },
  });
});

it("paid resume opens quote review without submitting a lifecycle action", async () => {
  fixture.machine.billing_state = "suspended";
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(
    screen.getByRole("button", { name: "Review quote to resume" }),
  );
  expect(screen.getByText("Paid quote acceptance review")).toBeInTheDocument();
  expect(fixture.action).not.toHaveBeenCalled();
});

it("uses explicit free-slot identity and renders only reported paid run deadlines", () => {
  Object.assign(fixture.machine, {
    free_slot: false,
    billing_state: "free",
    runtime_run_id: "run-verified",
    funded_until: "2026-10-14T08:05:00Z",
  });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(
    screen.getByRole("button", { name: "Review quote to resume" }),
  ).toBeEnabled();
  expect(screen.getByText("run-verified")).toBeInTheDocument();
  expect(screen.getByText("Current funded deadline")).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Resume machine" }),
  ).not.toBeInTheDocument();
});

it("retries an unknown result with the identical reviewed body and idempotency key", async () => {
  fixture.action
    .mockRejectedValueOnce(new Error("Network response unknown"))
    .mockResolvedValueOnce({ operation: { id: "op-delete" } });
  const view = render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} />,
  );
  await userEvent.click(
    screen.getByRole("tab", { name: "Settings and rules" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await userEvent.click(screen.getByRole("checkbox", { name: /I understand/ }));
  await userEvent.click(
    screen.getByRole("button", { name: "Delete permanently" }),
  );
  await screen.findByText("Network response unknown");
  fixture.machine.revision = 6;
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(
    screen.getByRole("button", { name: "Delete permanently" }),
  );
  await waitFor(() => expect(fixture.action).toHaveBeenCalledTimes(2));
  expect(fixture.action.mock.calls[1]?.[0]).toEqual(
    fixture.action.mock.calls[0]?.[0],
  );
});

it("a revision conflict requires reload and review instead of an automatic new action", async () => {
  fixture.action.mockRejectedValueOnce(
    new ApiError(409, {
      error: "conflict",
      error_code: 409,
      message: "Machine revision changed",
    }),
  );
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(
    screen.getByRole("tab", { name: "Settings and rules" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await userEvent.click(screen.getByRole("checkbox", { name: /I understand/ }));
  await userEvent.click(
    screen.getByRole("button", { name: "Delete permanently" }),
  );
  await screen.findByText("Machine revision changed");
  expect(
    screen.getByRole("button", { name: "Delete permanently" }),
  ).toBeDisabled();
  expect(fixture.action).toHaveBeenCalledTimes(1);
  await userEvent.click(
    screen.getByRole("button", { name: "Reload and review a new action" }),
  );
  expect(fixture.refetch).toHaveBeenCalled();
  expect(fixture.action).toHaveBeenCalledTimes(1);
});

it("deduplicates real notification events and preserves stable operation links", () => {
  const event: RemoteMachineEvent = {
    id: "event",
    machine_id: fixture.machine.node_id,
    operation_id: "op",
    kind: "cleanup_overdue",
    message: "Cleanup remains pending",
    notification: true,
    responsibility: "platform",
    created_at: "2026-10-14T08:00:00Z",
  };
  fixture.events = [event, event];
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getAllByText("Cleanup remains pending")).toHaveLength(1);
  expect(
    screen.getByRole("link", { name: "View operation op" }),
  ).toHaveAttribute(
    "href",
    `/assistant/machines?machine=${fixture.machine.node_id}&operation=op`,
  );
});

it("does not describe a completed live operation as still awaiting acceptance", () => {
  Object.assign(fixture.machine, {
    resource_state: "ready",
    authorization_state: "active",
    active_operation_id: "operation-1",
  });
  fixture.operation = {
    ...remoteMachineOperationFixture,
    id: "operation-1",
    machine_id: fixture.machine.node_id,
    action: "create",
    state: "succeeded",
  };
  fixture.events = [
    {
      id: "completed-event",
      machine_id: fixture.machine.node_id,
      operation_id: "operation-1",
      kind: "completed",
      message: "Lifecycle operation completed",
      notification: true,
      responsibility: "platform",
      created_at: "2026-10-22T03:36:32Z",
    },
  ];
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  const attention = within(
    screen.getByRole("region", { name: "Current attention" }),
  );
  expect(
    attention.getByText("Lifecycle operation completed"),
  ).toBeInTheDocument();
  expect(
    attention.queryByText(/An accepted request is awaiting/),
  ).not.toBeInTheDocument();
});

it.each(["succeeded", "failed"] as const)(
  "does not promote an old accepted event after the operation is %s",
  (state) => {
    Object.assign(fixture.machine, {
      resource_state: "ready",
      authorization_state: "active",
      active_operation_id: "operation-1",
    });
    fixture.operation = {
      ...remoteMachineOperationFixture,
      id: "operation-1",
      machine_id: fixture.machine.node_id,
      action: "create",
      state,
    };
    fixture.events = [
      {
        id: "accepted-event",
        machine_id: fixture.machine.node_id,
        operation_id: "operation-1",
        kind: "accepted",
        message: "Lifecycle operation accepted",
        notification: true,
        responsibility: "platform",
        created_at: "2026-10-22T03:36:13Z",
      },
    ];
    render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
    const attention = within(
      screen.getByRole("region", { name: "Current attention" }),
    );
    expect(
      attention.queryByText("Lifecycle operation accepted"),
    ).not.toBeInTheDocument();
    expect(
      attention.queryByText(/An accepted request is awaiting/),
    ).not.toBeInTheDocument();
  },
);

it("does not fall back to stale initial state after owner authorization fails", () => {
  fixture.error = new ApiError(403, {
    error: "forbidden",
    error_code: 403,
    message: "Access revoked",
  });
  render(
    <RemoteMachineDetail
      machineId={fixture.machine.node_id}
      initialMachine={fixture.machine as never}
    />,
  );
  expect(screen.getByText("Access revoked")).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Delete machine" }),
  ).not.toBeInTheDocument();
});

vi.mock("@/components/machines/remote-machine-service-usage", () => ({
  RemoteMachineServiceUsage: ({ machineId }: { machineId: string }) => (
    <div data-testid="machine-service-usage">{machineId}</div>
  ),
}));

it("explains native pause completion and fresh runtime resume without promising physical isolation", async () => {
  fixture.machine.resource_state = "ready";
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Pause machine" }));
  const dialog = within(screen.getByRole("dialog"));
  expect(
    dialog.getByText(
      /Pause stops the compute Pod and retains the same workspace/,
    ),
  ).toBeInTheDocument();
  expect(
    dialog.getByText(
      /confirmed only after current Sandbox and Pod observations/,
    ),
  ).toBeInTheDocument();
  expect(
    dialog.getByText(/does not restore processes, memory or old jobs/),
  ).toBeInTheDocument();
  expect(dialog.getByText(/later top-up never resumes/)).toBeInTheDocument();
});

it.each([
  ["pending", "Specialist assignment is pending"],
  ["assigned", "Specialist assigned"],
  ["permissions_required", "Specialist permissions need configuration"],
  ["failed", "Specialist assignment could not be completed"],
])(
  "shows hosted name and %s assignment with the existing permission controls",
  (state, message) => {
    Object.assign(fixture.machine, {
      name: "research-box",
      capabilities: ["shell", "files", "browser", "computer"],
      specialist_id: "specialist-1",
      specialist_assignment_state: state,
      specialist_assignment_error: "specialist_assignment_unavailable",
    });
    render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
    expect(
      screen.getByRole("heading", { name: "research-box" }),
    ).toBeInTheDocument();
    const capabilities = within(
      screen.getByRole("group", { name: "Enabled capability badges" }),
    );
    for (const label of ["Commands", "Files", "Browser", "Computer"])
      expect(capabilities.getByText(label)).toBeInTheDocument();
    expect(screen.getByText(message)).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Open specialist settings" }),
    ).toHaveAttribute("href", "/assistant/automations");
    expect(
      screen.getByRole("button", { name: "Review agent access" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("specialist_assignment_unavailable"),
    ).not.toBeInTheDocument();
  },
);

function desktopMachine(
  capabilities = ["shell", "files", "browser", "computer"],
) {
  Object.assign(fixture.machine, {
    name: "browser-machine",
    capabilities,
    resource_state: "ready",
    authorization_state: "active",
    specialist_assignment_state: "none",
  });
  return {
    id: fixture.machine.node_id,
    name: "browser-machine",
    is_connected: true,
    owner: { id: "owner", kind: "user", display_name: "Owner" },
    machine: {
      shell: capabilities.includes("shell"),
      files: capabilities.includes("files"),
      browser: capabilities.includes("browser"),
      computer: capabilities.includes("computer"),
      computer_ready: true,
    },
  } as NodeInfo;
}

it.each([
  ["Browser-only", ["browser"]],
  ["full", ["shell", "files", "browser", "computer"]],
])(
  "opens the existing owner desktop for a ready %s hosted machine",
  (_label, capabilities) => {
    const node = desktopMachine(capabilities);
    render(
      <RemoteMachineDetail machineId={fixture.machine.node_id} node={node} />,
    );
    expect(
      screen.getByRole("link", { name: "Watch / Take over desktop" }),
    ).toHaveAttribute(
      "href",
      `/assistant/machines/${fixture.machine.node_id}/desktop`,
    );
  },
);

it.each([
  ["Browser-only", ["browser"]],
  ["full", ["shell", "files", "browser", "computer"]],
])(
  "keeps the %s owner desktop reachable with the existing unavailable-display warning",
  (_label, capabilities) => {
    const node = desktopMachine(capabilities);
    render(
      <RemoteMachineDetail
        machineId={fixture.machine.node_id}
        node={{ ...node, machine: { ...node.machine!, computer_ready: false } }}
      />,
    );
    expect(
      screen.getByRole("link", { name: "Watch / Take over desktop" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Computer permissions or display unavailable"),
    ).toBeInTheDocument();
  },
);

it("disables hosted desktop navigation while the owner Node is disconnected", () => {
  const node = desktopMachine();
  render(
    <RemoteMachineDetail
      machineId={fixture.machine.node_id}
      node={{ ...node, is_connected: false }}
    />,
  );
  expect(
    screen.queryByRole("link", { name: "Watch / Take over desktop" }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Watch / Take over desktop" }),
  ).toBeDisabled();
  expect(
    screen.getByText("Desktop unavailable while this machine is disconnected."),
  ).toBeInTheDocument();
});

it.each(["provisioning", "paused", "reconciling", "deleting"])(
  "does not open a hosted desktop from stale connected state while %s",
  (resourceState) => {
    const node = desktopMachine();
    fixture.machine.resource_state = resourceState;
    render(
      <RemoteMachineDetail machineId={fixture.machine.node_id} node={node} />,
    );
    expect(
      screen.queryByRole("link", { name: "Watch / Take over desktop" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Watch / Take over desktop" }),
    ).toBeDisabled();
    expect(
      screen.getByText(
        "Desktop unavailable until this machine has an active runtime.",
      ),
    ).toBeInTheDocument();
  },
);

it.each(["pending", "revoked"])(
  "does not open the desktop while hosted runtime authorization is %s",
  (authorization) => {
    const node = desktopMachine();
    fixture.machine.authorization_state = authorization;
    render(
      <RemoteMachineDetail machineId={fixture.machine.node_id} node={node} />,
    );
    expect(
      screen.queryByRole("link", { name: "Watch / Take over desktop" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Watch / Take over desktop" }),
    ).toBeDisabled();
  },
);

it.each([
  "missing",
  "other owner",
  "other machine",
  "live capabilities unavailable",
])("does not infer owner desktop access when the Node is %s", (condition) => {
  let node: NodeInfo | undefined = desktopMachine();
  if (condition === "missing") node = undefined;
  else if (condition === "other owner")
    node = { ...node, owner: { ...node.owner, id: "other" } };
  else if (condition === "other machine")
    node = { ...node, id: "different-node" };
  else
    node = {
      ...node,
      machine: { ...node.machine!, computer: false, browser: false },
    };
  render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} node={node} />,
  );
  expect(
    screen.queryByRole("link", { name: "Watch / Take over desktop" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Watch / Take over desktop" }),
  ).not.toBeInTheDocument();
});

it("removes the desktop entry when current owner authorization cannot be loaded", () => {
  const node = desktopMachine();
  fixture.error = new ApiError(403, {
    error: "forbidden",
    error_code: 403,
    message: "Access revoked",
  });
  render(
    <RemoteMachineDetail
      machineId={fixture.machine.node_id}
      node={node}
      initialMachine={fixture.machine as never}
    />,
  );
  expect(screen.getByText("Access revoked")).toBeInTheDocument();
  expect(
    screen.queryByRole("link", { name: "Watch / Take over desktop" }),
  ).not.toBeInTheDocument();
});

it.each([
  ["Browser-only", ["browser"], ["Browser"]],
  [
    "full",
    ["shell", "files", "browser", "computer"],
    ["Commands", "Files", "Browser", "Computer"],
  ],
  ["Commands and Files", ["shell", "files"], ["Commands", "Files"]],
])(
  "shows saved %s capability badges and full-image policy without implying grants",
  async (_label, capabilities, labels) => {
    const node = desktopMachine(capabilities);
    render(
      <RemoteMachineDetail machineId={fixture.machine.node_id} node={node} />,
    );
    const badges = within(
      screen.getByRole("group", { name: "Enabled capability badges" }),
    );
    for (const label of ["Commands", "Files", "Browser", "Computer"]) {
      if (labels.includes(label))
        expect(badges.getByText(label)).toBeInTheDocument();
      else expect(badges.queryByText(label)).not.toBeInTheDocument();
    }
    expect(screen.queryByText("Shell and files")).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("tab", { name: "Settings and rules" }),
    );
    const policy = within(screen.getByRole("tabpanel"));
    expect(
      policy.queryByText(
        /Shell and files only|profile has no desktop, browser/,
      ),
    ).not.toBeInTheDocument();
    expect(
      policy.getByText(
        /Full Machine image includes Chromium, desktop and computer tooling/,
      ),
    ).toBeInTheDocument();
    expect(
      policy.getByText(/Only the selected capabilities are enabled/),
    ).toBeInTheDocument();
    expect(
      policy.getByText(
        /External OAuth and Agent Key tools remain limited to authorized Commands and Files/,
      ),
    ).toBeInTheDocument();
    expect(
      policy.getByText(
        /All authorized clients share the Machine and workspace/,
      ),
    ).toBeInTheDocument();
    expect(
      policy.getByText(/Authorized agents also share browser sessions/),
    ).toBeInTheDocument();
    expect(
      policy.getByText(
        /Only \/workspace persists.*browser profiles and downloads are temporary/,
      ),
    ).toBeInTheDocument();
    expect(
      policy.getByText(/Only the human owner can watch or take over/),
    ).toBeInTheDocument();
    expect(
      policy.getByText(
        /Paused workspaces have no automatic expiry and no storage fee/,
      ),
    ).toBeInTheDocument();
    expect(
      policy.getByText(/Job metadata retention: 30 days/),
    ).toBeInTheDocument();
  },
);

it("discloses unknown temporary capacity and disk pressure in detail and policy views", async () => {
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getByText("No fixed capacity declared")).toBeInTheDocument();
  expect(
    screen.getByText(/Disk pressure can evict the Pod/),
  ).toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("tab", { name: "Settings and rules" }),
  );
  expect(screen.getByText("No fixed capacity declared")).toBeInTheDocument();
});

it("shows the current profile with undeclared temporary capacity in both detail views", async () => {
  const oldProfile = fixture.machine.profile;
  const oldPolicy = fixture.machine.policy_version;
  Object.assign(fixture.machine, {
    profile: {
      ...oldProfile,
      profile_version: "remote-machine",
      ephemeral_gib: null,
    },
    policy_version: "remote-machine-mvp",
  });
  try {
    render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
    expect(screen.getByText("No fixed capacity declared")).toBeInTheDocument();
    expect(
      screen.queryByText(/null GiB|undefined GiB|2 GiB temporary/),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(
        /Temporary storage is managed by the deployment environment/,
      ),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("tab", { name: "Settings and rules" }),
    );
    expect(screen.getByText("No fixed capacity declared")).toBeInTheDocument();
  } finally {
    fixture.machine.profile = oldProfile;
    fixture.machine.policy_version = oldPolicy;
  }
});

it("resumes a current free Machine with explicit saved policy acceptance", async () => {
  fixture.action.mockResolvedValue({ operation: { id: "op-legacy-resume" } });
  render(
    <RemoteMachineDetail
      machineId={fixture.machine.node_id}
      policy={remoteMachinePaidQuoteFixture.policy}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Resume machine" }));
  const dialog = within(screen.getByRole("dialog"));
  expect(dialog.getByText(/remote-machine-mvp/)).toBeInTheDocument();
  expect(fixture.quote).toHaveBeenCalledWith({
    free_slot: true,
    machine_id: fixture.machine.node_id,
    profile_version: fixture.machine.profile.profile_version,
  });
  await userEvent.click(dialog.getByRole("checkbox"));
  await userEvent.click(dialog.getByRole("button", { name: "Resume machine" }));
  await waitFor(() => expect(fixture.action).toHaveBeenCalledTimes(1));
  expect(fixture.action.mock.calls[0]![0].body).toMatchObject({
    action: "resume",
    profile_version: "remote-machine",
    policy_version: "remote-machine-mvp" as const,
    data_policy_version: "remote-machine-data" as const,
  });
});

// useFeature still reads the actual auth store; its inactive org query is a double.
vi.mock("@/hooks/use-orgs", () => ({ useOrg: () => ({ data: undefined }) }));

it.each(["off", "loading", "account"])(
  "closes a free resume review on %s and keeps delete available",
  async (change) => {
    render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
    await userEvent.click(
      screen.getByRole("button", { name: "Resume machine" }),
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(fixture.quote).toHaveBeenCalledTimes(1);
    act(() =>
      useAuthStore.setState(
        change === "loading"
          ? { isLoading: true }
          : {
              user: {
                id: change === "account" ? "another-owner" : "owner",
                capabilities: { enabled_features: [] },
              } as never,
            },
      ),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Resume machine" }),
    ).toBeDisabled();
    await userEvent.click(
      screen.getByRole("tab", { name: "Settings and rules" }),
    );
    expect(
      screen.getByRole("button", { name: "Delete machine" }),
    ).toBeEnabled();
    expect(fixture.quote).toHaveBeenCalledTimes(1);
    expect(fixture.action).not.toHaveBeenCalled();
  },
);
it("keeps owner pause and delete actionable when the flag is OFF", async () => {
  fixture.machine.resource_state = "ready";
  useAuthStore.setState({
    user: { id: "owner", capabilities: { enabled_features: [] } } as never,
  });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getByRole("button", { name: "Pause machine" })).toBeEnabled();
  await userEvent.click(
    screen.getByRole("tab", { name: "Settings and rules" }),
  );
  expect(screen.getByRole("button", { name: "Delete machine" })).toBeEnabled();
  expect(fixture.quote).not.toHaveBeenCalled();
});

it("keeps a retry conflict blocked through polling until the owner refreshes and reviews", async () => {
  stoppedDelete();
  fixture.retryDelete.mockRejectedValueOnce(
    new ApiError(409, {
      error: "conflict",
      error_code: 409,
      message: "Deletion basis changed",
    }),
  );
  const view = render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  await screen.findByText("Deletion basis changed");
  fixture.machine.revision = 9;
  fixture.operation = { ...fixture.operation!, retry_count: 15640 };
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getByRole("button", { name: "Retry deletion" })).toBeDisabled();
  expect(fixture.retryDelete).toHaveBeenCalledTimes(1);
  await userEvent.click(
    screen.getByRole("button", { name: "Refresh and review deletion" }),
  );
  expect(fixture.refetch).toHaveBeenCalled();
  expect(fixture.operationRefetch).toHaveBeenCalled();
  expect(fixture.retryDelete).toHaveBeenCalledTimes(1);
  fixture.retryDelete.mockResolvedValueOnce({
    operation: {
      ...fixture.operation!,
      delete_retry_stopped: false,
      delete_retry_attempts: 1,
      retry_count: 15641,
    },
  });
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  await waitFor(() => expect(fixture.retryDelete).toHaveBeenCalledTimes(2));
  expect(fixture.retryDelete.mock.calls[1]?.[0].body).toEqual({
    expected_revision: 9,
    expected_retry_count: 15640,
  });
});

it("shows acceptance with actual returned counts and refetches both machine and original operation", async () => {
  stoppedDelete();
  fixture.retryDelete.mockResolvedValueOnce({
    operation: {
      ...fixture.operation!,
      retry_count: 15641,
      delete_retry_attempts: 2,
      delete_retry_limit: 12,
      delete_retry_stopped: false,
    },
  });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  expect(
    await screen.findByText(/Deletion retry request accepted/),
  ).toBeInTheDocument();
  expect(screen.getByText("2 / 12")).toBeInTheDocument();
  expect(screen.getByText("15641")).toBeInTheDocument();
  expect(screen.queryByText("0 / 10")).not.toBeInTheDocument();
  expect(fixture.refetch).toHaveBeenCalled();
  expect(fixture.operationRefetch).toHaveBeenCalled();
  expect(
    screen.queryByRole("button", { name: "Retry deletion" }),
  ).not.toBeInTheDocument();
});

it("labels an unknown retry outcome and retains its request when the revision changes", async () => {
  stoppedDelete();
  fixture.retryDelete.mockRejectedValueOnce(new Error("Connection lost"));
  const view = render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  expect(
    await screen.findByText(/Deletion retry result unknown/),
  ).toBeInTheDocument();
  const request = fixture.retryDelete.mock.calls[0]?.[0];
  fixture.machine.revision = 12;
  fixture.operation = { ...fixture.operation!, retry_count: 15640 };
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  fixture.retryDelete.mockResolvedValueOnce({ operation: fixture.operation });
  await userEvent.click(
    screen.getByRole("button", { name: "Confirm retry result" }),
  );
  expect(fixture.retryDelete.mock.calls[1]?.[0]).toEqual(request);
});

it("shows overdue, a specific platform diagnosis and stopped budget independently", () => {
  stoppedDelete();
  fixture.operation = {
    ...fixture.operation!,
    deadline_at: "2020-01-01T00:00:00Z",
    error_code: "delete_kubernetes_access_denied",
    error_message: "Platform access was denied.",
  };
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getByText(/Cleanup target exceeded/)).toBeInTheDocument();
  expect(
    screen.getByText("delete_kubernetes_access_denied"),
  ).toBeInTheDocument();
  expect(screen.getByText("Platform access was denied.")).toBeInTheDocument();
  expect(
    screen.getByText(/Automatic deletion attempts stopped/),
  ).toBeInTheDocument();
  expect(screen.getByText(/Retrying does not guarantee/)).toBeInTheDocument();
});

it("reports absent diagnostics without inventing a cleanup error code", () => {
  stoppedDelete();
  fixture.operation!.deadline_at = "2020-01-01T00:00:00Z";
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getByText("No specific cause reported")).toBeInTheDocument();
  expect(screen.queryByText("cleanup_overdue")).not.toBeInTheDocument();
});

it("preserves deleted finality during supplemental cleanup without reopening retry or deletion", async () => {
  stoppedDelete();
  fixture.machine.resource_state = "deleted";
  fixture.machine.cleanup_state = "pending";
  fixture.operation = {
    ...fixture.operation!,
    state: "succeeded",
    terminal_at: "2026-10-20T00:00:00Z",
  };
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(
    screen.getByText(/Background supplemental cleanup/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/original deletion remains successful/),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Retry deletion" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText(/Automatic deletion attempts stopped/),
  ).not.toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("tab", { name: "Settings and rules" }),
  );
  expect(
    screen.queryByRole("button", { name: "Delete machine" }),
  ).not.toBeInTheDocument();
});

it("opens a linked historical operation separately from current deletion progress", async () => {
  stoppedDelete();
  fixture.historyOperation = {
    ...remoteMachineOperationFixture,
    id: "old-operation",
    machine_id: fixture.machine.node_id,
    state: "succeeded",
    terminal_at: "2026-10-01T00:00:00Z",
  };
  render(
    <RemoteMachineDetail
      machineId={fixture.machine.node_id}
      operationId="old-operation"
    />,
  );
  const history = within(
    screen.getByRole("dialog", { name: "Historical operation" }),
  );
  expect(history.getByText("old-operation")).toBeInTheDocument();
  expect(
    history.queryByRole("button", { name: "Retry deletion" }),
  ).not.toBeInTheDocument();
  await userEvent.click(history.getByRole("button", { name: "Close" }));
  expect(screen.getByText("operation-1")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Retry deletion" })).toBeEnabled();
});

it("puts deletion in settings and reads the confirmation policy from Node detail", async () => {
  fixture.node = { ...desktopMachine(), machine_confirm: "changes" };
  render(
    <RemoteMachineDetail
      machineId={fixture.machine.node_id}
      node={fixture.node}
    />,
  );
  expect(screen.getAllByRole("tab")).toHaveLength(7);
  expect(
    screen.queryByRole("button", { name: "Delete machine" }),
  ).not.toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("tab", { name: "Settings and rules" }),
  );
  expect(screen.getByText("Confirm changes")).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: /Save/ }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Delete machine" })).toBeEnabled();
});

it("keeps billing finality unknown on an independent billing read failure after object cleanup", () => {
  stoppedDelete();
  Object.assign(fixture.machine, {
    free_slot: false,
    runtime_run_id: "run-current",
    cleanup_state: "complete",
    billing_state: "closed",
  });
  fixture.billingError = new Error("Billing unavailable");
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(fixture.billingRead).toHaveBeenCalledWith(
    fixture.machine.node_id,
    "run-current",
  );
  expect(
    screen.getAllByText(/Accounting finality unknown/).length,
  ).toBeGreaterThan(0);
  expect(
    screen.queryByText(/Awaiting accounting closure/),
  ).not.toBeInTheDocument();
  expect(
    screen.getByText(/Platform object cleanup confirmed/),
  ).toBeInTheDocument();
});

it("shows loaded event counts and event detail without replacing the active operation", async () => {
  stoppedDelete();
  fixture.events = [
    {
      id: "event-detail",
      machine_id: fixture.machine.node_id,
      operation_id: "old-operation",
      kind: "accepted",
      message: "Pause accepted",
      notification: false,
      responsibility: "platform",
      created_at: "2026-10-01T00:00:00Z",
    },
  ];
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("tab", { name: "Events" }));
  expect(screen.getByText("Loaded 1 event on this page")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Event details" }));
  expect(
    within(screen.getByRole("dialog", { name: "Event details" })).getByText(
      "old-operation",
    ),
  ).toBeInTheDocument();
});

it("renders the loading and missing-detail states without lifecycle actions", () => {
  fixture.missing = true;
  fixture.loading = true;
  const view = render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} />,
  );
  expect(
    screen.getByRole("status", { name: "Loading remote machine" }),
  ).toBeInTheDocument();
  fixture.loading = false;
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getByText("Remote machine not found.")).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Pause machine" }),
  ).not.toBeInTheDocument();
});

it("distinguishes a definite retry rejection and permits a newly keyed reviewed request", async () => {
  stoppedDelete();
  fixture.retryDelete.mockRejectedValueOnce(
    new ApiError(400, {
      error: "bad_request",
      error_code: 400,
      message: "Retry was rejected",
    }),
  );
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  expect(await screen.findByText("Retry was rejected")).toBeVisible();
  expect(
    screen.queryByText(/Deletion retry result unknown/),
  ).not.toBeInTheDocument();
  expect(screen.getByText("15639")).toBeInTheDocument();
  fixture.retryDelete.mockResolvedValueOnce({
    operation: { ...fixture.operation!, delete_retry_stopped: false },
  });
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  const [first, second] = fixture.retryDelete.mock.calls.map((call) => call[0]);
  expect(second.body).toEqual(first.body);
  expect(second.idempotencyKey).not.toEqual(first.idempotencyKey);
});

it("requires another review if a confirmed replay has already exhausted its renewed round", async () => {
  stoppedDelete();
  fixture.retryDelete.mockResolvedValueOnce({
    operation: { ...fixture.operation!, retry_count: 15649 },
  });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  await screen.findByText(/Deletion retry request accepted/);
  expect(
    screen.queryByRole("button", { name: "Retry deletion" }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Review latest deletion" }),
  ).toBeEnabled();
});

it("does not unlock a retry conflict when either refreshed record fails", async () => {
  stoppedDelete();
  fixture.retryDelete.mockRejectedValueOnce(
    new ApiError(409, {
      error: "conflict",
      error_code: 409,
      message: "Revision changed",
    }),
  );
  fixture.operationRefetch.mockResolvedValueOnce({
    error: new Error("read failed"),
  });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  await screen.findByText("Revision changed");
  await userEvent.click(
    screen.getByRole("button", { name: "Refresh and review deletion" }),
  );
  expect(await screen.findByText(/could not both be read/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Retry deletion" })).toBeDisabled();
  expect(fixture.retryDelete).toHaveBeenCalledTimes(1);
});

it("displays the server retry limit and schedule without an invented limit or countdown", () => {
  stoppedDelete();
  fixture.operation = {
    ...fixture.operation!,
    delete_retry_stopped: false,
    delete_retry_attempts: 4,
    delete_retry_limit: 12,
    next_attempt_at: "2099-01-01T00:03:00Z",
  };
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getByText("4 / 12")).toBeInTheDocument();
  expect(
    screen.getByText(new Date("2099-01-01T00:03:00Z").toLocaleString()),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Retry deletion" }),
  ).not.toBeInTheDocument();
});

it("never treats a terminal deletion with missing terminal_at as retryable", () => {
  stoppedDelete();
  fixture.operation!.state = "succeeded";
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(
    screen.queryByRole("button", { name: "Retry deletion" }),
  ).not.toBeInTheDocument();
});

it("shows confirmation-read failures without manufacturing a policy", async () => {
  fixture.nodeError = new Error("Node detail unavailable");
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(
    screen.getByRole("tab", { name: "Settings and rules" }),
  );
  expect(
    screen.getByText(/Confirmation policy unavailable/),
  ).toBeInTheDocument();
  expect(screen.queryByText("Confirm changes")).not.toBeInTheDocument();
});

it("recovers the identical unknown retry after remount even if the lifetime count advanced", async () => {
  stoppedDelete();
  fixture.retryDelete.mockRejectedValueOnce(new Error("Reply lost"));
  const view = render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  await screen.findByText(/Deletion retry result unknown/);
  const first = fixture.retryDelete.mock.calls[0]?.[0];
  view.unmount();
  fixture.machine.revision = 20;
  fixture.operation = { ...fixture.operation!, retry_count: 15649 };
  fixture.retryDelete.mockResolvedValueOnce({ operation: fixture.operation });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(
    screen.getByRole("button", { name: "Confirm retry result" }),
  );
  expect(fixture.retryDelete.mock.calls[1]?.[0]).toEqual(first);
});
