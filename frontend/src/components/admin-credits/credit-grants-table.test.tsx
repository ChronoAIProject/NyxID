import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
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

describe("CreditGrantsTable", () => {
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

    expect(screen.getByText("Pending activation")).toBeInTheDocument();
    const rolloutBadge = screen.getByText("Billing rollout off");
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
});
