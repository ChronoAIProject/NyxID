import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { zodResolver } from "@hookform/resolvers/zod";
import { describe, expect, it, vi } from "vitest";
import {
  scheduleFormSchema,
  type ScheduleForm,
} from "@/schemas/billing-credits";
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

function Harness({
  recipientSchedules = false,
  timing = "calendar",
  onSubmit = vi.fn(),
}: {
  recipientSchedules?: boolean;
  timing?: ScheduleForm["timing"];
  onSubmit?: (value: ScheduleForm) => Promise<void>;
}) {
  const form = useAppForm<ScheduleForm>({
    resolver: zodResolver(scheduleFormSchema),
    defaultValues: {
      amount_credits: 50,
      recurrence: "monthly",
      timing,
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
      onSubmit={onSubmit}
      recipientSchedules={recipientSchedules}
    />
  );
}

describe("ScheduleDialog", () => {
  it("preserves input and blocks recipient creation when its capability disappears", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { rerender } = render(
      <Harness recipientSchedules timing="recipient" onSubmit={onSubmit} />,
    );
    fireEvent.change(screen.getByLabelText("Credits per recipient"), {
      target: { value: "123" },
    });
    const submit = screen.getByRole("button", { name: "Create schedule" });
    await waitFor(() => expect(submit).toBeEnabled());

    rerender(<Harness timing="recipient" onSubmit={onSubmit} />);
    expect(screen.getByLabelText("Credits per recipient")).toHaveValue(123);
    expect(
      screen.getByRole("combobox", { name: "Renewal cycle" }),
    ).toHaveTextContent("From when each recipient joins");
    expect(
      screen.getByText(/Recipient scheduling is currently unavailable/),
    ).toBeInTheDocument();
    expect(submit).toBeDisabled();
    await act(async () => {
      fireEvent.submit(submit.closest("form")!);
    });
    expect(onSubmit).not.toHaveBeenCalled();

    await userEvent.click(
      screen.getByRole("combobox", { name: "Renewal cycle" }),
    );
    expect(
      screen.getByRole("option", { name: "From when each recipient joins" }),
    ).toHaveAttribute("aria-disabled", "true");
    await userEvent.click(
      screen.getByRole("option", { name: "Calendar periods (UTC)" }),
    );
    await waitFor(() => expect(submit).toBeEnabled());
    await userEvent.click(submit);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        amount_credits: 123,
        timing: "calendar",
      }),
    );
  });

  it("defaults to period expiry and reveals days only for fixed expiry", () => {
    render(<Harness />);

    expect(screen.getByText("Credits per recipient")).toBeInTheDocument();
    expect(
      screen.getByText(/Issued as a separate credit allocation/),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("End of each period")).toBeChecked();
    expect(
      screen.queryByLabelText("Days until expiry"),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("After a fixed number of days"));
    expect(screen.getByLabelText("Days until expiry")).toBeInTheDocument();
  });

  it("shows a validation error when fixed expiry days are cleared", async () => {
    render(<Harness />);

    fireEvent.click(screen.getByLabelText("After a fixed number of days"));
    fireEvent.change(screen.getByLabelText("Days until expiry"), {
      target: { value: "" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create schedule" }));

    expect(
      await screen.findByText("Enter 1 to 3,650 days"),
    ).toBeInTheDocument();
  });
});
