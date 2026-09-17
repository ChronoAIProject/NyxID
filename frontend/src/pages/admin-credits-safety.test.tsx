import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { AdminCreditsPage } from "./admin-credits";
const mock = vi.hoisted(() => ({
  allowanceActive: true,
  scheduleActive: true,
  updateAllowance: vi.fn(),
  updateSchedule: vi.fn(),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (s: unknown) => unknown) =>
    select({ user: { is_admin: true } }),
}));
vi.mock("@/hooks/use-services", () => ({ useServices: () => ({ data: [] }) }));
vi.mock("@/hooks/use-billing-credits", () => ({
  useAdminCreditGrants: () => ({ data: { grants: [] } }),
  useAdminAllowances: () => ({
    data: {
      allowances: [
        {
          id: "allowance-a",
          service_id: "service-a",
          service_slug: "test",
          quantity: 10,
          metric: "requests",
          recurrence: "monthly",
          target_kind: "all_users",
          target_user_ids: [],
          is_active: mock.allowanceActive,
        },
      ],
    },
  }),
  useAdminCreditSchedules: () => ({
    data: {
      schedules: [
        {
          id: "schedule-a",
          amount_credits: 20,
          expiry: { kind: "end_of_period" },
          recurrence: "monthly",
          target_kind: "all_users",
          target_user_ids: [],
          is_active: mock.scheduleActive,
          skipped_periods: 0,
          scope: { all_services: true, service_ids: [] },
        },
      ],
    },
  }),
  useIssueCreditGrant: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useRevokeCreditGrant: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useCreateAllowance: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useCreateCreditSchedule: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useUpdateAllowance: () => ({
    isPending: false,
    mutateAsync: mock.updateAllowance,
  }),
  useUpdateCreditSchedule: () => ({
    isPending: false,
    mutateAsync: mock.updateSchedule,
  }),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mock.allowanceActive = true;
  mock.scheduleActive = true;
});
it("reviews allowance disabling and sends only status after confirmation", async () => {
  const user = userEvent.setup();
  render(<AdminCreditsPage />);
  await user.click(screen.getByRole("tab", { name: "Free allowances" }));
  await user.click(screen.getByRole("button", { name: "Disable" }));
  expect(mock.updateAllowance).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(mock.updateAllowance).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Disable" }));
  await user.click(screen.getByRole("button", { name: "Confirm changes" }));
  await waitFor(() =>
    expect(mock.updateAllowance).toHaveBeenCalledWith({
      id: "allowance-a",
      body: { is_active: false },
    }),
  );
});
it("blocks a reviewed schedule status change after an observed concurrent change", async () => {
  const user = userEvent.setup();
  const view = render(<AdminCreditsPage />);
  await user.click(screen.getByRole("tab", { name: "Schedules" }));
  await user.click(screen.getByRole("switch", { name: "Pause schedule" }));
  expect(mock.updateSchedule).not.toHaveBeenCalled();
  mock.scheduleActive = false;
  view.rerender(<AdminCreditsPage />);
  expect(
    screen.getByRole("button", { name: "Confirm changes" }),
  ).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Confirm changes" }));
  expect(mock.updateSchedule).not.toHaveBeenCalled();
});
