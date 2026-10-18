import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { CreditGrant } from "@/schemas/billing-credits";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
  it("explains scheduled reclaim scope and how to stop future periods", () => {
    render(
      <Dialog open>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Reclaim remaining credits</DialogTitle>
            <GrantRevokeDescription grant={grant} />
          </DialogHeader>
        </DialogContent>
      </Dialog>,
    );

    expect(screen.getByText("Allocation grant-1")).toBeInTheDocument();
    expect(screen.getByText(/Recurring schedule-1/)).toBeInTheDocument();
    expect(screen.getByText(/affects this allocation only/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Pausing recurring credits stops future issuance/i),
    ).toBeInTheDocument();
  });
});
