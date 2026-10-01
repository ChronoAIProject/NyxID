import { render, screen, fireEvent } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { MachineUpdate } from "./machine-update";
import {
  machineMigrationCommand,
  machineCompanionCommand,
  type MachineUpdateStatus,
} from "@/schemas/machines";

const start = vi.hoisted(() => vi.fn());
const image = vi.hoisted(() => ({
  value: ("ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater@sha256:" +
    "ab".repeat(32)) as string | null,
}));
vi.mock("@/hooks/use-machines", () => ({
  useVerifiedUpdaterImage: () => ({
    data: { version: "0.41.0", image: image.value },
  }),
  useStartMachineUpdate: () => ({ mutate: start, isPending: false }),
}));
vi.mock("@/components/shared/copyable-field", () => ({
  CopyableField: ({ value }: { value: string }) => <pre>{value}</pre>,
}));
const status: MachineUpdateStatus = {
  node_id: "machine",
  current_version: "0.40.0",
  target_version: "0.41.0",
  update_available: true,
  updater_ready: true,
  installation: "container",
  automatic: false,
  phase: "idle",
  code: null,
  settings_path: "/assistant/machines?machine=machine",
};
beforeEach(() => start.mockReset());
it("requires explicit human confirmation before sending update and shows progress", () => {
  const { rerender } = render(
    <MachineUpdate name="work-machine" status={status} />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Update machine" }));
  expect(start).not.toHaveBeenCalled();
  expect(screen.getByText(/restarts and interrupts/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Confirm update" }));
  expect(start).toHaveBeenCalledTimes(1);
  rerender(
    <MachineUpdate
      name="work-machine"
      status={{ ...status, phase: "restarting" }}
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent("restarting");
});
it("shows an exact token-free host migration with preserved volume inspection", () => {
  render(
    <MachineUpdate
      name="work-machine"
      status={{ ...status, updater_ready: false }}
    />,
  );
  expect(
    screen.getByText(/identity, workspace and settings/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/bootstrap 'work-machine' '0.41.0'/),
  ).toBeInTheDocument();
  const command = machineMigrationCommand(
    "work-machine",
    "0.41.0",
    "ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater@sha256:" +
      "ab".repeat(32),
  );
  expect(command).toContain("work-machine-nyxid-update");
  expect(command).not.toMatch(/token|nyx_nreg|latest/);
  expect(
    machineCompanionCommand(
      "work-machine",
      "0.41.0",
      "ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater@sha256:" +
        "ab".repeat(32),
    ),
  ).toContain("--read-only");
  expect(() => machineMigrationCommand("machine; evil", "0.41.0")).toThrow();
});

it("never offers an unpinned command while attestation verification is unavailable", () => {
  image.value = null;
  const view = render(
    <MachineUpdate
      name="work-machine"
      status={{ ...status, updater_ready: false }}
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent(
    "Verifying the updater image",
  );
  expect(screen.queryByText(/docker run/)).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Start guided update" }),
  ).toBeVisible();
  expect(
    screen.getByText("How updates work").closest("details"),
  ).not.toHaveAttribute("open");
  for (const image of [
    undefined,
    "ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater:0.41.0",
    "attacker/image@sha256:" + "ab".repeat(32),
  ]) {
    expect(() =>
      machineMigrationCommand("work-machine", "0.41.0", image),
    ).toThrow();
  }
  view.unmount();
  image.value =
    "ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater@sha256:" +
    "ab".repeat(32);
});
