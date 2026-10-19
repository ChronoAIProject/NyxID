import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { RemoteMachineUsage } from "./remote-machine-usage";
import { remoteMachineUsageSchema } from "@/schemas/remote-machine-usage";

const mock = vi.hoisted(() => ({ query: vi.fn() }));
vi.mock("@/hooks/use-remote-machine-usage", () => ({
  useRemoteMachineUsage: mock.query,
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
const unavailable = {
  available: false,
  stale: true,
  received_at: null,
  workspace: null,
  temporary: null,
  disk_full: false,
  write_limit_bytes: 1048576,
  assistant_arguments_json_limit_bytes: 131072,
  edit_file_limit_bytes: 16777216,
  recovery: "No observation is not an empty disk.",
};

it("renders real absence as unknown without a zero-used measurement", () => {
  mock.query.mockReturnValue({
    data: remoteMachineUsageSchema.parse(unavailable),
  });
  render(<RemoteMachineUsage machineId="machine" />);
  expect(screen.getByText(/Disk usage is unknown/)).toBeInTheDocument();
  expect(screen.queryByText(/0 bytes used/)).not.toBeInTheDocument();
});

it("preserves stale and partial evidence and shows reported disk-full recovery", () => {
  mock.query.mockReturnValue({
    data: remoteMachineUsageSchema.parse({
      ...unavailable,
      available: true,
      disk_full: true,
      received_at: "2026-10-14T00:00:00Z",
      workspace: {
        capacity_bytes: 1024,
        used_bytes: 1024,
        available_bytes: 0,
        complete: false,
        measurement: "bounded_scan",
      },
    }),
  });
  render(<RemoteMachineUsage machineId="machine" />);
  expect(screen.getByText(/Stale observation/)).toBeInTheDocument();
  expect(screen.getByText(/Partial observation/)).toBeInTheDocument();
  expect(screen.getByRole("alert")).toHaveTextContent(/full disk/);
  expect(
    screen.getByText(/Usage unknown; no current measurement/),
  ).toBeInTheDocument();
});

it("does not render retained observations after a failed live read", () => {
  mock.query.mockReturnValue({
    data: unavailable,
    error: new Error("Forbidden"),
    refetch: vi.fn(),
  });
  render(<RemoteMachineUsage machineId="machine" />);
  expect(
    screen.getByText(/Storage observations are unavailable/),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/Node decoded file-write limit/),
  ).not.toBeInTheDocument();
});

it("distinguishes the real decoded write, whole-JSON and edit limits", () => {
  mock.query.mockReturnValue({
    data: remoteMachineUsageSchema.parse(unavailable),
  });
  render(<RemoteMachineUsage machineId="machine" />);
  expect(screen.getByText(/Node decoded file-write limit/)).toHaveTextContent(
    "1,048,576 bytes",
  );
  expect(
    screen.getByText(/NyxBot whole-JSON argument limit/),
  ).toHaveTextContent("131,072 bytes");
  expect(
    screen.getByText(/Exact-edit original and result file limit/),
  ).toHaveTextContent("16,777,216 bytes");
  expect(
    screen.getByText(/Escaped content and every other argument/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/equally large edit request fits the transport/),
  ).toBeInTheDocument();
});

it("keeps a partial temporary scan separate from remaining Pod quota", () => {
  mock.query.mockReturnValue({
    data: remoteMachineUsageSchema.parse({
      ...unavailable,
      available: true,
      stale: false,
      temporary: {
        capacity_bytes: null,
        used_bytes: 100,
        available_bytes: null,
        complete: false,
        measurement: "node_paths_lower_bound",
      },
    }),
  });
  render(<RemoteMachineUsage machineId="machine" />);
  expect(
    screen.getByText(/remaining Pod quota cannot be inferred/),
  ).toBeInTheDocument();
  expect(screen.getByText(/Available: Unknown/)).toBeInTheDocument();
});

it("keeps older server limits unknown instead of inserting invented values", () => {
  const legacy: Partial<typeof unavailable> = { ...unavailable };
  delete legacy.assistant_arguments_json_limit_bytes;
  delete legacy.edit_file_limit_bytes;
  mock.query.mockReturnValue({ data: remoteMachineUsageSchema.parse(legacy) });
  render(<RemoteMachineUsage machineId="machine" />);
  expect(screen.getAllByText(/Not reported by this server/)).toHaveLength(2);
});

it("keeps stale provider totals separate from fresh Node reports and never sums aggregate components", () => {
  mock.query.mockReturnValue({
    data: remoteMachineUsageSchema.parse({
      ...unavailable,
      available: true,
      stale: false,
      platform_storage: {
        source: "kubelet_summary",
        used_bytes: 500,
        capacity_bytes: null,
        available_bytes: null,
        complete: false,
        stale: true,
        received_at: "2026-10-14T00:00:00Z",
        components: [
          {
            kind: "pod_ephemeral_aggregate",
            used_bytes: 500,
            sampled_at: "2026-10-14T00:00:00Z",
          },
          {
            kind: "container_logs",
            used_bytes: 200,
            sampled_at: "2026-10-14T00:00:00Z",
          },
        ],
        missing_reasons: ["container_rootfs_stale"],
      },
    }),
  });
  render(<RemoteMachineUsage machineId="machine" />);
  expect(
    screen.getByText(
      /Stale platform observation; fresh Node reports do not refresh/,
    ),
  ).toBeInTheDocument();
  expect(screen.getByText(/500 bytes reported used/)).toBeInTheDocument();
  expect(screen.queryByText(/700 bytes reported used/)).not.toBeInTheDocument();
  expect(screen.getByText(/Available: Unknown/)).toBeInTheDocument();
  expect(
    remoteMachineUsageSchema.safeParse({
      ...unavailable,
      platform_storage: { available_bytes: 1 },
    }).success,
  ).toBe(false);
});


it("keeps reliable workspace reporting available without an optional Pod monitor", () => {
  mock.query.mockReturnValue({
    data: remoteMachineUsageSchema.parse({
      ...unavailable,
      available: true,
      stale: false,
      workspace: { capacity_bytes: 2048, used_bytes: 512, available_bytes: 1536, complete: true, measurement: "filesystem" },
      platform_storage: null,
    }),
  });
  render(<RemoteMachineUsage machineId="machine" />);
  expect(screen.getByText(/512 bytes used.*2,048 bytes capacity/)).toBeInTheDocument();
  expect(screen.getByText(/Optional Pod monitoring is unavailable/)).toBeInTheDocument();
  expect(screen.getByText(/1,536 bytes/)).toBeInTheDocument();
  expect(screen.queryByText(/0 bytes reported used/)).not.toBeInTheDocument();
});

it("does not present partial components as total Pod temporary usage", () => {
  mock.query.mockReturnValue({
    data: remoteMachineUsageSchema.parse({
      ...unavailable,
      platform_storage: {
        source: "kubelet_components_partial", used_bytes: 200, capacity_bytes: null, available_bytes: null, complete: false, stale: false,
        received_at: "2026-10-14T00:00:00Z",
        components: [{ kind: "container_logs", used_bytes: 200, sampled_at: "2026-10-14T00:00:00Z" }],
        missing_reasons: ["pod_level_logs_hosts_coverage_unknown"],
      },
    }),
  });
  render(<RemoteMachineUsage machineId="machine" />);
  expect(screen.getByText(/Total Pod usage unknown/)).toBeInTheDocument();
  expect(screen.queryByText(/200 bytes reported used/)).not.toBeInTheDocument();
  expect(screen.getByText(/container logs: 200 bytes/)).toBeInTheDocument();
});


it.each([null, undefined])("renders undeclared temporary capacity without inventing an allocation: %s", (capacity) => {
  const temporary = { capacity_bytes: capacity, used_bytes: 300, available_bytes: null, complete: false, measurement: "node_paths_lower_bound" };
  if (capacity === undefined) delete temporary.capacity_bytes;
  mock.query.mockReturnValue({ data: remoteMachineUsageSchema.parse({
    ...unavailable, available: true, received_at: "2026-10-14T00:01:00Z", sampled_at: "2026-10-14T00:00:00Z", temporary,
    workspace: { capacity_bytes: 2048, used_bytes: 512, available_bytes: 1536, complete: true, measurement: "filesystem" },
  }) });
  render(<RemoteMachineUsage machineId="machine" />);
  expect(screen.getByText(/300 bytes used.*Capacity: Unknown/)).toBeInTheDocument();
  expect(screen.getByText(/512 bytes used.*2,048 bytes capacity/)).toBeInTheDocument();
  expect(screen.getByText(/Measured:/)).toBeInTheDocument();
  expect(screen.getByText(/node_paths_lower_bound/)).toBeInTheDocument();
  expect(screen.getByText(/remaining Pod quota cannot be inferred/)).toBeInTheDocument();
});

it("rejects fabricated numeric temporary capacity or free space", () => {
  for (const patch of [
    { temporary: { capacity_bytes: 2 ** 31, used_bytes: 100, available_bytes: null, complete: false, measurement: "node_paths_lower_bound" } },
    { temporary: { capacity_bytes: null, used_bytes: 100, available_bytes: 2 ** 31, complete: false, measurement: "node_paths_lower_bound" } },
    { platform_storage: { source: "kubelet_summary", used_bytes: null, capacity_bytes: 2 ** 31, available_bytes: null, complete: false, stale: false, received_at: "2026-10-14T00:00:00Z", components: [], missing_reasons: [] } },
  ]) expect(remoteMachineUsageSchema.safeParse({ ...unavailable, ...patch }).success).toBe(false);
});


it("accepts an omitted platform capacity while preserving the observed source and timestamp", () => {
  mock.query.mockReturnValue({ data: remoteMachineUsageSchema.parse({
    ...unavailable,
    platform_storage: { source: "kubelet_summary", used_bytes: 500, available_bytes: null, complete: false, stale: false,
      received_at: "2026-10-14T00:00:00Z", components: [{ kind: "pod_ephemeral_aggregate", used_bytes: 500, sampled_at: "2026-10-14T00:00:00Z" }], missing_reasons: [] },
  }) });
  render(<RemoteMachineUsage machineId="machine" />);
  expect(screen.getByText(/500 bytes reported used.*No fixed capacity declared/)).toBeInTheDocument();
  expect(screen.getByText(/Source: kubelet_summary.*Received:/)).toBeInTheDocument();
});
