import { zodResolver } from "@hookform/resolvers/zod";
import { allowanceFormSchema } from "@/schemas/billing-credits";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AllowanceForm, UsageAllowance } from "@/schemas/billing-credits";
import type { DownstreamService } from "@/types/api";
import { useAppForm } from "@/components/ui/form";
import { AllowanceDialog } from "./credits-dialogs";

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

const tokenService = {
  id: "service-token",
  name: "Token service",
  slug: "llm-token",
  is_active: true,
  effective_platform_metric: "tokens",
} as DownstreamService;

const includedUsage: UsageAllowance = {
  id: "included-1",
  service_id: tokenService.id,
  service_slug: tokenService.slug,
  metric: "tokens",
  quantity: 1_000_000,
  recurrence: "monthly",
  target_kind: "all_users",
  target_user_ids: [],
  is_active: true,
  created_by: "admin-1",
  created_at: "2026-10-17T00:00:00Z",
  updated_at: "2026-10-17T00:00:00Z",
};

function AllowanceHarness({
  onSubmit = vi.fn(),
  servicesError = false,
  onRetryServices = vi.fn(),
  values = {},
  editing = false,
}: {
  readonly onSubmit?: (value: AllowanceForm) => Promise<void>;
  readonly servicesError?: boolean;
  readonly onRetryServices?: () => void;
  readonly values?: Partial<AllowanceForm>;
  readonly editing?: boolean;
} = {}) {
  const form = useAppForm<AllowanceForm>({
    mode: "onChange",
    resolver: zodResolver(allowanceFormSchema),
    defaultValues: {
      service_ref: "",
      quantity: 1_000_000,
      recurrence: "monthly",
      target_kind: "all_users",
      target_user_ids: [],
      ...values,
    },
  });

  return (
    <AllowanceDialog
      open
      onOpenChange={vi.fn()}
      form={form}
      services={[tokenService]}
      servicesLoading={false}
      servicesError={servicesError}
      onRetryServices={onRetryServices}
      pending={false}
      editingAllowance={editing ? includedUsage : null}
      onSubmit={onSubmit}
    />
  );
}

describe("credits dialogs", () => {
  it("requires a service and changed values, and guards empty quantities and direct submit", async () => {
    const onSubmit = vi.fn();
    render(<AllowanceHarness onSubmit={onSubmit} />);
    const save = screen.getByRole("button", { name: "Add included usage" });
    expect(save).toBeDisabled();
    fireEvent.click(screen.getByText("Token service"));
    expect(save).toBeEnabled();
    fireEvent.change(screen.getByLabelText("Free tokens"), {
      target: { value: "" },
    });
    expect(screen.getByLabelText("Free tokens")).toHaveValue(null);
    expect(save).toBeDisabled();
    fireEvent.submit(save.closest("form")!);
    await waitFor(() =>
      expect(screen.getByLabelText("Free tokens")).toHaveAttribute(
        "aria-invalid",
        "true",
      ),
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("blocks failed service discovery and saves included usage after retry", async () => {
    const onSubmit = vi.fn();
    const retry = vi.fn();
    const values = { service_ref: tokenService.id };
    const view = render(
      <AllowanceHarness
        editing
        onSubmit={onSubmit}
        servicesError
        onRetryServices={retry}
        values={values}
      />,
    );
    const save = screen.getByRole("button", { name: "Save changes" });
    expect(save).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Free tokens"), {
      target: { value: "2000" },
    });
    expect(
      screen.getByText(/Services could not be loaded/),
    ).toBeInTheDocument();
    fireEvent.submit(save.closest("form")!);
    await waitFor(() => expect(save).toBeDisabled());
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledOnce();
    view.rerender(
      <AllowanceHarness editing onSubmit={onSubmit} values={values} />,
    );
    fireEvent.click(save);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
    expect(onSubmit.mock.calls[0]?.[0]).toMatchObject({
      quantity: 2000,
      service_ref: tokenService.id,
    });
  });

  it("requires selected owners and reports owner lookup failures during edits", async () => {
    owners.error = new Error("owners offline");
    const onSubmit = vi.fn();
    render(
      <AllowanceHarness
        editing
        onSubmit={onSubmit}
        values={{
          service_ref: tokenService.id,
          target_kind: "selected_users",
          target_user_ids: ["owner-1"],
        }}
      />,
    );
    fireEvent.change(screen.getByLabelText("Free tokens"), {
      target: { value: "2000" },
    });
    expect(screen.getByText(/Owners could not be loaded/)).toBeInTheDocument();
    const save = screen.getByRole("button", { name: "Save changes" });
    expect(save).toBeDisabled();
    fireEvent.submit(save.closest("form")!);
    await waitFor(() => expect(save).toBeDisabled());
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("updates the allowance label and preview from the selected service metric", () => {
    render(<AllowanceHarness />);

    expect(screen.getByText("Free units")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Token service"));

    expect(screen.getByText("Free tokens")).toBeInTheDocument();
    expect(
      screen.getByText(/1,000,000 tokens \(1M\) free each month/),
    ).toBeInTheDocument();
  });
});
