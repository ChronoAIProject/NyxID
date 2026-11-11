import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import type { MachineProfile } from "@/schemas/machines";
import {
  MachineIsolationBadge,
  MachineIsolationDetails,
  MachineSeparationSupport,
} from "./machine-isolation";

it("distinguishes verified command isolation from browser isolation and old nodes", () => {
  const machine: MachineProfile = {
    version: 1, cua_version: null, computer_ready: false, saved_login_ready: false,
    shell: true,
    files: true,
    computer: false,
    os: "linux",
    arch: "aarch64",
    roots: ["/workspace"],
    computer_tools: [],
    computer_mode: "standard",
    browser_isolated: false,
  };
  const view = render(<MachineIsolationBadge machine={machine} />);
  expect(screen.getByText("Isolation unknown: update to check")).toBeVisible();
  view.rerender(
    <MachineIsolationBadge machine={{ ...machine, commands_isolated: true }} />,
  );
  expect(screen.queryByText(/isolated|Isolation/)).not.toBeInTheDocument();
  view.rerender(
    <>
      <MachineIsolationBadge
        machine={{
          ...machine,
          browser_isolated: true,
          commands_isolated: false,
        }}
      />
      <MachineIsolationDetails
        machine={{ ...machine, commands_isolated: false }}
      />
    </>,
  );
  expect(screen.getByText("Not isolated")).toBeVisible();
  expect(screen.getByText(/Agent commands can read/)).toBeVisible();
  expect(screen.getByText("Details").closest("details")).not.toHaveAttribute(
    "open",
  );
});

const supported: MachineProfile = {
  version: 1, shell: true, files: true, computer: false, os: "linux", arch: "aarch64",
  roots: ["/workspace"], computer_mode: "standard", cua_version: null,
  computer_tools: [], computer_ready: false, browser_isolated: true,
  commands_isolated: true, saved_login_ready: false, authority_versions: [2],
  separation_choice: "shared_legacy",
  separated: { available: true, landlock_abi: 8, reason: null },
  separation_evidence: {
    version: 4, os: "linux", landlock_abi: 8, separate_users: true,
    managed_browser_configured: true, command_seccomp: true,
    landlock_enforced: true, managed_browser_sandbox: true, managed_browser_policy: true,
    credential_custody: true,
  },
};

it("shows confirmed support without changing the owner's existing legacy choice", () => {
  render(<MachineSeparationSupport machine={supported} />);
  expect(screen.getByText("Separation support confirmed")).toBeVisible();
  expect(screen.getByText(/existing shared legacy assignments stay shared/)).toBeVisible();
});

it.each([undefined, null, { ...supported.separation_evidence!, version: 1 }, { ...supported.separation_evidence!, version: 2 }, { ...supported.separation_evidence!, version: 3 }, { ...supported.separation_evidence!, version: 5 }])(
  "does not treat old booleans or unsupported evidence versions as confirmation (%j)",
  separation_evidence => {
    render(<MachineSeparationSupport machine={{ ...supported, separation_evidence }} />);
    expect(screen.getByText("Separation support unknown")).toBeVisible();
    expect(screen.queryByText("Separation support confirmed")).not.toBeInTheDocument();
    expect(screen.getByText(/Upgrade or reconnect the node/)).toBeVisible();
  },
);

it.each([undefined, null, false])(
  "requires positively observed credential custody (%j)",
  credential_custody => {
    render(<MachineSeparationSupport machine={{ ...supported, separation_evidence: { ...supported.separation_evidence!, credential_custody } }} />);
    expect(screen.getByText("Separation prerequisites not met")).toBeVisible();
    expect(screen.getByText(/Credential custody could not be verified/)).toBeVisible();
    expect(screen.getByText(/protected credential storage and rerun setup/)).toBeVisible();
    expect(screen.getByText("node_custody_unverified")).toBeInTheDocument();
    expect(screen.queryByText("Separation support confirmed")).not.toBeInTheDocument();
  },
);

it.each(["separate_users", "managed_browser_configured", "command_seccomp", "landlock_enforced", "managed_browser_sandbox", "managed_browser_policy"] as const)(
  "requires positive evidence for %s and displays a failed prerequisite separately",
  field => {
    const view = render(<MachineSeparationSupport machine={{ ...supported, separation_evidence: { ...supported.separation_evidence!, [field]: null } }} />);
    expect(screen.getByText("Separation support unknown")).toBeVisible();
    view.rerender(<MachineSeparationSupport machine={{ ...supported, separation_evidence: { ...supported.separation_evidence!, [field]: false } }} />);
    expect(screen.getByText("Separation prerequisites not met")).toBeVisible();
  },
);

it.each([
  [{ os: "macos" }, /macOS has no supported per-agent sandbox/],
  [{ landlock_abi: 5 }, /Working Landlock ABI 6/],
  [{ managed_browser_sandbox: false }, /actual managed Chromium.*AppArmor/],
  [{ managed_browser_policy: false }, /actual managed secure browser.*blocks DevTools/],
] as const)("explains the actual host prerequisite (%j)", (patch, message) => {
  render(<MachineSeparationSupport machine={{ ...supported, separation_evidence: { ...supported.separation_evidence!, ...patch } }} />);
  expect(screen.getByText("Separation prerequisites not met")).toBeVisible();
  expect(screen.getByText(message)).toBeVisible();
});

it("gives timeout retry guidance without claiming the host cannot support separation", () => {
  render(<MachineSeparationSupport machine={{ ...supported, separation_evidence: { ...supported.separation_evidence!, probe_timed_out: true } }} />);
  expect(screen.getByText("Separation probe timed out")).toBeVisible();
  expect(screen.getByText(/cold Chromium start.*rerun setup/)).toBeVisible();
  expect(screen.queryByText("Separation prerequisites not met")).not.toBeInTheDocument();
});

it.each([undefined, false, true])("explains inaccessible container observations and never accepts the sandbox boolean (%j)", managed_browser_sandbox => {
  render(<MachineSeparationSupport machine={{ ...supported, separation_evidence: {
    ...supported.separation_evidence!, managed_browser_sandbox,
    browser_sandbox_unobservable_in_container: true, probe_timed_out: true,
  } }} />);
  expect(screen.getByText("Separation prerequisites not met")).toBeVisible();
  expect(screen.getByText(/container does not expose every kernel observation/)).toBeVisible();
  expect(screen.getByText(/separate machine container or VM per agent/)).toBeVisible();
  expect(screen.getByText("separated_browser_sandbox_unobservable_in_container")).toBeInTheDocument();
  expect(screen.queryByText("Separation support confirmed")).not.toBeInTheDocument();
});

it("requires the current authority protocol and rejects unknown future reasons", () => {
  const view = render(<MachineSeparationSupport machine={{ ...supported, authority_versions: [1] }} />);
  expect(screen.getByText("Separation support unknown")).toBeVisible();
  view.rerender(<MachineSeparationSupport support={{ available: false, reason: "future_reason" }} />);
  expect(screen.getByText("Separation support unknown")).toBeVisible();
});
