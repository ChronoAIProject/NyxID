import { act, render, screen, within, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { billingSearchSchema } from "@/schemas/billing";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ApiError } from "@/lib/api-client";
import {
  billingCatalog,
  billingRow as row,
  billingUsage as usage,
  billingWallet,
} from "@/test/billing-fixture";
import { BillingPage } from "./billing";

const mocks = vi.hoisted(() => ({
  wallet: vi.fn(),
  usage: vi.fn(),
  history: vi.fn(),
  grants: vi.fn(),
  allowances: vi.fn(),
  catalog: vi.fn(),
  provision: vi.fn(),
  topup: vi.fn(),
  receipt: vi.fn(),
  openExternal: vi.fn(),
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
vi.mock("@/hooks/use-keys", () => ({ useCatalog: mocks.catalog }));
vi.mock("@/lib/navigation", () => ({ openExternal: mocks.openExternal }));
const query = (data: unknown, error: unknown = null) => ({
  data,
  error,
  isLoading: false,
  isSuccess: !error,
  isFetching: false,
  isError: Boolean(error),
  refetch: vi.fn(),
});
async function renderPage(url = "/billing?tab=usage") {
  const root = createRootRoute();
  const route = createRoute({
    getParentRoute: () => root,
    path: "/billing",
    validateSearch: (search: Record<string, unknown>) =>
      billingSearchSchema.parse(search),
    component: BillingPage,
  });
  const history = createMemoryHistory({ initialEntries: [url] });
  const router = createRouter({
    routeTree: root.addChildren([route]),
    history,
  });
  render(
    <TooltipProvider>
      <RouterProvider router={router} />
    </TooltipProvider>,
  );
  await act(() => router.load());
  await screen.findByRole("heading", { name: "Billing & Usage" });
  return { history, router };
}
async function select(label: string, option: string) {
  await userEvent.click(screen.getByRole("combobox", { name: label }));
  await userEvent.click(screen.getByRole("option", { name: option }));
}
beforeEach(() => {
  vi.resetAllMocks();
  mocks.wallet.mockReturnValue(query(billingWallet()));
  mocks.usage.mockReturnValue(query(usage()));
  mocks.history.mockReturnValue(query({ topups: [], total: 0 }));
  mocks.grants.mockReturnValue(query({ grants: [] }));
  mocks.allowances.mockReturnValue(query({ allowances: [] }));
  mocks.catalog.mockReturnValue(query(billingCatalog));
});
describe("BillingPage", () => {
  it("defaults to Billing and preserves the wallet and top-up history actions", async () => {
    await renderPage("/billing");
    expect(screen.getByRole("tab", { name: "Billing" })).toHaveAttribute(
      "data-state",
      "active",
    );
    expect(screen.getByText("95 credits")).toBeVisible();
    expect(screen.queryByText("Usage breakdown")).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "View breakdown" }),
    );
    expect(screen.getByText("100 credits")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Add credits" }));
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(screen.getAllByText("Top-up history")).toHaveLength(1);
  });
  it("preserves exact fractional funding and independent allowance quantities in disclosures", async () => {
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
    expect(
      screen.getAllByText("0.00244", { exact: false }).length,
    ).toBeGreaterThan(0);
    await userEvent.click(screen.getByText("All metrics & funding"));
    const funding = screen.getByText("Funding breakdown").parentElement!;
    expect(within(funding).getByText("0.001 credits")).toBeVisible();
    expect(within(funding).getByText("0.0012 credits")).toBeVisible();
    expect(within(funding).getByText("0.00024 credits")).toBeVisible();
    expect(within(funding).getByText("1,200")).toBeVisible();
    await userEvent.click(
      screen.getByText("Example LLM", {
        selector: ".expandable-name > strong",
      }),
    );
    await userEvent.click(screen.getByText("Models, agents & billing layers"));
    await userEvent.click(screen.getByText("Full metering & funding details"));
    expect(screen.getByText("platform_tokens")).toBeVisible();
  });
  it("offers only used services and filters every summary, detail, and funding value", async () => {
    mocks.usage.mockReturnValue(
      query(
        usage([
          row(),
          row({
            service_slug: "free-service",
            quantity: 10,
            metric: "requests",
            estimated_credits_micros: 0,
            grant_credits_micros: 0,
            billable: false,
          }),
        ]),
      ),
    );
    const { history } = await renderPage();
    expect(
      screen.getByRole("combobox", { name: "Time filter" }),
    ).toHaveTextContent("Last 30 days");
    await userEvent.click(
      screen.getByRole("combobox", { name: "Service filter" }),
    );
    expect(
      screen.queryByRole("option", { name: "Unused service" }),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("option", { name: "Free service" }));
    expect(screen.queryByText("Example LLM")).not.toBeInTheDocument();
    expect(
      screen.getByText("Free", { exact: true, selector: ".usage-status" }),
    ).toBeVisible();
    expect(history.location.search).toContain("service=free-service");
    await act(() => history.back());
    await waitFor(() =>
      expect(
        screen.getByText("Example LLM", {
          selector: ".expandable-name > strong",
        }),
      ).toBeVisible(),
    );
  });
  it("resets an unavailable service after a time change while keeping history independent", async () => {
    mocks.usage.mockImplementation((period) =>
      query(usage(period === "24h" ? [] : [row()])),
    );
    const { history } = await renderPage(
      "/billing?tab=usage&service=example-llm&period=7d",
    );
    await select("Time filter", "Last 24 hours");
    await waitFor(() =>
      expect(history.location.search).toContain("service=all"),
    );
    expect(screen.getByText("No usage in this period.")).toBeVisible();
    await userEvent.click(screen.getByRole("tab", { name: "Billing" }));
    expect(
      screen.getByRole("combobox", { name: "Top-up history period" }),
    ).toHaveTextContent("Last 30 days");
  });
  it("does not turn missing costs into zero or count free records as pending", async () => {
    mocks.usage.mockReturnValue(
      query(
        usage([
          row({ estimated_credits_micros: null }),
          row({ billable: false, lago_acked: false, grant_credits_micros: 0 }),
        ]),
      ),
    );
    await renderPage();
    expect(
      screen.getAllByText("Unavailable", { exact: false }).length,
    ).toBeGreaterThan(0);
    expect(screen.getByText("Includes free usage")).toBeVisible();
    expect(
      screen.getByText("Acknowledged", { selector: ".usage-status" }),
    ).toBeVisible();
    expect(
      screen.queryByText("Pending", { exact: true }),
    ).not.toBeInTheDocument();
  });
  it("shows usage and history errors as retryable failures instead of empty results", async () => {
    mocks.usage.mockReturnValue(query(undefined, new Error("Usage failed")));
    mocks.history.mockReturnValue(
      query(undefined, new Error("History failed")),
    );
    await renderPage();
    expect(screen.getByText("Usage failed")).toBeVisible();
    expect(
      screen.queryByText("No usage in this period."),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(mocks.usage().refetch).toHaveBeenCalled();
    await userEvent.click(screen.getByRole("tab", { name: "Billing" }));
    expect(screen.getByText("Failed to load top-up history.")).toBeVisible();
    expect(screen.queryByText("No top-ups yet.")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add credits" })).toBeDisabled();
  });
  it("keeps wallet provisioning connected to its real mutation", async () => {
    mocks.wallet.mockReturnValue(
      query(
        undefined,
        new ApiError(503, {
          error: "billing",
          error_code: 11301,
          message: "No wallet",
        }),
      ),
    );
    await renderPage("/billing");
    await userEvent.click(
      screen.getByRole("button", { name: "Provision Wallet" }),
    );
    expect(mocks.provision).toHaveBeenCalledWith({});
  });
});
