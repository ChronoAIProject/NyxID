import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { MachineSettings } from "./machine-settings";
import type { NodeInfo } from "@/types/nodes";
const save = vi.hoisted(() => vi.fn());
const grant = vi.hoisted(() => vi.fn());
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));
vi.mock("@/hooks/use-machines", () => ({
  useMachineSettings: () => ({ mutate: save }),
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useSetNyxBotAgentGrants: () => ({ mutate: grant }),
  useNyxBotAgents: () => ({
    data: {
      agents: [
        {
          id: "specialist",
          name: "Coder",
          kind: "specialist",
          machines: ["node", "other"],
          services: ["github"],
          account_read: true,
        },
      ],
    },
  }),
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
const node = {
  id: "node",
  machine_confirm: "none",
  allow_single_user_saved_logins: false,
  machine: {
    shell: true,
    files: true,
    computer: true,
    roots: ["/workspace"],
    os: "macos",
    arch: "arm64",
    browser_isolated: false,
    computer_ready: true,
    cua_version: "0.30.4",
    computer_mode: "standard",
  },
} as NodeInfo;
it("requires the owner to acknowledge the single-user warning before enabling filling", () => {
  render(<MachineSettings node={node} canManage />);
  expect(screen.getByRole("checkbox", { name: "Coder" })).toBeChecked();
  expect(
    screen.getByText(/prompt-injected agent could read/),
  ).toBeInTheDocument();
  fireEvent.click(
    screen.getByRole("checkbox", {
      name: "Allow saved-login typing on this machine",
    }),
  );
  expect(
    screen.getByRole("button", { name: "Save machine settings" }),
  ).toBeDisabled();
  fireEvent.click(
    screen.getByRole("checkbox", {
      name: "I understand the shared-user risk and allow typing.",
    }),
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Save machine settings" }),
  );
  expect(save).toHaveBeenCalledWith({
    machine_confirm: "none",
    allow_single_user_saved_logins: true,
    acknowledge_single_user_risk: true,
  });
});
it("does not expose owner settings to a reader", () => {
  render(<MachineSettings node={node} canManage={false} />);
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Save machine settings" }),
  ).not.toBeInTheDocument();
});

it("keeps the non-isolated shell warning visible to readers", () => {
  render(<MachineSettings node={node} canManage={false} />);
  expect(screen.getByText("Not isolated")).toBeInTheDocument();
  expect(
    screen.getByText(/stored credentials, signing secret and node token/),
  ).toBeInTheDocument();
});
it("does not label separated machines as non-isolated", () => {
  render(
    <MachineSettings
      node={{ ...node, machine: { ...node.machine!, browser_isolated: true } }}
      canManage
    />,
  );
  expect(screen.queryByText("Not isolated")).not.toBeInTheDocument();
});

it("changes one machine grant while preserving the specialist services and other machines", () => {
  render(<MachineSettings node={node} canManage />);
  fireEvent.click(screen.getByRole("checkbox", { name: "Coder" }));
  expect(grant).toHaveBeenCalledWith({
    id: "specialist",
    services: ["github"],
    account_read: true,
    machines: ["other"],
  });
  expect(grant.mock.calls[0]?.[0]).not.toHaveProperty("logins");
  expect(grant.mock.calls[0]?.[0]).not.toHaveProperty("guest_access");
});
