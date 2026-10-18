import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { zodResolver } from "@hookform/resolvers/zod";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  scheduleFormSchema,
  type ScheduleForm,
  type CreditSchedule,
} from "@/schemas/billing-credits";
import type { DownstreamService } from "@/types/api";
import { useAppForm } from "@/components/ui/form";
import { ScheduleDialog } from "./schedule-dialog";

const owners = vi.hoisted(() => ({
  error: null as Error | null,
  refetch: vi.fn(),
}));
vi.mock("@/hooks/use-admin", () => ({
  useAdminUsers: () => ({
    data: { users: [] },
    isFetching: false,
    error: owners.error,
    refetch: owners.refetch,
  }),
}));
beforeEach(() => {
  owners.error = null;
  vi.clearAllMocks();
});

const service = {
  id: "service-1",
  name: "Token service",
  slug: "llm-token",
  is_active: true,
  effective_platform_metric: "tokens",
} as DownstreamService;

const schedule = {
  id: "schedule-1",
  amount_credits: 50,
  amount_micros: 50_000_000,
  recurrence: "monthly",
  expiry: { kind: "end_of_period" },
  target_kind: "all_users",
  target_user_ids: [],
  scope: { all_services: true, service_ids: [], service_slugs: [] },
  reason: null,
  is_active: true,
  created_by: "admin",
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  skipped_periods: 0,
} as CreditSchedule;

function Harness({
  onSubmit = vi.fn(),
  servicesError = false,
  onRetryServices = vi.fn(),
  values = {},
}: {
  readonly onSubmit?: (value: ScheduleForm) => Promise<void>;
  readonly servicesError?: boolean;
  readonly onRetryServices?: () => void;
  readonly values?: Partial<ScheduleForm>;
} = {}) {
  const form = useAppForm<ScheduleForm>({
    mode: "onChange",
    resolver: zodResolver(scheduleFormSchema),
    defaultValues: {
      amount_credits: 50,
      recurrence: "monthly",
      expiry: { kind: "end_of_period" },
      target_kind: "all_users",
      target_user_ids: [],
      all_services: true,
      service_refs: [],
      reason: "",
      ...values,
    },
  });
  return (
    <ScheduleDialog
      open
      onOpenChange={vi.fn()}
      form={form}
      services={[service]}
      servicesLoading={false}
      servicesError={servicesError}
      onRetryServices={onRetryServices}
      pending={false}
      editingSchedule={schedule}
      onSubmit={onSubmit}
    />
  );
}

describe("ScheduleDialog", () => {
  it("disables unchanged and required-empty saves, including direct form submissions", async () => {
    const onSubmit = vi.fn();
    render(<Harness onSubmit={onSubmit} />);
    const save = screen.getByRole("button", { name: "Save changes" });
    expect(save).toBeDisabled();
    fireEvent.submit(save.closest("form")!);
    fireEvent.change(screen.getByLabelText("Credits per owner"), {
      target: { value: "" },
    });
    expect(screen.getByLabelText("Credits per owner")).toHaveValue(null);
    expect(save).toBeDisabled();
    fireEvent.submit(save.closest("form")!);
    await waitFor(() =>
      expect(screen.getByLabelText("Credits per owner")).toHaveAttribute(
        "aria-invalid",
        "true",
      ),
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("shows scoped lookup failures, retries, and saves the edited policy after recovery", async () => {
    const onSubmit = vi.fn();
    const retry = vi.fn();
    const values: Partial<ScheduleForm> = {
      all_services: false,
      service_refs: [service.id],
    };
    const view = render(
      <Harness
        onSubmit={onSubmit}
        servicesError
        onRetryServices={retry}
        values={values}
      />,
    );
    fireEvent.change(screen.getByLabelText("Credits per owner"), {
      target: { value: "75" },
    });
    expect(
      screen.getByText(/Services could not be loaded/),
    ).toBeInTheDocument();
    const save = screen.getByRole("button", { name: "Save changes" });
    expect(save).toBeDisabled();
    fireEvent.submit(save.closest("form")!);
    await waitFor(() => expect(save).toBeDisabled());
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledOnce();
    view.rerender(<Harness onSubmit={onSubmit} values={values} />);
    fireEvent.click(save);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
    expect(onSubmit.mock.calls[0]?.[0]).toMatchObject({
      amount_credits: 75,
      service_refs: [service.id],
    });
  });

  it("blocks owner lookup failures and recovers without losing selected recipients", async () => {
    owners.error = new Error("owners offline");
    const onSubmit = vi.fn();
    const values: Partial<ScheduleForm> = {
      target_kind: "selected_users",
      target_user_ids: ["owner-1"],
    };
    const view = render(<Harness onSubmit={onSubmit} values={values} />);
    fireEvent.change(screen.getByLabelText("Credits per owner"), {
      target: { value: "75" },
    });
    const save = screen.getByRole("button", { name: "Save changes" });
    expect(screen.getByText(/Owners could not be loaded/)).toBeInTheDocument();
    expect(save).toBeDisabled();
    fireEvent.submit(save.closest("form")!);
    await waitFor(() => expect(save).toBeDisabled());
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(owners.refetch).toHaveBeenCalledTimes(2);
    owners.error = null;
    view.rerender(<Harness onSubmit={onSubmit} values={values} />);
    fireEvent.click(save);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
    expect(onSubmit.mock.calls[0]?.[0].target_user_ids).toEqual(["owner-1"]);
  });

  it("defaults to period expiry and reveals days only for fixed expiry", () => {
    render(<Harness />);

    expect(screen.getByText("Credits per owner")).toBeInTheDocument();
    expect(
      screen.getByText(/Set the amount, recipients, expiry, and service limits/),
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
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    expect(
      await screen.findByText("Enter 1 to 3,650 days"),
    ).toBeInTheDocument();
  });
  it("edits recurring service limits for future allocations without changing selected owners", async () => {
    const onSubmit = vi.fn();
    render(<Harness onSubmit={onSubmit} values={{ target_kind: "selected_users", target_user_ids: ["owner-1"], service_limits: [{ service_refs: [service.id], amount_credits: 20, label: "LLM" }] }} />);
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
    fireEvent.click(screen.getByText(/LLM · Token service/));
    expect(within(screen.getByRole("group", { name: "Services for limit 1" })).getByRole("checkbox")).toBeChecked();
    fireEvent.change(screen.getByLabelText("Credit limit"), { target: { value: "30" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
    expect(onSubmit.mock.calls[0]?.[0]).toMatchObject({ target_user_ids: ["owner-1"], service_limits: [{ amount_credits: 30, service_refs: [service.id] }] });
    expect(screen.getByText(/Changes apply to future/)).toBeInTheDocument();
  });

});
