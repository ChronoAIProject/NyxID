import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import type { MachineProfile } from "@/schemas/machines";
import {
  MachineIsolationBadge,
  MachineIsolationDetails,
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
