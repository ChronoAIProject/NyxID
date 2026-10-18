import userEvent from "@testing-library/user-event";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AdminCreditGrant } from "@/schemas/billing-credits";
import { TooltipProvider } from "@/components/ui/tooltip";
import { AdminCreditsPage } from "./admin-credits";
import { ApiError } from "@/lib/api-client";

const state = vi.hoisted(() => ({ canWrite: true, grants: [] as AdminCreditGrant[], reclaim: vi.fn(), summaryError: null as Error | null, summaryLoading: false, staleSummary: false }));
beforeEach(() => { state.canWrite = true; state.grants = []; state.summaryError = null; state.summaryLoading = false; state.staleSummary = false; vi.clearAllMocks(); });
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: object }) => unknown) =>
    selector({ user: {} }),
}));
vi.mock("@/types/api", () => ({ canAdminWrite: () => state.canWrite }));
vi.mock("@/hooks/use-services", () => ({
  useServices: () => ({
    data: [],
    isLoading: false,
    isFetching: false,
    error: null,
    refetch: vi.fn(),
  }),
}));
vi.mock("@/hooks/use-billing-credits", () => ({
  useAdminCreditSummary: () => ({
    data: state.summaryLoading || (state.summaryError && !state.staleSummary) ? undefined : {
      outstanding_micros: "100250001",
      available_micros: "95250001",
      reserved_micros: "5000000",
      pending_activation_micros: "0",
      expiring_within_30_days_micros: "25250001",
      restricted_micros: "25250001",
      expired_awaiting_cleanup_micros: "0",
      active_grant_count: 1,
      pending_activation_count: 0,
      recipient_count: 1,
      generated_at: "2026-10-17T00:00:00Z",
    },
    isLoading: state.summaryLoading,
    isError: Boolean(state.summaryError),
    error: state.summaryError,
    refetch: vi.fn(),
  }),
  useAdminCreditGrants: () => ({
    data: { grants: state.grants, page: 1, per_page: 50, total: state.grants.length },
    isLoading: false,
    isError: false,
    isFetching: false,
    error: null,
    refetch: vi.fn(),
  }),
  useAdminAllowances: () => ({
    data: { allowances: [] },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useAdminCreditSchedules: () => ({
    data: { schedules: [] },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useAddCredits: () => ({ isPending: false, mutateAsync: vi.fn(), reset: vi.fn() }),
  useRevokeCreditGrant: () => ({ isPending: false, mutateAsync: state.reclaim }),
  useCreateAllowance: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useUpdateAllowance: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useUpdateCreditSchedule: () => ({ isPending: false, mutateAsync: vi.fn() }),
}));

describe("AdminCreditsPage", () => {
  it("blocks credit changes against a backend without the new summary API", () => {
    state.summaryError = new ApiError(404, { error: "not_found", error_code: 1003, message: "Not found" });
    const view = render(<AdminCreditsPage />);
    expect(screen.getByRole("button", { name: "Allocate credits" })).toBeDisabled();
    expect(screen.getByText(/This backend does not support this Credits page/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Allocate credits" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    state.summaryError = null;
    view.rerender(<AdminCreditsPage />);
    expect(screen.getByRole("button", { name: "Allocate credits" })).toBeEnabled();
  });

  it("requires a current successful summary even when a cached balance is present", () => {
    state.summaryLoading = true;
    const view = render(<AdminCreditsPage />);
    expect(screen.getByRole("button", { name: "Allocate credits" })).toBeDisabled();
    state.summaryLoading = false;
    view.rerender(<AdminCreditsPage />);
    expect(screen.getByRole("button", { name: "Allocate credits" })).toBeEnabled();
    state.summaryError = new Error("Backend unavailable");
    state.staleSummary = true;
    view.rerender(<AdminCreditsPage />);
    expect(screen.getByText("100.250001 credits")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Allocate credits" })).toBeDisabled();
  });
  it("keeps a compact balance and the existing management tabs", () => {
    render(<AdminCreditsPage />);
    expect(screen.getByText("100.250001 credits")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Allocate credits" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("View balance details").closest("details"),
    ).not.toHaveAttribute("open");
    fireEvent.click(screen.getByText("View balance details"));
    expect(screen.getByText("Available to use")).toBeInTheDocument();
    expect(screen.getByText("Expires within 30 days")).toBeInTheDocument();
  });
  it("preserves the original tabs and makes allocation history visible immediately", async () => {
    const user = userEvent.setup();
    render(<AdminCreditsPage />);
    expect(screen.getAllByRole("tab").map(tab => tab.textContent)).toEqual(["Credit grants", "Free allowances", "Schedules"]);
    expect(screen.getByRole("tab", { name: "Credit grants" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("columnheader", { name: "Allocated" })).toBeVisible();
    expect(screen.getByRole("columnheader", { name: "Used" })).toBeVisible();
    await user.click(screen.getByRole("tab", { name: "Schedules" }));
    expect(screen.getByRole("button", { name: "Create schedule" })).toBeEnabled();
    await user.click(screen.getByRole("tab", { name: "Free allowances" }));
    expect(screen.getByRole("button", { name: "Create allowance" })).toBeEnabled();
  });

  it("confirms reclaim of the selected held allocation and supports operator read-only history", async () => {
    state.grants = [{ id: "allocation-1", batch_id: "batch", recipient_user_id: "owner", recipient_display_name: "Owner", recipient_billing_enabled: true, activation_state: "active", target_kind: "selected_users", amount_credits: 1000, amount_micros: 1_000_000_000, remaining_micros: 100_000_000, reserved_micros: 40_000_000, scope: { all_services: true, service_ids: [], service_slugs: [] }, status: "active", created_at: "2026-10-17T00:00:00Z", updated_at: "2026-10-17T00:00:00Z", granted_by: "admin" }];
    state.reclaim.mockResolvedValue({ ...state.grants[0], revoke_requested_at: "2026-10-17T01:00:00Z" });
    const view = render(<TooltipProvider><AdminCreditsPage /></TooltipProvider>);
    fireEvent.click(screen.getAllByRole("button", { name: "Reclaim remaining credits" })[0]!);
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Allocation allocation-1")).toBeInTheDocument();
    expect(within(dialog).getByText(/Requests already holding credits may finish/)).toBeInTheDocument();
    state.summaryError = new Error("Backend unavailable");
    state.staleSummary = true;
    view.rerender(<TooltipProvider><AdminCreditsPage /></TooltipProvider>);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(state.reclaim).not.toHaveBeenCalled();
    state.summaryError = null;
    view.rerender(<TooltipProvider><AdminCreditsPage /></TooltipProvider>);
    const restoredDialog = screen.getByRole("dialog");
    fireEvent.click(within(restoredDialog).getByRole("button", { name: "Reclaim remaining credits" }));
    await waitFor(() => expect(state.reclaim).toHaveBeenCalledWith("allocation-1"));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    state.canWrite = false;
    view.rerender(<TooltipProvider><AdminCreditsPage /></TooltipProvider>);
    expect(screen.queryByRole("button", { name: "Allocate credits" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Reclaim remaining credits" })).not.toBeInTheDocument();
    expect(screen.queryByText("Allocation breakdown")).not.toBeInTheDocument();
  });

});
