import { act, render, screen, within } from "@testing-library/react";
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

function history(row: AdminCreditGrant) {
  return render(
    <CreditGrantsTable
      grants={[row]}
      allocationPolicies
      canWrite
      revokePending={false}
      page={1}
      perPage={50}
      total={1}
      fetching={false}
      onPageChange={vi.fn()}
      onRevoke={vi.fn()}
    />,
  );
}
it.each(["revoked", "expired"] as const)(
  "does not infer spent value from old %s API rows lacking terminal detail",
  (status) => {
    history(grant({ status, remaining_micros: 0 }));
    expect(
      screen.getByText("Unavailable from this server"),
    ).toBeInTheDocument();
  },
);
it("shows exact used, reclaimed and matching group held value inline", () => {
  history(
    grant({
      amount: "1000",
      remaining: "0",
      terminal_amount: "100",
      reclaimed: "100",
      status: "revoked",
      service_limits: [
        {
          key: "group",
          service_ids: ["a", "b"],
          service_slugs: ["openai", "anthropic"],
          label: "LLM",
          limit: "300",
          consumed: "299.999999999999",
          reserved: "0",
          held: "0.000000000001",
        },
      ],
    }),
  );
  const used = screen.getByText("Used:").parentElement!;
  expect(within(used).getByText("900 credits")).toBeInTheDocument();
  expect(screen.getByText(/LLM: openai, anthropic/)).toBeInTheDocument();
  expect(screen.getByText(/held 0.000000000001/)).toBeInTheDocument();
  expect(screen.getByText("Reclaimed:")).toBeInTheDocument();
});
it("updates wall-clock expiry on an open history without remount", () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-19T00:00:00Z"));
  try {
    history(grant({ expires_at: "2026-10-19T00:00:01Z" }));
    expect(screen.getByText("active")).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(15_000);
    });
    expect(screen.getByText("Expired")).toBeInTheDocument();
  } finally {
    vi.useRealTimers();
  }
});

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

it.each([
  [
    "org_members",
    "3 organizations · members",
    { target_org_ids: ["a", "b", "c"] },
  ],
  ["groups", "2 groups · members", { target_group_ids: ["a", "b"] }],
] as const)(
  "labels %s provenance on each personal grant",
  (kind, label, targets) => {
    render(
      <TooltipProvider>
        <CreditGrantsTable
          grants={[
            grant({
              target_kind: kind,
              target_org_ids:
                "target_org_ids" in targets ? [...targets.target_org_ids] : [],
              target_group_ids:
                "target_group_ids" in targets
                  ? [...targets.target_group_ids]
                  : [],
            }),
          ]}
          canWrite={false}
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
    expect(screen.getByText(label)).toBeInTheDocument();
    expect(screen.getByText("Example User")).toBeInTheDocument();
  },
);
