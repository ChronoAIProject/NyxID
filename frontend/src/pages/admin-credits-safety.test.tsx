import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { AdminCreditsPage } from "./admin-credits";
const mock = vi.hoisted(() => ({
  allowanceActive: true,
  extraDisabled: false,
  replaceAllowance: vi.fn(),
  targetKind: "all_users",
  scheduleActive: true,
  updateAllowance: vi.fn(),
  updateSchedule: vi.fn(),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (s: unknown) => unknown) =>
    select({ user: { is_admin: true } }),
}));
vi.mock("@/hooks/use-services", () => ({
  useServices: () => ({
    data: [
      {
        id: "service-a",
        slug: "test",
        name: "Test service",
        allowance_metrics: ["requests", "images"],
        effective_platform_metric: "requests",
      },
    ],
  }),
}));
vi.mock("@/hooks/use-billing-credits", () => ({
  useAdminCreditGrants: () => ({ data: { grants: [] } }),
  useAdminAllowances: () => ({
    data: {
      allowances: [
        {
          id: "allowance-a",
          bundle_id: mock.extraDisabled ? "bundle" : null,
          service_id: "service-a",
          service_slug: "test",
          quantity: 10,
          metric: "requests",
          recurrence: "monthly",
          target_kind: mock.targetKind,
          target_org_ids:
            mock.targetKind === "org_members" ? ["a", "b", "c"] : [],
          target_group_ids: mock.targetKind === "groups" ? ["a", "b"] : [],
          target_user_ids: [],
          is_active: mock.allowanceActive,
        },
        ...(mock.extraDisabled
          ? [
              {
                id: "allowance-b",
                bundle_id: "bundle",
                service_id: "service-a",
                service_slug: "test",
                quantity: 20,
                metric: "images",
                recurrence: "daily",
                target_kind: "all_users",
                target_user_ids: [],
                target_org_ids: [],
                target_group_ids: [],
                is_active: false,
              },
            ]
          : []),
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
          target_kind: mock.targetKind,
          target_org_ids:
            mock.targetKind === "org_members" ? ["a", "b", "c"] : [],
          target_group_ids: mock.targetKind === "groups" ? ["a", "b"] : [],
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
  useCreateAllowanceBundle: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useReplaceAllowanceBundle: () => ({
    isPending: false,
    mutateAsync: mock.replaceAllowance,
  }),
  useCreateCreditSchedule: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useSetAllowanceBundleActive: () => ({
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
  mock.extraDisabled = false;
  mock.targetKind = "all_users";
  mock.scheduleActive = true;
});
it("reviews allowance disabling and sends only status after confirmation", async () => {
  const user = userEvent.setup();
  render(<AdminCreditsPage />);
  await user.click(screen.getByRole("tab", { name: "Free allowances" }));
  await user.click(screen.getAllByRole("button", { name: "Disable" })[0]!);
  expect(mock.updateAllowance).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(mock.updateAllowance).not.toHaveBeenCalled();
  await user.click(screen.getAllByRole("button", { name: "Disable" })[0]!);
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

it.each([
  ["org_members", "3 organizations · members"],
  ["groups", "2 groups · members"],
])("shows %s allowance recipients", async (kind, label) => {
  mock.targetKind = kind;
  render(<AdminCreditsPage />);
  await userEvent.click(screen.getByRole("tab", { name: "Free allowances" }));
  expect(screen.getAllByText(label)).toHaveLength(2);
});

it.each([false, true])(
  "editing preserves removed units unless explicitly re-added (%s)",
  async (reAdd) => {
    mock.extraDisabled = true;
    const user = userEvent.setup();
    render(<AdminCreditsPage />);
    await user.click(screen.getByRole("tab", { name: "Free allowances" }));
    await user.click(
      screen.getAllByRole("button", { name: "Edit test allowance" })[0]!,
    );
    expect(screen.getAllByRole("combobox", { name: "Unit" })).toHaveLength(1);
    expect(
      screen.getByText(/Disabled units stay disabled/),
    ).toBeInTheDocument();
    fireEvent.change(
      screen.getByRole("spinbutton", { name: "Free quantity" }),
      { target: { value: "15" } },
    );
    if (reAdd)
      await user.click(screen.getByRole("button", { name: "Add unit" }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    // Only an explicitly re-added unit appears as a disabled -> active change.
    expect(
      screen.queryByText("images", { selector: "p.font-medium" }) !== null,
    ).toBe(reAdd);
    await user.click(screen.getByRole("button", { name: "Confirm changes" }));
    await waitFor(() => expect(mock.replaceAllowance).toHaveBeenCalledOnce());
    expect(mock.replaceAllowance.mock.calls[0]![0].body.units).toEqual([
      { metric: "requests", quantity: 15, recurrence: "monthly" },
      ...(reAdd
        ? [{ metric: "images", quantity: 1000, recurrence: "monthly" }]
        : []),
    ]);
  },
);

it("editing a fully disabled bundle loads all units and reviews their re-enablement", async () => {
  mock.allowanceActive = false;
  mock.extraDisabled = true;
  const user = userEvent.setup();
  render(<AdminCreditsPage />);
  await user.click(screen.getByRole("tab", { name: "Free allowances" }));
  await user.click(
    screen.getAllByRole("button", { name: "Edit test allowance" })[0]!,
  );
  expect(screen.getAllByRole("combobox", { name: "Unit" })).toHaveLength(2);
  expect(
    screen.getByText(/Saving re-enables the listed units/),
  ).toBeInTheDocument();
  fireEvent.change(
    screen.getAllByRole("spinbutton", { name: "Free quantity" })[0]!,
    {
      target: { value: "15" },
    },
  );
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  expect(
    screen.getByText("requests", { selector: "p.font-medium" }),
  ).toBeInTheDocument();
  expect(
    screen.getByText("images", { selector: "p.font-medium" }),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Confirm changes" }));
  await waitFor(() => expect(mock.replaceAllowance).toHaveBeenCalledOnce());
  expect(mock.replaceAllowance.mock.calls[0]![0].body.units).toEqual([
    { metric: "requests", quantity: 15, recurrence: "monthly" },
    { metric: "images", quantity: 20, recurrence: "daily" },
  ]);
});
