vi.mock("@/components/native-platform-entry-setup", () => ({
  NativePlatformEntrySetup: () => <div>Native shared entry setup</div>,
}));
import { beforeEach, expect, it, vi } from "vitest";
import { act, render, screen, fireEvent } from "@testing-library/react";
import {
  AdminPlatformBotsPage,
  GatewayCleanupControls,
} from "./admin-platform-bots";
import type { PlatformProfile } from "@/schemas/platform-onboarding";
const { profile, mutate, guideMutate, catalog, cleanup } = vi.hoisted(() => ({
  profile: {
    id: "profile",
    channel_bot_id: "manager-channel",
    execution_profile: "platform_onboarding" as const,
    manager_bot_id: "100",
    generation: 1,
    enabled: false,
    deleted_at: null,
    funding_owner_user_id: "org",
    guide: null as PlatformProfile["guide"],
  },
  mutate: vi.fn(),
  guideMutate: vi.fn(),
  cleanup: [] as { id: string; captured_version: number }[],
  catalog: [
    {
      id: "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
      name: "Public platform model service",
    },
  ],
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (s: unknown) => unknown) =>
    select({ user: { id: "admin", is_admin: true, role: "admin" } }),
}));
vi.mock("@/hooks/use-platform-onboarding", () => ({
  useGatewayCleanup: () => ({ data: cleanup }),
  useGatewayCleanupAction: () => ({ mutate, isPending: false }),
  useAdminPlatformTargets: () => ({ data: { bots: [], orgs: [] } }),
  useAdminPlatformProfile: () => ({ data: undefined }),
  useAdminPlatformProfiles: () => ({
    data: { profiles: [{ ...profile }], next_before_id: null },
  }),
  useAdminPlatformAction: () => ({ mutate }),
  useAdminGuideServices: () => ({
    data: { services: catalog, next_after_id: null },
  }),
  useAdminGuideMutation: () => ({ mutate: guideMutate, isPending: false }),
}));
vi.mock("@/components/platform-manager-runtime", () => ({
  ManagerRuntimeSettings: () => <div>Bot access controls</div>,
}));
beforeEach(() => {
  cleanup.splice(0);
  Object.assign(profile, {
    execution_profile: "platform_onboarding",
    enabled: false,
    deleted_at: null,
  });
  profile.generation = 1;
  profile.guide = null;
  mutate.mockClear();
  guideMutate.mockClear();
});
it("requires rereview if retirement generation changes during confirmation", () => {
  const view = render(<AdminPlatformBotsPage />);
  fireEvent.click(screen.getByText("Identifiers, cleanup and retirement"));
  fireEvent.click(screen.getByRole("button", { name: "Review retirement" }));
  profile.generation = 2;
  view.rerender(<AdminPlatformBotsPage />);
  expect(
    screen.getByRole("button", { name: "Retire platform manager" }),
  ).toBeDisabled();
  expect(mutate).not.toHaveBeenCalled();
  fireEvent.click(
    screen.getByRole("button", { name: "Review current profile" }),
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Retire platform manager" }),
  );
  expect(mutate.mock.calls[0]![0].profile.generation).toBe(2);
});

it("requires exact gateway review and resets confirmation on profile drift", () => {
  cleanup.push({ id: "cleanup", captured_version: 7 });
  const page = render(<GatewayCleanupControls profile={profile} />);
  fireEvent.click(screen.getByRole("button", { name: "Inspect connection" }));
  const review = {
    task_id: "cleanup",
    profile_generation: 1,
    captured_version: 7,
    observed_version: 8,
    remote_exists: true,
    remote_channel_id: "remote",
    completed: false,
  };
  act(() => mutate.mock.calls[0]![1].onSuccess(review));
  const resolve = screen.getByRole("button", {
    name: "Keep changed remote binding",
  });
  expect(resolve).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(resolve);
  expect(mutate.mock.calls[1]![0]).toEqual({
    task: "cleanup",
    action: "preserve_changed",
    review,
  });
  profile.generation = 2;
  page.rerender(<GatewayCleanupControls profile={profile} />);
  expect(
    screen.queryByRole("button", { name: "Keep changed remote binding" }),
  ).not.toBeInTheDocument();
});
it("native entry does not label the legacy guide flag as its paused runtime", () => {
  Object.assign(profile, {
    execution_profile: "native_entry",
    platform: "discord",
    transport: "direct",
  });
  render(<AdminPlatformBotsPage />);
  expect(screen.getByText("Bot access controls")).toBeInTheDocument();
  expect(screen.queryByText(/^Paused$/)).not.toBeInTheDocument();
});

it("native entry avoids Telegram child and native-creation repair guidance", () => {
  Object.assign(profile, {
    execution_profile: "native_entry",
    platform: "discord",
    transport: "direct",
  });
  render(<AdminPlatformBotsPage />);
  expect(
    screen.queryByText(
      /Changing this profile pauses replies from existing client bots/,
    ),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText(/Repair retains the shared native creation webhook/),
  ).not.toBeInTheDocument();
});

it("retired native entry shows retirement and no active controls or Telegram repair", () => {
  Object.assign(profile, {
    execution_profile: "native_entry",
    platform: "discord",
    deleted_at: "2026-10-17T00:00:00Z",
  });
  render(<AdminPlatformBotsPage />);
  expect(screen.getByText("Retired")).toBeInTheDocument();
  expect(screen.queryByText("Bot access controls")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Repair webhook" }),
  ).not.toBeInTheDocument();
});
