import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AllowanceForm, IssueGrantForm } from "@/schemas/billing-credits";
import type { DownstreamService } from "@/types/api";
import { useAppForm } from "@/components/ui/form";
import { AllowanceDialog, GrantDialog } from "./credits-dialogs";

const tokenService: DownstreamService = {
  id: "service-token",
  name: "Token service",
  slug: "llm-token",
  description: null,
  base_url: "https://example.com",
  service_type: "http",
  visibility: "public",
  auth_method: "bearer",
  auth_type: "bearer",
  auth_key_name: "Authorization",
  is_active: true,
  oauth_client_id: null,
  api_spec_url: null,
  service_category: "provider",
  requires_user_credential: true,
  created_by: "admin",
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  effective_platform_metric: "tokens",
  allowance_metrics: ["tokens"],
};

function AllowanceHarness({
  mixed = false,
  service,
  onSubmit = vi.fn(),
}: {
  readonly mixed?: boolean;
  readonly service?: DownstreamService;
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
        service ??
          (mixed
            ? ({
                ...tokenService,
                allowance_metrics: [
                  "requests",
                  "cache_read_tokens",
                  "images",
                  "tokens",
                ],
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
            : tokenService),
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

it.each([
  { primary: "synced", metrics: ["input_tokens", "output_tokens"] },
  { primary: "pending", metrics: ["input_tokens", "output_tokens", "bytes"] },
])(
  "uses server allowance units when the primary is $primary",
  async ({ primary, metrics }) => {
    render(
      <AllowanceHarness
        service={
          {
            ...tokenService,
            effective_platform_metric: "input_tokens",
            allowance_metrics: metrics,
            // Deliberately disagree with the server fallback to catch client heuristics.
            billing: {
              platform_metric: "requests",
              byok_pricing: {
                metric: "input_tokens",
                credits_per_unit: "1",
                sync_status: primary,
                components: [
                  {
                    metric: "output_tokens",
                    credits_per_unit: "1",
                    sync_status: "pending",
                  },
                ],
              },
            },
          } as DownstreamService
        }
      />,
    );
    await userEvent.click(screen.getByText("Token service"));
    await userEvent.click(
      screen.getByRole("combobox", { name: "Allowance unit" }),
    );
    expect(
      screen.getByRole("option", { name: "input tokens" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: "output tokens" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("option", { name: "tokens" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("option", { name: "requests" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "bytes" }) !== null).toBe(
      primary === "pending",
    );
  },
);
