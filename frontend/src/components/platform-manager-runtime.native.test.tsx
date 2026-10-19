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

it.each([false, true])(
  "native access badges follow runtime flags %s despite legacy enabled=false",
  (enabled) => {
    query.data.runtime!.public_enabled = enabled;
    query.data.runtime!.personal_enabled = enabled;
    render(<ManagerRuntimeSettings profile={profile} />);
    expect(
      screen.getByText(`Public ${enabled ? "enabled" : "paused"}`),
    ).toBeInTheDocument();
    expect(
      screen.getByText(`Personal ${enabled ? "enabled" : "paused"}`),
    ).toBeInTheDocument();
    expect(mutateAsync).not.toHaveBeenCalled();
  },
);

it("native loading does not claim runtime is paused", () => {
  query.isPending = true;
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByRole("status")).toHaveTextContent(
    "Loading bot access settings",
  );
  expect(screen.queryByText("Public paused")).not.toBeInTheDocument();
  expect(mutateAsync).not.toHaveBeenCalled();
});

it("native unavailable query is reported without claiming a runtime state", () => {
  query.error = new Error("fixture unavailable");
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(
    screen.getByText(/Public and personal bot access is unavailable/),
  ).toBeInTheDocument();
  expect(screen.queryByText("Public paused")).not.toBeInTheDocument();
  expect(mutateAsync).not.toHaveBeenCalled();
});

it("native transport failure does not blame Telegram", () => {
  query.data.public_readiness.reason = "transport_unavailable";
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByRole("status")).not.toHaveTextContent("Telegram");
  expect(screen.getByRole("status")).toHaveTextContent(
    /Connection repair required/i,
  );
  expect(mutateAsync).not.toHaveBeenCalled();
});

it("resumes remembered FAQ-only modes atomically without public AI setup", async () => {
  const { fireEvent, waitFor } = await import("@testing-library/react");
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
      public_policy: null,
    },
  });
});

it("explains both-off Resume and permits an explicit FAQ selection", async () => {
  const { fireEvent, waitFor } = await import("@testing-library/react");
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
