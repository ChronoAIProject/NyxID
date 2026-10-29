import {
  act,
  cleanup,
  fireEvent,
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
import {
  acceptRemoteMachineIntent,
  getRemoteMachineIntent,
  observeAcceptedRemoteMachineIntent,
  parseAcceptedRemoteMachineIntent,
  remoteMachineAcceptedSnapshot,
} from "@/lib/remote-machine-intent";
import {
  RemoteMachineDetail,
  RemoteMachineTimeline,
} from "./remote-machine-detail";
import {
  remoteMachineOperationFixture,
  remoteMachinePaidQuoteFixture,
} from "@/test/remote-machine-fixture";

const fixture = vi.hoisted(() => ({
  machine: {
    node_id: "11111111-1111-4111-8111-111111111111",
    name: "machine-one",
    capabilities: ["files", "shell"],
    separation_choice: "automatic",
    separation_policy_version: "remote-machine-separation-v1",
    machine_configuration: {
      version: 2,
      revision: 1,
      digest: "a".repeat(64),
      runtime_generation: 2,
      state: "pending",
      reason: "current_runtime_configuration_evidence_required",
    },
    specialist_id: null,
    specialist_assignment_state: "none",
    specialist_assignment_error: null,
    free_slot: true,
    runtime_run_id: null,
    funded_until: null,
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
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({ data: { agents: [] }, isPending: false }),
  useSetNyxBotAgentGrants: () => ({ mutateAsync: vi.fn() }),
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
  fixture.refetch.mockImplementation(async () => ({ data: fixture.machine }));
  fixture.operationRefetch.mockImplementation(async () => ({
    data: fixture.operation,
  }));
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
  fixture.machine.machine_configuration.state = "pending";
  fixture.machine.machine_configuration.reason =
    "current_runtime_configuration_evidence_required";
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
    screen.getByRole("button", { name: "Check retry status" }),
  );
  await waitFor(() => expect(fixture.operationRefetch).toHaveBeenCalled());
  expect(fixture.retryDelete).toHaveBeenCalledTimes(1);
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  await waitFor(() => expect(fixture.retryDelete).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(fixture.operationRefetch).toHaveBeenCalled());
  if (fixture.retryDelete.mock.calls.length === 1)
    await userEvent.click(
      screen.getByRole("button", { name: "Retry deletion" }),
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
  expect(screen.getByText(/Awaiting machine report/)).toBeInTheDocument();
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
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  const dialog = within(screen.getByRole("dialog"));
  const submit = screen.getByRole("button", { name: "Delete machine" });
  expect(submit).toBeDisabled();
  expect(
    dialog.getByText(/permanently deletes the machine and its workspace/),
  ).toBeInTheDocument();
  expect(dialog.getByText(/cannot be undone/)).toBeInTheDocument();
  expect(
    dialog.getByText(/free slot is released after deletion is confirmed/),
  ).toBeInTheDocument();
  expect(dialog.queryByText(/Kubernetes|Sandbox|PVC/)).not.toBeInTheDocument();
  expect(dialog.getByText(/Reviewed resource revision/)).not.toBeVisible();
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
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await userEvent.click(screen.getByRole("checkbox", { name: /I understand/ }));
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await screen.findByText("Network response unknown");
  expect(screen.getByRole("button", { name: "Delete machine" })).toBeDisabled();
  await userEvent.click(
    screen.getByRole("button", { name: "Check request status" }),
  );
  fixture.machine.revision = 6;
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
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
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await userEvent.click(screen.getByRole("checkbox", { name: /I understand/ }));
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await screen.findByText("Machine revision changed");
  expect(screen.getByRole("button", { name: "Delete machine" })).toBeDisabled();
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
    screen.getByRole("button", { name: "View operation op" }),
  ).toBeEnabled();
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
    attention.queryByText("Lifecycle operation completed"),
  ).not.toBeInTheDocument();
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

it("pauses through one concise confirmation without an extra acknowledgement", async () => {
  fixture.machine.resource_state = "ready";
  fixture.action.mockResolvedValueOnce({ operation: { id: "original-pause" } });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Pause machine" }));
  const dialog = within(screen.getByRole("dialog"));
  expect(dialog.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(dialog.getByText(/workspace will be kept/)).toBeVisible();
  expect(
    dialog.getByText(/Active tasks and running processes will stop/),
  ).toBeVisible();
  expect(dialog.getByText(/continue to occupy your free slot/)).toBeVisible();
  await userEvent.click(dialog.getByRole("button", { name: "Pause machine" }));
  await waitFor(() => expect(fixture.action).toHaveBeenCalledTimes(1));
  expect(fixture.action.mock.calls[0]![0].body).toEqual({
    action: "pause",
    expected_revision: 5,
  });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByText("Pausing your machine")).toBeVisible();
  expect(screen.getByRole("button", { name: "Pausing…" })).toBeDisabled();
  expect(
    screen.getByRole("region", { name: "Operation progress" }),
  ).toBeVisible();
});

it.each([
  ["pause", "stopping", "ready", "Pausing…", "Pause machine"],
  ["resume", "resuming", "paused", "Resuming…", "Resume machine"],
] as const)("keeps failed %s feedback separate from current resource action eligibility", async (action, transitional, eligible, busyLabel, label) => {
  Object.assign(fixture.machine, { resource_state: transitional, active_operation_id: "current-b", revision: 6 });
  fixture.operation = { ...remoteMachineOperationFixture, id: "current-b", machine_id: fixture.machine.node_id, action, state: "failed", terminal_at: "2026-10-29T00:00:00Z", error_message: "Current operation failed" };
  fixture.action.mockResolvedValueOnce({ operation: { id: "recovery" } });
  const view = render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getByRole("region", { name: "Operation progress" })).toHaveTextContent("Current operation failed");
  const button = screen.getByRole("button", { name: busyLabel });
  expect(button).toBeDisabled();
  await userEvent.click(button);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(fixture.action).not.toHaveBeenCalled();
  fixture.machine.resource_state = eligible;
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getByRole("button", { name: label })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: label }));
  await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: label }));
  await waitFor(() => expect(fixture.action).toHaveBeenCalledTimes(1));
  expect(fixture.action.mock.calls[0]![0].body).toMatchObject({ action, expected_revision: 6 });
});

it.each([
  ["pause", "ready", "stopping", "Pause machine"],
  ["resume", "paused", "resuming", "Resume machine"],
] as const)("rechecks %s resource eligibility when an open confirmation submits", async (action, eligible, transitional, label) => {
  fixture.machine.resource_state = eligible;
  fixture.action.mockResolvedValueOnce({ operation: { id: "reviewed-operation" } });
  const view = render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: label }));
  Object.assign(fixture.machine, { resource_state: transitional, revision: 6 });
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  const confirm = within(screen.getByRole("dialog")).getByRole("button", { name: label });
  fireEvent.submit(confirm.closest("form")!);
  await act(async () => {});
  expect(fixture.action).not.toHaveBeenCalled();
  expect(confirm).toBeDisabled();
  fixture.machine.resource_state = eligible;
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(confirm).toBeEnabled();
  await userEvent.click(confirm);
  await waitFor(() => expect(fixture.action).toHaveBeenCalledTimes(1));
  expect(fixture.action.mock.calls[0]![0].body).toMatchObject({ action, expected_revision: 5 });
});

it.each([
  ["create", "provisioning", "ready", "Setting up your machine"],
  ["pause", "stopping", "paused", "Pausing your machine"],
  ["resume", "resuming", "ready", "Resuming your machine"],
  ["delete", "deleting", "deleted", "Deleting your machine"],
] as const)("waits for the actual resource after another client's current %s succeeds", (action, transitional, final, heading) => {
  const machineId = fixture.machine.node_id;
  const intent = getRemoteMachineIntent("owner", `${machineId}:pause`, { action: "pause", expected_revision: 4 });
  acceptRemoteMachineIntent("owner", machineId, "pause", intent.idempotencyKey, "accepted-a");
  observeAcceptedRemoteMachineIntent("owner", machineId, "accepted-a", 5);
  Object.assign(fixture.machine, { resource_state: transitional, active_operation_id: "other-client-b", revision: 6 });
  fixture.operation = { ...remoteMachineOperationFixture, id: "other-client-b", machine_id: machineId, action, state: "succeeded", terminal_at: "2026-10-29T00:00:00Z", deadline_at: "2099-01-01T00:00:00Z" };
  const view = render(<RemoteMachineDetail machineId={machineId} />);
  if (action === "resume" || action === "pause")
    expect(screen.getByRole("button", { name: action === "resume" ? "Resuming…" : "Pausing…" })).toBeDisabled();
  expect(screen.getByRole("region", { name: "Operation progress" })).toHaveTextContent(heading);
  view.unmount();
  const reopened = render(<RemoteMachineDetail machineId={machineId} />);
  expect(screen.getByRole("region", { name: "Operation progress" })).toHaveTextContent(heading);
  Object.assign(fixture.machine, { resource_state: final, revision: 7, cleanup_state: action === "delete" ? "complete" : "not_required" });
  reopened.rerender(<RemoteMachineDetail machineId={machineId} />);
  expect(screen.queryByRole("region", { name: "Operation progress" })).not.toBeInTheDocument();
  if (action === "pause") expect(screen.getByRole("button", { name: "Resume machine" })).toBeEnabled();
  if (action === "resume" || action === "create") expect(screen.getByRole("button", { name: "Pause machine" })).toBeEnabled();
  expect(parseAcceptedRemoteMachineIntent(remoteMachineAcceptedSnapshot("owner", machineId))).toMatchObject({ operationId: "accepted-a", action: "pause", body: intent.body, idempotencyKey: intent.idempotencyKey, currentObservation: { operationId: "other-client-b", revision: 7 } });
  expect(fixture.action).not.toHaveBeenCalled();
});

it.each([
  ["pause", "ready", "create", "succeeded", "Pausing your machine"],
  ["resume", "paused", "pause", "succeeded", "Resuming your machine"],
  ["resume", "paused", "resume", "failed", "Resuming your machine"],
  ["delete", "ready", "resume", "failed", "Deleting your machine"],
] as const)(
  "keeps a new %s associated with its returned operation while the previous %s/%s %s read lags",
  async (action, resource, previousAction, state, heading) => {
    Object.assign(fixture.machine, {
      resource_state: resource,
      active_operation_id: "previous-operation",
    });
    fixture.operation = {
      ...remoteMachineOperationFixture,
      id: "previous-operation",
      machine_id: fixture.machine.node_id,
      action: previousAction,
      state,
      terminal_at: "2026-10-26T00:00:00Z",
      deadline_at: "2020-01-01T00:00:00Z",
      error_message: "Previous operation failure",
    };
    fixture.action.mockResolvedValueOnce({
      operation: { id: "new-operation" },
    });
    const view = render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
    const label = action === "pause" ? "Pause machine" : action === "delete" ? "Delete machine" : "Resume machine";
    await userEvent.click(screen.getByRole("button", { name: label }));
    if (action === "delete")
      await userEvent.click(within(screen.getByRole("dialog")).getByRole("checkbox"));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: label }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    const progress = within(
      screen.getByRole("region", { name: "Operation progress" }),
    );
    expect(progress.getByText(heading)).toBeVisible();
    expect(
      progress.queryByText("Previous operation failure"),
    ).not.toBeInTheDocument();
    expect(
      progress.queryByText("Taking longer than usual"),
    ).not.toBeInTheDocument();
    expect(progress.getAllByLabelText("Confirmed")).toHaveLength(1);
    expect(fixture.action.mock.calls[0]![0].body).toMatchObject({
      action,
      expected_revision: 5,
    });
    // Closing Settings, moving to Activity and a reload all destroy this view.
    // The accepted response must outlive that component while reads still lag.
    view.unmount();
    render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
    const reopened = within(screen.getByRole("region", { name: "Operation progress" }));
    expect(reopened.getByText(heading)).toBeVisible();
    expect(reopened.queryByText("Previous operation failure")).not.toBeInTheDocument();
    expect(reopened.queryByText("Taking longer than usual")).not.toBeInTheDocument();
    expect(fixture.action).toHaveBeenCalledTimes(1);
  },
);

it.each([
  ["pending", "Specialist assignment is pending"],
  ["not_assigned", "Specialist is no longer assigned"],
  ["failed", "Specialist assignment could not be completed"],
])(
  "keeps %s assignment notices in the existing Specialist settings",
  (state, message) => {
    Object.assign(fixture.machine, {
      name: "research-box",
      specialist_id: "specialist-1",
      specialist_assignment_state: state,
    });
    render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
    expect(
      within(
        screen.getByRole("region", { name: "Specialist access" }),
      ).getByText(message),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Review agent access" }),
    ).not.toBeInTheDocument();
  },
);

it("keeps a ready free detail concise while retaining Activity and shared settings", () => {
  Object.assign(fixture.machine, {
    resource_state: "ready",
    specialist_id: "specialist-1",
    specialist_assignment_state: "assigned",
    machine_configuration: {
      ...fixture.machine.machine_configuration,
      state: "applied",
      reason: null,
    },
    active_operation_id: "operation-1",
  });
  fixture.operation = {
    ...remoteMachineOperationFixture,
    machine_id: fixture.machine.node_id,
    state: "succeeded",
    action: "create",
  };
  render(
    <RemoteMachineDetail
      machineId={fixture.machine.node_id}
      onActivity={vi.fn()}
    />,
  );
  for (const label of [
    "Jobs",
    "Confirmations",
    "Billing",
    "Review agent access",
    "Check status",
  ])
    expect(
      screen.queryByRole("button", { name: label }),
    ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("link", { name: "Open specialist settings" }),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("Specialist assigned")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("region", { name: "create operation" }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "View activity" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("link", { name: "Manage saved logins" }),
  ).toBeInTheDocument();
});

it("retains paid Billing management without a free-slot claim", async () => {
  Object.assign(fixture.machine, {
    free_slot: false,
    billing_state: "suspended",
  });
  const view = render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} />,
  );
  expect(
    screen.queryByText("This machine uses your free slot."),
  ).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Billing" }));
  expect(screen.getByText("Billing panel")).toBeVisible();
  await userEvent.keyboard("{Escape}");
  stoppedDelete();
  fixture.operation = {
    ...fixture.operation!,
    delete_retry_stopped: false,
  };
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(
    screen.queryByText(/Your free slot stays occupied until deletion/),
  ).not.toBeInTheDocument();
  Object.assign(fixture.machine, {
    resource_state: "deleted",
    cleanup_state: "complete",
  });
  view.rerender(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(
    screen.queryByText(/Your free slot is available only/),
  ).not.toBeInTheDocument();
});

it("opens external confirmation deep links without a permanent confirmation button", () => {
  render(
    <RemoteMachineDetail
      machineId={fixture.machine.node_id}
      confirmationId="confirmation-1"
    />,
  );
  expect(screen.getByText("Confirmation panel")).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Confirmations" }),
  ).not.toBeInTheDocument();
});

function desktopMachine(
  capabilities = ["shell", "files", "browser", "computer"],
): NodeInfo {
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
    status: "active",
    last_heartbeat_at: null,
    connected_at: null,
    metadata: null,
    metrics: null,
    binding_count: 0,
    created_at: "2026-10-26T00:00:00Z",
    machine: {
      version: 1,
      roots: [],
      os: "linux",
      arch: "x86_64",
      computer_mode: "standard",
      cua_version: null,
      computer_tools: [],
      browser_isolated: false,
      saved_login_ready: false,
      shell: capabilities.includes("shell"),
      files: capabilities.includes("files"),
      browser: capabilities.includes("browser"),
      computer: capabilities.includes("computer"),
      computer_ready: true,
    },
  };
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
      screen.getByRole("link", { name: "Open live desktop" }),
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
  "disables the %s owner desktop when its display is unavailable",
  (_label, capabilities) => {
    const node = desktopMachine(capabilities);
    render(
      <RemoteMachineDetail
        machineId={fixture.machine.node_id}
        node={{ ...node, machine: { ...node.machine!, computer_ready: false } }}
      />,
    );
    expect(
      screen.getByRole("button", { name: "Open live desktop" }),
    ).toBeDisabled();
    expect(screen.getByText(/Display unavailable/)).toBeInTheDocument();
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
    screen.queryByRole("link", { name: "Open live desktop" }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Open live desktop" }),
  ).toBeDisabled();
  expect(screen.getByText(/Display unavailable/)).toBeInTheDocument();
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
      screen.queryByRole("link", { name: "Open live desktop" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open live desktop" }),
    ).toBeDisabled();
    expect(screen.getByText(/Display unavailable/)).toBeInTheDocument();
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
      screen.queryByRole("link", { name: "Open live desktop" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open live desktop" }),
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
    screen.queryByRole("link", { name: "Open live desktop" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Open live desktop" }),
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
    screen.queryByRole("link", { name: "Open live desktop" }),
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
      screen.getByRole("button", { name: "Settings and rules" }),
    );
    const policy = within(
      screen.getByRole("dialog", { name: "Settings and rules" }),
    );
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
        /Sharing is set per Agent.*Shared legacy Agents and external clients share/,
      ),
    ).toBeInTheDocument();
    expect(
      policy.getByText(/Shared legacy Agents also share browser sessions/),
    ).toBeInTheDocument();
    expect(
      policy.getByText(
        /legacy \/workspace and separated workspaces are retained.*browser profiles and downloads are temporary/,
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
  expect(
    screen.getAllByText("No fixed capacity declared")[0],
  ).toBeInTheDocument();
  expect(
    screen.getByText(/Disk pressure can evict the Pod/),
  ).toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("button", { name: "Settings and rules" }),
  );
  expect(
    screen.getAllByText("No fixed capacity declared")[0],
  ).toBeInTheDocument();
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
    expect(
      screen.getAllByText("No fixed capacity declared")[0],
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/null GiB|undefined GiB|2 GiB temporary/),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(
        /Temporary storage is managed by the deployment environment/,
      ),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Settings and rules" }),
    );
    expect(
      screen.getAllByText("No fixed capacity declared")[0],
    ).toBeInTheDocument();
  } finally {
    fixture.machine.profile = oldProfile;
    fixture.machine.policy_version = oldPolicy;
  }
});

it("resumes a current free Machine without a paid-balance or extra policy checkbox", async () => {
  fixture.action.mockResolvedValue({ operation: { id: "op-legacy-resume" } });
  render(
    <RemoteMachineDetail
      machineId={fixture.machine.node_id}
      policy={remoteMachinePaidQuoteFixture.policy}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Resume machine" }));
  const dialog = within(screen.getByRole("dialog"));
  expect(dialog.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(
    dialog.getByText(/browser sessions will not be restored/),
  ).toBeInTheDocument();
  expect(
    dialog.getByText(/Model and external API usage may be billed separately/),
  ).toBeInTheDocument();
  expect(fixture.quote).toHaveBeenCalledWith({
    free_slot: true,
    machine_id: fixture.machine.node_id,
    profile_version: fixture.machine.profile.profile_version,
  });
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
    screen.getByRole("button", { name: "Check retry status" }),
  );
  await waitFor(() => expect(fixture.operationRefetch).toHaveBeenCalled());
  expect(fixture.retryDelete).toHaveBeenCalledTimes(1);
  await userEvent.click(screen.getByRole("button", { name: "Retry deletion" }));
  await waitFor(() => expect(fixture.retryDelete).toHaveBeenCalledTimes(2));
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
    screen.getAllByText(/original deletion remains successful/)[0],
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Retry deletion" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText(/Automatic deletion attempts stopped/),
  ).not.toBeInTheDocument();
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
  expect(screen.queryByRole("tab")).not.toBeInTheDocument();
  expect(screen.getAllByText("Confirm changes")[0]).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Save machine settings" }),
  ).toBeDisabled();
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
  render(
    <RemoteMachineTimeline
      machineId={fixture.machine.node_id}
      onOperation={vi.fn()}
    />,
  );
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
    screen.getByRole("button", { name: "Settings and rules" }),
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
    screen.getByRole("button", { name: "Check retry status" }),
  );
  await waitFor(() => expect(fixture.operationRefetch).toHaveBeenCalled());
  if (fixture.retryDelete.mock.calls.length === 1)
    await userEvent.click(
      screen.getByRole("button", { name: "Retry deletion" }),
    );
  await waitFor(() => expect(fixture.retryDelete).toHaveBeenCalledTimes(2));
  expect(fixture.retryDelete.mock.calls[1]?.[0]).toEqual(first);
});

it("keeps the original lifecycle action identity after closing an unknown request and reopening", async () => {
  fixture.action.mockRejectedValueOnce(new Error("Response lost"));
  const view = render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await userEvent.click(screen.getByRole("checkbox", { name: /I understand/ }));
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await screen.findByText("Response lost");
  const original = fixture.action.mock.calls[0]![0];
  view.unmount();
  fixture.machine.revision = 9;
  fixture.action.mockResolvedValueOnce({
    operation: { id: "original-operation" },
  });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await userEvent.click(
    screen.getByRole("button", { name: "Check request status" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await waitFor(() => expect(fixture.action).toHaveBeenCalledTimes(2));
  expect(fixture.action.mock.calls[1]![0]).toEqual(original);
});
const safePresentation = {
  agent_version: "0.73.1",
  os: "linux",
  arch: "x86_64",
  roots: ["/workspace"],
  capabilities: ["shell", "files", "browser", "computer"],
  commands_isolated: true,
  browser_isolated: false,
  separation_support: true,
  owner_confirmation: "changes",
  allow_single_user_saved_logins: false,
  computer_mode: "standard",
  cua_version: "0.30.4",
  connected: true,
  desktop_ready: true,
  historical: false,
  configuration_current: true,
  last_reported_at: "2026-10-26T00:00:00Z",
};
it("renders the safe paused report after a reload without treating it as online or enabling writes", () => {
  Object.assign(fixture.machine, {
    presentation: { ...safePresentation, historical: true },
  });
  try {
    render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
    expect(screen.getByRole("region", { name: "Version" })).toHaveTextContent(
      "0.73.1",
    );
    expect(screen.getByText(/Last reported information/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open live desktop" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("combobox", { name: "Owner confirmation" }),
    ).toBeDisabled();
    expect(screen.queryByText(/Update automatically/)).not.toBeInTheDocument();
  } finally {
    Reflect.deleteProperty(fixture.machine, "presentation");
  }
});
it.each([
  [false, true],
  [true, false],
])(
  "keeps separation support %s separate from actual display readiness %s",
  (support, desktopReady) => {
    Object.assign(fixture.machine, {
      resource_state: "ready",
      authorization_state: "active",
      presentation: {
        ...safePresentation,
        separation_support: support,
        desktop_ready: desktopReady,
      },
    });
    try {
      render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
      if (!support)
        expect(
          screen.getByText("Separation prerequisites not met"),
        ).toBeInTheDocument();
      if (desktopReady)
        expect(
          screen.getByRole("link", { name: "Open live desktop" }),
        ).toBeInTheDocument();
      else
        expect(
          screen.getByRole("button", { name: "Open live desktop" }),
        ).toBeDisabled();
    } finally {
      Reflect.deleteProperty(fixture.machine, "presentation");
    }
  },
);
it("permits a new reviewed lifecycle action after the previous operation is terminal", () => {
  Object.assign(fixture.machine, {
    resource_state: "paused",
    active_operation_id: "operation-1",
  });
  fixture.operation = {
    ...remoteMachineOperationFixture,
    id: "operation-1",
    machine_id: fixture.machine.node_id,
    action: "resume",
    state: "failed",
    terminal_at: "2026-10-26T00:00:00Z",
  };
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(screen.getByRole("button", { name: "Delete machine" })).toBeEnabled();
});
it("keeps configuration revisions and generation diagnostics in Technical details", () => {
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  const diagnostic = screen
    .getByText(/Current hosted configuration revision/)
    .closest("details");
  expect(diagnostic).not.toBeNull();
  expect(diagnostic).not.toHaveAttribute("open");
});

it("keeps final deletion concise with diagnostics collapsed and no agent configuration controls", () => {
  fixture.machine.resource_state = "deleted";
  fixture.machine.cleanup_state = "complete";
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  expect(
    screen.queryByRole("button", { name: "Review agent access" }),
  ).not.toBeInTheDocument();
  expect(screen.getByText(/Hosted configuration/)).not.toBeVisible();
  expect(screen.getByRole("region", { name: "Machine deleted" })).toBeVisible();
});

it("recovers an unknown original delete after refresh even while backend deletion is active", async () => {
  fixture.action.mockRejectedValueOnce(new Error("Delete reply lost"));
  const view = render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await userEvent.click(screen.getByRole("checkbox", { name: /I understand/ }));
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await screen.findByText("Delete reply lost");
  const original = fixture.action.mock.calls[0]![0];
  view.unmount();
  Object.assign(fixture.machine, {
    resource_state: "deleting",
    revision: 8,
    active_operation_id: "original-delete",
  });
  fixture.operation = {
    ...remoteMachineOperationFixture,
    id: "original-delete",
    machine_id: fixture.machine.node_id,
    action: "delete",
  };
  fixture.action.mockResolvedValueOnce({ operation: fixture.operation });
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(
    screen.getByRole("button", { name: "Recover delete request" }),
  );
  expect(screen.getByRole("button", { name: "Delete machine" })).toBeDisabled();
  await userEvent.click(
    screen.getByRole("button", { name: "Check request status" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await waitFor(() => expect(fixture.action).toHaveBeenCalledTimes(2));
  expect(fixture.action.mock.calls[1]![0]).toEqual(original);
  expect(fixture.refetch.mock.invocationCallOrder[0]).toBeLessThan(
    fixture.operationRefetch.mock.invocationCallOrder[0]!,
  );
  expect(fixture.operationRefetch.mock.invocationCallOrder[0]).toBeLessThan(
    fixture.action.mock.invocationCallOrder[1]!,
  );
});

it("keeps an unknown lifecycle request blocked if its live status read fails", async () => {
  fixture.action.mockRejectedValueOnce(new Error("Reply lost"));
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await userEvent.click(screen.getByRole("checkbox", { name: /I understand/ }));
  await userEvent.click(screen.getByRole("button", { name: "Delete machine" }));
  await screen.findByText("Reply lost");
  fixture.refetch.mockResolvedValueOnce({ error: new Error("Not authorized") });
  await userEvent.click(
    screen.getByRole("button", { name: "Check request status" }),
  );
  await screen.findByText(
    /current machine and operation could not both be confirmed/,
  );
  expect(screen.getByRole("button", { name: "Delete machine" })).toBeDisabled();
  expect(fixture.action).toHaveBeenCalledTimes(1);
});

it("allows closing a pending pause and does not repeat it when the drawer reopens", async () => {
  fixture.machine.resource_state = "ready";
  let resolve!: (value: unknown) => void;
  fixture.action.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const view = render(
    <RemoteMachineDetail machineId={fixture.machine.node_id} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Pause machine" }));
  await userEvent.dblClick(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "Pause machine",
    }),
  );
  expect(fixture.action).toHaveBeenCalledTimes(1);
  await userEvent.click(
    screen.getByRole("button", { name: "Close · work continues" }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  view.unmount();
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(
    screen.getByRole("button", { name: "Recover pause request" }),
  );
  expect(fixture.action).toHaveBeenCalledTimes(1);
  expect(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "Pause machine",
    }),
  ).toBeDisabled();
  await act(async () => resolve({ operation: { id: "original-pause" } }));
  expect(fixture.action).toHaveBeenCalledTimes(1);
});

it("retains an old owner's unresolved intent without accepting its late response for a new owner", async () => {
  fixture.machine.resource_state = "ready";
  let resolve!: (value: unknown) => void;
  fixture.action.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  render(<RemoteMachineDetail machineId={fixture.machine.node_id} />);
  await userEvent.click(screen.getByRole("button", { name: "Pause machine" }));
  await userEvent.click(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "Pause machine",
    }),
  );
  const original = fixture.action.mock.calls[0]![0];
  act(() =>
    useAuthStore.setState({
      user: {
        id: "different-owner",
        capabilities: { enabled_features: [] },
      } as never,
    }),
  );
  await act(async () => resolve({ operation: { id: "old-owner-operation" } }));
  expect(screen.queryByText(/pause request accepted/)).not.toBeInTheDocument();
  expect(
    sessionStorage.getItem(
      `nyxid:remote-machine-intent:owner:${encodeURIComponent(`${fixture.machine.node_id}:pause`)}`,
    ),
  ).toContain(original.idempotencyKey);
  expect(
    screen.queryByRole("button", { name: "Recover pause request" }),
  ).not.toBeInTheDocument();
});

it("opens a newly selected confirmation deep link while the same Machine detail stays mounted", async () => {
  const view = render(
    <RemoteMachineDetail
      machineId={fixture.machine.node_id}
      confirmationId="confirmation-1"
    />,
  );
  await userEvent.click(
    within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  view.rerender(
    <RemoteMachineDetail
      machineId={fixture.machine.node_id}
      confirmationId="confirmation-2"
    />,
  );
  expect(screen.getByText("Confirmation panel")).toBeVisible();
});
