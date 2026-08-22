import { fireEvent, render, screen } from "@testing-library/react";
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

function AllowanceHarness() {
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
      services={[tokenService]}
      pending={false}
      editingAllowance={null}
      onSubmit={vi.fn()}
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
