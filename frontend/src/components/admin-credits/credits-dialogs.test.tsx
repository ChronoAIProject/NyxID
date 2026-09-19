import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AllowanceForm, IssueGrantForm } from "@/schemas/billing-credits";
import type { DownstreamService } from "@/types/api";
import { useAppForm } from "@/components/ui/form";
import { AllowanceDialog, GrantDialog } from "./credits-dialogs";

const tokenService = {
  id: "service-token",
  name: "Token service",
  slug: "llm-token",
  is_active: true,
  effective_platform_metric: "tokens",
} as DownstreamService;

function AllowanceHarness({
  mixed = false,
  onSubmit = vi.fn(),
}: {
  readonly mixed?: boolean;
  readonly onSubmit?: (value: AllowanceForm) => Promise<void>;
}) {
  const form = useAppForm<AllowanceForm>({
    defaultValues: {
      service_ref: "",
      quantity: 1_000_000,
      recurrence: "monthly",
      target_kind: "all_users",
      target_user_ids: [],
    },
  });

  return (
    <AllowanceDialog
      open
      onOpenChange={vi.fn()}
      form={form}
      services={[
        mixed
          ? ({
              ...tokenService,
              billing: {
                byok_pricing: {
                  metric: "requests",
                  credits_per_unit: "1",
                  components: [
                    {
                      metric: "cache_read_tokens",
                      credits_per_unit: "0.000000250001",
                    },
                    { metric: "images", credits_per_unit: "2" },
                  ],
                },
                platform_key_pricing: {
                  metric: "tokens",
                  credits_per_unit: "0.01",
                },
              },
            } as DownstreamService)
          : tokenService,
      ]}
      pending={false}
      editingAllowance={null}
      onSubmit={onSubmit}
    />
  );
}

function GrantHarness() {
  const form = useAppForm<IssueGrantForm>({
    defaultValues: {
      amount_credits: 100,
      target_kind: "all_users",
      target_user_ids: [],
      all_services: true,
      service_refs: [],
      expires_at: "",
      reason: "",
    },
  });

  return (
    <GrantDialog
      open
      onOpenChange={vi.fn()}
      form={form}
      services={[tokenService]}
      pending={false}
      onSubmit={vi.fn()}
    />
  );
}

describe("credits dialogs", () => {
  it("updates the allowance label and preview from the selected service metric", () => {
    render(<AllowanceHarness />);

    expect(screen.getByText("Free units")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Token service"));

    expect(screen.getByText("Free tokens")).toBeInTheDocument();
    expect(
      screen.getByText(/1,000,000 tokens \(1M\) free each month/),
    ).toBeInTheDocument();
  });

  it("labels grant amounts as wallet currency rather than service units", () => {
    render(<GrantHarness />);

    expect(screen.getByText("Wallet credits per owner")).toBeInTheDocument();
    expect(
      screen.getByText(/Credits are not service units/),
    ).toBeInTheDocument();
    expect(screen.getByText(/A credit is wallet currency/)).toBeInTheDocument();
  });
});

it("selects the allowance unit for mixed credential lanes", async () => {
  const onSubmit = vi.fn().mockResolvedValue(undefined);
  render(<AllowanceHarness mixed onSubmit={onSubmit} />);
  await userEvent.click(screen.getByText("Token service"));
  await userEvent.click(
    screen.getByRole("combobox", { name: "Allowance unit" }),
  );
  expect(
    screen.getByRole("option", { name: "cache-read tokens" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("option", { name: "images" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("option", { name: "requests" }));
  expect(screen.getByText("Free requests")).toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("button", { name: "Create allowance" }),
  );
  expect(onSubmit).toHaveBeenCalledWith(
    expect.objectContaining({ metric: "requests" }),
    expect.anything(),
  );
});
