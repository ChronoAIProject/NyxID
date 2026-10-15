import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RemoteMachineCreate } from "./remote-machine-create";
import { RemoteMachinePaidStart } from "./remote-machine-paid-start";
import { useAuthStore } from "@/stores/auth-store";
import { ApiError } from "@/lib/api-client";
import { getRemoteMachineIntent } from "@/lib/remote-machine-intent";
import {
  remoteMachineFixture,
  remoteMachinePlatformFixture,
  remoteMachineOperationFixture,
} from "@/test/remote-machine-fixture";
import type { RemoteMachine } from "@/schemas/machines";

const offer = {
  platform: remoteMachinePlatformFixture,
  policy_version: "remote-machine-mvp",
  profile: remoteMachineFixture.profile,
  free_slot_requested: false,
  paid_admission: "quote_acceptance_required",
  pricing_status: "synced",
  valid_until: "2099-01-01T01:15:00Z",
  quote_id: "quote-one",
  rate_version: "rate-one",
  credits_per_second: "0.000040000001",
  funding_mode: "account_balance",
  no_instance_cap: true,
  cumulative_whole_seconds: true,
  paused_storage_free: true,
  policy: {
    policy_version: "remote-machine-mvp",
    data_policy_version: "remote-machine-data",
    free_slot_per_personal_account: 1,
    free_entitlement_expires: false,
    free_monthly_runtime_limit_seconds: null,
    paused_workspace_retention: "indefinite_until_owner_confirmed_delete",
    paused_storage_price_credits: "0",
    workspace_backup_or_restore: false,
    owner_confirmed_delete_required: true,
    cleanup_target_hours: 24,
    job_history_days: 30,
    operation_history_days: 90,
    terminal_output_max_retention_seconds: 3600,
    shared_environment_mode: "shared_legacy",
    cross_client_secret_isolation: false,
    inbound_network_service: false,
    persistent_environment_variables: false,
    service_charges_separate: true,
    concurrent_jobs: 2,
    default_command_timeout_seconds: 120,
    maximum_command_timeout_seconds: 3600,
    job_output_bytes: 1048576,
    machine_output_cache_bytes: 33554432,
  },
};
const state = vi.hoisted(() => ({
  quote: vi.fn(),
  create: vi.fn(),
  action: vi.fn(),
  accepted: vi.fn(),
  close: vi.fn(),
  data: undefined as unknown,
  error: null as Error | null,
  walletOwner: "owner",
  walletError: null as Error | null,
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({ data: { agents: [] } }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));
vi.mock("@/hooks/use-machines", () => ({
  useRemoteMachineEntitlement: () => ({
    data: undefined,
    error: null,
    refetch: vi.fn(),
  }),
  useRemoteMachineQuote: () => ({
    mutate: state.quote,
    data: state.data,
    error: state.error,
    isPending: false,
  }),
  useCreateRemoteMachine: () => ({
    mutateAsync: state.create,
    isPending: false,
  }),
  useRemoteMachineAction: () => ({
    mutateAsync: state.action,
    isPending: false,
  }),
}));
const auth = useAuthStore.getState();
vi.mock("@/hooks/use-billing", () => ({
  useBillingWallet: () => ({
    data: {
      owner_id: state.walletOwner,
      available: "12.123456789012",
      available_credits: 12,
    },
    error: state.walletError,
  }),
}));
beforeEach(() => {
  useAuthStore.setState({ user: { ...auth.user!, id: "owner" } });
  sessionStorage.clear();
  state.data = structuredClone(offer);
  state.error = null;
  state.walletOwner = "owner";
  state.walletError = null;
  state.create.mockResolvedValue({
    machine: remoteMachineFixture,
    operation: remoteMachineOperationFixture,
    replayed: false,
  });
  state.action.mockResolvedValue({
    operation: remoteMachineOperationFixture,
    replayed: false,
  });
});
afterEach(() => {
  cleanup();
  useAuthStore.setState(auth);
  sessionStorage.clear();
  vi.resetAllMocks();
});
const show = (machine?: RemoteMachine) =>
  render(
    machine ? (
      <RemoteMachinePaidStart
        open
        onOpenChange={state.close}
        onAccepted={state.accepted}
        machine={machine}
      />
    ) : (
      <RemoteMachineCreate initialFreeSlot={false} onCreated={state.accepted} />
    ),
  );
const accept = () =>
  userEvent.click(
    screen.getByRole("checkbox", { name: /I accept this exact rate/ }),
  );
const submit = () =>
  userEvent.click(
    screen.getByRole("button", { name: "Accept quote and create" }),
  );

it("shows the exact offered rate and requires explicit paid terms before real acceptance", async () => {
  show();
  expect(screen.getByText(/Temporary storage is managed by the deployment environment/)).toBeInTheDocument();
  expect(screen.queryByText(/2 GiB temporary storage/)).not.toBeInTheDocument();
  expect(state.quote).toHaveBeenCalledWith({ free_slot: false });
  expect(screen.getByText(/0.000040000001 Credits per/)).toBeInTheDocument();
  expect(
    screen.getByText(/Account wallet available: 12.123456789012 Credits/),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Accept quote and create" }),
  ).toBeDisabled();
  expect(state.accepted).not.toHaveBeenCalled();
  await accept();
  await submit();
  await waitFor(() => expect(state.create).toHaveBeenCalledTimes(1));
  expect(state.create.mock.calls[0]![0]).toEqual({
    body: {
      name: "my-remote-machine",
      capabilities: ["shell", "files"],
      specialist_id: null,
      free_slot: false,
      quote_id: "quote-one",
      accept_account_balance: true,
      policy_version: offer.policy.policy_version,
      data_policy_version: offer.policy.data_policy_version,
      acknowledge_shared_environment: true,
    },
    idempotencyKey: expect.any(String),
  });
  expect(state.accepted).toHaveBeenCalledWith("machine-1", "operation-1");
  expect(sessionStorage.length).toBe(0);
});

it("rejects an expired quote and an unpriced legacy response", () => {
  state.data = { ...offer, valid_until: "2020-01-01T00:00:00Z" };
  const view = show();
  expect(screen.getByText(/This preflight has expired/)).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Accept quote and create" }),
  ).toBeDisabled();
  state.data = { ...offer, quote_id: null, credits_per_second: null };
  view.rerender(
    <RemoteMachineCreate initialFreeSlot={false} onCreated={state.accepted} />,
  );
  expect(
    screen.getByText(/A synchronized, priced runtime quote is unavailable/),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Accept quote and create" }),
  ).toBeDisabled();
});

it("does not accept cached paid pricing after its live request failed", () => {
  state.error = new Error("Quote service unavailable");
  show();
  expect(screen.getByText(/Quote service unavailable/)).toBeInTheDocument();
  expect(
    screen.queryByText(/0.000040000001 Credits per/),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Accept quote and create" }),
  ).toBeDisabled();
});

it("replays exactly the original paid body and key after an unknown response", async () => {
  state.create.mockRejectedValueOnce(new Error("Response lost"));
  show();
  await accept();
  await submit();
  await screen.findByText("Response lost");
  await userEvent.click(
    screen.getByRole("button", { name: "Retry saved create request" }),
  );
  await waitFor(() => expect(state.create).toHaveBeenCalledTimes(2));
  expect(state.create.mock.calls[1]![0]).toEqual(
    state.create.mock.calls[0]![0],
  );
});

it("recovers a saved paid request after quote expiry without accepting the replacement", async () => {
  const saved = getRemoteMachineIntent("owner", "create-paid", {
    free_slot: false,
    quote_id: "original-quote",
    accept_account_balance: true,
    policy_version: "remote-machine-mvp",
    data_policy_version: "remote-machine-data",
    acknowledge_shared_environment: true,
  });
  state.data = { ...offer, valid_until: "2020-01-01T00:00:00Z" };
  show();
  await accept();
  await userEvent.click(
    screen.getByRole("button", { name: "Retry saved create request" }),
  );
  await waitFor(() =>
    expect(state.create).toHaveBeenCalledWith({
      body: {
        ...saved.body,
        name: "my-remote-machine",
        capabilities: ["shell", "files"],
        specialist_id: null,
      },
      idempotencyKey: saved.idempotencyKey,
    }),
  );
});

it("requires a fresh review and a new intent after a revision conflict", async () => {
  state.create.mockRejectedValueOnce(
    new ApiError(409, {
      error_code: 1,
      message: "Revision changed",
      error: "conflict",
    }),
  );
  show();
  await accept();
  await submit();
  await screen.findByText("Revision changed");
  expect(
    screen.getByRole("button", { name: "Retry saved create request" }),
  ).toBeDisabled();
  await userEvent.click(
    screen.getByRole("button", {
      name: "Discard saved request and review again",
    }),
  );
  expect(
    screen.getByRole("checkbox", { name: /I accept this exact rate/ }),
  ).not.toBeChecked();
  expect(sessionStorage.length).toBe(0);
  await accept();
  await submit();
  await waitFor(() => expect(state.create).toHaveBeenCalledTimes(2));
  expect(state.create.mock.calls[1]![0].idempotencyKey).not.toBe(
    state.create.mock.calls[0]![0].idempotencyKey,
  );
});

it("resumes a paid paused Machine only with the accepted quote and revision", async () => {
  show({
    ...remoteMachineFixture,
    resource_state: "paused",
    billing_state: "suspended",
    free_slot: false,
    revision: 9,
  });
  expect(screen.getByText(/Adding funds never resumes/)).toBeInTheDocument();
  expect(state.action).not.toHaveBeenCalled();
  await accept();
  await userEvent.click(
    screen.getByRole("button", { name: "Accept quote and resume" }),
  );
  await waitFor(() => expect(state.action).toHaveBeenCalledTimes(1));
  expect(state.action.mock.calls[0]![0]).toMatchObject({
    machineId: "machine-1",
    body: {
      action: "resume",
      expected_revision: 9,
      quote_id: "quote-one",
      accept_account_balance: true,
    },
  });
  expect(state.create).not.toHaveBeenCalled();
});

it("clears reviewed acceptance when the signed-in account changes", async () => {
  show();
  await accept();
  act(() =>
    useAuthStore.setState({ user: { ...auth.user!, id: "another-owner" } }),
  );
  expect(
    screen.getByRole("checkbox", { name: /I accept this exact rate/ }),
  ).not.toBeChecked();
  expect(
    screen.getByRole("button", { name: "Accept quote and create" }),
  ).toBeDisabled();
});

it("never presents another owner's wallet as available funds", () => {
  state.walletOwner = "another-owner";
  show();
  expect(
    screen.getByText(/Account wallet available: Unavailable/),
  ).toBeInTheDocument();
  expect(screen.queryByText(/12.123456789012 Credits/)).not.toBeInTheDocument();
});

it("does not present stale wallet funds after a read error", () => {
  state.walletError = new Error("Forbidden");
  show();
  expect(
    screen.getByText(/Account wallet available: Unavailable/),
  ).toBeInTheDocument();
  expect(screen.queryByText(/12.123456789012 Credits/)).not.toBeInTheDocument();
});

it("requires a fresh manual review after insufficient funds and never retries automatically", async () => {
  state.create.mockRejectedValueOnce(
    new ApiError(402, {
      error_code: 1,
      message: "Insufficient funds",
      error: "insufficient_credits",
    }),
  );
  show();
  await accept();
  await submit();
  await screen.findByText("Insufficient funds");
  expect(state.create).toHaveBeenCalledTimes(1);
  expect(
    screen.getByRole("button", { name: "Retry saved create request" }),
  ).toBeDisabled();
  expect(
    screen.getByRole("button", {
      name: "Discard saved request and review again",
    }),
  ).toBeEnabled();
  await userEvent.click(
    screen.getByRole("button", {
      name: "Discard saved request and review again",
    }),
  );
  expect(
    screen.getByRole("checkbox", { name: /I accept this exact rate/ }),
  ).not.toBeChecked();
  expect(state.create).toHaveBeenCalledTimes(1);
});


it.each(["remote-machine-mvp-v1", "remote-machine-mvp-v2"])("rejects an obsolete paid resume quote %s before dispatch", (policy_version) => {
  const current = { ...remoteMachineFixture, resource_state: "paused" as const, billing_state: "suspended" as const };
  state.data = { ...offer, policy_version, policy: { ...offer.policy, policy_version } };
  show(current);
  expect(screen.getByRole("button", { name: "Accept quote and resume" })).toBeDisabled();
  expect(state.action).not.toHaveBeenCalled();
});
