import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AdminCreditGrant } from "@/schemas/billing-credits";
import { TooltipProvider } from "@/components/ui/tooltip";
import { CreditGrantsTable } from "./credit-grants-table";
import { rolloutWarningMessage } from "./credit-grant-visibility";

function grant(overrides: Partial<AdminCreditGrant> = {}): AdminCreditGrant {
  return {
    id: "grant-1",
    batch_id: "batch-1",
    recipient_user_id: "user-1",
    recipient_email: "user@example.com",
    recipient_display_name: "Example User",
    recipient_billing_enabled: true,
    activation_state: "active",
    target_kind: "selected_users",
    amount_credits: 5,
    amount_micros: 5_000_000,
    remaining_micros: 5_000_000,
    reserved_micros: 0,
    scope: { all_services: true, service_ids: [], service_slugs: [] },
    granted_by: "admin-1",
    status: "active",
    created_at: "2026-08-24T00:00:00Z",
    updated_at: "2026-08-24T00:00:00Z",
    ...overrides,
  };
}

afterEach(() => vi.useRealTimers());

describe("CreditGrantsTable", () => {
  it("changes active to expired on the open page and reevaluates refreshed props", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-17T00:00:00Z"));
    const props = {
      canWrite: false,
      revokePending: false,
      page: 1,
      perPage: 50,
      total: 1,
      fetching: false,
      onPageChange: vi.fn(),
      onRevoke: vi.fn(),
    };
    const view = render(
      <TooltipProvider>
        <CreditGrantsTable
          {...props}
          grants={[grant({ expires_at: "2026-10-17T00:00:05Z" })]}
        />
      </TooltipProvider>,
    );
    expect(screen.getAllByText("Active")).toHaveLength(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(screen.queryByText("Active")).not.toBeInTheDocument();
    expect(screen.getAllByText("Expired")).toHaveLength(2);
    view.rerender(
      <TooltipProvider>
        <CreditGrantsTable
          {...props}
          grants={[grant({ expires_at: "2026-10-17T00:00:10Z" })]}
        />
      </TooltipProvider>,
    );
    expect(screen.getAllByText("Active")).toHaveLength(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(screen.getAllByText("Expired")).toHaveLength(2);
    view.rerender(
      <TooltipProvider>
        <CreditGrantsTable
          {...props}
          grants={[
            grant({
              status: "revoked",
              activation_state: "pending_activation",
              expires_at: "2026-10-17T00:00:01Z",
            }),
          ]}
        />
      </TooltipProvider>,
    );
    expect(screen.getAllByText("Reclaimed", { selector: "div" })).toHaveLength(2);
    expect(screen.queryByText("Pending activation")).not.toBeInTheDocument();
  });

  it("distinguishes pending activation and explains rollout-hidden grants", async () => {
    const user = userEvent.setup();
    render(
      <TooltipProvider delayDuration={0}>
        <CreditGrantsTable
          grants={[
            grant({
              activation_state: "pending_activation",
              recipient_billing_enabled: false,
            }),
          ]}
          canWrite
          revokePending={false}
          page={1}
          perPage={50}
          total={1}
          fetching={false}
          onPageChange={vi.fn()}
          onRevoke={vi.fn()}
        />
      </TooltipProvider>,
    );

    expect(screen.getAllByText("Pending activation")).not.toHaveLength(0);
    const rolloutBadge = screen.getAllByText("Billing rollout off")[0];
    expect(rolloutBadge).toBeTruthy();
    if (!rolloutBadge) throw new Error("Expected rollout badge");
    await user.hover(rolloutBadge);
    expect(
      await screen.findAllByText(
        "Not in billing rollout - user cannot see billing yet.",
      ),
    ).not.toHaveLength(0);
  });

  it("builds a post-issuance warning only for disabled recipients", () => {
    expect(
      rolloutWarningMessage([
        {
          recipient_user_id: "user-1",
          recipient_billing_enabled: true,
          activation_state: "active",
        },
      ]),
    ).toBeNull();
    expect(
      rolloutWarningMessage([
        {
          recipient_user_id: "user-1",
          recipient_billing_enabled: false,
          activation_state: "active",
        },
        {
          recipient_user_id: "user-2",
          recipient_billing_enabled: false,
          activation_state: "pending_activation",
        },
      ]),
    ).toBe(
      "2 recipients are not in the billing rollout - users cannot see billing yet.",
    );
  });
  it("allows reclaim during holds and shows pending state in both layouts", () => {
    const onRevoke = vi.fn();
    const issued = grant({ reserved_micros: 5_000_000, reserved_micros_exact: "5000000", remaining_micros_exact: "25250001", schedule_id: "recurring-1", period_start: "2026-10-01T00:00:00Z" });
    const props = { grants: [issued], canWrite: true, revokePending: false, page: 1, perPage: 50, total: 1, fetching: false, onPageChange: vi.fn(), onRevoke };
    const view = render(<TooltipProvider><CreditGrantsTable {...props} /></TooltipProvider>);
    const reclaim = screen.getAllByRole("button", { name: "Reclaim remaining credits" });
    expect(reclaim).toHaveLength(2);
    expect(reclaim[0]).toBeEnabled();
    fireEvent.click(reclaim[0]!);
    expect(onRevoke).toHaveBeenCalledWith(issued);
    expect(screen.queryByText("Allocation breakdown")).not.toBeInTheDocument();
    expect(screen.getAllByText(/5 credits held; reclaim can be requested now/)).toHaveLength(2);
    view.rerender(<TooltipProvider><CreditGrantsTable {...props} grants={[{ ...issued, revoke_requested_at: "2026-10-17T00:00:00Z", available_micros: "0" }]} /></TooltipProvider>);
    expect(screen.getAllByText("Reclaim pending")).toHaveLength(2);
    expect(screen.getAllByRole("button", { name: "Reclaim remaining credits" }).every(button => button.hasAttribute("disabled"))).toBe(true);
    view.rerender(<TooltipProvider><CreditGrantsTable {...props} canWrite={false} /></TooltipProvider>);
    expect(screen.queryByRole("button", { name: "Reclaim remaining credits" })).not.toBeInTheDocument();
  });

});
