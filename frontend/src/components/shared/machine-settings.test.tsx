import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useMachineSettingsDraftStore } from "@/stores/machine-settings-draft-store";
import { MachineSettings } from "./machine-settings";
import type { NodeInfo } from "@/types/nodes";
const save = vi.hoisted(() => vi.fn());
const grant = vi.hoisted(() => vi.fn());
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => false }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    search,
  }: {
    children: React.ReactNode;
    to: string;
    search?: { agent?: string; details?: string };
  }) => (
    <a
      href={`${to}${search?.agent ? `?agent=${search.agent}${search.details ? `&details=${search.details}` : ""}` : ""}`}
    >
      {children}
    </a>
  ),
}));
vi.mock("@/hooks/use-machines", () => ({
  useVerifiedUpdaterImage: () => ({
    data: {
      version: "0.41.0",
      image:
        "ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater@sha256:" +
        "ab".repeat(32),
    },
  }),
  useMachineSettings: () => ({ mutateAsync: save }),
  useMachineUpdatePolicy: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useSetNyxBotAgentGrants: () => ({ mutateAsync: grant }),
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
  useMachineSettingsDraftStore.setState({ drafts: {} });
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
    commands_isolated: false,
    computer_ready: true,
    cua_version: "0.30.4",
    computer_mode: "standard",
  },
} as NodeInfo;
it("requires the owner to acknowledge the single-user warning before enabling filling", async () => {
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
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith({
      machine_confirm: "none",
      allow_single_user_saved_logins: true,
      acknowledge_single_user_risk: true,
    }),
  );
});
it("does not expose owner settings to a reader", () => {
  render(<MachineSettings node={node} canManage={false} />);
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Save machine settings" }),
  ).not.toBeInTheDocument();
});

it("explains configured shared access on a headless machine without a browser toggle", () => {
  render(
    <MachineSettings
      node={{
        ...node,
        machine: {
          ...node.machine!,
          computer: false,
          browser: false,
          separated: {
            available: false,
            landlock_abi: 8,
            reason: "separated_requires_managed_browser",
          },
        },
      }}
      canManage
    />,
  );
  expect(
    screen.getByText(
      /Agents configured for Shared legacy can run granted commands/,
    ),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/Browser permissions are not required/),
  ).toBeInTheDocument();
  expect(
    screen.getByText("separated_requires_managed_browser").closest("details"),
  ).not.toHaveAttribute("open");
  expect(screen.queryByText("Live desktop")).not.toBeInTheDocument();
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
      node={{
        ...node,
        machine: {
          ...node.machine!,
          browser_isolated: true,
          commands_isolated: true,
        },
      }}
      canManage
    />,
  );
  expect(screen.queryByText("Not isolated")).not.toBeInTheDocument();
});

it("changes one machine grant while preserving the specialist services and other machines", async () => {
  render(<MachineSettings node={node} canManage />);
  fireEvent.click(screen.getByRole("checkbox", { name: "Coder" }));
  expect(grant).not.toHaveBeenCalled();
  fireEvent.click(
    screen.getByRole("button", { name: "Save machine settings" }),
  );
  fireEvent.click(
    await screen.findByRole("button", { name: "Save and remove access" }),
  );
  await waitFor(() =>
    expect(grant).toHaveBeenCalledWith({
      id: "specialist",
      machine_membership: { node_id: node.id, assigned: false },
    }),
  );
  expect(grant.mock.calls[0]?.[0]).not.toHaveProperty("logins");
  expect(grant.mock.calls[0]?.[0]).not.toHaveProperty("guest_access");
});

it("makes missing selection recoverable from the machine settings link", () => {
  render(<MachineSettings node={node} canManage />);
  expect(
    screen.getByRole("link", { name: "Review Coder separation" }),
  ).toBeEnabled();
  expect(
    screen.getByText(
      /New assignments need a recorded choice and confirmed support/,
    ),
  ).toBeInTheDocument();
  expect(
    screen.getByText(
      /may read node tokens, signing secrets and stored credentials/,
    ),
  ).toBeInTheDocument();
});

it.each([true, false, null])("shows actual macOS permissions, including unknown: %s", (permission) => {
  render(<MachineSettings node={{ ...node, machine: { ...node.machine!, computer_permissions: { screen_recording: permission, accessibility: permission } } }} canManage />);
  const text = permission === null ? "Not reported" : permission ? "Allowed" : "Not allowed";
  expect(screen.getByText(`Screen Recording: ${text}`)).toBeVisible();
  expect(screen.getByText(`Accessibility: ${text}`)).toBeVisible();
  expect(screen.getByText(/In System Settings, open Privacy/)).toBeVisible();
});
