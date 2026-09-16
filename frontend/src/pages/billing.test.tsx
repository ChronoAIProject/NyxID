import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type BillingUsageResponse,
  type BillingUsageRow,
} from "@/schemas/billing";
import { TooltipProvider } from "@/components/ui/tooltip";
import { BillingPage } from "./billing";

const mocks = vi.hoisted(() => ({
  wallet: vi.fn(),
  usage: vi.fn(),
  history: vi.fn(),
  grants: vi.fn(),
  allowances: vi.fn(),
  provision: vi.fn(),
  topup: vi.fn(),
  receipt: vi.fn(),
}));
vi.mock("@/hooks/use-billing", () => ({
  useBillingWallet: mocks.wallet,
  useBillingUsage: mocks.usage,
  useTopUpHistory: mocks.history,
  useProvisionBillingWallet: () => ({ mutateAsync: mocks.provision }),
  useTopUpBilling: () => ({ mutateAsync: mocks.topup }),
  openInvoiceReceipt: mocks.receipt,
}));
vi.mock("@/hooks/use-billing-credits", () => ({
  useActiveCreditGrants: mocks.grants,
  useCurrentAllowances: mocks.allowances,
}));
vi.mock("@/lib/navigation", () => ({ openExternal: vi.fn() }));

const query = (data: unknown, error: unknown = null) => ({
  data,
  error,
  isLoading: false,
  isError: Boolean(error),
  refetch: vi.fn(),
});
function row(overrides: Partial<BillingUsageRow> = {}): BillingUsageRow {
  return {
    service_slug: "chrono-llm-public",
    metric: "tokens",
    lago_metric_code: "platform_tokens",
    layer: "platform",
    quantity: 2440,
    requests: 0,
    bytes: 0,
    events: 1,
    lago_acked: true,
    billable: true,
    estimated_credits_micros: 2440,
    wallet_credits_micros: 0,
    grant_credits_micros: 2440,
    allowance_credits_micros: 0,
    allowance_quantity: 0,
    ...overrides,
  };
}
function usage(rows: BillingUsageRow[]): BillingUsageResponse {
  return {
    owner_id: "person-1",
    period: "30d",
    rows,
    totals: {
      quantity: rows.reduce((sum, r) => sum + r.quantity, 0),
      requests: 0,
      bytes: 0,
      events: rows.length,
      estimated_credits_micros: rows.reduce(
        (sum, r) => sum + (r.estimated_credits_micros ?? 0),
        0,
      ),
      wallet_credits_micros: rows.reduce(
        (sum, r) => sum + (r.wallet_credits_micros ?? 0),
        0,
      ),
      grant_credits_micros: rows.reduce(
        (sum, r) => sum + (r.grant_credits_micros ?? 0),
        0,
      ),
      allowance_credits_micros: rows.reduce(
        (sum, r) => sum + (r.allowance_credits_micros ?? 0),
        0,
      ),
      allowance_quantity: rows.reduce(
        (sum, r) => sum + (r.allowance_quantity ?? 0),
        0,
      ),
    },
    billing: {
      charging_enabled: true,
      lago_configured: true,
      source: "usage_meter",
      rates_are_approximate: true,
    },
  };
}
async function renderPage() {
  render(
    <TooltipProvider>
      <BillingPage />
    </TooltipProvider>,
  );
  await screen.findByRole("heading", { name: "Billing" });
}
beforeEach(() => {
  vi.resetAllMocks();
  mocks.wallet.mockReturnValue(query(undefined));
  mocks.usage.mockReturnValue(query(usage([row()])));
  mocks.history.mockReturnValue(query({ topups: [], total: 0 }));
  mocks.grants.mockReturnValue(query({ grants: [] }));
  mocks.allowances.mockReturnValue(query({ allowances: [] }));
});

describe("BillingPage", () => {
  it("shows gross fractional costs and funding split consistently in totals and details", async () => {
    await renderPage();
    expect(screen.getAllByText("0.00244 credits")).toHaveLength(2);
    expect(
      screen.getByText(
        "Funded by grants 0.00244 credits · Charged to wallet 0 credits",
      ),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Expand chrono-llm-public" }),
    );
    expect(screen.getAllByText("0.00244 credits")).toHaveLength(3);
    expect(
      screen.getAllByText("grants 0.00244 credits · wallet 0 credits"),
    ).toHaveLength(2);
  });

  it("shows allowance units and exact mixed funding without wallet rounding", async () => {
    mocks.usage.mockReturnValue(
      query(
        usage([
          row({
            wallet_credits_micros: 240,
            grant_credits_micros: 1000,
            allowance_credits_micros: 1200,
            allowance_quantity: 1200,
          }),
        ]),
      ),
    );
    await renderPage();
    await userEvent.click(
      screen.getByRole("button", { name: "Expand chrono-llm-public" }),
    );
    expect(
      screen.getAllByText(
        "grants 0.001 credits · allowance 0.0012 credits (1,200 tokens) · wallet 0.00024 credits",
      ),
    ).toHaveLength(2);
    expect(
      screen.getByText(
        "Funded by grants 0.001 credits · Funded by allowances 0.0012 credits · Charged to wallet 0.00024 credits",
      ),
    ).toBeInTheDocument();
  });

  it("shows Free and no cost for uncharged usage without making charged rows Pending", async () => {
    const free = row({
      billable: false,
      lago_acked: false,
      estimated_credits_micros: 0,
      grant_credits_micros: 0,
    });
    mocks.usage.mockReturnValue(
      query(usage([row(), free, { ...free, service_slug: "free-service" }])),
    );
    await renderPage();
    expect(screen.getByText("Includes free usage")).toBeInTheDocument();
    const freeService = screen
      .getByRole("button", { name: "Expand free-service" })
      .closest("tr")!;
    expect(within(freeService).getByText("Free")).toBeInTheDocument();
    expect(within(freeService).getByText("—")).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Expand chrono-llm-public" }),
    );
    expect(screen.getAllByText("Acked")).toHaveLength(2);
    expect(screen.queryByText("Pending")).not.toBeInTheDocument();
    expect(screen.getAllByText("Free")).toHaveLength(2);
  });
});
