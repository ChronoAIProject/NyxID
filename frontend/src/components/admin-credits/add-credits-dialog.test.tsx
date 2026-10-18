import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { zodResolver } from "@hookform/resolvers/zod";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AddCreditsForm } from "@/schemas/billing-credits";
import { addCreditsFormSchema } from "@/schemas/billing-credits";
import type { DownstreamService } from "@/types/api";
import { useAppForm } from "@/components/ui/form";
import { AddCreditsDialog } from "./add-credits-dialog";

const service = {
  id: "service-1",
  name: "Service one",
  slug: "service-one",
  is_active: true,
  effective_platform_metric: "requests",
} as DownstreamService;

const owners = vi.hoisted(() => ({
  error: null as Error | null,
  refetch: vi.fn(),
}));
beforeEach(() => {
  owners.error = null;
  vi.clearAllMocks();
});

vi.mock("@/hooks/use-admin", () => ({
  useAdminUsers: () => ({
    data: { users: [] },
    isFetching: false,
    error: owners.error,
    refetch: owners.refetch,
  }),
}));

function Harness({
  onSubmit,
  values = {},
  servicesError = false,
  servicesLoading = false,
  onRetryServices = vi.fn(),
  pending = false,
  services = [service],
  onOpenChange = vi.fn(),
}: {
  readonly onSubmit: (value: AddCreditsForm) => void | Promise<void>;
  readonly values?: Partial<AddCreditsForm>;
  readonly servicesError?: boolean;
  readonly servicesLoading?: boolean;
  readonly onRetryServices?: () => void;
  readonly pending?: boolean;
  readonly services?: DownstreamService[];
  readonly onOpenChange?: (open: boolean) => void;
}) {
  const form = useAppForm<AddCreditsForm>({
    mode: "onChange",
    resolver: zodResolver(addCreditsFormSchema),
    defaultValues: {
      amount_credits: 10,
      recurrence: "one_time",
      target_kind: "all_users",
      target_user_ids: [],
      all_services: false,
      service_refs: [service.id],
      expires_at: "",
      expiry: { kind: "end_of_period" },
      reason: "",
      ...values,
    },
  });
  return (
    <AddCreditsDialog
      open
      onOpenChange={onOpenChange}
      form={form}
      services={services}
      servicesLoading={servicesLoading}
      servicesError={servicesError}
      onRetryServices={onRetryServices}
      pending={pending}
      onSubmit={async (value) => onSubmit(value)}
    />
  );
}

describe("AddCreditsDialog", () => {
  it("labels pending service discovery as loading and enables scoped submit after it completes", async () => {
    const onSubmit = vi.fn();
    const view = render(<Harness onSubmit={onSubmit} servicesLoading />);
    fireEvent.change(screen.getByLabelText("Credits per owner"), {
      target: { value: "20" },
    });
    const submit = screen.getByRole("button", { name: "Allocate credits" });
    expect(submit).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Loading owners or services",
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    view.rerender(<Harness onSubmit={onSubmit} />);
    expect(submit).toBeEnabled();
    fireEvent.click(submit);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
  });

  it.each([
    {
      target_kind: "selected_users" as const,
      target_user_ids: [],
      all_services: true,
      service_refs: [],
    },
    {
      target_kind: "all_users" as const,
      target_user_ids: [],
      all_services: false,
      service_refs: [],
    },
  ])("requires each selected target independently: %j", async (values) => {
    const onSubmit = vi.fn();
    render(<Harness onSubmit={onSubmit} values={values} />);
    fireEvent.change(screen.getByLabelText("Credits per owner"), {
      target: { value: "20" },
    });
    const submit = screen.getByRole("button", { name: "Allocate credits" });
    expect(submit).toBeDisabled();
    fireEvent.submit(submit.closest("form")!);
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("Cannot save"),
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("requires recipients, scoped services, and amount even after another field is changed", async () => {
    const onSubmit = vi.fn();
    render(
      <Harness
        onSubmit={onSubmit}
        values={{
          target_kind: "selected_users",
          target_user_ids: [],
          service_refs: [],
        }}
      />,
    );
    const submit = screen.getByRole("button", { name: "Allocate credits" });
    fireEvent.change(screen.getByLabelText("Reason / note"), {
      target: { value: "test" },
    });
    expect(submit).toBeDisabled();
    fireEvent.submit(submit.closest("form")!);
    expect(
      await screen.findByText("Select at least one owner", { exact: true }),
    ).toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("Credits per owner"), {
      target: { value: "" },
    });
    expect(screen.getByLabelText("Credits per owner")).toHaveValue(null);
    expect(submit).toBeDisabled();
  });

  it("blocks scoped discovery failures with retry but permits an explicit all-service choice", async () => {
    const onSubmit = vi.fn();
    const retry = vi.fn();
    render(
      <Harness onSubmit={onSubmit} servicesError onRetryServices={retry} />,
    );
    fireEvent.change(screen.getByLabelText("Credits per owner"), {
      target: { value: "20" },
    });
    const submit = screen.getByRole("button", { name: "Allocate credits" });
    expect(submit).toBeDisabled();
    expect(
      screen.getByText(/Services could not be loaded/),
    ).toBeInTheDocument();
    fireEvent.submit(submit.closest("form")!);
    await waitFor(() => expect(submit).toBeDisabled());
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("switch"));
    expect(submit).toBeEnabled();
    fireEvent.click(submit);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
    expect(onSubmit.mock.calls[0]?.[0]).toMatchObject({
      all_services: true,
      service_refs: [],
    });
  });

  it("guards duplicate form submission and closing while the request is in flight", async () => {
    let finish: (() => void) | undefined;
    const onSubmit = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    const onOpenChange = vi.fn();
    render(<Harness onSubmit={onSubmit} onOpenChange={onOpenChange} />);
    fireEvent.change(screen.getByLabelText("Credits per owner"), {
      target: { value: "20" },
    });
    const submit = screen.getByRole("button", { name: "Allocate credits" });
    fireEvent.submit(submit.closest("form")!);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
    fireEvent.submit(submit.closest("form")!);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(onSubmit).toHaveBeenCalledOnce();
    finish?.();
    await waitFor(() => expect(submit).toBeEnabled());
  });

  it("cannot submit an untouched recipient form", () => {
    const onSubmit = vi.fn();
    render(<Harness onSubmit={onSubmit} />);
    expect(screen.getByRole("button", { name: "Allocate credits" })).toBeDisabled();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("clears scoped services when broadening to all services", async () => {
    const onSubmit = vi.fn();
    render(<Harness onSubmit={onSubmit} />);

    fireEvent.click(screen.getByRole("switch"));
    fireEvent.click(screen.getByRole("button", { name: "Allocate credits" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit.mock.calls[0]?.[0].all_services).toBe(true);
    expect(onSubmit.mock.calls[0]?.[0].service_refs).toEqual([]);
  });

  it("allows one-time to recurring and back without a hidden expiry error", async () => {
    const onSubmit = vi.fn();
    render(<Harness onSubmit={onSubmit} />);

    fireEvent.click(screen.getAllByRole("combobox")[0]!);
    fireEvent.click(
      await screen.findByRole("option", { name: "Recurring · monthly" }),
    );
    fireEvent.click(screen.getAllByRole("combobox")[2]!);
    fireEvent.click(
      await screen.findByRole("option", { name: "After a number of days" }),
    );
    fireEvent.change(screen.getByRole("spinbutton", { name: "Expiry days" }), {
      target: { value: "45" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Allocate credits" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit.mock.calls[0]?.[0].recurrence).toBe("monthly");

    fireEvent.click(screen.getAllByRole("combobox")[0]!);
    fireEvent.click(await screen.findByRole("option", { name: "One time" }));
    fireEvent.click(screen.getByRole("button", { name: "Allocate credits" }));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(onSubmit).toHaveBeenCalledTimes(2);
    expect(onSubmit.mock.calls[1]?.[0].recurrence).toBe("one_time");
  });

  it("shows the days error beside the field and never submits invalid recurring values", async () => {
    const onSubmit = vi.fn();
    render(<Harness onSubmit={onSubmit} />);
    fireEvent.click(screen.getAllByRole("combobox")[0]!);
    fireEvent.click(
      await screen.findByRole("option", { name: "Recurring · monthly" }),
    );
    fireEvent.click(screen.getAllByRole("combobox")[2]!);
    fireEvent.click(
      await screen.findByRole("option", { name: "After a number of days" }),
    );
    fireEvent.change(screen.getByRole("spinbutton", { name: "Expiry days" }), {
      target: { value: "" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Allocate credits" }));
    expect(
      await screen.findByText("Enter 1 to 3,650 days"),
    ).toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();
  });
  it("creates grouped limits within a shared per-owner total and retains them across modes", async () => {
    const onSubmit = vi.fn();
    const services = [service, { ...service, id: "service-2", name: "Second service", slug: "second" }];
    render(<Harness onSubmit={onSubmit} services={services} values={{ amount_credits: 1000, all_services: true, service_refs: [] }} />);
    fireEvent.click(screen.getByRole("button", { name: "Add service limit" }));
    const editor = screen.getByRole("group", { name: "Services for limit 1" });
    fireEvent.click(within(editor).getByRole("checkbox", { name: /Service one/ }));
    fireEvent.click(within(editor).getByRole("checkbox", { name: /Second service/ }));
    expect(within(editor).queryByText("requests")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Group label (optional)"), { target: { value: "LLM" } });
    fireEvent.change(screen.getByLabelText("Credit limit"), { target: { value: "300" } });
    expect(screen.getByText(/LLM: Service one, Second service/)).toHaveTextContent("up to 300 credits from each allocation");
    fireEvent.click(screen.getAllByRole("combobox")[0]!);
    fireEvent.click(await screen.findByRole("option", { name: "Recurring · monthly" }));
    expect(screen.getByText(/Each recurring allocation receives fresh limits/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Allocate credits" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
    expect(onSubmit.mock.calls[0]?.[0]).toMatchObject({ amount_credits: 1000, recurrence: "monthly", all_services: true, service_limits: [{ service_refs: ["service-1", "service-2"], amount_credits: 300, label: "LLM" }] });
    fireEvent.click(screen.getAllByRole("combobox")[0]!);
    fireEvent.click(await screen.findByRole("option", { name: "One time" }));
    expect(screen.getByLabelText("Credit limit")).toHaveValue(300);
  });

  it("opens both disclosures and focuses an invalid limit after the total is reduced", async () => {
    const onSubmit = vi.fn();
    render(<Harness onSubmit={onSubmit} values={{ amount_credits: 1000, all_services: true, service_refs: [], service_limits: [{ service_refs: [service.id], amount_credits: 500, label: "Twitter" }] }} />);
    const outer = screen.getByText(/Service limits \(optional\)/).closest("details")!;
    outer.removeAttribute("open");
    expect(outer).not.toHaveAttribute("open");
    fireEvent.change(screen.getByLabelText("Credits per owner"), { target: { value: "100" } });
    fireEvent.submit(screen.getByRole("button", { name: "Allocate credits" }).closest("form")!);
    const limit = screen.getByLabelText("Credit limit");
    await waitFor(() => expect(limit).toHaveAttribute("aria-invalid", "true"));
    expect(outer).toHaveAttribute("open");
    expect(limit.closest("details")).toHaveAttribute("open");
    await waitFor(() => expect(limit).toHaveFocus());
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.change(limit, { target: { value: "50" } });
    fireEvent.click(screen.getByRole("button", { name: "Allocate credits" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
  });

  it("requires service lookup with caps even for all services and recovers through Retry", async () => {
    const onSubmit = vi.fn();
    const retry = vi.fn();
    const values: Partial<AddCreditsForm> = { all_services: true, service_refs: [], service_limits: [{ service_refs: [service.id], amount_credits: 5, label: "" }] };
    const view = render(<Harness onSubmit={onSubmit} values={values} servicesError onRetryServices={retry} />);
    fireEvent.click(screen.getByText(/Limit 1 · Service one/));
    fireEvent.change(screen.getByLabelText("Credits per owner"), { target: { value: "20" } });
    const submit = screen.getByRole("button", { name: "Allocate credits" });
    expect(submit).toBeDisabled();
    await act(async () => { fireEvent.submit(submit.closest("form")!); });
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledOnce();
    view.rerender(<Harness onSubmit={onSubmit} values={values} />);
    fireEvent.change(screen.getByLabelText("Credits per owner"), { target: { value: "20" } });
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
  });

});
