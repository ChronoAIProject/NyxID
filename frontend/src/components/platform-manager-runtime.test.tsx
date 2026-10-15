import { beforeEach, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ManagerRuntimeSettings } from "./platform-manager-runtime";
import type { ManagerRuntime } from "@/schemas/platform-runtime";
import type { PlatformProfile } from "@/schemas/platform-onboarding";
const sponsor = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const { query, mutateAsync } = vi.hoisted(() => ({
  query: {
    data: {} as ManagerRuntime,
    isPending: false,
    error: null,
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
const unavailable = {
  status: "unavailable" as const,
  reason: "public_not_ready" as const,
  checked_at: null,
  expires_at: null,
  evidence_digest: null,
};
const ready = () => ({
  status: "ready",
  reason: null,
  checked_at: new Date().toISOString(),
  expires_at: new Date(Date.now() + 60000).toISOString(),
  evidence_digest: "server-proof",
});
const profile = {
  id: "profile",
  generation: 7,
  funding_owner_user_id: sponsor,
  guide: null,
  deleted_at: null,
} as PlatformProfile;
beforeEach(() => {
  query.data = {
    contract_version: 1,
    supported: true,
    runtime: null,
    public_readiness: { ...unavailable, reason: "runtime_uninitialized" },
  };
  mutateAsync.mockReset().mockImplementation(async ({ action }) =>
    action === "profiles"
      ? {
          profiles: [{ id: "nyxagent/chat", label: "NyxAgent chat" }],
          default_profile: "nyxagent/chat",
          readiness: unavailable,
        }
      : action === "readiness"
        ? { public_readiness: ready() }
        : query.data,
  );
});

it("does no automatic paid discovery and initializes both lanes with the exact creation CAS", async () => {
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(mutateAsync).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Total public budget per day")).toHaveValue("1");
  expect(screen.getByLabelText("Free quota per person per day")).toHaveValue(
    "0.1",
  );
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(3));
  expect(mutateAsync.mock.calls[2]![0]).toEqual({
    action: "update",
    body: {
      initialize: true,
      expected_profile_generation: 7,
      expected_manager_authority_epoch: 0,
      expected_personal_control_revision: 0,
      expected_public_policy_revision: 0,
      personal_enabled: true,
      public_enabled: true,
      public_policy: {
        source: "nyxagent",
        profile: "nyxagent/chat",
        total_daily_credits: "1",
        sender_daily_credits: "0.1",
      },
    },
  });
});

it("does not enable either lane on unavailable public proof, with explicit personal-only initialization available", async () => {
  mutateAsync.mockImplementation(async ({ action }) =>
    action === "profiles"
      ? {
          profiles: [{ id: "nyxagent/chat", label: "Chat" }],
          readiness: unavailable,
        }
      : { public_readiness: unavailable },
  );
  render(<ManagerRuntimeSettings profile={profile} />);
  fireEvent.click(screen.getByRole("button", { name: "Enable bot" }));
  await screen.findByText(
    /Public access is unavailable. Settings were not enabled/,
  );
  expect(mutateAsync.mock.calls.map(([x]) => x.action)).toEqual([
    "profiles",
    "readiness",
  ]);
  fireEvent.click(screen.getByText("Advanced", { selector: "summary" }));
  fireEvent.click(
    screen.getByRole("button", { name: "Enable personal access only" }),
  );
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(3));
  expect(mutateAsync.mock.calls[2]![0]).toEqual({
    action: "update",
    body: {
      initialize: true,
      expected_profile_generation: 7,
      expected_manager_authority_epoch: 0,
      expected_personal_control_revision: 0,
      expected_public_policy_revision: 0,
      personal_enabled: true,
      public_enabled: false,
    },
  });
});

it("retains exact existing money and keeps personal control out of public save and pause", async () => {
  query.data.runtime = {
    manager_authority_epoch: 1,
    personal_control_revision: 2,
    public_policy_revision: 3,
    personal_enabled: false,
    public_enabled: true,
    emergency_stopped: false,
    public_policy: {
      source: "nyxagent",
      profile: "nyxagent/chat",
      sponsor_owner_user_id: sponsor,
      total_daily_credits: "2.000000000001",
      sender_daily_credits: "0.123456789012",
    },
  };
  render(<ManagerRuntimeSettings profile={profile} />);
  expect(screen.getByLabelText("Free quota per person per day")).toHaveValue(
    "0.123456789012",
  );
  fireEvent.change(screen.getByLabelText("Total public budget per day"), {
    target: { value: "3.000000000001" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save public settings" }));
  await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(3));
  expect(mutateAsync.mock.calls[2]![0].body).toEqual({
    expected_manager_authority_epoch: 1,
    expected_personal_control_revision: 2,
    expected_public_policy_revision: 3,
    public_enabled: true,
    public_policy: {
      source: "nyxagent",
      profile: "nyxagent/chat",
      total_daily_credits: "3.000000000001",
      sender_daily_credits: "0.123456789012",
    },
  });
  fireEvent.click(screen.getByRole("button", { name: "Pause public guide" }));
  expect(mutateAsync.mock.calls[3]![0].body).toEqual({
    expected_manager_authority_epoch: 1,
    expected_personal_control_revision: 2,
    expected_public_policy_revision: 3,
    public_enabled: false,
  });
});

it("preserves legacy saved monetary limits and distinguishes unsupported routes", () => {
  const saved = {
    ...profile,
    guide: {
      limits: { daily_credits: "9.123456789012" },
      anonymous_daily_credits: "0.2",
    },
  } as PlatformProfile;
  const view = render(<ManagerRuntimeSettings profile={saved} />);
  expect(screen.getByLabelText("Total public budget per day")).toHaveValue(
    "9.123456789012",
  );
  query.data = { ...query.data, supported: false };
  view.rerender(<ManagerRuntimeSettings profile={saved} />);
  expect(screen.getByText(/not supported by this server/)).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Enable bot" }),
  ).not.toBeInTheDocument();
});
