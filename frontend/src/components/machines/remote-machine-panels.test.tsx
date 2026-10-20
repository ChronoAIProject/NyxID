import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RemoteMachineJobs, MachineOutput } from "./remote-machine-jobs";
import {
  RemoteMachineConfirmations,
  ConfirmationDecision,
} from "./remote-machine-confirmations";
import { RemoteMachineBilling } from "./remote-machine-billing";
import { useAuthStore } from "@/stores/auth-store";
import { ApiError } from "@/lib/api-client";
import type {
  RemoteMachineConfirmation,
  RemoteMachineJob,
  RemoteMachineRuntimeBilling,
} from "@/schemas/machines";
const http = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn(), read: vi.fn() }));
vi.mock("@/lib/api-client", async (load) => ({
  ...(await load<typeof import("@/lib/api-client")>()),
  api: { get: http.get, post: http.post },
  apiClient: http.read,
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => (
    <a href={to}>{children}</a>
  ),
}));
const id = "11111111-1111-4111-8111-111111111111";
const job: RemoteMachineJob = {
  id,
  node_id: id,
  subject: {
    kind: "api_key",
    user_id: id,
    api_key_id: id,
    api_key_credential_id: null,
  },
  grant_id: id,
  runtime_generation: 2,
  status: "succeeded",
  created_at: "2026-10-14T08:00:00Z",
  started_at: "2026-10-14T08:00:00Z",
  finished_at: "2026-10-14T08:01:00Z",
  expires_at: "2099-10-14T08:01:00Z",
  exit_code: 0,
  stop_reason: null,
  output_availability: "available",
  output_expires_at: "2099-10-14T09:01:00Z",
  output_reason: null,
  services: [],
};
const clients: QueryClient[] = [];
function renderPanel(element: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  clients.push(client);
  return {
    client,
    ...render(
      <QueryClientProvider client={client}>{element}</QueryClientProvider>,
    ),
  };
}
beforeEach(() => {
  useAuthStore.setState({ user: { id } as never });
});
afterEach(() => {
  cleanup();
  clients.forEach((client) => client.clear());
  clients.length = 0;
  vi.clearAllMocks();
});

it("reports missing job/confirmation/Billing endpoints as unavailable rather than empty or zero", async () => {
  http.get.mockRejectedValue(
    new ApiError(404, {
      error: "not_found",
      error_code: 404,
      message: "Not found",
    }),
  );
  renderPanel(
    <>
      <RemoteMachineJobs machineId={id} />
      <RemoteMachineConfirmations machineId={id} />
      <RemoteMachineBilling machineId={id} />
    </>,
  );
  expect(
    await screen.findByText(/Job history is unavailable on this server/),
  ).toBeInTheDocument();
  expect(
    await screen.findByText(/External confirmations are unavailable/),
  ).toBeInTheDocument();
  expect(
    await screen.findByText(/Runtime billing details are unavailable/),
  ).toBeInTheDocument();
  expect(screen.queryByText(/No jobs match/)).not.toBeInTheDocument();
  expect(screen.queryByText(/0 Credits/)).not.toBeInTheDocument();
});

it("current owner history renders real metadata and keeps absent execution details unavailable", async () => {
  http.get.mockResolvedValue({
    jobs: [
      {
        id,
        node_id: id,
        runtime_generation: 2,
        state: "running",
        created_at: job.created_at,
        expires_at: job.expires_at,
        finished_at: null,
        binding: {
          kind: "external",
          principal: job.subject,
          grant_id: id,
          grant_revision: 7,
        },
        services: [],
        output_persisted: false,
      },
    ],
    next_cursor: null,
  });
  renderPanel(<RemoteMachineJobs machineId={id} />);
  await userEvent.click(
    (await screen.findAllByRole("button", { name: "View job" }))[0]!,
  );
  expect(
    screen.getByRole("button", { name: "Read live output" }),
  ).toBeDisabled();
  expect(screen.getByRole("button", { name: "Cancel job" })).toBeDisabled();
  expect(screen.getByLabelText("Created from")).toBeInTheDocument();
  expect(
    screen.getByText(/Output is not queryable in the current runtime/),
  ).toBeInTheDocument();
  expect(http.read).not.toHaveBeenCalled();
});

it("the output component reads authorized live output only on demand and keeps it out of caches", async () => {
  http.get.mockResolvedValue({ jobs: [job], next_cursor: null });
  http.read.mockResolvedValue({
    job_id: id,
    status: "succeeded",
    exit_code: 0,
    stdout: "transient-private-result",
    stderr: "",
    stdout_bytes: 24,
    stderr_bytes: 0,
    output_offset: 24,
    stderr_offset: 0,
    truncated: false,
    timed_out: false,
    duration_ms: 1,
  });
  const { client, unmount } = renderPanel(
    <MachineOutput machineId={id} job={job} />,
  );
  expect(http.read).not.toHaveBeenCalled();
  await userEvent.click(
    screen.getByRole("button", { name: "Read live output" }),
  );
  expect(
    await screen.findByText("transient-private-result"),
  ).toBeInTheDocument();
  expect(http.read.mock.calls[0]?.[1]).toMatchObject({
    headers: { "Cache-Control": "no-store", Pragma: "no-cache" },
  });
  expect(
    JSON.stringify(
      client
        .getQueryCache()
        .getAll()
        .map((q) => q.state.data),
    ),
  ).not.toContain("transient-private-result");
  expect(
    JSON.stringify(
      client
        .getMutationCache()
        .getAll()
        .map((q) => q.state.data),
    ),
  ).not.toContain("transient-private-result");
  unmount();
  expect(
    screen.queryByText("transient-private-result"),
  ).not.toBeInTheDocument();
});

it("does not offer to recover expired output", async () => {
  http.get.mockResolvedValue({
    jobs: [{ ...job, output_availability: "expired" }],
    next_cursor: null,
  });
  renderPanel(
    <MachineOutput
      machineId={id}
      job={{ ...job, output_availability: "expired" }}
    />,
  );
  expect(
    screen.getByRole("button", { name: "Read live output" }),
  ).toBeDisabled();
  expect(
    screen.getByText(/Lost or expired output cannot be recovered/),
  ).toBeInTheDocument();
});

const confirmation: RemoteMachineConfirmation = {
  id,
  node_id: id,
  subject: {
    kind: "api_key",
    user_id: id,
    api_key_id: id,
    api_key_credential_id: id,
  },
  grant_id: id,
  grant_revision: 7,
  assignment_revision: 3,
  runtime_generation: 2,
  operation: "write file",
  parameters_digest: "safe-digest",
  runtime_id: id,
  review_parameters_available: true,
  safe_parameters: [
    { name: "Action", value: "Write a file inside the approved root" },
  ],
  status: "pending",
  created_at: "2026-10-14T08:00:00Z",
  expires_at: "2099-10-14T08:15:00Z",
  approved_at: null,
  approved_until: null,
};
it("a safe review requires acknowledgement and submits the bound decision contract", async () => {
  http.get.mockResolvedValue({
    confirmations: [confirmation],
    next_cursor: null,
  });
  http.read.mockResolvedValue({
    confirmation: {
      ...confirmation,
      status: "approved",
      approved_at: "2026-10-14T08:01:00Z",
      approved_until: "2099-10-14T08:25:00Z",
    },
  });
  renderPanel(
    <ConfirmationDecision
      machineId={id}
      confirmation={confirmation}
      onClose={() => {}}
    />,
  );
  expect(
    screen.getByRole("button", { name: "Approve one retry" }),
  ).toBeDisabled();
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I reviewed this identity/ }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Approve one retry" }),
  );
  await waitFor(() => expect(http.read).toHaveBeenCalled());
  expect(http.read.mock.calls[0]?.[0]).toBe(
    `/remote-machines/${id}/confirmations/${id}/decision`,
  );
  expect(http.read.mock.calls[0]?.[1].body).toEqual({
    approve: true,
    expected_grant_revision: 7,
    expected_runtime_generation: 2,
    parameters_digest: "safe-digest",
  });
});

it("a pending but locally expired safe review cannot be approved", async () => {
  renderPanel(
    <ConfirmationDecision
      machineId={id}
      confirmation={{ ...confirmation, expires_at: "2000-01-01T00:00:00Z" }}
      onClose={() => {}}
    />,
  );
  expect(
    screen.getByRole("button", { name: "Approve one retry" }),
  ).toBeDisabled();
  expect(http.read).not.toHaveBeenCalled();
});

it("reads current confirmation metadata but never enables decisions without a safe summary", async () => {
  http.get.mockResolvedValue({
    confirmations: [
      {
        id,
        node_id: id,
        principal: confirmation.subject,
        grant_id: id,
        grant_revision: 7,
        assignment_revision: 3,
        runtime_id: id,
        runtime_generation: 2,
        operation: "write file",
        parameters_digest: "safe-digest",
        state: "pending",
        created_at: confirmation.created_at,
        expires_at: confirmation.expires_at,
        decided_at: null,
        owner_only: true,
      },
    ],
  });
  const { client } = renderPanel(<RemoteMachineConfirmations machineId={id} />);
  expect(
    await screen.findByRole("button", { name: "Review decision" }),
  ).toBeDisabled();
  expect(
    screen.getByText(
      /A server-confirmed safe summary and known runtime generation/,
    ),
  ).toBeInTheDocument();
  http.get.mockRejectedValue(
    new ApiError(403, {
      error: "forbidden",
      error_code: 403,
      message: "Access revoked",
    }),
  );
  await act(async () => {
    await client.refetchQueries();
  });
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "Review decision" }),
    ).not.toBeInTheDocument(),
  );
  expect(http.read).not.toHaveBeenCalled();
});

function runtimeBillingFixture(): RemoteMachineRuntimeBilling {
  return {
    node_id: id,
    run_id: id,
    state: "stop_pending",
    rate_version: id,
    accepted_credits_per_second: "0.000040",
    covered_until: "2026-10-14T08:15:00Z",
    cumulative_seconds: 4,
    settled_credits: "0.000000000001",
    reserved_credits: "0.02",
    pending_credits: "0.01",
    funds: {
      wallet_available: "3",
      wallet_reserved: "0.02",
      pending_debits: "0",
      pending_expiry: "0",
      matching_allowance_seconds: 0,
      available_grants: "0",
      estimated_seconds: "75000",
      shared_account_estimate_only: true,
    },
    intervals: [],
    next_interval_offset: null,
    attribution: "machine_runtime",
    service_costs_included: false,
  };
}

it("renders exact runtime Credits without adding pending liability to reserved and selects historical rates", async () => {
  const billing = runtimeBillingFixture();
  const historicalId = "22222222-2222-4222-8222-222222222222";
  http.get.mockImplementation(async (path: string) => {
    if (path.includes("/billing/runs"))
      return {
        runs: [
          {
            run_id: historicalId,
            rate_version: historicalId,
            accepted_credits_per_second: "0.000020",
            state: "closed",
            runtime_generation: 1,
            created_at: "2026-10-13T08:00:00Z",
            updated_at: "2026-10-13T09:00:00Z",
          },
        ],
        next_offset: null,
      };
    if (path.includes(`run_id=${historicalId}`))
      return {
        ...billing,
        run_id: historicalId,
        rate_version: historicalId,
        accepted_credits_per_second: "0.000020",
      };
    return billing;
  });
  renderPanel(<RemoteMachineBilling machineId={id} />);
  expect(await screen.findByText("0.000000000001 Credits")).toBeInTheDocument();
  expect(screen.getByText("Pending within reserved")).toBeInTheDocument();
  expect(screen.queryByText("0.03 Credits")).not.toBeInTheDocument();
  expect(screen.getByText(/Service costs are not added/i)).toBeInTheDocument();
  await userEvent.click(
    await screen.findByRole("button", { name: `View run ${historicalId}` }),
  );
  expect(
    await screen.findByText("0.000020 Credits / second"),
  ).toBeInTheDocument();
  expect(
    http.get.mock.calls.some(([path]) =>
      (path as string).includes(`run_id=${historicalId}`),
    ),
  ).toBe(true);
});

it("keeps current billing visible when historical discovery is unavailable", async () => {
  http.get.mockImplementation(async (path: string) => {
    if (path.includes("/billing/runs"))
      throw new ApiError(404, {
        error: "not_found",
        error_code: 404,
        message: "Not mounted",
      });
    return runtimeBillingFixture();
  });
  renderPanel(<RemoteMachineBilling machineId={id} />);
  expect(
    await screen.findByText(/Historical contract lookup is unavailable/),
  ).toBeInTheDocument();
  expect(await screen.findByText("0.000000000001 Credits")).toBeInTheDocument();
});

it("checks query-required output and represents the real Node cache-evicted variant without fake empty success", async () => {
  http.read.mockResolvedValue({
    job_id: id,
    status: "finished",
    exit_code: 0,
    stdout: "",
    stderr: "",
    stdout_bytes: 123,
    stderr_bytes: 0,
    output_offset: 123,
    stderr_offset: 0,
    truncated: true,
    output_available: false,
    output_unavailable_reason: "cache_evicted",
    timed_out: false,
  });
  renderPanel(
    <MachineOutput
      machineId={id}
      job={{
        ...job,
        output_availability: "query_required",
        output_query_available: true,
        output_expires_at: null,
      }}
    />,
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Read live output" }),
  );
  expect(
    await screen.findByText(/Output unavailable: cache evicted/),
  ).toBeInTheDocument();
  expect(screen.queryByText("Standard output")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Next output page" }),
  ).not.toBeInTheDocument();
});

it("uses actual owner history cancellation acknowledgment without claiming physical stop", async () => {
  http.get.mockResolvedValue({
    jobs: [
      {
        id,
        job_id: id,
        node_id: id,
        runtime_id: id,
        runtime_generation: 2,
        state: "running",
        exit_code: null,
        stop_reason: null,
        duration_ms: null,
        cancellation_requested_by: null,
        cancellation_requested_at: null,
        binding: {
          kind: "external",
          principal: job.subject,
          grant_id: id,
          grant_revision: 7,
        },
        services: [],
        created_at: job.created_at,
        expires_at: job.expires_at,
        finished_at: null,
        retain_until: null,
        output_persisted: false,
        output_query_available: true,
        output_availability: "query_required",
        cancellation_available: true,
      },
    ],
    next_cursor: null,
    has_more: false,
    output_persisted: false,
  });
  http.read.mockResolvedValue({
    job_id: id,
    status: "cancelled",
    cancellation_acknowledged: true,
    stop_acknowledged: false,
  });
  renderPanel(<RemoteMachineJobs machineId={id} />);
  await userEvent.click(
    (await screen.findAllByRole("button", { name: "View job" }))[0]!,
  );
  expect(
    screen.getByRole("button", { name: "Read live output" }),
  ).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "Cancel job" }));
  await userEvent.click(
    screen.getByRole("button", { name: "Confirm cancellation" }),
  );
  expect(
    await screen.findByText(
      /Cancellation request acknowledged.*Physical stop is not confirmed/,
    ),
  ).toBeInTheDocument();
});
