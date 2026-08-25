import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { CreditSchedule } from "@/schemas/billing-credits";
import { SchedulesTable } from "./schedules-table";

function schedule(overrides: Partial<CreditSchedule> = {}): CreditSchedule {
  return {
    id: "schedule-1",
    amount_credits: 50,
    amount_micros: 50_000_000,
    recurrence: "monthly",
    expiry: { kind: "end_of_period" },
    target_kind: "all_users",
    target_user_ids: [],
    scope: { all_services: true, service_ids: [], service_slugs: [] },
    is_active: true,
    created_by: "admin-1",
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
    skipped_periods: 0,
    current_period: {
      start: "2026-08-01T00:00:00Z",
      end: "2026-09-01T00:00:00Z",
      status: "disbursing",
      disbursed_count: 412,
      amount_micros: 50_000_000,
      expires_at: "2026-09-01T00:00:00Z",
    },
    ...overrides,
  };
}

describe("SchedulesTable", () => {
  it("shows current-period progress and exposes pause as a switch", async () => {
    const onToggle = vi.fn();
    const user = userEvent.setup();
    render(
      <SchedulesTable
        schedules={[schedule()]}
        canWrite
        updatePending={false}
        onEdit={vi.fn()}
        onToggle={onToggle}
      />,
    );

    expect(screen.getByText("Disbursing 412")).toBeInTheDocument();
    const toggle = screen.getByRole("switch", { name: "Pause schedule" });
    await user.click(toggle);
    expect(onToggle).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "schedule-1",
      }),
    );
  });

  it("summarizes completed count and expiry", () => {
    render(
      <SchedulesTable
        schedules={[
          schedule({
            current_period: {
              start: "2026-08-01T00:00:00Z",
              end: "2026-09-01T00:00:00Z",
              status: "complete",
              disbursed_count: 1_204,
              amount_micros: 50_000_000,
              expires_at: "2026-09-01T00:00:00Z",
              completed_at: "2026-08-01T00:01:00Z",
            },
          }),
        ]}
        canWrite={false}
        updatePending={false}
        onEdit={vi.fn()}
        onToggle={vi.fn()}
      />,
    );

    expect(screen.getByText(/Complete.*1,204.*expires/)).toBeInTheDocument();
  });

  it("surfaces periods abandoned before completion", () => {
    render(
      <SchedulesTable
        schedules={[schedule({ skipped_periods: 2 })]}
        canWrite={false}
        updatePending={false}
        onEdit={vi.fn()}
        onToggle={vi.fn()}
      />,
    );

    expect(screen.getByText("2 skipped periods")).toBeInTheDocument();
  });
});
