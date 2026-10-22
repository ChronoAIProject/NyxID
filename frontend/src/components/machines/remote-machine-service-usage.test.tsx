import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
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
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { RemoteMachineServiceUsage } from "./remote-machine-service-usage";

const transport = vi.fn<typeof fetch>();
const auth = useAuthStore.getState();
const clients: QueryClient[] = [];
const attribution = {
  node_id: "8b3ecfd0-4d32-4bc7-9655-bb03f2ebf9df",
  job_id: "job-one",
  runtime_id: "runtime-one",
  runtime_generation: 3,
  grant_id: "grant-one",
  principal_kind: "api_key",
};
const row = {
  service_slug: "gateway-service",
  service_id: "service-one",
  metric: "requests",
  lago_metric_code: "platform_svc_gateway",
  layer: "platform",
  operation: null,
  model: null,
  api_key_id: null,
  api_key_name: null,
  quantity: 2,
  requests: 2,
  bytes: 100,
  events: 2,
  lago_acked: true,
  billable: true,
  estimated_credits: "0.123456789012",
  estimated_credits_micros: 999999,
  machine_usage: attribution,
};
const payload = {
  machine_id: attribution.node_id,
  job_id: null,
  owner_id: "owner-one",
  period: "30d",
  rows: [row],
  totals: {
    quantity: 99,
    requests: 99,
    bytes: 100,
    events: 99,
    estimated_credits: "99",
  },
  billing: {
    charging_enabled: true,
    lago_configured: true,
    source: "usage_meter",
    rates_are_approximate: true,
  },
};
const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), {
    status,
    headers: { "Content-Type": "application/json" },
  });
function mount() {
  const client = new QueryClient();
  clients.push(client);
  return render(
    <QueryClientProvider client={client}>
      <RemoteMachineServiceUsage machineId="8b3ecfd0-4d32-4bc7-9655-bb03f2ebf9df" />
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  transport.mockReset();
  vi.stubGlobal("fetch", transport);
  useAuthStore.setState({ user: { id: "owner-one" } as User });
});
afterEach(() => {
  cleanup();
  clients.splice(0).forEach((client) => client.clear());
  vi.unstubAllGlobals();
  useAuthStore.setState(auth);
});

it("loads only on demand using the supported period and keeps exact service/job attribution", async () => {
  transport.mockResolvedValueOnce(json(payload));
  mount();
  expect(transport).not.toHaveBeenCalled();
  await userEvent.click(
    screen.getByRole("button", { name: "Load service usage" }),
  );
  expect(await screen.findByText("gateway-service")).toBeInTheDocument();
  expect(transport.mock.calls[0]?.[0]).toBe(
    `/api/v1/billing/usage?period=30d&machine_id=${attribution.node_id}`,
  );
  expect(screen.getByText("job-one")).toBeInTheDocument();
  expect(screen.getByText("runtime-one")).toBeInTheDocument();
  expect(screen.getByText("grant-one")).toBeInTheDocument();
  expect(screen.getByText("0.123457 Credits")).toHaveAttribute(
    "title",
    "0.123456789012 Credits",
  );
  expect(screen.queryByText("99 Credits")).not.toBeInTheDocument();
  expect(screen.getByText(/Last 30 days/)).toBeInTheDocument();
});

it("excludes different-Machine and missing-attribution rows without claiming zero cost", async () => {
  transport.mockResolvedValueOnce(
    json({
      ...payload,
      rows: [
        {
          ...row,
          service_slug: "other-machine-service",
          machine_usage: { ...attribution, node_id: "other-machine" },
        },
        {
          ...row,
          machine_usage: undefined,
          service_slug: "unattributed-service",
        },
      ],
    }),
  );
  mount();
  await userEvent.click(
    screen.getByRole("button", { name: "Load service usage" }),
  );
  expect(
    await screen.findByText(/No attributed service usage was returned/),
  ).toBeInTheDocument();
  expect(screen.queryByText("other-machine-service")).not.toBeInTheDocument();
  expect(screen.queryByText("unattributed-service")).not.toBeInTheDocument();
  expect(
    screen.getByText(/does not establish zero service cost/),
  ).toBeInTheDocument();
});

it.each([
  { owner_id: "other-owner" },
  { period: "all" },
  { machine_id: "b3ecfd00-4d32-4bc7-9655-bb03f2ebf9df" },
  { machine_id: undefined },
])("rejects mismatched account/period payload %j", async (change) => {
  transport.mockResolvedValueOnce(json({ ...payload, ...change }));
  mount();
  await userEvent.click(
    screen.getByRole("button", { name: "Load service usage" }),
  );
  expect(
    await screen.findByText(/Service usage could not be verified/),
  ).toBeInTheDocument();
  expect(screen.queryByText("gateway-service")).not.toBeInTheDocument();
});

it("hides earlier rows when a live refresh fails", async () => {
  transport.mockResolvedValueOnce(json(payload));
  mount();
  await userEvent.click(
    screen.getByRole("button", { name: "Load service usage" }),
  );
  await screen.findByText("gateway-service");
  transport.mockResolvedValueOnce(
    json(
      { error: "unavailable", error_code: 503, message: "Unavailable" },
      503,
    ),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Refresh service usage" }),
  );
  expect(
    await screen.findByText(/Service usage could not be verified/),
  ).toBeInTheDocument();
  expect(screen.queryByText("gateway-service")).not.toBeInTheDocument();
});

it("discards a late read and requires a fresh explicit load after an account switch", async () => {
  let resolve!: (response: Response) => void;
  transport.mockImplementationOnce(
    () =>
      new Promise<Response>((done) => {
        resolve = done;
      }),
  );
  mount();
  await userEvent.click(
    screen.getByRole("button", { name: "Load service usage" }),
  );
  await waitFor(() => expect(transport).toHaveBeenCalledTimes(1));
  act(() => {
    useAuthStore.setState({ user: { id: "owner-two" } as User });
  });
  await act(async () => {
    resolve(json(payload));
  });
  expect(screen.queryByText("gateway-service")).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Load service usage" }),
  ).toBeInTheDocument();
  expect(transport).toHaveBeenCalledTimes(1);
});

it("does not replace an explicitly unavailable exact cost with legacy micros or zero", async () => {
  transport.mockResolvedValueOnce(
    json({
      ...payload,
      rows: [
        {
          ...row,
          estimated_credits: null,
          machine_usage: {
            ...attribution,
            runtime_generation: null,
            grant_id: null,
          },
        },
      ],
    }),
  );
  mount();
  await userEvent.click(
    screen.getByRole("button", { name: "Load service usage" }),
  );
  expect(await screen.findByText("Unavailable Credits")).toBeInTheDocument();
  expect(screen.getByText("Unknown · api_key")).toBeInTheDocument();
  expect(screen.getByText("Not reported")).toBeInTheDocument();
  expect(screen.queryByText("0.999999 Credits")).not.toBeInTheDocument();
});

it("labels metering-only rows as uncharged", async () => {
  transport.mockResolvedValueOnce(
    json({
      ...payload,
      rows: [{ ...row, billable: false, estimated_credits: null }],
    }),
  );
  mount();
  await userEvent.click(
    screen.getByRole("button", { name: "Load service usage" }),
  );
  expect(await screen.findByText("Not charged")).toBeInTheDocument();
  expect(screen.getByText("Metering only")).toBeInTheDocument();
});

it("keeps billable zero estimates, unavailable amounts and metering-only usage distinct", async () => {
  transport.mockResolvedValueOnce(
    json({
      ...payload,
      rows: [
        { ...row, service_slug: "zero-service", estimated_credits: "0" },
        { ...row, service_slug: "unknown-service", estimated_credits: null },
        {
          ...row,
          service_slug: "metered-service",
          billable: false,
          estimated_credits: null,
        },
      ],
    }),
  );
  mount();
  await userEvent.click(
    screen.getByRole("button", { name: "Load service usage" }),
  );
  const zero = within(
    (await screen.findByText("zero-service")).closest("article")!,
  );
  expect(zero.getByText("Billable")).toBeInTheDocument();
  expect(zero.getByText("Zero estimate")).toBeInTheDocument();
  expect(zero.getByText("0 Credits")).toHaveAttribute("title", "0 Credits");
  const unknown = within(
    screen.getByText("unknown-service").closest("article")!,
  );
  expect(unknown.getByText("Amount unknown")).toBeInTheDocument();
  expect(unknown.getByText("Unavailable Credits")).toBeInTheDocument();
  const metered = within(
    screen.getByText("metered-service").closest("article")!,
  );
  expect(metered.getByText("Metering only")).toBeInTheDocument();
  expect(metered.getByText("Not charged")).toBeInTheDocument();
  expect(metered.queryByText("Amount unknown")).not.toBeInTheDocument();
});
