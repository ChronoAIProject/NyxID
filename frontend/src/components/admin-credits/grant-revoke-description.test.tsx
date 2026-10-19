import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { CreditGrant } from "@/schemas/billing-credits";
import { Dialog, DialogContent, DialogHeader } from "@/components/ui/dialog";
import { GrantRevokeDescription } from "./grant-revoke-description";

const grant = {
  id: "grant-1",
  batch_id: "schedule-1:1785542400000",
  schedule_id: "schedule-1",
  period_start: "2026-08-01T00:00:00Z",
  recipient_user_id: "owner-1",
  recipient_email: "owner@example.com",
  activation_state: "active",
  target_kind: "selected_users",
  amount_credits: 50,
  amount_micros: 50_000_000,
  remaining_micros: 50_000_000,
  reserved_micros: 0,
  scope: { all_services: true, service_ids: [], service_slugs: [] },
  granted_by: "schedule-1",
  status: "active",
  created_at: "2026-08-01T00:00:00Z",
  updated_at: "2026-08-01T00:00:00Z",
} satisfies CreditGrant;

describe("GrantRevokeDescription", () => {
  it("explains scheduled revocation scope and how to stop future periods", () => {
    render(
      <Dialog open>
        <DialogContent>
          <DialogHeader>
            <GrantRevokeDescription grant={grant} />
          </DialogHeader>
        </DialogContent>
      </Dialog>,
    );

    expect(screen.getByText(/affects this period only/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Pausing the schedule stops future disbursements/i),
    ).toBeInTheDocument();
  });
});
