import { beforeEach, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ManagerRuntimeSettings } from "./platform-manager-runtime";
import {
  managerRuntimeSchema,
  type ManagerRuntime,
} from "@/schemas/platform-runtime";
import type { PlatformProfile } from "@/schemas/platform-onboarding";

const { query, mutateAsync } = vi.hoisted(() => ({
  query: {
    data: {} as ManagerRuntime,
    isPending: false,
    error: null as Error | null,
    refetch: vi.fn(),
  },
  mutateAsync: vi.fn(),
}));
vi.mock("@/hooks/use-platform-runtime", () => ({
  useManagerRuntime: () => query,
  useManagerRuntimeAction: () => ({ mutateAsync, isPending: false }),
}));
vi.mock("@/hooks/use-platform-onboarding", () => ({
  useAdminPlatformTargets: () => ({ data: { orgs: [] } }),
}));
const profile = {
  id: "11111111-1111-4111-8111-111111111111",
  generation: 3,
  funding_owner_user_id: "22222222-2222-4222-8222-222222222222",
  channel_bot_id: "33333333-3333-4333-8333-333333333333",
  manager_bot_id: "native-discord",
  execution_profile: "native_entry",
  platform: "discord",
  transport: "direct",
  enabled: false,
  guide: null,
  deleted_at: null,
} as PlatformProfile;
beforeEach(() => {
  query.isPending = false;
  query.error = null;
  query.data = managerRuntimeSchema.parse({
    contract_version: 1,
    supported: true,
    state: "setup_incomplete",
    runtime: {
      manager_authority_epoch: 1,
      personal_control_revision: 2,
      public_policy_revision: 3,
      personal_enabled: false,
      public_enabled: false,
      emergency_stopped: false,
      public_policy: null,
    },
    public_readiness: {
      status: "unavailable",
      reason: "public_disabled",
      checked_at: null,
      expires_at: null,
      evidence_digest: null,
    },
  });
  mutateAsync.mockReset();
});

it("shows healthy personal-only operation without a public AI blocker", () => {
  query.data.state = "active";
  query.data.runtime!.personal_enabled = true;
  query.data.public_readiness.reason = "public_disabled";
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByText("Bot active")).toBeVisible();
  expect(screen.getByText("Personal enabled")).toBeVisible();
  expect(screen.getByRole("status")).toHaveTextContent(
    "Personal NyxBot is active. Public answers are off.",
  );
  expect(
    screen.queryByText(/restricted public runtime|Public access is paused/),
  ).not.toBeInTheDocument();
  expect(mutateAsync).not.toHaveBeenCalled();
});

it("gives initialized both-off controls an initial Enable action", async () => {
  const { fireEvent, waitFor } = await import("@testing-library/react");
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByText("Setup required")).toBeVisible();
  expect(screen.getByRole("status")).toHaveTextContent(/choose Enable bot/);
  expect(
    screen.queryByText(/restricted public runtime/),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Enable bot" })).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox", { name: /Personal NyxBot/ }));
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledOnce());
  expect(mutateAsync.mock.calls[0]![0].body).toMatchObject({
    personal_enabled: true,
    public_enabled: false,
  });
});

it("shows enabled built-in FAQ independently of public AI evidence", () => {
  query.data.state = "active";
  query.data.runtime!.public_enabled = true;
  query.data.public_readiness = {
    ...query.data.public_readiness,
    status: "faq_ready",
    reason: null,
  };
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByText("Public enabled")).toBeVisible();
  expect(screen.getByRole("status")).toHaveTextContent(
    "Public answers are enabled. Personal NyxBot is off.",
  );
  expect(screen.getByText(/Built-in FAQ is ready/)).toBeVisible();
  expect(
    screen.queryByText(/restricted public runtime/),
  ).not.toBeInTheDocument();
});

it("native loading does not claim runtime is paused", () => {
  query.isPending = true;
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByRole("status")).toHaveTextContent(
    "Loading bot access settings",
  );
  expect(screen.queryByText("Public off")).not.toBeInTheDocument();
  expect(mutateAsync).not.toHaveBeenCalled();
});

it("native unavailable query is reported without claiming a runtime state", () => {
  query.error = new Error("fixture unavailable");
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(
    screen.getByText(/Public and personal bot access is unavailable/),
  ).toBeInTheDocument();
  expect(screen.queryByText("Public off")).not.toBeInTheDocument();
  expect(mutateAsync).not.toHaveBeenCalled();
});

it("native transport failure does not blame Telegram", () => {
  query.data.state = "connection_repair_required";
  query.data.runtime!.public_enabled = true;
  query.data.runtime!.personal_enabled = true;
  query.data.public_readiness.reason = "transport_unavailable";
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByText("Repair required")).toBeVisible();
  expect(screen.getByText("Public configured")).toBeVisible();
  expect(screen.getByText("Personal configured")).toBeVisible();
  expect(screen.queryByText("Public enabled")).not.toBeInTheDocument();
  expect(screen.queryByText("Personal enabled")).not.toBeInTheDocument();
  expect(screen.getByRole("status")).not.toHaveTextContent("Telegram");
  expect(screen.getByRole("status")).toHaveTextContent(
    /Connection repair required/i,
  );
  expect(mutateAsync).not.toHaveBeenCalled();
});

it("resumes remembered FAQ-only modes atomically without public AI setup", async () => {
  const { fireEvent, waitFor } = await import("@testing-library/react");
  query.data.state = "paused";
  query.data.runtime!.emergency_stopped = true;
  query.data.runtime!.remembered_public_enabled = true;
  query.data.runtime!.remembered_personal_enabled = false;
  query.data.public_readiness.reason = "emergency_stopped";
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByRole("checkbox", { name: /Public FAQ/ })).toBeChecked();
  expect(
    screen.getByRole("checkbox", { name: /Personal NyxBot/ }),
  ).not.toBeChecked();
  expect(
    screen.queryByRole("combobox", { name: "Public answers" }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent(/choose Resume bot/);
  fireEvent.click(screen.getByRole("button", { name: "Resume bot" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledOnce());
  expect(mutateAsync).toHaveBeenCalledWith({
    action: "update",
    body: {
      expected_manager_authority_epoch: 1,
      expected_personal_control_revision: 2,
      expected_public_policy_revision: 3,
      personal_enabled: false,
      public_enabled: true,
      emergency_stopped: false,
    },
  });
});

it("explains both-off Resume and permits an explicit FAQ selection", async () => {
  const { fireEvent, waitFor } = await import("@testing-library/react");
  query.data.state = "paused";
  query.data.runtime!.emergency_stopped = true;
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(
    screen.getByText("Select at least one mode to enable or resume this bot."),
  ).toBeVisible();
  expect(screen.getByRole("button", { name: "Resume bot" })).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox", { name: /Public FAQ/ }));
  fireEvent.click(screen.getByRole("button", { name: "Resume bot" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledOnce());
  expect(mutateAsync.mock.calls[0]![0].body).toMatchObject({
    public_enabled: true,
    personal_enabled: false,
    emergency_stopped: false,
  });
});

it("keeps a single access flow when optional AI is expanded and requires an org sponsor", async () => {
  const { fireEvent, waitFor } = await import("@testing-library/react");
  render(<ManagerRuntimeSettings profile={profile} />);
  fireEvent.click(
    screen.getByText("Advanced public AI deployment", { selector: "summary" }),
  );
  await waitFor(() =>
    expect(screen.getByLabelText("Sponsor organization")).toBeVisible(),
  );
  expect(screen.getByLabelText("Sponsor organization")).toHaveValue("");
  expect(screen.getAllByRole("button", { name: "Pause bot" })).toHaveLength(1);
  expect(
    screen.queryByRole("button", {
      name: /Clear manager emergency stop|Pause personal access|Pause public guide|Enable personal access only/,
    }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText("Advanced", { selector: "summary" }),
  ).not.toBeInTheDocument();
  expect(mutateAsync).not.toHaveBeenCalled();
});

it("keeps saved public AI readiness visible through unsaved source and mode edits", async () => {
  const { fireEvent } = await import("@testing-library/react");
  query.data.state = "active";
  query.data.runtime!.public_enabled = true;
  query.data.runtime!.public_policy = {
    source: "nyxagent",
    profile: "saved-ai",
    sponsor_owner_user_id: profile.funding_owner_user_id,
    total_daily_credits: "1",
    sender_daily_credits: "0.1",
  };
  query.data.public_readiness.reason = "public_not_ready";
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByRole("status")).toHaveTextContent(
    "Public answers are enabled.",
  );
  expect(screen.getByRole("status")).not.toHaveTextContent(
    "Public answers are active",
  );
  expect(
    screen.getByText("The restricted public runtime is not ready."),
  ).toBeVisible();
  fireEvent.change(screen.getByRole("combobox", { name: "Public answers" }), {
    target: { value: "faq" },
  });
  fireEvent.click(screen.getByRole("checkbox", { name: /Public FAQ/ }));
  expect(
    screen.getByText("The restricted public runtime is not ready."),
  ).toBeVisible();
  expect(screen.queryByText(/Built-in FAQ is ready/)).not.toBeInTheDocument();
  expect(screen.getByText("Public enabled")).toBeVisible();
  expect(mutateAsync).not.toHaveBeenCalled();
});

it("sends only changed lanes on a normal save with saved public AI", async () => {
  const { fireEvent, waitFor } = await import("@testing-library/react");
  query.data.state = "active";
  query.data.runtime!.personal_enabled = true;
  query.data.runtime!.public_enabled = true;
  query.data.runtime!.public_policy = {
    source: "nyxagent",
    profile: "saved-ai",
    sponsor_owner_user_id: profile.funding_owner_user_id,
    total_daily_credits: "1",
    sender_daily_credits: "0.1",
  };
  render(<ManagerRuntimeSettings profile={profile} />);
  fireEvent.click(screen.getByRole("checkbox", { name: /Personal NyxBot/ }));
  fireEvent.click(screen.getByRole("button", { name: "Save modes" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledOnce());
  const body = mutateAsync.mock.calls[0]![0].body;
  expect(body.personal_enabled).toBe(false);
  expect(body).not.toHaveProperty("public_enabled");
  expect(body).not.toHaveProperty("public_policy");
  expect(body).not.toHaveProperty("emergency_stopped");
});

it.each([
  ["Public deployment is not ready", true],
  ["Runtime controls changed; refresh before deciding", false],
  ["Connection transport changed", false],
])(
  "preserves the save error %s and adds only relevant AI recovery guidance",
  async (message, hint) => {
    const { fireEvent } = await import("@testing-library/react");
    query.data.state = "paused";
    query.data.runtime!.emergency_stopped = true;
    query.data.runtime!.remembered_personal_enabled = true;
    query.data.runtime!.remembered_public_enabled = true;
    query.data.runtime!.public_policy = {
      source: "nyxagent",
      profile: "saved-ai",
      sponsor_owner_user_id: profile.funding_owner_user_id,
      total_daily_credits: "1",
      sender_daily_credits: "0.1",
    };
    mutateAsync.mockRejectedValue(new Error(message));
    render(<ManagerRuntimeSettings profile={profile} />);
    fireEvent.click(screen.getByRole("button", { name: "Resume bot" }));
    expect(await screen.findByText(message)).toBeVisible();
    const guidance = screen.queryByText(/To continue without public AI/);
    if (hint) expect(guidance).toBeVisible();
    else expect(guidance).not.toBeInTheDocument();
  },
);
