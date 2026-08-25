import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ScheduleForm } from "@/schemas/billing-credits";
import type { DownstreamService } from "@/types/api";
import { useAppForm } from "@/components/ui/form";
import { ScheduleDialog } from "./schedule-dialog";

const service = {
  id: "service-1",
  name: "Token service",
  slug: "llm-token",
  is_active: true,
  effective_platform_metric: "tokens",
} as DownstreamService;

function Harness() {
  const form = useAppForm<ScheduleForm>({
    defaultValues: {
      amount_credits: 50,
      recurrence: "monthly",
      expiry: { kind: "end_of_period" },
      target_kind: "all_users",
      target_user_ids: [],
      all_services: true,
      service_refs: [],
      reason: "",
    },
  });
  return (
    <ScheduleDialog
      open
      onOpenChange={vi.fn()}
      form={form}
      services={[service]}
      pending={false}
      editingSchedule={null}
      onSubmit={vi.fn()}
    />
  );
}

describe("ScheduleDialog", () => {
  it("defaults to period expiry and reveals days only for fixed expiry", () => {
    render(<Harness />);

    expect(screen.getByText("Wallet credits per owner")).toBeInTheDocument();
    expect(screen.getByText(/Credits are wallet currency/)).toBeInTheDocument();
    expect(screen.getByLabelText("End of each period")).toBeChecked();
    expect(
      screen.queryByLabelText("Days until expiry"),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("After a fixed number of days"));
    expect(screen.getByLabelText("Days until expiry")).toBeInTheDocument();
  });
});
