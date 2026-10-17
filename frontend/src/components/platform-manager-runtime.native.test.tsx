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
    /transport.*unavailable/i,
  );
  expect(mutateAsync).not.toHaveBeenCalled();
});
