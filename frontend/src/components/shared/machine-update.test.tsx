import { render, screen, fireEvent } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { MachineUpdate } from "./machine-update";
import {
  machineMigrationCommand,
  machineCompanionCommand,
  machineCompanionReplacementCommand,
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
beforeEach(() => {
  start.mockReset();
  image.value = "ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater@sha256:" + "ab".repeat(32);
});
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
  expect(command).toContain("--tmpfs /tmp:rw,noexec,nosuid,size=16m");
  expect(command).not.toMatch(/token|nyx_nreg|latest/);
  expect(
    machineCompanionCommand(
      "work-machine",
      "0.41.0",
      "ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater@sha256:" +
        "ab".repeat(32),
    ),
  ).toContain("--read-only --tmpfs /tmp:rw,noexec,nosuid,size=16m");
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


it("shows the fixed failure code and actionable server guidance", () => {
  render(
    <MachineUpdate name="work-machine" status={{
      ...status, phase: "failed", code: "verify_updater_image:trust_root_unavailable",
      guidance: "Copy the current Machines command, which includes the required /tmp tmpfs.",
    }} />,
  );
  expect(screen.getByRole("status")).toHaveTextContent("verify_updater_image:trust_root_unavailable");
  expect(screen.getByRole("alert")).toHaveTextContent("required /tmp tmpfs");
});

it("shows companion progress separately from a connected machine and allows retry after failure", () => {
  const updater = {
    version: "0.41.0", digest: `sha256:${"ab".repeat(32)}`,
    target_version: "0.41.3", phase: "pending" as const, code: null,
  };
  const { rerender } = render(<MachineUpdate name="work-machine" status={{ ...status, phase: "connected", updater }} />);
  expect(screen.getByText("Updater: 0.41.0")).toBeVisible();
  expect(screen.getByText(/Updater update pending/)).toBeVisible();
  expect(screen.getByText(updater.digest)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Update machine" })).toBeDisabled();
  rerender(<MachineUpdate name="work-machine" status={{ ...status, phase: "connected",
    updater: { ...updater, phase: "failed", code: "update_companion:successor_unhealthy" },
    updater_guidance: "The previous updater retained control. Retry from Machines.",
  }} />);
  expect(screen.getByText(/Updater update failed/)).toHaveTextContent("update_companion:successor_unhealthy");
  expect(screen.getByRole("alert")).toHaveTextContent("previous updater retained control");
  expect(screen.getByRole("button", { name: "Update machine" })).toBeEnabled();
});


it("shows unknown version for legacy companions without inventing an installed release", () => {
  render(<MachineUpdate name="work-machine" status={status} />);
  expect(screen.getByText("Updater: version unavailable")).toBeVisible();
});

it("offers the exact pinned legacy companion replacement while keeping the machine and volume", () => {
  const legacy = { version: "", digest: null, target_version: null, phase: "legacy" as const, code: "update_companion:legacy_companion" };
  render(<MachineUpdate name="work-machine" status={{ ...status, updater: legacy }} />);
  expect(screen.getByText("Updater predates self-update. Replace it once")).toBeVisible();
  const expected = "docker rm -f 'work-machine-updater' && docker run -d --name 'work-machine-updater' --restart unless-stopped --label 'dev.nyxid.machine.updater=work-machine' --read-only --tmpfs /tmp:rw,noexec,nosuid,size=16m --cap-drop=ALL --security-opt=no-new-privileges --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock --mount 'type=volume,src=work-machine-nyxid-update,dst=/var/lib/nyxid-machine-update' " + image.value + " watch 'work-machine'";
  expect(screen.getByText(expected)).toBeInTheDocument();
  expect(machineCompanionReplacementCommand("work-machine", "0.41.0", image.value!)).toBe(expected);
  for (const invalid of ["bad;name", "$(touch bad)", "-work"]) {
    expect(() => machineCompanionReplacementCommand(invalid, "0.41.0", image.value!)).toThrow();
  }
  expect(() => machineCompanionReplacementCommand("work-machine", "0.41.0")).toThrow();
  fireEvent.click(screen.getByRole("button", { name: "Start guided update" }));
  expect(screen.getByText(/machine stays running/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Confirm update" }));
  expect(start).toHaveBeenCalledTimes(1);
});

it("withholds the legacy replacement command until the pinned image is verified", () => {
  image.value = null;
  render(<MachineUpdate name="work-machine" status={{ ...status, updater: {
    version: "", digest: null, target_version: null, phase: "legacy", code: "update_companion:legacy_companion",
  } }} />);
  expect(screen.getByText("Verifying the updater image, try again shortly.")).toBeVisible();
  expect(screen.queryByText(/docker rm/)).not.toBeInTheDocument();
});
