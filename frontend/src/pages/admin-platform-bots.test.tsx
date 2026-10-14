import { beforeEach, expect, it, vi } from "vitest";
import {
  act,
  render,
  screen,
  fireEvent,
  waitFor,
} from "@testing-library/react";
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
  useAdminPlatformProfiles: () => ({
    data: { profiles: [{ ...profile }], next_before_id: null },
  }),
  useAdminPlatformAction: () => ({ mutate }),
  useAdminGuideServices: () => ({
    data: { services: catalog, next_after_id: null },
  }),
  useAdminGuideMutation: () => ({ mutate: guideMutate, isPending: false }),
}));
beforeEach(() => {
  cleanup.splice(0);
  profile.generation = 1;
  profile.guide = null;
  mutate.mockClear();
  guideMutate.mockClear();
});
it("requires rereview if retirement generation changes during confirmation", () => {
  const view = render(<AdminPlatformBotsPage />);
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

it("bootstraps an empty guide through named service discovery, paused save and explicit validation", async () => {
  const page = render(<AdminPlatformBotsPage />);
  const service = catalog[0]!;
  fireEvent.change(screen.getByLabelText("Platform inference service"), {
    target: { value: service.id },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Discover sponsor-funded models" }),
  );
  expect(guideMutate.mock.calls[0]![0]).toMatchObject({
    action: "discover",
    model_service_id: service.id,
  });
  const callback = guideMutate.mock.calls[0]![1] as {
    onSuccess: (value: unknown) => void;
  };
  act(() =>
    callback.onSuccess({
      generation: 1,
      model_service_id: service.id,
      models: ["live-model"],
    }),
  );
  fireEvent.change(screen.getByLabelText("Exact advertised model"), {
    target: { value: "live-model" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Save paused configuration" }),
  );
  await waitFor(() =>
    expect(guideMutate.mock.calls[1]![0]).toMatchObject({
      action: "configure",
      enabled: false,
      guide: { model_service_id: service.id, model_id: "live-model" },
    }),
  );
  profile.guide = guideMutate.mock.calls[1]![0].guide;
  profile.generation = 2;
  page.rerender(<AdminPlatformBotsPage />);
  fireEvent.click(
    screen.getByRole("button", { name: "Validate model and enable guide" }),
  );
  expect(guideMutate.mock.calls[2]![0]).toMatchObject({
    action: "configure",
    enabled: true,
    profile: { generation: 2 },
    guide: { model_id: "live-model" },
  });
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
