import { beforeEach, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ManagerRuntimeSettings } from "./platform-manager-runtime";
import type { ManagerRuntime } from "@/schemas/platform-runtime";
import type { PlatformProfile } from "@/schemas/platform-onboarding";
const { query, mutate, recovery, pending } = vi.hoisted(() => ({
  query: {
    data: {} as ManagerRuntime,
    isPending: false,
    error: null,
    refetch: vi.fn(),
  },
  mutate: vi.fn(),
  recovery: vi.fn(),
  pending: { modes: false, connection: false },
}));
vi.mock("@/hooks/use-platform-runtime", () => ({
  useManagerRuntime: () => query,
  useManagerRuntimeAction: () => ({ mutate, isPending: pending.modes }),
}));
vi.mock("@/hooks/use-platform-onboarding", () => ({
  useAdminPlatformAction: () => ({
    mutate: recovery,
    reset: vi.fn(),
    isPending: pending.connection,
  }),
}));
const profile = {
  id: "profile",
  generation: 7,
  deleted_at: null,
} as PlatformProfile;
beforeEach(() => {
  vi.clearAllMocks();
  pending.modes = false;
  pending.connection = false;
  query.data = {
    contract_version: 1,
    supported: true,
    profile_generation: 7,
    runtime: null,
    public_readiness: {
      status: "unavailable",
      reason: "runtime_uninitialized",
      checked_at: null,
      expires_at: null,
      evidence_digest: null,
    },
    state: "setup_incomplete",
    reason: null,
    next_action: "enable",
    platform: "slack",
    display_name: "Support bot",
    connection_url: "/admin/platform-bots#profile",
    faq_preview: "Approved public help",
    connection: {
      status: "active",
      webhook_url: "https://local.test/callback",
      credential_source: "user",
      fields: [{ name: "bot_token", label: "Bot token", secret: true }],
      instructions: [],
    },
  };
});
it("enables FAQ and personal linking with one CAS and no model, sponsor or paid discovery", async () => {
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(
    screen.queryByLabelText(/budget|sponsor|model/i),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  await waitFor(() => expect(mutate).toHaveBeenCalledTimes(1));
  expect(mutate.mock.calls[0]![0]).toEqual({
    action: "update",
    body: {
      initialize: true,
      expected_profile_generation: 7,
      expected_manager_authority_epoch: 0,
      expected_personal_control_revision: 0,
      expected_public_policy_revision: 0,
      public_enabled: true,
      personal_enabled: true,
      emergency_stopped: false,
      public_policy: null,
    },
  });
  expect(screen.getByText(/exact plain text help/)).toBeInTheDocument();
});
it("remembers personal-only modes across paused reload and atomically resumes them", async () => {
  query.data.state = "paused";
  query.data.runtime = {
    manager_authority_epoch: 4,
    personal_control_revision: 2,
    public_policy_revision: 3,
    personal_enabled: false,
    public_enabled: false,
    emergency_stopped: true,
    public_policy: null,
    resume_personal_enabled: true,
    resume_public_enabled: false,
  };
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(
    screen.getByRole("checkbox", { name: "Public FAQ" }),
  ).not.toBeChecked();
  expect(
    screen.getByRole("checkbox", { name: "Personal NyxBot linking" }),
  ).toBeChecked();
  fireEvent.click(screen.getByRole("button", { name: "Resume bot" }));
  await waitFor(() => expect(mutate).toHaveBeenCalledOnce());
  expect(mutate.mock.calls[0]![0].body).toMatchObject({
    personal_enabled: true,
    public_enabled: false,
    emergency_stopped: false,
    expected_manager_authority_epoch: 4,
  });
});
it("Enter in replacement credentials repairs the connection and never submits modes", async () => {
  const user = userEvent.setup();
  render(<ManagerRuntimeSettings profile={profile} />);
  fireEvent.click(screen.getByText("Connection setup and recovery"));
  await user.type(
    screen.getByLabelText("Bot token replacement (optional)"),
    "replacement{Enter}",
  );
  await waitFor(() => expect(recovery).toHaveBeenCalledOnce());
  expect(recovery.mock.calls[0]![0]).toMatchObject({
    action: "repair",
    fields: { bot_token: "replacement" },
    profile: { generation: 7 },
  });
  expect(mutate).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Enable bot" })).toBeDisabled();
  expect(
    screen.getByLabelText("Bot token replacement (optional)"),
  ).toHaveAttribute("type", "password");
});
it("explains both-off Resume and requires an explicit mode choice", async () => {
  query.data.state = "paused";
  query.data.runtime = {
    manager_authority_epoch: 4,
    personal_control_revision: 2,
    public_policy_revision: 3,
    personal_enabled: false,
    public_enabled: false,
    emergency_stopped: true,
    public_policy: null,
    resume_personal_enabled: false,
    resume_public_enabled: false,
  };
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByRole("button", { name: "Resume bot" })).toBeDisabled();
  expect(
    screen.getByText("Select at least one mode to enable or resume this bot."),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole("checkbox", { name: "Public FAQ" }));
  expect(screen.getByRole("button", { name: "Resume bot" })).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Resume bot" }));
  await waitFor(() => expect(mutate).toHaveBeenCalledOnce());
  expect(mutate.mock.calls[0]![0].body).toMatchObject({
    personal_enabled: false,
    public_enabled: true,
    emergency_stopped: false,
  });
});
it("shows provider recovery and refetches status without reloading the page", () => {
  query.data.state = "connection_repair_required";
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByRole("button", { name: "Enable bot" })).toBeDisabled();
  fireEvent.click(screen.getByText("Connection setup and recovery"));
  fireEvent.click(screen.getByRole("button", { name: "Refresh status" }));
  expect(query.refetch).toHaveBeenCalledOnce();
});
it("disables recovery while modes are pending", () => {
  pending.modes = true;
  query.data.profile_generation = 9;
  render(<ManagerRuntimeSettings profile={profile} />);
  fireEvent.click(screen.getByText("Connection setup and recovery"));
  expect(
    screen.getByRole("button", { name: "Repair connection" }),
  ).toBeDisabled();
});

it.each(["connection_repair_required", "setup_incomplete"] as const)(
  "retains remembered choices across paused %s recovery and refresh",
  async (state) => {
    query.data.state = state;
    query.data.reason =
      state === "setup_incomplete"
        ? "discord_commands_required"
        : "connection_unavailable";
    query.data.runtime = {
      manager_authority_epoch: 4,
      personal_control_revision: 2,
      public_policy_revision: 3,
      personal_enabled: false,
      public_enabled: false,
      emergency_stopped: true,
      public_policy: null,
      resume_personal_enabled: true,
      resume_public_enabled: false,
    };
    const view = render(<ManagerRuntimeSettings profile={profile} />);
    expect(screen.getByRole("button", { name: "Resume bot" })).toBeDisabled();
    query.data = { ...query.data, state: "paused", reason: null };
    view.rerender(<ManagerRuntimeSettings profile={profile} />);
    expect(
      screen.getByRole("checkbox", { name: "Personal NyxBot linking" }),
    ).toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "Public FAQ" }),
    ).not.toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "Resume bot" }));
    await waitFor(() => expect(mutate).toHaveBeenCalledOnce());
    expect(mutate.mock.calls[0]![0].body).toMatchObject({
      personal_enabled: true,
      public_enabled: false,
      emergency_stopped: false,
    });
  },
);
